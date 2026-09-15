//! The list cache as the one-shot commands use it: `list` prints a fresh
//! one instead of connecting, and commands that connect anyway keep it
//! current. Cache problems are logged at verbose level and never fail a
//! command.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use passalong_core::cache::{self, ListCache};
use passalong_core::config::{Config, EnvProvider};
use passalong_core::crypto::KeyId;
use passalong_core::fs::BoxRead;
use passalong_core::model::{ContentDigest, ContentKey, ItemId, ItemMeta, NewItem};
use passalong_core::store::{PutOutcome, Store, StoreError, WriteProbe};

use crate::daemon::{Os, StatePaths};

/// The cache file of the configured store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheFile {
    path: PathBuf,
    identity: String,
    max_age: Duration,
}

impl CacheFile {
    /// The cache of `config`'s store, or `None` when it has none: a `local`
    /// store, `serve.list_cache` off, or no home folder.
    pub fn for_config(config: &Config, env: &dyn EnvProvider) -> Option<Self> {
        let identity = ListCache::store_identity(config).filter(|_| config.serve.list_cache)?;
        let paths = StatePaths::resolve(env, Os::current())?;
        let interval = Duration::from_secs(config.serve.list_cache_check_secs);
        Some(Self::new(paths.cache, identity, cache::max_age(interval)))
    }

    /// The cache in `path` of the store named `identity`, usable for
    /// `max_age` after each check.
    pub fn new(path: PathBuf, identity: String, max_age: Duration) -> Self {
        Self {
            path,
            identity,
            max_age,
        }
    }

    /// The saved cache when it lists this store and is recent enough to be
    /// used instead of the store.
    pub fn usable(&self, now: DateTime<Utc>) -> Option<ListCache> {
        self.load()
            .filter(|cache| cache.usable(&self.identity, now, self.max_age))
    }

    /// The store's items, read from `store`, and written to the cache. With
    /// `reuse`, a saved cache of this store, however old, is refreshed: one
    /// id listing plus the metadata of new ids. Otherwise every item's
    /// metadata is read.
    ///
    /// # Errors
    ///
    /// The store's error; writing the cache cannot fail this.
    pub async fn read(
        &self,
        store: &dyn Store,
        now: DateTime<Utc>,
        reuse: bool,
    ) -> Result<Vec<ItemMeta>, StoreError> {
        let cache = match self.load().filter(|_| reuse) {
            Some(mut cache) => {
                cache.refresh(store, now).await?;
                cache
            }
            None => ListCache::read(store, self.identity.clone(), now).await?,
        };
        self.save(&cache);
        Ok(cache.items)
    }

    /// Brings a saved cache of this store up to date, for a command that is
    /// connected anyway. Does nothing when there is none.
    pub async fn refresh(&self, store: &dyn Store, now: DateTime<Utc>) {
        let Some(mut cache) = self.load() else {
            return;
        };
        match cache.refresh(store, now).await {
            Ok(()) => self.save(&cache),
            Err(err) => tracing::debug!(error = %err, "cannot refresh the list cache"),
        }
    }

    /// Applies this device's own changes to a saved cache of this store,
    /// without asking the store. Its check time stays as it was.
    pub fn apply(&self, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        let Some(mut cache) = self.load() else {
            return;
        };
        for change in changes {
            match change {
                Change::Put(meta) => cache.apply_put(meta),
                Change::Deleted(id) => cache.apply_delete(&id),
            }
        }
        self.save(&cache);
    }

    /// The saved cache of this store, however old.
    fn load(&self) -> Option<ListCache> {
        ListCache::load(&self.path).filter(|cache| cache.store == self.identity)
    }

    fn save(&self, cache: &ListCache) {
        match cache.save(&self.path) {
            Ok(()) => tracing::debug!(
                path = %self.path.display(),
                items = cache.items.len() as u64,
                "list cache written"
            ),
            Err(err) => tracing::debug!(
                path = %self.path.display(),
                error = %err,
                "cannot write the list cache"
            ),
        }
    }
}

/// A change this device made to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// An item was stored, or was found already stored.
    Put(ItemMeta),
    /// An item was deleted.
    Deleted(ItemId),
}

/// A store that records the items put and deleted through it, so the cache
/// can follow without asking the store again.
pub struct Recording<'a> {
    inner: &'a dyn Store,
    changes: Mutex<Vec<Change>>,
}

impl<'a> Recording<'a> {
    /// Records changes made through `inner`.
    pub fn new(inner: &'a dyn Store) -> Self {
        Self {
            inner,
            changes: Mutex::default(),
        }
    }

    /// The changes, in the order they were made.
    pub fn into_changes(self) -> Vec<Change> {
        self.changes
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, change: Change) {
        self.changes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(change);
    }
}

#[async_trait]
impl Store for Recording<'_> {
    async fn put(&self, item: NewItem, content: BoxRead) -> Result<PutOutcome, StoreError> {
        let outcome = self.inner.put(item, content).await?;
        self.record(Change::Put(outcome.meta.clone()));
        Ok(outcome)
    }
    async fn list(&self) -> Result<Vec<ItemMeta>, StoreError> {
        self.inner.list().await
    }
    async fn list_after(&self, after: Option<&ItemId>) -> Result<Vec<ItemMeta>, StoreError> {
        self.inner.list_after(after).await
    }
    async fn list_ids(&self) -> Result<Vec<ItemId>, StoreError> {
        self.inner.list_ids().await
    }
    async fn get(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError> {
        self.inner.get(id).await
    }
    async fn get_meta(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
        self.inner.get_meta(id).await
    }
    async fn exists(&self, id: &ItemId) -> Result<bool, StoreError> {
        self.inner.exists(id).await
    }
    async fn find_by_content_key(&self, key: &ContentKey) -> Result<Option<ItemMeta>, StoreError> {
        self.inner.find_by_content_key(key).await
    }
    async fn resolve(&self, input: &str) -> Result<ItemId, StoreError> {
        self.inner.resolve(input).await
    }
    async fn delete(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
        let meta = self.inner.delete(id).await?;
        self.record(Change::Deleted(id.clone()));
        Ok(meta)
    }
    async fn clean_staging(&self, older_than: Duration) -> Result<usize, StoreError> {
        self.inner.clean_staging(older_than).await
    }
    async fn probe_write(&self) -> Result<WriteProbe, StoreError> {
        self.inner.probe_write().await
    }
    fn key_id(&self) -> Option<KeyId> {
        self.inner.key_id()
    }
    fn content_key(&self, digest: &ContentDigest) -> ContentKey {
        self.inner.content_key(digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{T, TestStore, bytes};
    use chrono::TimeDelta;
    use tempfile::TempDir;

    const MINUTE: Duration = Duration::from_secs(60);

    fn now() -> DateTime<Utc> {
        T.parse().unwrap()
    }

    fn file(dir: &TempDir) -> CacheFile {
        CacheFile::new(dir.path().join("state/list-cache.json"), "s".into(), MINUTE)
    }

    async fn put(ts: &TestStore, text: &str) -> ItemMeta {
        let meta = ts
            .store
            .put(NewItem::text("box"), bytes(text.as_bytes()))
            .await
            .unwrap()
            .meta;
        ts.clock.advance(1);
        meta
    }

    fn saved(file: &CacheFile) -> Option<ListCache> {
        ListCache::load(&file.path)
    }

    #[test]
    fn only_a_recent_cache_of_this_store_is_usable() {
        let dir = TempDir::new().unwrap();
        let file = file(&dir);
        assert_eq!(file.usable(now()), None, "no file");
        ListCache::new("other".into(), now(), Vec::new())
            .save(&file.path)
            .unwrap();
        assert_eq!(file.usable(now()), None, "another store");
        ListCache::new("s".into(), now(), Vec::new())
            .save(&file.path)
            .unwrap();
        assert!(file.usable(now() + TimeDelta::seconds(60)).is_some());
        assert_eq!(file.usable(now() + TimeDelta::seconds(61)), None, "old");
    }

    #[tokio::test]
    async fn reading_refreshes_a_saved_cache_or_reads_everything() {
        let (dir, ts) = (TempDir::new().unwrap(), TestStore::new());
        let first = put(&ts, "a").await;
        let file = file(&dir);
        // A saved cache whose metadata is reused: its device name differs.
        let mut stale = first.clone();
        stale.device = "cached".into();
        ListCache::new("s".into(), now(), vec![stale.clone()])
            .save(&file.path)
            .unwrap();
        let second = put(&ts, "b").await;
        let later = now() + TimeDelta::hours(1);

        let items = file.read(&ts.store, later, true).await.unwrap();
        assert_eq!(items, [second.clone(), stale]);
        assert_eq!(saved(&file).unwrap().checked_at, later);

        let items = file.read(&ts.store, later, false).await.unwrap();
        assert_eq!(items, [second, first], "every item read again");
        assert_eq!(saved(&file).unwrap().items, items);
    }

    #[tokio::test]
    async fn commands_that_connect_refresh_only_a_saved_cache() {
        let (dir, ts) = (TempDir::new().unwrap(), TestStore::new());
        let first = put(&ts, "a").await;
        let file = file(&dir);
        file.refresh(&ts.store, now()).await;
        assert_eq!(saved(&file), None, "nothing to refresh");

        ListCache::new("s".into(), now(), Vec::new())
            .save(&file.path)
            .unwrap();
        let later = now() + TimeDelta::seconds(5);
        file.refresh(&ts.store, later).await;
        let cache = saved(&file).unwrap();
        assert_eq!((cache.items, cache.checked_at), (vec![first], later));
    }

    #[tokio::test]
    async fn recorded_puts_and_deletes_apply_to_a_saved_cache_only() {
        let (dir, ts) = (TempDir::new().unwrap(), TestStore::new());
        let kept = put(&ts, "kept").await;
        let file = file(&dir);
        let recording = Recording::new(&ts.store);
        let added = recording
            .put(NewItem::text("box"), bytes(b"added"))
            .await
            .unwrap()
            .meta;
        recording.delete(&kept.id).await.unwrap();
        assert!(recording.delete(&kept.id).await.is_err());
        let changes = recording.into_changes();
        assert_eq!(
            changes,
            [Change::Put(added.clone()), Change::Deleted(kept.id.clone())],
            "a failed delete is not recorded"
        );

        file.apply(changes.clone());
        assert_eq!(saved(&file), None, "no cache is started");

        ListCache::new("s".into(), now(), vec![kept])
            .save(&file.path)
            .unwrap();
        file.apply(changes);
        let cache = saved(&file).unwrap();
        assert_eq!((cache.items, cache.checked_at), (vec![added], now()));
    }

    #[tokio::test]
    async fn recording_passes_every_read_through() {
        let ts = TestStore::new();
        let meta = put(&ts, "a").await;
        let recording = Recording::new(&ts.store);
        let only = std::slice::from_ref(&meta);
        assert_eq!(recording.list().await.unwrap(), only);
        assert_eq!(recording.list_after(None).await.unwrap(), only);
        assert_eq!(
            recording.list_ids().await.unwrap(),
            std::slice::from_ref(&meta.id)
        );
        assert_eq!(recording.get(&meta.id).await.unwrap().0, meta);
        assert_eq!(recording.get_meta(&meta.id).await.unwrap(), meta);
        assert!(recording.exists(&meta.id).await.unwrap());
        let key = ContentKey::parse(&meta.sha256[..12]).unwrap();
        assert_eq!(
            recording.find_by_content_key(&key).await.unwrap(),
            Some(meta.clone())
        );
        assert_eq!(recording.resolve(meta.id.as_str()).await.unwrap(), meta.id);
        assert_eq!(recording.clean_staging(MINUTE).await.unwrap(), 0);
        assert_eq!(
            recording.probe_write().await.unwrap(),
            ts.store.probe_write().await.unwrap()
        );
        assert_eq!(recording.into_changes(), []);
    }

    #[tokio::test]
    async fn a_cache_that_cannot_be_written_fails_nothing() {
        let (dir, ts) = (TempDir::new().unwrap(), TestStore::new());
        let meta = put(&ts, "a").await;
        // The cache's folder is a file.
        std::fs::write(dir.path().join("state"), b"").unwrap();
        let file = file(&dir);
        assert_eq!(
            file.read(&ts.store, now(), true).await.unwrap(),
            std::slice::from_ref(&meta)
        );
        file.refresh(&ts.store, now()).await;
        file.apply(vec![Change::Deleted(meta.id)]);
    }
}
