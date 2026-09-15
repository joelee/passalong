//! [`RemoteFs`] over the local filesystem: used for `server.kind = "local"`
//! (for example a mounted network share) and throughout the tests.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::fs::{BoxRead, BoxWrite, DirEntry, FsError, Metadata, RemoteFs, RemotePath};

/// A [`RemoteFs`] rooted at a local directory.
#[derive(Debug, Clone)]
pub struct LocalFs {
    root: PathBuf,
}

impl LocalFs {
    /// Serves the tree below `root`, which should already exist.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, path: &RemotePath) -> PathBuf {
        path.components()
            .fold(self.root.clone(), |acc, part| acc.join(part))
    }
}

#[async_trait]
impl RemoteFs for LocalFs {
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        tokio::fs::create_dir_all(self.resolve(path))
            .await
            .map_err(|err| FsError::from_io(path, err))
    }

    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        tokio::fs::create_dir(self.resolve(path))
            .await
            .map_err(|err| FsError::from_io(path, err))
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        match tokio::fs::remove_file(self.resolve(path)).await {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                Err(FsError::from_io(path, err))
            }
            _ => Ok(()),
        }
    }

    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
        let io = |err| FsError::from_io(path, err);
        let mut dir = tokio::fs::read_dir(self.resolve(path)).await.map_err(io)?;
        let mut entries = Vec::new();
        while let Some(entry) = dir.next_entry().await.map_err(io)? {
            // Names that are not UTF-8 cannot be item ids; skip them.
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let is_dir = entry.file_type().await.map_err(io)?.is_dir();
            entries.push(DirEntry { name, is_dir });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
        let file = tokio::fs::File::open(self.resolve(path))
            .await
            .map_err(|err| FsError::from_io(path, err))?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
        let file = tokio::fs::File::create(self.resolve(path))
            .await
            .map_err(|err| FsError::from_io(path, err))?;
        Ok(Box::new(file))
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
        let target = self.resolve(to);
        // POSIX rename replaces an empty target directory; refuse instead so
        // behaviour matches SFTP and publishing stays first-writer-wins.
        if tokio::fs::try_exists(&target)
            .await
            .map_err(|err| FsError::from_io(to, err))?
        {
            return Err(FsError::AlreadyExists(to.to_string()));
        }
        tokio::fs::rename(self.resolve(from), target)
            .await
            .map_err(|err| FsError::from_io(from, err))
    }

    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        match tokio::fs::remove_dir_all(self.resolve(path)).await {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                Err(FsError::from_io(path, err))
            }
            _ => Ok(()),
        }
    }

    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
        match tokio::fs::metadata(self.resolve(path)).await {
            Ok(meta) => Ok(Some(Metadata {
                is_dir: meta.is_dir(),
                size: meta.len(),
                modified: meta.modified().ok().map(DateTime::<Utc>::from),
            })),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FsError::from_io(path, err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{FsError, RemoteFs, RemotePath};
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn p(path: &str) -> RemotePath {
        RemotePath::new(path).unwrap()
    }

    async fn write(fs: &LocalFs, path: &str, bytes: &[u8]) {
        let mut w = fs.open_write(&p(path)).await.unwrap();
        w.write_all(bytes).await.unwrap();
        w.shutdown().await.unwrap();
    }

    async fn read(fs: &LocalFs, path: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        fs.open_read(&p(path))
            .await
            .unwrap()
            .read_to_end(&mut buf)
            .await
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn writes_and_reads_files_under_the_root() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        assert_eq!(fs.root(), dir.path());
        fs.create_dir_all(&p("a/b")).await.unwrap();
        fs.create_dir_all(&p("a/b")).await.unwrap();
        write(&fs, "a/b/f", b"hello").await;
        assert_eq!(read(&fs, "a/b/f").await, b"hello");
        assert_eq!(std::fs::read(dir.path().join("a/b/f")).unwrap(), b"hello");
        write(&fs, "a/b/f", b"hi").await;
        assert_eq!(read(&fs, "a/b/f").await, b"hi", "open_write truncates");
    }

    #[tokio::test]
    async fn lists_directories_sorted_with_kinds() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        fs.create_dir_all(&p("d/sub")).await.unwrap();
        write(&fs, "d/b.txt", b"").await;
        write(&fs, "d/a.txt", b"").await;
        let entries = fs.read_dir(&p("d")).await.unwrap();
        let listed: Vec<_> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_dir))
            .collect();
        assert_eq!(listed, [("a.txt", false), ("b.txt", false), ("sub", true)]);
        assert!(
            fs.read_dir(&RemotePath::root())
                .await
                .unwrap()
                .iter()
                .any(|e| e.name == "d")
        );
    }

    #[tokio::test]
    async fn missing_paths_are_not_found_or_none() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        assert!(fs.stat(&p("nope")).await.unwrap().is_none());
        assert!(matches!(
            fs.read_dir(&p("nope")).await.unwrap_err(),
            FsError::NotFound(_)
        ));
        assert!(matches!(
            fs.open_read(&p("nope")).await.err().unwrap(),
            FsError::NotFound(_)
        ));
        assert!(matches!(
            fs.open_write(&p("nope/f")).await.err().unwrap(),
            FsError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn stat_reports_kind_size_and_mtime() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        write(&fs, "f", b"12345").await;
        let meta = fs.stat(&p("f")).await.unwrap().unwrap();
        assert!(!meta.is_dir);
        assert_eq!(meta.size, 5);
        assert!(meta.modified.is_some());
        assert!(fs.stat(&RemotePath::root()).await.unwrap().unwrap().is_dir);
    }

    #[tokio::test]
    async fn create_dir_is_exclusive_and_remove_file_removes_files_only() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        fs.create_dir(&p("lock")).await.unwrap();
        assert!(matches!(
            fs.create_dir(&p("lock")).await,
            Err(FsError::AlreadyExists(_))
        ));
        assert!(matches!(
            fs.create_dir(&p("missing/child")).await,
            Err(FsError::NotFound(_))
        ));
        write(&fs, "lock/f", b"1").await;
        fs.remove_file(&p("lock/f")).await.unwrap();
        assert!(fs.stat(&p("lock/f")).await.unwrap().is_none());
        fs.remove_file(&p("lock/f")).await.unwrap();
        assert!(fs.remove_file(&p("lock")).await.is_err());
    }

    #[tokio::test]
    async fn rename_never_replaces_an_existing_target() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        fs.create_dir_all(&p("tmp/x")).await.unwrap();
        write(&fs, "tmp/x/content", b"1").await;
        fs.create_dir_all(&p("items")).await.unwrap();
        fs.rename(&p("tmp/x"), &p("items/x")).await.unwrap();
        assert_eq!(read(&fs, "items/x/content").await, b"1");

        fs.create_dir_all(&p("tmp/y")).await.unwrap();
        fs.create_dir_all(&p("items/empty")).await.unwrap();
        let err = fs.rename(&p("tmp/y"), &p("items/empty")).await.unwrap_err();
        assert!(matches!(err, FsError::AlreadyExists(_)), "{err:?}");
        let err = fs.rename(&p("tmp/y"), &p("items/x")).await.unwrap_err();
        assert!(matches!(err, FsError::AlreadyExists(_)), "{err:?}");
        assert!(matches!(
            fs.rename(&p("tmp/missing"), &p("items/z"))
                .await
                .unwrap_err(),
            FsError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn remove_dir_all_removes_trees_and_ignores_missing_paths() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        fs.create_dir_all(&p("t/a/b")).await.unwrap();
        write(&fs, "t/a/b/f", b"x").await;
        fs.remove_dir_all(&p("t")).await.unwrap();
        assert!(fs.stat(&p("t")).await.unwrap().is_none());
        fs.remove_dir_all(&p("t")).await.unwrap();
    }
}
