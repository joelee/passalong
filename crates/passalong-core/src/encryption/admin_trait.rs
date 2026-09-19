//! What `passalong encrypt`, `init`, `check`, `list`, and `prune` need from
//! a store's encryption, whatever holds the store.
//!
//! [`FsEncryptionAdmin`] does it on a filesystem, through this module's
//! siblings: journals, a stop file, and folders renamed into place. A store
//! behind an API implements [`EncryptionAdmin`] with calls instead, and its
//! re-encryption with [`Rewrite`], which [`run_rewrite`] drives the same way
//! for both.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::AsyncReadExt;

use super::{StoreState, admin, rewrite};
use crate::clock::Clock;
use crate::crypto::{DataKey, KdfParams, KeyId, Words};
use crate::fs::{BoxRead, RemoteFs};
use crate::model::{ContentHasher, ItemId, ItemMeta};
use crate::store::{Store, StoreError};

/// How often a long re-encryption tells its store it is still running.
/// A server's lease lasts ten minutes by default.
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(60);

/// Changing a store's encryption.
#[async_trait]
pub trait EncryptionAdmin: Send + Sync {
    /// What the store says about its encryption.
    async fn inspect(&self) -> Result<StoreState, StoreError>;

    /// Encrypts an empty plaintext store under a new key wrapped with
    /// `words`, and returns the key.
    async fn set_up(&self, words: &Words, kdf: KdfParams) -> Result<DataKey, StoreError>;

    /// Encrypts a plaintext store under a new key, setting its items aside
    /// unencrypted, and returns the key.
    async fn fresh_start(&self, words: &Words, kdf: KdfParams) -> Result<DataKey, StoreError>;

    /// The data key of an encrypted store, unwrapped with its `words`.
    async fn join(&self, words: &Words) -> Result<DataKey, StoreError>;

    /// Wraps the same key with `new` words; no item changes. Returns the
    /// key's id.
    async fn change_words(
        &self,
        current: &Words,
        new: &Words,
        kdf: KdfParams,
    ) -> Result<KeyId, StoreError>;

    /// Re-encrypts every item of a plaintext store under a new key, and
    /// returns the key.
    async fn migrate(
        &self,
        words: &Words,
        kdf: KdfParams,
        clock: Arc<dyn Clock>,
    ) -> Result<DataKey, StoreError>;

    /// Moves every item from the `old` key to a new one, and returns the
    /// new key.
    async fn rotate(
        &self,
        old: &DataKey,
        words: &Words,
        kdf: KdfParams,
        clock: Arc<dyn Clock>,
    ) -> Result<DataKey, StoreError>;

    /// The plaintext items a fresh start set aside, as a store of their own.
    fn plain_store(&self) -> Box<dyn Store + '_>;

    /// Removes the place of the plaintext items once it is empty, where the
    /// store has one; `true` when something was removed.
    async fn remove_plain_if_empty(&self) -> Result<bool, StoreError>;

    /// The filesystem, for what only a filesystem has: its journals and
    /// their recovery, the stop file, and the leftovers of v0.2.0. `None`
    /// for a store behind an API.
    fn fs(&self) -> Option<&dyn RemoteFs> {
        None
    }
}

/// A re-encryption in progress: items are read from a source under the old
/// key, or none, and stored again under the new key, then read back, then
/// the new key takes over at [`Rewrite::commit`].
#[async_trait]
pub trait Rewrite: Send + Sync {
    /// The items to re-encrypt, opened as they are now.
    fn source(&self) -> &dyn Store;

    /// The id `meta`'s item gets under the new key.
    ///
    /// # Errors
    ///
    /// When the id cannot be formed from `meta`.
    fn target_id(&self, meta: &ItemMeta) -> Result<ItemId, StoreError>;

    /// Whether the item with the new id `id` is stored already, as it is
    /// after a resumed run.
    async fn staged(&self, id: &ItemId) -> Result<bool, StoreError>;

    /// Stores `meta`'s item under the new key, keeping its metadata and
    /// creation time.
    async fn import(&self, meta: &ItemMeta, content: BoxRead) -> Result<ItemMeta, StoreError>;

    /// Reads a newly stored item back, by its new id.
    async fn read_back(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError>;

    /// Tells the store the re-encryption is still running.
    async fn heartbeat(&self) -> Result<(), StoreError>;

    /// Lets the new key take over and drops the source.
    async fn commit(&self) -> Result<(), StoreError>;
}

/// Re-encrypts every source item not yet staged, oldest first, reads every
/// new copy back and compares its SHA-256 and size, then commits. Returns
/// how many items there are. Run again after an interruption, it repeats
/// nothing already staged.
///
/// # Errors
///
/// The store's errors, and [`StoreError::Corrupt`] for a copy that does not
/// match its source; nothing is committed then.
pub async fn run_rewrite(rewrite: &dyn Rewrite) -> Result<usize, StoreError> {
    let mut alive = KeepAlive {
        last: Instant::now(),
    };
    let source = rewrite.source();
    let mut ids = source.list_ids().await?;
    ids.reverse();
    let mut metas = Vec::with_capacity(ids.len());
    for id in &ids {
        let meta = source.get_meta(id).await?;
        if !rewrite.staged(&rewrite.target_id(&meta)?).await? {
            let (meta, content) = source.get(id).await?;
            rewrite.import(&meta, content).await?;
        }
        metas.push(meta);
        alive.tick(rewrite).await?;
    }
    for meta in &metas {
        verify(rewrite, meta).await?;
        alive.tick(rewrite).await?;
    }
    rewrite.commit().await?;
    Ok(metas.len())
}

/// Sends a heartbeat once [`HEARTBEAT_EVERY`] has passed since the last.
struct KeepAlive {
    last: Instant,
}

impl KeepAlive {
    async fn tick(&mut self, rewrite: &dyn Rewrite) -> Result<(), StoreError> {
        if self.last.elapsed() >= HEARTBEAT_EVERY {
            rewrite.heartbeat().await?;
            self.last = Instant::now();
        }
        Ok(())
    }
}

/// Reads back the new copy of `meta`'s item and compares its SHA-256 and
/// size.
async fn verify(rewrite: &dyn Rewrite, meta: &ItemMeta) -> Result<(), StoreError> {
    let id = rewrite.target_id(meta)?;
    let (_, mut content) = rewrite.read_back(&id).await?;
    let mut hasher = ContentHasher::new();
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let n = content
            .read(&mut buf)
            .await
            .map_err(|err| StoreError::Content(err.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    if digest.sha256_hex() != meta.sha256 || digest.size() != meta.size {
        return Err(StoreError::Corrupt {
            id: id.to_string(),
            reason: format!("its re-encrypted copy does not match {}", meta.id),
        });
    }
    Ok(())
}

/// [`EncryptionAdmin`] on a filesystem: the `local` and `ssh` backends.
#[derive(Debug)]
pub struct FsEncryptionAdmin<F> {
    fs: F,
}

impl<F: RemoteFs> FsEncryptionAdmin<F> {
    /// Changes the encryption of the store on `fs`.
    pub fn new(fs: F) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl<F: RemoteFs> EncryptionAdmin for FsEncryptionAdmin<F> {
    async fn inspect(&self) -> Result<StoreState, StoreError> {
        admin::inspect(&self.fs).await
    }

    async fn set_up(&self, words: &Words, kdf: KdfParams) -> Result<DataKey, StoreError> {
        admin::set_up(&self.fs, words, kdf).await
    }

    async fn fresh_start(&self, words: &Words, kdf: KdfParams) -> Result<DataKey, StoreError> {
        admin::fresh_start(&self.fs, words, kdf).await
    }

    async fn join(&self, words: &Words) -> Result<DataKey, StoreError> {
        admin::join(&self.fs, words).await
    }

    async fn change_words(
        &self,
        current: &Words,
        new: &Words,
        kdf: KdfParams,
    ) -> Result<KeyId, StoreError> {
        admin::change_words(&self.fs, current, new, kdf).await
    }

    async fn migrate(
        &self,
        words: &Words,
        kdf: KdfParams,
        clock: Arc<dyn Clock>,
    ) -> Result<DataKey, StoreError> {
        rewrite::migrate(&self.fs, words, kdf, clock).await
    }

    async fn rotate(
        &self,
        old: &DataKey,
        words: &Words,
        kdf: KdfParams,
        clock: Arc<dyn Clock>,
    ) -> Result<DataKey, StoreError> {
        rewrite::rotate(&self.fs, old, words, kdf, clock).await
    }

    fn plain_store(&self) -> Box<dyn Store + '_> {
        Box::new(admin::plain_store(&self.fs))
    }

    async fn remove_plain_if_empty(&self) -> Result<bool, StoreError> {
        admin::remove_plain_if_empty(&self.fs).await
    }

    fn fs(&self) -> Option<&dyn RemoteFs> {
        Some(&self.fs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::CryptoError;
    use crate::fs::LocalFs;
    use crate::model::NewItem;
    use crate::testing::ManualClock;
    use tempfile::TempDir;

    use crate::encryption::rewrite::tests::{W1, W2, quick};

    fn words(text: &str) -> Words {
        Words::parse(text).unwrap()
    }

    fn clock() -> Arc<dyn Clock> {
        Arc::new(ManualClock::at("2026-09-19T15:00:00Z"))
    }

    async fn put(dir: &TempDir, key: Option<&DataKey>, text: &str) {
        let store = crate::encryption::open_with_key(
            LocalFs::new(dir.path()),
            key.cloned(),
            clock(),
            Box::new(crate::random::StdRandom::new()),
        )
        .await
        .unwrap();
        store
            .put(
                NewItem::text("box"),
                Box::new(std::io::Cursor::new(text.as_bytes().to_vec())),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_filesystem_admin_does_what_the_functions_do() {
        let dir = TempDir::new().unwrap();
        let admin = FsEncryptionAdmin::new(LocalFs::new(dir.path()));
        assert!(admin.fs().is_some());
        assert_eq!(
            admin.inspect().await.unwrap(),
            StoreState::Plain { items: 0 }
        );

        let key = admin.set_up(&words(W1), quick()).await.unwrap();
        assert_eq!(
            admin.inspect().await.unwrap(),
            StoreState::Encrypted {
                key_id: key.key_id(),
                plain_left: 0
            }
        );
        assert_eq!(admin.join(&words(W1)).await.unwrap().key_id(), key.key_id());
        let id = admin
            .change_words(&words(W1), &words(W2), quick())
            .await
            .unwrap();
        assert_eq!(id, key.key_id());
        assert!(matches!(
            admin.join(&words(W1)).await,
            Err(StoreError::Encryption(
                crate::encryption::EncryptionError::Crypto(CryptoError::WrongWords)
            ))
        ));

        put(&dir, Some(&key), "one").await;
        put(&dir, Some(&key), "two").await;
        let new = admin
            .rotate(&key, &words(W1), quick(), clock())
            .await
            .unwrap();
        assert_ne!(new.key_id(), key.key_id());
        let store = crate::encryption::open_with_key(
            LocalFs::new(dir.path()),
            Some(new),
            clock(),
            Box::new(crate::random::StdRandom::new()),
        )
        .await
        .unwrap();
        assert_eq!(store.list().await.unwrap().len(), 2);
        assert_eq!(admin.plain_store().list_ids().await.unwrap().len(), 0);
        assert!(!admin.remove_plain_if_empty().await.unwrap());
    }

    #[tokio::test]
    async fn a_migration_and_a_fresh_start_go_through_the_trait() {
        let migrated = TempDir::new().unwrap();
        put(&migrated, None, "keep me").await;
        let admin: Box<dyn EncryptionAdmin> =
            Box::new(FsEncryptionAdmin::new(LocalFs::new(migrated.path())));
        let key = admin.migrate(&words(W1), quick(), clock()).await.unwrap();
        assert!(matches!(
            admin.inspect().await.unwrap(),
            StoreState::Encrypted { key_id, plain_left: 0 } if key_id == key.key_id()
        ));

        let fresh = TempDir::new().unwrap();
        put(&fresh, None, "set aside").await;
        let admin = FsEncryptionAdmin::new(LocalFs::new(fresh.path()));
        admin.fresh_start(&words(W1), quick()).await.unwrap();
        assert_eq!(admin.plain_store().list_ids().await.unwrap().len(), 1);
    }

    /// A rewrite whose copies come back altered.
    struct Lying<'a> {
        inner: &'a dyn Rewrite,
    }

    #[async_trait]
    impl Rewrite for Lying<'_> {
        fn source(&self) -> &dyn Store {
            self.inner.source()
        }
        fn target_id(&self, meta: &ItemMeta) -> Result<ItemId, StoreError> {
            self.inner.target_id(meta)
        }
        async fn staged(&self, id: &ItemId) -> Result<bool, StoreError> {
            self.inner.staged(id).await
        }
        async fn import(&self, meta: &ItemMeta, content: BoxRead) -> Result<ItemMeta, StoreError> {
            self.inner.import(meta, content).await
        }
        async fn read_back(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError> {
            let (meta, _) = self.inner.read_back(id).await?;
            Ok((meta, Box::new(std::io::Cursor::new(b"altered".to_vec()))))
        }
        async fn heartbeat(&self) -> Result<(), StoreError> {
            Ok(())
        }
        async fn commit(&self) -> Result<(), StoreError> {
            panic!("a copy that does not match must not be committed")
        }
    }

    /// Re-encrypts a plaintext store's items into a sealed one, in memory
    /// terms: a source store and a target store on two folders.
    struct Copying {
        source: crate::store::FsStore<LocalFs>,
        target: crate::store::FsStore<LocalFs>,
        committed: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl Rewrite for Copying {
        fn source(&self) -> &dyn Store {
            &self.source
        }
        fn target_id(&self, meta: &ItemMeta) -> Result<ItemId, StoreError> {
            self.target.id_for(meta)
        }
        async fn staged(&self, id: &ItemId) -> Result<bool, StoreError> {
            self.target.exists(id).await
        }
        async fn import(&self, meta: &ItemMeta, content: BoxRead) -> Result<ItemMeta, StoreError> {
            self.target.import(meta, content).await
        }
        async fn read_back(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError> {
            self.target.get(id).await
        }
        async fn heartbeat(&self) -> Result<(), StoreError> {
            Ok(())
        }
        async fn commit(&self) -> Result<(), StoreError> {
            self.committed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn the_engine_copies_verifies_and_commits_only_matching_copies() {
        let from = TempDir::new().unwrap();
        let to = TempDir::new().unwrap();
        put(&from, None, "a").await;
        put(&from, None, "b").await;
        let rng = || Box::new(crate::random::StdRandom::new());
        let copying = Copying {
            source: crate::store::FsStore::new(LocalFs::new(from.path()), clock(), rng()),
            target: crate::store::FsStore::sealed(
                LocalFs::new(to.path()),
                clock(),
                rng(),
                crate::crypto::Sealer::new(DataKey::generate().unwrap()),
            ),
            committed: std::sync::atomic::AtomicBool::new(false),
        };
        assert!(matches!(
            run_rewrite(&Lying { inner: &copying }).await,
            Err(StoreError::Corrupt { reason, .. }) if reason.contains("does not match")
        ));
        assert_eq!(run_rewrite(&copying).await.unwrap(), 2);
        assert!(copying.committed.load(std::sync::atomic::Ordering::SeqCst));
        // Run again, it stores nothing twice.
        assert_eq!(run_rewrite(&copying).await.unwrap(), 2);
        assert_eq!(copying.target.list_ids().await.unwrap().len(), 2);
    }
}
