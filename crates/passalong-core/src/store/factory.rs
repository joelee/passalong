//! Opens the storage backend named by `server.kind`.
//!
//! Backends register an opener under their kind in a [`BackendRegistry`].
//! `passalong-core` provides `local`; other crates add their own, such as
//! `passalong_ssh::register`, so the core never depends on them.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::config::Config;
use crate::encryption::{self, EncryptionAdmin, FsEncryptionAdmin};
use crate::fs::{FsError, LocalFs, RemoteFs};
use crate::store::{Store, StoreError};

/// The future a [`BackendOpener`] returns.
pub type BackendFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn Store>, StoreError>> + Send + 'a>>;

/// Opens one kind of backend from the full configuration.
pub type BackendOpener = fn(&Config) -> BackendFuture<'_>;

/// The future an [`FsOpener`] returns.
pub type FsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn RemoteFs>, StoreError>> + Send + 'a>>;

/// Opens the filesystem of one kind of file-like backend. The registry
/// builds the store on it as the store's header says
/// ([`encryption::open_store`]), so encryption works on every such backend.
pub type FsOpener = fn(&Config) -> FsFuture<'_>;

/// The backends available to this program, by `server.kind`.
///
/// A file-like backend registers an [`FsOpener`]; any other backend
/// registers a [`BackendOpener`] and opens its store itself, without
/// encryption.
#[derive(Clone, Default)]
pub struct BackendRegistry {
    openers: BTreeMap<String, BackendOpener>,
    fs_openers: BTreeMap<String, FsOpener>,
}

impl BackendRegistry {
    /// A registry with no backends.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A registry with the backends built into `passalong-core`: `local`.
    pub fn with_builtin() -> Self {
        let mut registry = Self::empty();
        registry.register_fs("local", local_fs_opener);
        registry
    }

    /// Registers `opener` for `kind`, replacing any earlier opener.
    pub fn register(&mut self, kind: &str, opener: BackendOpener) {
        self.fs_openers.remove(kind);
        self.openers.insert(kind.to_owned(), opener);
    }

    /// Registers the filesystem `opener` of a file-like backend for `kind`,
    /// replacing any earlier opener.
    pub fn register_fs(&mut self, kind: &str, opener: FsOpener) {
        self.openers.remove(kind);
        self.fs_openers.insert(kind.to_owned(), opener);
    }

    /// The registered kinds, sorted.
    pub fn kinds(&self) -> Vec<&str> {
        let mut kinds: Vec<&str> = self
            .openers
            .keys()
            .chain(self.fs_openers.keys())
            .map(String::as_str)
            .collect();
        kinds.sort_unstable();
        kinds
    }

    /// Opens the backend named by `server.kind`; a file-like one as its
    /// store header says.
    ///
    /// # Errors
    ///
    /// [`StoreError::UnsupportedBackend`] when no backend is registered for
    /// the kind, [`StoreError::Encryption`] when the store is refused, and
    /// otherwise whatever the backend's opener returns.
    pub async fn open(&self, config: &Config) -> Result<Box<dyn Store>, StoreError> {
        let kind = &config.server.kind;
        if let Some(opener) = self.fs_openers.get(kind) {
            let fs = opener(config).await?;
            return encryption::open_store(fs, config).await;
        }
        match self.openers.get(kind) {
            Some(opener) => opener(config).await,
            None => Err(StoreError::UnsupportedBackend(kind.clone())),
        }
    }

    /// Opens the filesystem of the file-like backend named by
    /// `server.kind`, for commands that change a store's layout, such as
    /// `encrypt`.
    ///
    /// # Errors
    ///
    /// [`StoreError::UnsupportedBackend`] when no file-like backend is
    /// registered for the kind, and otherwise the opener's error.
    pub async fn open_fs(&self, config: &Config) -> Result<Box<dyn RemoteFs>, StoreError> {
        match self.fs_openers.get(&config.server.kind) {
            Some(opener) => opener(config).await,
            None => Err(StoreError::UnsupportedBackend(config.server.kind.clone())),
        }
    }

    /// Changes the encryption of the store `server.kind` names: through its
    /// filesystem for a file-like backend.
    ///
    /// # Errors
    ///
    /// [`StoreError::UnsupportedBackend`] when no backend that can change
    /// encryption is registered for the kind, and otherwise the opener's
    /// error.
    pub async fn open_admin(
        &self,
        config: &Config,
    ) -> Result<Box<dyn EncryptionAdmin>, StoreError> {
        Ok(Box::new(FsEncryptionAdmin::new(
            self.open_fs(config).await?,
        )))
    }
}

impl fmt::Debug for BackendRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackendRegistry")
            .field("kinds", &self.kinds())
            .finish()
    }
}

/// Opens the store `config` names with the built-in backends only.
///
/// # Errors
///
/// As [`BackendRegistry::open`].
pub async fn open_store(config: &Config) -> Result<Box<dyn Store>, StoreError> {
    BackendRegistry::with_builtin().open(config).await
}

fn local_fs_opener(config: &Config) -> FsFuture<'_> {
    Box::pin(async move { Ok(Box::new(open_local_fs(config).await?) as Box<dyn RemoteFs>) })
}

/// The `local` backend's filesystem, creating its root if needed.
async fn open_local_fs(config: &Config) -> Result<LocalFs, StoreError> {
    let local =
        config.server.local.as_ref().ok_or_else(|| {
            StoreError::Config("the `server.local` section is missing".to_owned())
        })?;
    tokio::fs::create_dir_all(&local.path)
        .await
        .map_err(|err| FsError::from_io(local.path.display(), err))?;
    Ok(LocalFs::new(&local.path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::model::NewItem;
    use crate::store::StoreError;
    use crate::testing::MapEnv;
    use std::path::Path;
    use tempfile::TempDir;

    fn config(text: &str) -> Config {
        config::parse(
            text,
            Path::new("/c.toml"),
            &MapEnv::new().with("HOME", "/home/u"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn local_backend_creates_its_root_and_works() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("share/passalong");
        let cfg = config(&format!(
            "[server]\nkind = \"local\"\n[server.local]\npath = '{}'\n",
            root.display()
        ));
        let store = open_store(&cfg).await.unwrap();
        assert!(root.is_dir());
        let out = store
            .put(
                NewItem::text("box"),
                Box::new(std::io::Cursor::new(b"hi".to_vec())),
            )
            .await
            .unwrap();
        assert_eq!(store.list().await.unwrap(), vec![out.meta]);
    }

    #[tokio::test]
    async fn unknown_backends_are_rejected_by_name() {
        let ssh = "[server]\nkind = \"ssh\"\n[server.ssh]\nhost = \"h\"\nuser = \"u\"\nhost_key = \"k\"\nidentity_file = \"/id\"\nremote_path = \"/r\"\n";
        for (kind, text) in [("nope", "[server]\nkind = \"nope\"\n"), ("ssh", ssh)] {
            let cfg = config(text);
            match open_store(&cfg).await.err().unwrap() {
                StoreError::UnsupportedBackend(k) => assert_eq!(k, kind),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn local_backend_without_its_section_is_a_config_error() {
        let mut cfg = config("[server]\nkind = \"local\"\n[server.local]\npath = \"/x\"\n");
        cfg.server.local = None;
        let err = open_store(&cfg).await.err().unwrap();
        assert!(err.to_string().contains("server.local"), "{err}");
    }

    #[tokio::test]
    async fn unwritable_local_root_is_reported() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, b"").unwrap();
        let cfg = config(&format!(
            "[server]\nkind = \"local\"\n[server.local]\npath = '{}/sub'\n",
            file.display()
        ));
        assert!(matches!(
            open_store(&cfg).await.err().unwrap(),
            StoreError::Fs(_)
        ));
    }

    fn stub(_: &Config) -> BackendFuture<'_> {
        Box::pin(async { Err(StoreError::Backend("stub opened".into())) })
    }

    #[tokio::test]
    async fn the_builtin_registry_knows_only_local() {
        let registry = BackendRegistry::with_builtin();
        assert_eq!(registry.kinds(), ["local"]);
        assert!(format!("{registry:?}").contains("local"));
        let ssh = "[server]\nkind = \"ssh\"\n[server.ssh]\nhost = \"h\"\nuser = \"u\"\nhost_key = \"k\"\nidentity_file = \"/id\"\nremote_path = \"/r\"\n";
        match registry.open(&config(ssh)).await.err().unwrap() {
            StoreError::UnsupportedBackend(kind) => assert_eq!(kind, "ssh"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_filesystem_backend_opens_through_the_store_header() {
        let dir = TempDir::new().unwrap();
        let cfg = config(&format!(
            "[server]\nkind = \"local\"\n[server.local]\npath = '{}'\n",
            dir.path().display()
        ));
        let registry = BackendRegistry::with_builtin();
        assert!(registry.open_fs(&cfg).await.is_ok());
        std::fs::write(dir.path().join("items"), "stop").unwrap();
        assert!(matches!(
            registry.open(&cfg).await.err().unwrap(),
            StoreError::Encryption(crate::encryption::EncryptionError::HeaderMissing)
        ));
        match BackendRegistry::empty().open_fs(&cfg).await {
            Err(StoreError::UnsupportedBackend(kind)) => assert_eq!(kind, "local"),
            _ => panic!("an empty registry opened a filesystem"),
        }
    }

    fn fs_stub(_: &Config) -> FsFuture<'_> {
        Box::pin(async { Err(StoreError::Backend("fs stub opened".into())) })
    }

    #[tokio::test]
    async fn registering_a_kind_again_replaces_its_opener_of_either_sort() {
        let mut registry = BackendRegistry::empty();
        registry.register_fs("x", fs_stub);
        registry.register("x", stub);
        assert_eq!(registry.kinds(), ["x"]);
        let cfg = config("[server]\nkind = \"x\"\n");
        assert!(
            matches!(registry.open(&cfg).await.err().unwrap(), StoreError::Backend(m) if m == "stub opened")
        );
        registry.register_fs("x", fs_stub);
        assert_eq!(registry.kinds(), ["x"]);
        assert!(
            matches!(registry.open(&cfg).await.err().unwrap(), StoreError::Backend(m) if m == "fs stub opened")
        );
    }

    #[tokio::test]
    async fn registered_backends_are_dispatched_by_kind() {
        let mut registry = BackendRegistry::empty();
        assert!(registry.kinds().is_empty());
        registry.register("s3", stub);
        registry.register("s3", stub);
        assert_eq!(registry.kinds(), ["s3"]);
        match registry
            .open(&config("[server]\nkind = \"s3\"\n"))
            .await
            .err()
            .unwrap()
        {
            StoreError::Backend(message) => assert_eq!(message, "stub opened"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_file_like_backend_changes_encryption_through_its_filesystem() {
        let dir = TempDir::new().unwrap();
        let cfg = config(&format!(
            "[server]\nkind = \"local\"\n[server.local]\npath = '{}'\n",
            dir.path().display()
        ));
        let admin = BackendRegistry::with_builtin()
            .open_admin(&cfg)
            .await
            .unwrap();
        assert!(admin.fs().is_some());
        assert_eq!(
            admin.inspect().await.unwrap(),
            crate::encryption::StoreState::Plain { items: 0 }
        );
        let other = config("[server]\nkind = \"nope\"\n");
        assert!(matches!(
            BackendRegistry::with_builtin().open_admin(&other).await,
            Err(StoreError::UnsupportedBackend(kind)) if kind == "nope"
        ));
    }
}
