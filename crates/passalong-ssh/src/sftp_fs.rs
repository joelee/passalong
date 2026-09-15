//! [`RemoteFs`] over SFTP, rooted at `server.ssh.remote_path`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use passalong_core::fs::{BoxRead, BoxWrite, DirEntry, FsError, Metadata, RemoteFs, RemotePath};
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{Status, StatusCode};

use crate::connect::{SshParams, SshSession, connect};
use crate::error::SshError;

/// A [`RemoteFs`] on the server, below the resolved remote root.
pub struct SftpFs {
    sftp: SftpSession,
    root: String,
    // Keeps the SSH connection open for as long as the SFTP session lives.
    _session: SshSession,
}

impl SftpFs {
    /// Connects, starts SFTP, resolves `remote_path` against the login
    /// home, and creates the root if it does not exist.
    ///
    /// # Errors
    ///
    /// Any [`SshError`] from connecting, or [`SshError::Sftp`] when SFTP
    /// cannot start or the root cannot be created.
    pub async fn open(params: &SshParams, remote_path: &str) -> Result<Self, SshError> {
        let session = connect(params).await?;
        let address = session.address().to_owned();
        let sftp_error = |message: String| SshError::Sftp {
            address: address.clone(),
            message,
        };
        let channel = session
            .handle()
            .channel_open_session()
            .await
            .map_err(|err| sftp_error(err.to_string()))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|err| sftp_error(err.to_string()))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|err| sftp_error(err.to_string()))?;
        let home = sftp
            .canonicalize(".")
            .await
            .map_err(|err| sftp_error(err.to_string()))?;
        let root = resolve_remote_root(remote_path, &home);
        let fs = Self {
            sftp,
            root: root.clone(),
            _session: session,
        };
        fs.create_absolute_dir_all(&root)
            .await
            .map_err(|err| sftp_error(format!("cannot create the storage directory: {err}")))?;
        tracing::debug!(path = %root, "SFTP storage ready on {address}");
        Ok(fs)
    }

    /// The absolute remote root.
    pub fn root(&self) -> &str {
        &self.root
    }

    fn full(&self, path: &RemotePath) -> String {
        join_remote(&self.root, path)
    }

    async fn is_dir(&self, full: &str) -> Result<Option<bool>, SftpError> {
        match self.sftp.metadata(full).await {
            Ok(meta) => Ok(Some(meta.file_type().is_dir())),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn create_absolute_dir_all(&self, full: &str) -> Result<(), FsError> {
        let mut current = String::new();
        for part in full.split('/').filter(|part| !part.is_empty()) {
            current = format!("{current}/{part}");
            match self.is_dir(&current).await {
                Ok(Some(true)) => continue,
                Ok(Some(false)) => return Err(FsError::AlreadyExists(current)),
                Ok(None) => {}
                Err(err) => return Err(map_sftp_error(&current, err)),
            }
            if let Err(err) = self.sftp.create_dir(current.as_str()).await {
                // Another client may have created it in the meantime.
                if !matches!(self.is_dir(&current).await, Ok(Some(true))) {
                    return Err(map_sftp_error(&current, err));
                }
            }
        }
        Ok(())
    }

    /// Removes a tree depth-first without recursion: files as they are
    /// found, then directories deepest first.
    async fn remove_tree(&self, top: String) -> Result<(), FsError> {
        match self
            .is_dir(&top)
            .await
            .map_err(|err| map_sftp_error(&top, err))?
        {
            None => return Ok(()),
            Some(false) => {
                return self
                    .sftp
                    .remove_file(top.as_str())
                    .await
                    .map_err(|err| map_sftp_error(&top, err));
            }
            Some(true) => {}
        }
        let mut dirs = vec![top.clone()];
        let mut pending = vec![top];
        while let Some(dir) = pending.pop() {
            let entries = self
                .sftp
                .read_dir(dir.as_str())
                .await
                .map_err(|err| map_sftp_error(&dir, err))?;
            for entry in entries {
                let child = format!("{dir}/{}", entry.file_name());
                if entry.file_type().is_dir() {
                    dirs.push(child.clone());
                    pending.push(child);
                } else {
                    self.sftp
                        .remove_file(child.as_str())
                        .await
                        .map_err(|err| map_sftp_error(&child, err))?;
                }
            }
        }
        for dir in dirs.iter().rev() {
            self.sftp
                .remove_dir(dir.as_str())
                .await
                .map_err(|err| map_sftp_error(dir, err))?;
        }
        Ok(())
    }
}

#[async_trait]
impl RemoteFs for SftpFs {
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        self.create_absolute_dir_all(&self.full(path)).await
    }

    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        let full = self.full(path);
        match self.sftp.create_dir(full.as_str()).await {
            Ok(()) => Ok(()),
            // SFTP v3 does not say why mkdir failed; an existing path is the
            // reason a lock must tell apart.
            Err(err) => match self.sftp.try_exists(full.as_str()).await {
                Ok(true) => Err(FsError::AlreadyExists(path.to_string())),
                _ => Err(map_sftp_error(&path.to_string(), err)),
            },
        }
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        match self.sftp.remove_file(self.full(path)).await {
            Err(err) if is_not_found(&err) => Ok(()),
            other => other.map_err(|err| map_sftp_error(&path.to_string(), err)),
        }
    }

    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
        let entries = self
            .sftp
            .read_dir(self.full(path))
            .await
            .map_err(|err| map_sftp_error(&path.to_string(), err))?;
        let mut entries: Vec<DirEntry> = entries
            .map(|entry| DirEntry {
                name: entry.file_name(),
                is_dir: entry.file_type().is_dir(),
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
        let file = self
            .sftp
            .open(self.full(path))
            .await
            .map_err(|err| map_sftp_error(&path.to_string(), err))?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
        let file = self
            .sftp
            .create(self.full(path))
            .await
            .map_err(|err| map_sftp_error(&path.to_string(), err))?;
        Ok(Box::new(file))
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
        let target = self.full(to);
        // SFTP v3 rename failures do not say why; check first so an existing
        // target is reported as such, matching `LocalFs`.
        if self
            .sftp
            .try_exists(target.as_str())
            .await
            .map_err(|err| map_sftp_error(&to.to_string(), err))?
        {
            return Err(FsError::AlreadyExists(to.to_string()));
        }
        self.sftp
            .rename(self.full(from), target)
            .await
            .map_err(|err| map_sftp_error(&from.to_string(), err))
    }

    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        self.remove_tree(self.full(path)).await
    }

    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
        match self.sftp.metadata(self.full(path)).await {
            Ok(meta) => Ok(Some(Metadata {
                is_dir: meta.file_type().is_dir(),
                size: meta.len(),
                modified: meta.modified().ok().map(DateTime::<Utc>::from),
            })),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(map_sftp_error(&path.to_string(), err)),
        }
    }
}

/// The storage root: `remote_path` if absolute, otherwise below the login
/// home. Trailing slashes are dropped.
pub fn resolve_remote_root(remote_path: &str, home: &str) -> String {
    let joined = if remote_path.starts_with('/') {
        remote_path.to_owned()
    } else {
        format!("{}/{remote_path}", home.trim_end_matches('/'))
    };
    let trimmed = joined.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The absolute server path of `path` below `root`.
pub fn join_remote(root: &str, path: &RemotePath) -> String {
    match (root, path.is_root()) {
        (_, true) => root.to_owned(),
        ("/", false) => format!("/{}", path.as_str()),
        (_, false) => format!("{root}/{}", path.as_str()),
    }
}

fn is_not_found(err: &SftpError) -> bool {
    matches!(
        err,
        SftpError::Status(Status {
            status_code: StatusCode::NoSuchFile,
            ..
        })
    )
}

/// Classifies an SFTP error on `path`.
pub fn map_sftp_error(path: &str, err: SftpError) -> FsError {
    match err {
        SftpError::Status(Status {
            status_code: StatusCode::NoSuchFile,
            ..
        }) => FsError::NotFound(path.to_owned()),
        SftpError::Status(Status {
            status_code: StatusCode::PermissionDenied,
            ..
        }) => FsError::PermissionDenied(path.to_owned()),
        SftpError::Status(status) => FsError::Other {
            path: path.to_owned(),
            message: format!("{}: {}", status.status_code, status.error_message),
        },
        other => FsError::Other {
            path: path.to_owned(),
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use passalong_core::fs::{FsError, RemotePath};
    use russh_sftp::protocol::{Status, StatusCode};

    #[test]
    fn remote_root_is_absolute_or_under_the_login_home() {
        assert_eq!(
            resolve_remote_root("/srv/passalong", "/home/u"),
            "/srv/passalong"
        );
        assert_eq!(
            resolve_remote_root("/srv/passalong/", "/home/u"),
            "/srv/passalong"
        );
        assert_eq!(
            resolve_remote_root("passalong", "/home/u"),
            "/home/u/passalong"
        );
        assert_eq!(
            resolve_remote_root("data/pa/", "/home/u/"),
            "/home/u/data/pa"
        );
        assert_eq!(resolve_remote_root("/", "/home/u"), "/");
    }

    #[test]
    fn remote_paths_are_joined_below_the_root() {
        let items = RemotePath::new("items/6aa52107-2cf24dba5fb0/meta.json").unwrap();
        assert_eq!(
            join_remote("/srv/pa", &items),
            "/srv/pa/items/6aa52107-2cf24dba5fb0/meta.json"
        );
        assert_eq!(join_remote("/srv/pa", &RemotePath::root()), "/srv/pa");
        assert_eq!(
            join_remote("/", &RemotePath::new("items").unwrap()),
            "/items"
        );
    }

    fn status(code: StatusCode) -> russh_sftp::client::error::Error {
        russh_sftp::client::error::Error::Status(Status {
            id: 1,
            status_code: code,
            error_message: "server says no".into(),
            language_tag: "en".into(),
        })
    }

    #[test]
    fn sftp_errors_map_to_filesystem_errors() {
        assert_eq!(
            map_sftp_error("items/x", status(StatusCode::NoSuchFile)),
            FsError::NotFound("items/x".into())
        );
        assert_eq!(
            map_sftp_error("items/x", status(StatusCode::PermissionDenied)),
            FsError::PermissionDenied("items/x".into())
        );
        match map_sftp_error("items/x", status(StatusCode::Failure)) {
            FsError::Other { path, message } => {
                assert_eq!(path, "items/x");
                assert!(message.contains("server says no"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
        let io = russh_sftp::client::error::Error::IO("broken pipe".into());
        assert!(matches!(map_sftp_error("p", io), FsError::Other { .. }));
    }
}
