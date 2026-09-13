//! Writing items to local files: verified writes and download naming,
//! shared by `passalong load` and pull mode.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::fs::BoxRead;
use crate::model::{ContentHasher, ItemMeta};
use crate::random::{RandomSource, StdRandom};

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

/// Reads all of `content` and checks it against the item's SHA-256 and size.
///
/// # Errors
///
/// [`DownloadError::Read`] or [`DownloadError::Integrity`].
pub async fn read_verified(
    mut content: BoxRead,
    meta: &ItemMeta,
) -> Result<Vec<u8>, DownloadError> {
    let mut bytes = Vec::new();
    content
        .read_to_end(&mut bytes)
        .await
        .map_err(DownloadError::Read)?;
    let mut hasher = ContentHasher::new();
    hasher.update(&bytes);
    check_integrity(meta, hasher)?;
    Ok(bytes)
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

/// Downloads `content` into `dir` as `name`, or as its first free numbered
/// variant (`name (1)`, `name (2)`, …) when that name is taken. The content
/// is written to its own part file and verified first; only then is it
/// linked into place under a free name. The file therefore appears only
/// when it is complete, and no existing file is ever replaced, even by
/// another download finishing at the same moment.
///
/// On filesystems without hard links, such as FAT, it falls back to
/// renaming into the first name that does not exist yet, which another
/// writer could take at the same moment.
///
/// # Errors
///
/// Any [`DownloadError`]. Nothing is left in `dir` after a failure.
pub async fn download_into(
    content: BoxRead,
    meta: &ItemMeta,
    dir: &Path,
    name: &str,
) -> Result<PathBuf, DownloadError> {
    let part = create_part(dir, name).await?;
    let result = match stream_to(content, meta, &part).await {
        Ok(()) => link_to_free_name(&part, dir, name).await,
        Err(err) => Err(err),
    };
    // After a link the content lives on under the final name; after a
    // failure, or a fallback rename, this removes nothing useful.
    let _ = tokio::fs::remove_file(&part).await;
    result
}

/// An empty part file, `<name>.<random>.passalong-part`, created
/// exclusively so no other download writes into it.
async fn create_part(dir: &Path, name: &str) -> Result<PathBuf, DownloadError> {
    let mut rng = StdRandom::new();
    let mut last = dir.join(name);
    for _ in 0..16 {
        last = dir.join(format!(
            "{name}.{:08x}{PART_SUFFIX}",
            rng.next_u64() & 0xffff_ffff
        ));
        let created = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&last)
            .await;
        match created {
            Ok(_) => return Ok(last),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(DownloadError::Create { path: last, source }),
        }
    }
    Err(DownloadError::Create {
        path: last,
        source: std::io::Error::other("no unused part file name"),
    })
}

/// Links `part` to the first free candidate name. A hard link fails
/// instead of replacing an existing file, which makes the choice atomic.
async fn link_to_free_name(part: &Path, dir: &Path, name: &str) -> Result<PathBuf, DownloadError> {
    for n in 0..=MAX_NUMBERED_NAMES {
        let candidate = candidate_name(dir, name, n);
        match tokio::fs::hard_link(part, &candidate).await {
            Ok(()) => return Ok(candidate),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                return rename_to_free_name(part, dir, name).await;
            }
            Err(source) => {
                return Err(DownloadError::Write {
                    path: candidate,
                    source,
                });
            }
        }
    }
    Err(no_free_name(dir, name))
}

/// Fallback for filesystems without hard links: rename into the first
/// name that does not exist yet.
async fn rename_to_free_name(
    part: &Path,
    dir: &Path,
    name: &str,
) -> Result<PathBuf, DownloadError> {
    for n in 0..=MAX_NUMBERED_NAMES {
        let candidate = candidate_name(dir, name, n);
        if !taken(&candidate).await {
            tokio::fs::rename(part, &candidate)
                .await
                .map_err(|source| DownloadError::Write {
                    path: candidate.clone(),
                    source,
                })?;
            return Ok(candidate);
        }
    }
    Err(no_free_name(dir, name))
}

fn candidate_name(dir: &Path, name: &str, n: u32) -> PathBuf {
    if n == 0 {
        dir.join(name)
    } else {
        dir.join(numbered_name(name, n))
    }
}

fn no_free_name(dir: &Path, name: &str) -> DownloadError {
    DownloadError::NoFreeName {
        dir: dir.to_path_buf(),
        name: name.to_owned(),
        max: MAX_NUMBERED_NAMES,
    }
}

/// The first of `dir/name`, `dir/name (1)`, … `dir/name (999)` that does
/// not exist yet. A path whose existence cannot be checked counts as taken.
///
/// # Errors
///
/// [`DownloadError::NoFreeName`] when every candidate is taken.
#[deprecated(
    since = "0.1.3",
    note = "another writer can take the name before it is written; use download_into"
)]
pub async fn free_target(dir: &Path, name: &str) -> Result<PathBuf, DownloadError> {
    for n in 0..=MAX_NUMBERED_NAMES {
        let candidate = candidate_name(dir, name, n);
        if !taken(&candidate).await {
            return Ok(candidate);
        }
    }
    Err(no_free_name(dir, name))
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

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn downloads_take_the_first_free_name_and_never_replace_a_file() {
        let out = TempDir::new().unwrap();
        std::fs::write(out.path().join("f.bin"), b"mine").unwrap();
        let (_store, meta, content) = stored(b"theirs").await;
        let target = download_into(content, &meta, out.path(), "f.bin")
            .await
            .unwrap();
        assert_eq!(target, out.path().join("f (1).bin"));
        assert_eq!(std::fs::read(out.path().join("f.bin")).unwrap(), b"mine");
        assert_eq!(std::fs::read(&target).unwrap(), b"theirs");
        assert_eq!(
            names_in(out.path()),
            ["f (1).bin", "f.bin"],
            "no part file is left"
        );
    }

    #[tokio::test]
    async fn simultaneous_downloads_of_the_same_name_keep_both_files() {
        let (_one, meta_one, content_one) = stored(b"first download").await;
        let (_two, meta_two, content_two) = stored(b"second download").await;
        let out = TempDir::new().unwrap();
        let (a, b) = tokio::join!(
            download_into(content_one, &meta_one, out.path(), "f.bin"),
            download_into(content_two, &meta_two, out.path(), "f.bin")
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a, b);
        let mut contents = vec![std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()];
        contents.sort();
        assert_eq!(
            contents,
            [b"first download".to_vec(), b"second download".to_vec()]
        );
        assert_eq!(names_in(out.path()).len(), 2, "no part files are left");
    }

    /// Notes whether the final name exists while the content is still
    /// being read.
    struct Watching {
        inner: std::io::Cursor<Vec<u8>>,
        target: PathBuf,
        seen: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl tokio::io::AsyncRead for Watching {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if self.target.exists() {
                self.seen.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
        }
    }

    #[tokio::test]
    async fn a_download_appears_only_when_complete() {
        let (_store, meta, _) = stored(b"payload").await;
        let out = TempDir::new().unwrap();
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let content = Box::new(Watching {
            inner: std::io::Cursor::new(b"payload".to_vec()),
            target: out.path().join("f.bin"),
            seen: seen.clone(),
        });
        let target = download_into(content, &meta, out.path(), "f.bin")
            .await
            .unwrap();
        assert!(
            !seen.load(std::sync::atomic::Ordering::SeqCst),
            "the final name existed before the content was complete"
        );
        assert_eq!(std::fs::read(target).unwrap(), b"payload");
    }

    #[tokio::test]
    async fn a_failed_download_leaves_nothing_behind() {
        let (_store, mut meta, content) = stored(b"payload").await;
        let mut hasher = ContentHasher::new();
        hasher.update(b"other");
        meta.sha256 = hasher.finalize().sha256_hex();
        let out = TempDir::new().unwrap();
        let err = download_into(content, &meta, out.path(), "f.bin")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("integrity check failed"), "{err}");
        assert!(names_in(out.path()).is_empty());
    }

    #[tokio::test]
    async fn numbering_stops_after_the_limit() {
        let out = TempDir::new().unwrap();
        std::fs::write(out.path().join("n"), b"x").unwrap();
        for n in 1..=MAX_NUMBERED_NAMES {
            std::fs::write(out.path().join(numbered_name("n", n)), b"x").unwrap();
        }
        let (_store, meta, content) = stored(b"payload").await;
        let err = download_into(content, &meta, out.path(), "n")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("999 numbered names are taken"),
            "{err}"
        );
        assert_eq!(names_in(out.path()).len(), 1000, "no part file is left");
    }
}
