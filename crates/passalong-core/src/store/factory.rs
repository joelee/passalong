//! Opens the storage backend named by `server.kind`.
//!
//! Backends register an opener under their kind in a [`BackendRegistry`].
//! `passalong-core` provides `local`; other crates add their own, such as
//! `passalong_ssh::register`, so the core never depends on them.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::clock::SystemClock;
use crate::config::Config;
use crate::fs::{FsError, LocalFs};
use crate::random::StdRandom;
use crate::store::{FsStore, Store, StoreError};

/// The future a [`BackendOpener`] returns.
pub type BackendFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn Store>, StoreError>> + Send + 'a>>;

/// Opens one kind of backend from the full configuration.
pub type BackendOpener = fn(&Config) -> BackendFuture<'_>;

/// The backends available to this program, by `server.kind`.
#[derive(Clone, Default)]
pub struct BackendRegistry {
    openers: BTreeMap<String, BackendOpener>,
}

impl BackendRegistry {
    /// A registry with no backends.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A registry with the backends built into `passalong-core`: `local`.
    pub fn with_builtin() -> Self {
        let mut registry = Self::empty();
        registry.register("local", local_opener);
        registry
    }

    /// Registers `opener` for `kind`, replacing any earlier one.
    pub fn register(&mut self, kind: &str, opener: BackendOpener) {
        self.openers.insert(kind.to_owned(), opener);
    }

    /// The registered kinds, sorted.
    pub fn kinds(&self) -> Vec<&str> {
        self.openers.keys().map(String::as_str).collect()
    }

    /// Opens the backend named by `server.kind`.
    ///
    /// # Errors
    ///
    /// [`StoreError::UnsupportedBackend`] when no backend is registered for
    /// the kind, otherwise whatever the backend's opener returns.
    pub async fn open(&self, config: &Config) -> Result<Box<dyn Store>, StoreError> {
        match self.openers.get(&config.server.kind) {
            Some(opener) => opener(config).await,
            None => Err(StoreError::UnsupportedBackend(config.server.kind.clone())),
        }
    }
}

impl fmt::Debug for BackendRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackendRegistry")
            .field("kinds", &self.kinds())
            .finish()
    }
}

/// Opens the backend named by `server.kind` using only the built-in
/// backends ([`BackendRegistry::with_builtin`]).
///
/// # Errors
///
/// As [`BackendRegistry::open`].
pub async fn open_store(config: &Config) -> Result<Box<dyn Store>, StoreError> {
    BackendRegistry::with_builtin().open(config).await
}

fn local_opener(config: &Config) -> BackendFuture<'_> {
    Box::pin(open_local(config))
}

async fn open_local(config: &Config) -> Result<Box<dyn Store>, StoreError> {
    let local =
        config.server.local.as_ref().ok_or_else(|| {
            StoreError::Config("the `server.local` section is missing".to_owned())
        })?;
    tokio::fs::create_dir_all(&local.path)
        .await
        .map_err(|err| FsError::from_io(local.path.display(), err))?;
    Ok(Box::new(FsStore::new(
        LocalFs::new(&local.path),
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
    )))
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
            "[server]\nkind = \"local\"\n[server.local]\npath = \"{}\"\n",
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
            "[server]\nkind = \"local\"\n[server.local]\npath = \"{}/sub\"\n",
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
}
