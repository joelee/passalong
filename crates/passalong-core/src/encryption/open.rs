//! Opening a store as its header says: plaintext, sealed, or refused.
//!
//! | Store | This device | Result |
//! |---|---|---|
//! | `.rewrite/` present | any | refused: being changed |
//! | a header, and `items` not a folder | key with the store's id | sealed store |
//! | a header, and `items` not a folder | no key, or another key | refused: `encrypt --join` |
//! | part of an encrypted layout: `encryption/` without a header, a header beside an `items/` folder, or a stop file without a header | any | refused: `encrypt --recover` |
//! | none of it | no key | plaintext store, as in v0.1.6 |
//! | none of it | a key | refused: the store is not encrypted |
//!
//! A sealed store opened here also checks, before every `put`, `delete`,
//! and `list_ids`, and again once `put` has published its item, that no
//! change of encryption started and that the header still names its key.
//! A plaintext store checks before the same calls that no lock and no part
//! of an encrypted layout appeared since.

use std::sync::Arc;

use tokio::io::AsyncReadExt;

use super::admin::{Layout, classify};
use super::{EncryptionError, REWRITE_DIR, SystemGit, load_key_file};
use crate::clock::{Clock, SystemClock};
use crate::config::Config;
use crate::crypto::{DataKey, Sealer};
use crate::fs::{RemoteFs, RemotePath};
use crate::random::{RandomSource, StdRandom};
use crate::store::{FsStore, Store, StoreError};

/// Opens the store on `fs` for `config`'s device, reading the device's key
/// from `client.key_file`.
///
/// # Errors
///
/// [`StoreError::Encryption`] when the key file cannot be used or the store
/// is refused (see the module table), and the filesystem's error otherwise.
pub async fn open_store<F: RemoteFs + 'static>(
    fs: F,
    config: &Config,
) -> Result<Box<dyn Store>, StoreError> {
    let key = match &config.client.key_file {
        Some(path) => load_key_file(path, &SystemGit::new()).map_err(EncryptionError::from)?,
        None => None,
    };
    open_with_key(fs, key, Arc::new(SystemClock), Box::new(StdRandom::new())).await
}

/// Opens the store on `fs` with this device's `key`, if it has one.
///
/// # Errors
///
/// As [`open_store`].
pub async fn open_with_key<F: RemoteFs + 'static>(
    fs: F,
    key: Option<DataKey>,
    clock: Arc<dyn Clock>,
    rng: Box<dyn RandomSource>,
) -> Result<Box<dyn Store>, StoreError> {
    let header = match classify(&fs).await? {
        Layout::Rewriting { started } => return Err(EncryptionError::Rewriting { started }.into()),
        Layout::Broken => return Err(EncryptionError::HeaderMissing.into()),
        Layout::Plain if key.is_some() => {
            return Err(EncryptionError::KeyWithoutEncryption.into());
        }
        Layout::Plain => return Ok(Box::new(FsStore::new(fs, clock, rng).plain_guarded())),
        Layout::Encrypted(header) => header,
    };
    let key = key.ok_or(EncryptionError::NoKey)?;
    if key.key_id() != header.key_id() {
        return Err(EncryptionError::KeyMismatch {
            device: key.key_id().short(),
            store: header.key_id().short(),
        }
        .into());
    }
    tracing::debug!(key = %header.key_id().short(), "opened an encrypted store");
    let store = FsStore::sealed(fs, clock, rng, Sealer::new(key));
    Ok(Box::new(store.guarded()))
}

/// When the re-encryption in progress started, from `.rewrite/plan.json`.
pub(super) async fn rewrite_started<F: RemoteFs + ?Sized>(fs: &F) -> Option<String> {
    let path = RemotePath::new(REWRITE_DIR).ok()?.join("plan.json").ok()?;
    let mut bytes = Vec::new();
    fs.open_read(&path)
        .await
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    let plan: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    plan.get("started_at")?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encryption::header::tests::quick_header;
    use crate::encryption::{create_header, replace_header, write_stop_file};
    use crate::fs::LocalFs;
    use crate::model::NewItem;
    use crate::testing::ManualClock;
    use tempfile::TempDir;

    struct Fixture {
        dir: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: TempDir::new().unwrap(),
            }
        }
        fn fs(&self) -> LocalFs {
            LocalFs::new(self.dir.path())
        }
        async fn open(&self, key: Option<&DataKey>) -> Result<Box<dyn Store>, StoreError> {
            open_with_key(
                self.fs(),
                key.cloned(),
                Arc::new(ManualClock::at("2026-09-15T08:00:00Z")),
                Box::new(StdRandom::new()),
            )
            .await
        }
        /// Encrypts the empty store under `key`.
        async fn encrypt(&self, key: &DataKey) {
            create_header(&self.fs(), &quick_header(key)).await.unwrap();
            write_stop_file(&self.fs()).await.unwrap();
        }
    }

    fn refusal(result: Result<Box<dyn Store>, StoreError>) -> EncryptionError {
        match result {
            Err(StoreError::Encryption(err)) => err,
            Err(other) => panic!("unexpected error {other}"),
            Ok(_) => panic!("the store opened"),
        }
    }

    fn text(bytes: &[u8]) -> crate::fs::BoxRead {
        Box::new(std::io::Cursor::new(bytes.to_vec()))
    }

    #[tokio::test]
    async fn a_plaintext_store_without_a_key_opens_as_before() {
        let fx = Fixture::new();
        let store = fx.open(None).await.unwrap();
        assert_eq!(store.key_id(), None);
        store
            .put(NewItem::text("box"), text(b"plain"))
            .await
            .unwrap();
        assert!(fx.dir.path().join("items").is_dir());
    }

    #[tokio::test]
    async fn a_key_for_a_plaintext_store_is_refused() {
        let fx = Fixture::new();
        let key = DataKey::generate().unwrap();
        let err = refusal(fx.open(Some(&key)).await);
        assert_eq!(err, EncryptionError::KeyWithoutEncryption);
        assert!(err.to_string().contains("passalong encrypt"));
    }

    #[tokio::test]
    async fn an_encrypted_store_opens_sealed_with_its_key_only() {
        let fx = Fixture::new();
        let key = DataKey::generate().unwrap();
        fx.encrypt(&key).await;

        let store = fx.open(Some(&key)).await.unwrap();
        assert_eq!(store.key_id(), Some(key.key_id()));
        store
            .put(NewItem::text("box"), text(b"sealed"))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_dir(fx.dir.path().join("v2/items"))
                .unwrap()
                .count(),
            1
        );

        let err = refusal(fx.open(None).await);
        assert_eq!(err, EncryptionError::NoKey);
        assert!(err.to_string().contains("encrypt --join"));

        let other = DataKey::generate().unwrap();
        let err = refusal(fx.open(Some(&other)).await);
        let message = err.to_string();
        assert!(matches!(err, EncryptionError::KeyMismatch { .. }));
        assert!(message.contains(&other.key_id().short()), "{message}");
        assert!(message.contains(&key.key_id().short()), "{message}");
        assert!(message.contains("encrypt --join"), "{message}");
    }

    #[tokio::test]
    async fn a_store_being_rewritten_is_refused_with_its_start_time() {
        let fx = Fixture::new();
        std::fs::create_dir(fx.dir.path().join(".rewrite")).unwrap();
        assert_eq!(
            refusal(fx.open(None).await),
            EncryptionError::Rewriting { started: None }
        );
        std::fs::write(
            fx.dir.path().join(".rewrite/plan.json"),
            r#"{"started_at":"2026-09-15T07:00:00Z"}"#,
        )
        .unwrap();
        let err = refusal(fx.open(Some(&DataKey::generate().unwrap())).await);
        assert!(
            err.to_string().contains("since 2026-09-15T07:00:00Z"),
            "{err}"
        );
        assert!(err.to_string().contains("encrypt --recover"), "{err}");
    }

    #[tokio::test]
    async fn a_stop_file_without_a_header_is_refused_never_opened_plain() {
        let fx = Fixture::new();
        write_stop_file(&fx.fs()).await.unwrap();
        for key in [None, Some(DataKey::generate().unwrap())] {
            let err = refusal(fx.open(key.as_ref()).await);
            assert_eq!(err, EncryptionError::HeaderMissing);
            assert!(err.to_string().contains("encrypt --recover"));
        }
    }

    /// What reaches a synced copy of a plaintext store, in some order, when
    /// another device encrypts it.
    #[derive(Debug, Clone, Copy)]
    enum Part {
        /// The stop file replaces the `items/` folder.
        Stop,
        /// The `encryption/` folder.
        Folder,
        /// `encryption/header.json`.
        Header,
    }

    const MARKER: &[u8] = b"plaintext-marker-5b1";

    /// Whether any file below `root` holds [`MARKER`].
    fn holds_marker(root: &std::path::Path) -> bool {
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if std::fs::read(&path)
                    .unwrap()
                    .windows(MARKER.len())
                    .any(|w| w == MARKER)
                {
                    return true;
                }
            }
        }
        false
    }

    #[tokio::test]
    async fn an_encrypted_layout_arriving_in_any_order_never_opens_as_plaintext() {
        let header = quick_header(&DataKey::generate().unwrap());
        let orders = [
            [Part::Stop, Part::Folder, Part::Header],
            [Part::Folder, Part::Stop, Part::Header],
            [Part::Folder, Part::Header, Part::Stop],
        ];
        for order in orders {
            for arrived in 1..=order.len() {
                let at = format!("{:?}", &order[..arrived]);
                let fx = Fixture::new();
                let root = fx.dir.path();
                // Opened while the store was still plaintext.
                let early = fx.open(None).await.unwrap();
                early
                    .put(NewItem::text("box"), text(b"before"))
                    .await
                    .unwrap();
                for part in &order[..arrived] {
                    match part {
                        Part::Stop => {
                            std::fs::remove_dir_all(root.join("items")).unwrap();
                            std::fs::write(root.join("items"), crate::encryption::STOP_TEXT)
                                .unwrap();
                        }
                        Part::Folder => std::fs::create_dir(root.join("encryption")).unwrap(),
                        Part::Header => {
                            std::fs::write(root.join("encryption/header.json"), header.to_json())
                                .unwrap();
                        }
                    }
                }
                let want = if arrived == order.len() {
                    EncryptionError::NoKey
                } else {
                    EncryptionError::HeaderMissing
                };
                assert_eq!(refusal(fx.open(None).await), want, "{at}");
                assert!(
                    early.put(NewItem::text("box"), text(MARKER)).await.is_err(),
                    "{at}"
                );
                assert!(!holds_marker(root), "{at}");
            }
        }
    }

    #[tokio::test]
    async fn an_opened_plaintext_store_stops_writing_once_it_is_being_encrypted() {
        let fx = Fixture::new();
        let store = fx.open(None).await.unwrap();
        let kept = store
            .put(NewItem::text("box"), text(b"one"))
            .await
            .unwrap()
            .meta;
        std::fs::create_dir(fx.dir.path().join(".rewrite")).unwrap();
        for result in [
            store
                .put(NewItem::text("box"), text(b"two"))
                .await
                .map(|_| ()),
            store.delete(&kept.id).await.map(|_| ()),
            store.list_ids().await.map(|_| ()),
        ] {
            assert!(
                matches!(
                    result,
                    Err(StoreError::Encryption(EncryptionError::Rewriting { .. }))
                ),
                "{result:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_opened_store_stops_writing_when_the_key_changes() {
        let fx = Fixture::new();
        let key = DataKey::generate().unwrap();
        fx.encrypt(&key).await;
        let store = fx.open(Some(&key)).await.unwrap();
        let kept = store
            .put(NewItem::text("box"), text(b"one"))
            .await
            .unwrap()
            .meta;

        // New words for the same key: nothing changes for this client.
        replace_header(&fx.fs(), &quick_header(&key)).await.unwrap();
        store.put(NewItem::text("box"), text(b"two")).await.unwrap();

        // A rotation: the header now names another key.
        replace_header(&fx.fs(), &quick_header(&DataKey::generate().unwrap()))
            .await
            .unwrap();
        for result in [
            store
                .put(NewItem::text("box"), text(b"three"))
                .await
                .map(|_| ()),
            store.delete(&kept.id).await.map(|_| ()),
            store.list_ids().await.map(|_| ()),
        ] {
            assert!(
                matches!(
                    result,
                    Err(StoreError::Encryption(EncryptionError::KeyChanged))
                ),
                "{result:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_opened_store_stops_writing_during_a_rewrite_or_without_a_header() {
        let fx = Fixture::new();
        let key = DataKey::generate().unwrap();
        fx.encrypt(&key).await;
        let store = fx.open(Some(&key)).await.unwrap();

        std::fs::create_dir(fx.dir.path().join(".rewrite")).unwrap();
        assert!(matches!(
            store.put(NewItem::text("box"), text(b"x")).await,
            Err(StoreError::Encryption(EncryptionError::Rewriting { .. }))
        ));
        std::fs::remove_dir(fx.dir.path().join(".rewrite")).unwrap();
        std::fs::remove_dir_all(fx.dir.path().join("encryption")).unwrap();
        assert!(matches!(
            store.put(NewItem::text("box"), text(b"x")).await,
            Err(StoreError::Encryption(EncryptionError::HeaderMissing))
        ));
    }

    #[tokio::test]
    async fn a_new_key_is_found_even_when_the_header_keeps_its_size_and_time() {
        let fx = Fixture::new();
        let key = DataKey::generate().unwrap();
        fx.encrypt(&key).await;
        let store = fx.open(Some(&key)).await.unwrap();
        let kept = store
            .put(NewItem::text("box"), text(b"one"))
            .await
            .unwrap()
            .meta;
        // Another key under the same settings: same size. SFTP gives times
        // in whole seconds, so the same time is plausible too.
        let path = fx.dir.path().join("encryption/header.json");
        let before = std::fs::metadata(&path).unwrap();
        let other = quick_header(&DataKey::generate().unwrap()).to_json();
        assert_eq!(other.len() as u64, before.len());
        std::fs::write(&path, other).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(before.modified().unwrap())
            .unwrap();
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(
            (after.len(), after.modified().unwrap()),
            (before.len(), before.modified().unwrap())
        );
        for result in [
            store
                .put(NewItem::text("box"), text(b"two"))
                .await
                .map(|_| ()),
            store.delete(&kept.id).await.map(|_| ()),
            store.list_ids().await.map(|_| ()),
        ] {
            assert!(
                matches!(
                    result,
                    Err(StoreError::Encryption(EncryptionError::KeyChanged))
                ),
                "{result:?}"
            );
        }
    }
}
