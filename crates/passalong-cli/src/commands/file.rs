//! `passalong file`: send a file.

use std::io::Write;
use std::path::Path;

use anyhow::Context as _;
use passalong_core::model::NewItem;
use passalong_core::store::Store;

/// Streams the file at `path` into the store and prints the item's id.
pub async fn run(
    store: &dyn Store,
    path: &Path,
    device: &str,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let shown = path.display();
    let metadata = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("cannot read {shown}"))?;
    if metadata.is_dir() {
        anyhow::bail!("{shown} is a directory; only files can be sent");
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .with_context(|| format!("{shown} has no file name"))?;
    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("cannot read {shown}"))?;
    let outcome = store
        .put(NewItem::file(name, device), Box::new(file))
        .await?;
    writeln!(out, "{}", outcome.meta.id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::TestStore;
    use passalong_core::model::{ContentHasher, ItemKind};
    use tempfile::TempDir;

    #[tokio::test]
    async fn sends_a_file_with_its_name_type_and_size() {
        let ts = TestStore::new();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"some notes").unwrap();
        let mut out = Vec::new();
        run(&ts.store, &path, "box", &mut out).await.unwrap();
        let items = ts.store.list().await.unwrap();
        let meta = &items[0];
        assert_eq!(String::from_utf8(out).unwrap(), format!("{}\n", meta.id));
        assert_eq!(meta.kind, ItemKind::File);
        assert_eq!(meta.name.as_deref(), Some("notes.txt"));
        assert_eq!(meta.mime, "text/plain");
        assert_eq!(meta.size, 10);
    }

    #[tokio::test]
    async fn large_files_keep_their_exact_digest() {
        let ts = TestStore::new();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("big.bin");
        let data: Vec<u8> = (0..5 * 1024 * 1024_u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8)
            .collect();
        std::fs::write(&path, &data).unwrap();
        run(&ts.store, &path, "box", &mut Vec::new()).await.unwrap();
        let mut hasher = ContentHasher::new();
        hasher.update(&data);
        let meta = &ts.store.list().await.unwrap()[0];
        assert_eq!(meta.sha256, hasher.finalize().sha256_hex());
        assert_eq!(meta.size, data.len() as u64);
    }

    #[tokio::test]
    async fn rejects_directories_and_missing_paths_by_name() {
        let ts = TestStore::new();
        let dir = TempDir::new().unwrap();
        let err = run(&ts.store, dir.path(), "box", &mut Vec::new())
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains(&dir.path().display().to_string()) && msg.contains("directory"),
            "{msg}"
        );
        let missing = dir.path().join("nope.txt");
        let err = run(&ts.store, &missing, "box", &mut Vec::new())
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("nope.txt"), "{err:#}");
        assert!(ts.store.list().await.unwrap().is_empty());
    }
}
