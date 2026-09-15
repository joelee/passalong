//! Minimal filesystem surface shared by every file-based backend.
//!
//! [`RemoteFs`] is the small set of operations the storage layout needs. It
//! is implemented over the local disk by [`LocalFs`] and over SFTP by the
//! `passalong-ssh` crate. Paths are [`RemotePath`]s: relative to the
//! backend's root and built only from validated components, so no caller
//! can reach outside the root.

pub mod local;

pub use local::LocalFs;

use std::fmt;
use std::io;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::io::{AsyncRead, AsyncWrite};

/// A boxed content stream being read.
pub type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
/// A boxed content stream being written. Callers must `shutdown` it to
/// flush and close.
pub type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// Path relative to a backend's root, made of validated components joined
/// by `/`. The empty path is the root itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RemotePath(String);

impl RemotePath {
    /// The backend's root directory.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Parses a relative `/`-separated path, validating every component as
    /// [`RemotePath::join`] does. The empty string is the root.
    ///
    /// # Errors
    ///
    /// [`FsError::InvalidPath`] for absolute paths, empty components, `.`,
    /// `..`, backslashes, or NUL bytes.
    pub fn new(path: &str) -> Result<Self, FsError> {
        if path.is_empty() {
            return Ok(Self::root());
        }
        path.split('/')
            .try_fold(Self::root(), |acc, part| acc.join(part))
    }

    /// Appends one component.
    ///
    /// # Errors
    ///
    /// [`FsError::InvalidPath`] when `component` is empty, `.` or `..`, or
    /// contains `/`, `\\`, or NUL.
    pub fn join(&self, component: &str) -> Result<Self, FsError> {
        let invalid = component.is_empty()
            || component == "."
            || component == ".."
            || component.contains(['/', '\\', '\0']);
        if invalid {
            return Err(FsError::InvalidPath(component.escape_debug().to_string()));
        }
        Ok(Self(if self.0.is_empty() {
            component.to_owned()
        } else {
            format!("{}/{component}", self.0)
        }))
    }

    /// Whether this is the root.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The path as `/`-separated text; empty for the root.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The path's components, outermost first.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|part| !part.is_empty())
    }

    /// The last component, or `None` for the root.
    pub fn file_name(&self) -> Option<&str> {
        self.components().last()
    }

    /// The containing path, or `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rsplit_once('/') {
            Some((parent, _)) => Self(parent.to_owned()),
            None => Self::root(),
        })
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.is_root() { "." } else { &self.0 })
    }
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// The entry's name within its directory.
    pub name: String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// What [`RemoteFs::stat`] reports about a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// Whether the path is a directory.
    pub is_dir: bool,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time, when the backend reports one.
    pub modified: Option<DateTime<Utc>>,
}

/// Filesystem errors, independent of the backend.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FsError {
    /// The path does not exist.
    #[error("{0}: not found")]
    NotFound(String),
    /// The target of a create or rename already exists.
    #[error("{0}: already exists")]
    AlreadyExists(String),
    /// The backend refused access.
    #[error("{0}: permission denied")]
    PermissionDenied(String),
    /// A path component failed validation.
    #[error("invalid path component `{0}`")]
    InvalidPath(String),
    /// Any other failure.
    #[error("{path}: {message}")]
    Other {
        /// The path being operated on.
        path: String,
        /// Description of the failure.
        message: String,
    },
}

impl FsError {
    /// Classifies an I/O error on `path`.
    pub fn from_io(path: impl fmt::Display, err: io::Error) -> Self {
        let path = path.to_string();
        match err.kind() {
            io::ErrorKind::NotFound => Self::NotFound(path),
            io::ErrorKind::AlreadyExists => Self::AlreadyExists(path),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(path),
            _ => Self::Other {
                path,
                message: err.to_string(),
            },
        }
    }
}

/// The filesystem operations the storage layout needs.
///
/// Implementations must be rooted: every [`RemotePath`] resolves inside the
/// backend's root.
#[async_trait]
pub trait RemoteFs: Send + Sync {
    /// Creates a directory and any missing parents; succeeds if it exists.
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError>;

    /// Lists a directory, sorted by name.
    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError>;

    /// Opens a file for reading.
    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError>;

    /// Creates or truncates a file for writing. The parent must exist.
    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError>;

    /// Renames `from` to `to`. Fails with [`FsError::AlreadyExists`] rather
    /// than replacing an existing `to`, which is what makes publishing an
    /// item atomic and first-writer-wins.
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError>;

    /// Removes a directory tree; succeeds if it does not exist.
    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError>;

    /// Returns metadata, or `None` if the path does not exist.
    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError>;

    /// Creates one directory, whose parent must exist, and fails with
    /// [`FsError::AlreadyExists`] when the path exists. Creating a directory
    /// exclusively is how clients take a lock, so backends override this
    /// default, which checks first and is therefore not atomic.
    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        if self.stat(path).await?.is_some() {
            return Err(FsError::AlreadyExists(path.to_string()));
        }
        self.create_dir_all(path).await
    }

    /// Removes one file; succeeds if it does not exist. This default reports
    /// that the backend cannot.
    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        Err(FsError::Other {
            path: path.to_string(),
            message: "this backend cannot remove a single file".to_owned(),
        })
    }
}

/// A borrowed filesystem is a filesystem, so a store can be built on one its
/// owner keeps using.
#[async_trait]
impl<T: RemoteFs + ?Sized> RemoteFs for &T {
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).create_dir_all(path).await
    }
    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
        (**self).read_dir(path).await
    }
    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
        (**self).open_read(path).await
    }
    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
        (**self).open_write(path).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
        (**self).rename(from, to).await
    }
    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).remove_dir_all(path).await
    }
    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
        (**self).stat(path).await
    }
    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).create_dir(path).await
    }
    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).remove_file(path).await
    }
}

/// A [`RemoteFs`] rooted at `prefix` inside another, such as the `plain/`
/// folder where an encrypted store keeps the plaintext items it had before.
#[derive(Debug, Clone)]
pub struct SubFs<F> {
    inner: F,
    prefix: RemotePath,
}

impl<F> SubFs<F> {
    /// Serves the tree below `prefix` in `inner`.
    pub fn new(inner: F, prefix: RemotePath) -> Self {
        Self { inner, prefix }
    }

    fn under(&self, path: &RemotePath) -> Result<RemotePath, FsError> {
        path.components()
            .try_fold(self.prefix.clone(), |acc, part| acc.join(part))
    }
}

#[async_trait]
impl<F: RemoteFs> RemoteFs for SubFs<F> {
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        self.inner.create_dir_all(&self.under(path)?).await
    }
    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
        self.inner.read_dir(&self.under(path)?).await
    }
    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
        self.inner.open_read(&self.under(path)?).await
    }
    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
        self.inner.open_write(&self.under(path)?).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
        self.inner
            .rename(&self.under(from)?, &self.under(to)?)
            .await
    }
    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        self.inner.remove_dir_all(&self.under(path)?).await
    }
    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
        self.inner.stat(&self.under(path)?).await
    }
    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        self.inner.create_dir(&self.under(path)?).await
    }
    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        self.inner.remove_file(&self.under(path)?).await
    }
}

/// A boxed filesystem, as a backend registry opens one, is a filesystem.
#[async_trait]
impl<T: RemoteFs + ?Sized> RemoteFs for Box<T> {
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).create_dir_all(path).await
    }
    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
        (**self).read_dir(path).await
    }
    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
        (**self).open_read(path).await
    }
    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
        (**self).open_write(path).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
        (**self).rename(from, to).await
    }
    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).remove_dir_all(path).await
    }
    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
        (**self).stat(path).await
    }
    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).create_dir(path).await
    }
    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).remove_file(path).await
    }
}

/// A shared filesystem is a filesystem, so a store can be built on one
/// that other code keeps using.
#[async_trait]
impl<T: RemoteFs + ?Sized> RemoteFs for std::sync::Arc<T> {
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).create_dir_all(path).await
    }
    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
        (**self).read_dir(path).await
    }
    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
        (**self).open_read(path).await
    }
    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
        (**self).open_write(path).await
    }
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
        (**self).rename(from, to).await
    }
    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).remove_dir_all(path).await
    }
    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
        (**self).stat(path).await
    }
    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).create_dir(path).await
    }
    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        (**self).remove_file(path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FaultyFs, FsOp};
    use tempfile::TempDir;

    #[test]
    fn remote_paths_join_validated_components() {
        let root = RemotePath::root();
        assert!(root.is_root());
        assert_eq!(root.as_str(), "");
        let file = root
            .join("items")
            .unwrap()
            .join("6aa52107-2cf24dba5fb0")
            .unwrap()
            .join("meta.json")
            .unwrap();
        assert_eq!(file.as_str(), "items/6aa52107-2cf24dba5fb0/meta.json");
        assert_eq!(file.to_string(), "items/6aa52107-2cf24dba5fb0/meta.json");
        assert_eq!(file.file_name(), Some("meta.json"));
        assert_eq!(
            file.parent().unwrap().as_str(),
            "items/6aa52107-2cf24dba5fb0"
        );
        assert_eq!(
            file.components().collect::<Vec<_>>(),
            ["items", "6aa52107-2cf24dba5fb0", "meta.json"]
        );
        assert_eq!(RemotePath::root().file_name(), None);
        assert!(RemotePath::root().parent().is_none());
        assert_eq!(root.to_string(), ".");
    }

    #[test]
    fn remote_paths_reject_traversal_and_separators() {
        for bad in ["", ".", "..", "a/b", "a\\b", "nul\0byte"] {
            let err = RemotePath::root().join(bad).unwrap_err();
            assert!(matches!(err, FsError::InvalidPath(_)), "{bad:?}: {err:?}");
        }
        assert_eq!(
            RemotePath::new("items/x/content").unwrap().as_str(),
            "items/x/content"
        );
        assert!(RemotePath::new("").unwrap().is_root());
        for bad in ["../x", "items/../../etc", "/abs", "a//b", "a/"] {
            assert!(RemotePath::new(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn io_errors_map_to_fs_errors_by_kind() {
        use std::io::{Error, ErrorKind};
        assert!(
            matches!(FsError::from_io("p", Error::from(ErrorKind::NotFound)), FsError::NotFound(p) if p == "p")
        );
        assert!(matches!(
            FsError::from_io("p", Error::from(ErrorKind::AlreadyExists)),
            FsError::AlreadyExists(_)
        ));
        assert!(matches!(
            FsError::from_io("p", Error::from(ErrorKind::PermissionDenied)),
            FsError::PermissionDenied(_)
        ));
        let other = FsError::from_io("p", Error::other("boom"));
        assert_eq!(other.to_string(), "p: boom");
    }

    #[test]
    fn remote_fs_is_object_safe() {
        let dir = TempDir::new().unwrap();
        let fs: Box<dyn RemoteFs> = Box::new(LocalFs::new(dir.path()));
        drop(fs);
    }

    /// Implements only the required methods, to exercise the defaults.
    struct Basic(crate::fs::LocalFs);

    #[async_trait]
    impl RemoteFs for Basic {
        async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
            self.0.create_dir_all(path).await
        }
        async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
            self.0.read_dir(path).await
        }
        async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
            self.0.open_read(path).await
        }
        async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
            self.0.open_write(path).await
        }
        async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
            self.0.rename(from, to).await
        }
        async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
            self.0.remove_dir_all(path).await
        }
        async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
            self.0.stat(path).await
        }
    }

    #[tokio::test]
    async fn the_default_create_dir_refuses_an_existing_path_and_remove_file_is_unsupported() {
        let dir = tempfile::TempDir::new().unwrap();
        let fs = Basic(crate::fs::LocalFs::new(dir.path()));
        let lock = RemotePath::new("lock").unwrap();
        fs.create_dir(&lock).await.unwrap();
        assert!(matches!(
            fs.create_dir(&lock).await,
            Err(FsError::AlreadyExists(_))
        ));
        assert!(matches!(
            fs.remove_file(&lock).await,
            Err(FsError::Other { .. })
        ));
    }

    #[tokio::test]
    async fn a_sub_filesystem_stays_below_its_prefix() {
        let dir = tempfile::TempDir::new().unwrap();
        let local = crate::fs::LocalFs::new(dir.path());
        let sub = SubFs::new(&local, RemotePath::new("plain").unwrap());
        let p = |s: &str| RemotePath::new(s).unwrap();
        sub.create_dir_all(&p("items/a")).await.unwrap();
        sub.create_dir(&p("items/b")).await.unwrap();
        let mut w = sub.open_write(&p("items/a/f")).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut w, b"x")
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::shutdown(&mut w).await.unwrap();
        assert!(dir.path().join("plain/items/a/f").is_file());
        assert_eq!(sub.read_dir(&p("items")).await.unwrap().len(), 2);
        let mut r = sub.open_read(&p("items/a/f")).await.unwrap();
        let mut got = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut r, &mut got)
            .await
            .unwrap();
        assert_eq!(got, b"x");
        sub.rename(&p("items/b"), &p("items/c")).await.unwrap();
        assert!(sub.stat(&p("items/c")).await.unwrap().unwrap().is_dir);
        sub.remove_file(&p("items/a/f")).await.unwrap();
        sub.remove_dir_all(&p("items")).await.unwrap();
        assert!(!dir.path().join("plain/items").exists());
    }

    #[tokio::test]
    async fn a_boxed_filesystem_forwards_every_call() {
        let dir = tempfile::TempDir::new().unwrap();
        let fs: Box<dyn RemoteFs> = Box::new(crate::fs::LocalFs::new(dir.path()));
        let p = |s: &str| RemotePath::new(s).unwrap();
        fs.create_dir_all(&p("a/b")).await.unwrap();
        fs.create_dir(&p("a/c")).await.unwrap();
        let mut w = fs.open_write(&p("a/f")).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut w, b"x")
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::shutdown(&mut w).await.unwrap();
        let mut r = fs.open_read(&p("a/f")).await.unwrap();
        let mut got = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut r, &mut got)
            .await
            .unwrap();
        assert_eq!(got, b"x");
        assert_eq!(fs.read_dir(&p("a")).await.unwrap().len(), 3);
        fs.rename(&p("a/c"), &p("a/d")).await.unwrap();
        assert!(fs.stat(&p("a/d")).await.unwrap().is_some());
        fs.remove_file(&p("a/f")).await.unwrap();
        fs.remove_dir_all(&p("a")).await.unwrap();
        assert!(fs.stat(&p("a")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn faulty_fs_fails_the_configured_calls_only() {
        let dir = TempDir::new().unwrap();
        let fs = FaultyFs::new(LocalFs::new(dir.path()));
        let a = RemotePath::new("a").unwrap();
        let b = RemotePath::new("b").unwrap();
        fs.create_dir_all(&a).await.unwrap();
        fs.fail_next(FsOp::Rename, 1);
        let err = fs.rename(&a, &b).await.unwrap_err();
        assert!(err.to_string().contains("injected"), "{err}");
        fs.rename(&a, &b).await.unwrap();
        assert_eq!(fs.calls(FsOp::Rename), 2);

        fs.fail_nth(FsOp::Stat, 2);
        assert!(fs.stat(&b).await.unwrap().is_some());
        assert!(fs.stat(&b).await.is_err());
        assert!(fs.stat(&b).await.is_ok());
        assert_eq!(fs.calls(FsOp::Stat), 3);
        assert_eq!(fs.calls(FsOp::ReadDir), 0);
    }

    #[tokio::test]
    async fn faulty_fs_passes_every_operation_through() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = TempDir::new().unwrap();
        let fs = FaultyFs::new(LocalFs::new(dir.path()));
        let d = RemotePath::new("d").unwrap();
        let f = d.join("f").unwrap();
        fs.create_dir_all(&d).await.unwrap();
        let mut w = fs.open_write(&f).await.unwrap();
        w.write_all(b"x").await.unwrap();
        w.shutdown().await.unwrap();
        let mut buf = Vec::new();
        fs.open_read(&f)
            .await
            .unwrap()
            .read_to_end(&mut buf)
            .await
            .unwrap();
        assert_eq!(buf, b"x");
        assert_eq!(fs.read_dir(&d).await.unwrap().len(), 1);
        fs.remove_dir_all(&d).await.unwrap();
        assert!(fs.inner().root().exists());
        for op in [
            FsOp::CreateDirAll,
            FsOp::ReadDir,
            FsOp::OpenRead,
            FsOp::OpenWrite,
            FsOp::RemoveDirAll,
        ] {
            fs.fail_next(op, 1);
        }
        assert!(fs.create_dir_all(&d).await.is_err());
        assert!(fs.read_dir(&d).await.is_err());
        assert!(fs.open_read(&f).await.is_err());
        assert!(fs.open_write(&f).await.is_err());
        assert!(fs.remove_dir_all(&d).await.is_err());
    }
}
