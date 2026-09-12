//! Opens the storage backend named by `server.kind`.

use std::sync::Arc;

use crate::clock::SystemClock;
use crate::config::Config;
use crate::fs::{FsError, LocalFs};
use crate::random::StdRandom;
use crate::store::{FsStore, Store, StoreError};

/// Opens the backend named by `server.kind`.
///
/// # Errors
///
/// [`StoreError::UnsupportedBackend`] for kinds this build does not
/// provide, [`StoreError::Config`] when the backend's section is missing,
/// and [`StoreError::Fs`] when a local root cannot be created.
pub async fn open_store(config: &Config) -> Result<Box<dyn Store>, StoreError> {
    match config.server.kind.as_str() {
        "local" => {
            let local = config.server.local.as_ref().ok_or_else(|| {
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
        other => Err(StoreError::UnsupportedBackend(other.to_owned())),
    }
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
}
