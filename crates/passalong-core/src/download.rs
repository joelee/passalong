//! Writing items to local files: verified writes and download naming,
//! shared by `passalong load` and pull mode.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::fs::BoxRead;
use crate::model::{ContentHasher, ItemMeta};

/// Suffix of the temporary file an item is written to before it is
/// verified and renamed into place.
pub const PART_SUFFIX: &str = ".passalong-part";
/// How many numbered alternatives, `name (1)` to `name (999)`, are tried.
pub const MAX_NUMBERED_NAMES: u32 = 999;
const CHUNK_SIZE: usize = 64 * 1024;

/// Failures while writing an item to a local file.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// Reading the item's content failed.
    #[error("reading the item")]
    Read(#[source] std::io::Error),
    /// A file or directory could not be created.
    #[error("cannot create {}", .path.display())]
    Create {
        /// What could not be created.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// Writing or renaming a file failed.
    #[error("cannot write {}", .path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// The content does not match the item's SHA-256 or size.
    #[error(
        "integrity check failed for {id}: expected sha256 {expected} ({expected_size} bytes), got {actual} ({actual_size} bytes)"
    )]
    Integrity {
        /// The item's id.
        id: String,
        /// SHA-256 recorded in `meta.json`.
        expected: String,
        /// Size recorded in `meta.json`.
        expected_size: u64,
        /// SHA-256 of the content received.
        actual: String,
        /// Size of the content received.
        actual_size: u64,
    },
    /// Every numbered alternative of a name is taken.
    #[error("no free name for {name} in {}: {max} numbered names are taken", .dir.display())]
    NoFreeName {
        /// The directory.
        dir: PathBuf,
        /// The item's file name.
        name: String,
        /// [`MAX_NUMBERED_NAMES`].
        max: u32,
    },
}

/// Checks the hashed content against the item's SHA-256 and size.
///
/// # Errors
///
/// [`DownloadError::Integrity`] on a mismatch.
pub fn check_integrity(meta: &ItemMeta, hasher: ContentHasher) -> Result<(), DownloadError> {
    let digest = hasher.finalize();
    let actual = digest.sha256_hex();
    if actual != meta.sha256 || digest.size() != meta.size {
        return Err(DownloadError::Integrity {
            id: meta.id.to_string(),
            expected: meta.sha256.clone(),
            expected_size: meta.size,
            actual,
            actual_size: digest.size(),
        });
    }
    Ok(())
}

/// Streams `content` into `<target>.passalong-part`, verifies it, then
/// renames it over `target`. The part file is removed on any failure, so a
/// damaged download never replaces anything.
///
/// # Errors
///
/// Any [`DownloadError`] except `NoFreeName`.
pub async fn write_verified(
    content: BoxRead,
    meta: &ItemMeta,
    target: &Path,
) -> Result<(), DownloadError> {
    let mut part = target.as_os_str().to_owned();
    part.push(PART_SUFFIX);
    let part = PathBuf::from(part);
    let result = match stream_to(content, meta, &part).await {
        Ok(()) => tokio::fs::rename(&part, target)
            .await
            .map_err(|source| DownloadError::Write {
                path: target.to_path_buf(),
                source,
            }),
        Err(err) => Err(err),
    };
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result
}

async fn stream_to(
    mut content: BoxRead,
    meta: &ItemMeta,
    part: &Path,
) -> Result<(), DownloadError> {
    let write_error = |source| DownloadError::Write {
        path: part.to_path_buf(),
        source,
    };
    let mut file = tokio::fs::File::create(part)
        .await
        .map_err(|source| DownloadError::Create {
            path: part.to_path_buf(),
            source,
        })?;
    let mut hasher = ContentHasher::new();
    let mut buf = vec![0_u8; CHUNK_SIZE];
    loop {
        let n = content.read(&mut buf).await.map_err(DownloadError::Read)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).await.map_err(write_error)?;
    }
    file.flush().await.map_err(write_error)?;
    drop(file);
    check_integrity(meta, hasher)
}

/// `name` with ` (n)` before its extension, as browsers name repeated
/// downloads: `report.pdf` becomes `report (1).pdf`. A leading dot, as in
/// `.bashrc`, or a trailing dot is not treated as an extension.
pub fn numbered_name(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => {
            format!("{} ({n}){}", &name[..dot], &name[dot..])
        }
        _ => format!("{name} ({n})"),
    }
}

/// The first of `dir/name`, `dir/name (1)`, … `dir/name (999)` that does
/// not exist yet. A path whose existence cannot be checked counts as taken.
///
/// # Errors
///
/// [`DownloadError::NoFreeName`] when every candidate is taken.
pub async fn free_target(dir: &Path, name: &str) -> Result<PathBuf, DownloadError> {
    let plain = dir.join(name);
    if !taken(&plain).await {
        return Ok(plain);
    }
    for n in 1..=MAX_NUMBERED_NAMES {
        let candidate = dir.join(numbered_name(name, n));
        if !taken(&candidate).await {
            return Ok(candidate);
        }
    }
    Err(DownloadError::NoFreeName {
        dir: dir.to_path_buf(),
        name: name.to_owned(),
        max: MAX_NUMBERED_NAMES,
    })
}

async fn taken(path: &Path) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContentHasher, ItemMeta, NewItem};
    use crate::store::Store as _;
    use crate::testing::FixedClock;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn numbered_names_go_before_the_extension() {
        assert_eq!(numbered_name("report.pdf", 1), "report (1).pdf");
        assert_eq!(numbered_name("archive.tar.gz", 2), "archive.tar (2).gz");
        assert_eq!(numbered_name("README", 3), "README (3)");
        assert_eq!(numbered_name(".bashrc", 1), ".bashrc (1)");
        assert_eq!(numbered_name("trailing.", 1), "trailing. (1)");
    }

    #[tokio::test]
    async fn free_names_skip_existing_files() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            free_target(dir.path(), "a.txt").await.unwrap(),
            dir.path().join("a.txt")
        );
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("a (1).txt"), b"x").unwrap();
        assert_eq!(
            free_target(dir.path(), "a.txt").await.unwrap(),
            dir.path().join("a (2).txt")
        );
    }

    #[tokio::test]
    async fn numbering_stops_after_the_limit() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("n"), b"x").unwrap();
        for n in 1..=MAX_NUMBERED_NAMES {
            std::fs::write(dir.path().join(numbered_name("n", n)), b"x").unwrap();
        }
        let err = free_target(dir.path(), "n").await.unwrap_err();
        assert!(
            err.to_string().contains("999 numbered names are taken"),
            "{err}"
        );
    }

    async fn stored(data: &[u8]) -> (TempDir, ItemMeta, crate::fs::BoxRead) {
        let dir = TempDir::new().unwrap();
        let store = crate::store::FsStore::new(
            crate::fs::LocalFs::new(dir.path()),
            Arc::new(FixedClock::at("2026-09-13T08:00:00Z")),
            Box::new(crate::random::StdRandom::new()),
        );
        let meta = store
            .put(
                NewItem::file("f.bin", "box"),
                Box::new(std::io::Cursor::new(data.to_vec())),
            )
            .await
            .unwrap()
            .meta;
        let (meta, content) = store.get(&meta.id).await.unwrap();
        (dir, meta, content)
    }

    #[tokio::test]
    async fn verified_writes_replace_the_target_only_when_the_content_matches() {
        let (_store, meta, content) = stored(b"payload").await;
        let out = TempDir::new().unwrap();
        let target = out.path().join("f.bin");
        write_verified(content, &meta, &target).await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"payload");

        let (_store, mut meta, content) = stored(b"payload").await;
        let mut hasher = ContentHasher::new();
        hasher.update(b"other");
        meta.sha256 = hasher.finalize().sha256_hex();
        std::fs::write(&target, b"keep me").unwrap();
        let err = write_verified(content, &meta, &target).await.unwrap_err();
        assert!(err.to_string().contains("integrity check failed"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
        let names: Vec<_> = std::fs::read_dir(out.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["f.bin"], "no part file is left behind");
    }
}
