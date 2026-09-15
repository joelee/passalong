//! A local copy of the store's item list, so `list` and `choose` can show
//! it without connecting. `serve` keeps it current; see the architecture
//! document.

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

use crate::clock::Clock;
use crate::config::Config;
use crate::encryption::{SystemGit, load_key_file};
use crate::model::{ItemId, ItemMeta};
use crate::serve::{StoreOpener, stopped};
use crate::store::{Store, StoreError};

/// The cache file's format version. A file with another version is ignored.
pub const CACHE_VERSION: u32 = 1;

/// The cache file's name, in `serve`'s state folder.
pub const CACHE_FILE: &str = "list-cache.json";

/// How long after its last check a cache may still be used, given how often
/// `serve` checks it: two checks, so one missed check is tolerated.
pub fn max_age(check_interval: Duration) -> Duration {
    check_interval * 2
}

/// A store's items as last seen, and when they were last compared with the
/// store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCache {
    /// Format version, [`CACHE_VERSION`].
    pub version: u32,
    /// Which store this lists; see [`ListCache::store_identity`].
    pub store: String,
    /// When the list was last compared with the store.
    pub checked_at: DateTime<Utc>,
    /// The items, newest first.
    pub items: Vec<ItemMeta>,
}

impl ListCache {
    /// A cache of `items` for `store`, checked at `checked_at`.
    pub fn new(store: String, checked_at: DateTime<Utc>, mut items: Vec<ItemMeta>) -> Self {
        sort_newest_first(&mut items);
        Self {
            version: CACHE_VERSION,
            store,
            checked_at,
            items,
        }
    }

    /// Which store `config` names, for telling caches apart, or `None` for
    /// backends that are not cached: listing a `local` store needs no
    /// network.
    ///
    /// The identity also names this device's key, or `plain`, so a cache
    /// from before a migration, a rotation, or a join is never used; it is
    /// `None` when the key file cannot be read.
    pub fn store_identity(config: &Config) -> Option<String> {
        let base = match (config.server.kind.as_str(), &config.server.ssh) {
            ("ssh", Some(ssh)) => format!(
                "ssh {}@{}:{} {}",
                ssh.user, ssh.host, ssh.port, ssh.remote_path
            ),
            _ => return None,
        };
        let key = match &config.client.key_file {
            Some(path) => load_key_file(path, &SystemGit::new()).ok()?,
            None => None,
        };
        Some(Self::identity_for(&base, key.map(|key| key.key_id())))
    }

    /// The identity of the store `base` names, listed with the key `key`,
    /// or without a key.
    pub fn identity_for(base: &str, key: Option<crate::crypto::KeyId>) -> String {
        match key {
            Some(key) => format!("{base} key {key}"),
            None => format!("{base} plain"),
        }
    }

    /// `identity` without the key it names.
    fn base_of(identity: &str) -> &str {
        identity
            .strip_suffix(" plain")
            .or_else(|| identity.rsplit_once(" key ").map(|(base, _)| base))
            .unwrap_or(identity)
    }

    /// The cache in `path`, or `None` when the file is missing, unreadable,
    /// or of another format version.
    pub fn load(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let cache: Self = serde_json::from_slice(&bytes).ok()?;
        (cache.version == CACHE_VERSION).then_some(cache)
    }

    /// Writes the cache to `path`, readable by its owner only, through a
    /// temporary file and a rename, so a reader never sees half a file.
    ///
    /// # Errors
    ///
    /// When the folder or the file cannot be written.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec(self).map_err(io::Error::other)?;
        let mut temp = path.as_os_str().to_owned();
        temp.push(format!(".{}.tmp", std::process::id()));
        let temp = PathBuf::from(temp);
        let written = write_private(&temp, &json).and_then(|()| std::fs::rename(&temp, path));
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        written
    }

    /// Whether the cache lists `store` and was checked within `limit` of
    /// `now`, either way, so a clock that moved back does not keep it
    /// forever.
    pub fn usable(&self, store: &str, now: DateTime<Utc>, limit: Duration) -> bool {
        let limit = TimeDelta::from_std(limit).unwrap_or(TimeDelta::MAX);
        self.store == store && (now - self.checked_at).abs() <= limit
    }

    /// Brings the cache up to date: one listing of the ids, then the
    /// metadata of new ids only. Ids that left the store are dropped. On
    /// error the cache is left as it was.
    ///
    /// # Errors
    ///
    /// The store's error.
    pub async fn refresh(
        &mut self,
        store: &dyn Store,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let ids = store.list_ids().await?;
        let present: HashSet<&ItemId> = ids.iter().collect();
        let mut items: Vec<ItemMeta> = self
            .items
            .iter()
            .filter(|meta| present.contains(&meta.id))
            .cloned()
            .collect();
        let known: HashSet<ItemId> = items.iter().map(|meta| meta.id.clone()).collect();
        for id in ids.iter().filter(|id| !known.contains(*id)) {
            match store.get_meta(id).await {
                Ok(meta) => items.push(meta),
                // Gone or damaged since it was listed; `list` skips it too.
                Err(StoreError::NotFound(_) | StoreError::Corrupt { .. }) => {}
                Err(err) => return Err(err),
            }
        }
        sort_newest_first(&mut items);
        self.items = items;
        self.checked_at = now;
        Ok(())
    }

    /// A complete cache of `store`, read with [`Store::list`].
    ///
    /// # Errors
    ///
    /// The store's error.
    pub async fn read(
        store: &dyn Store,
        identity: String,
        now: DateTime<Utc>,
    ) -> Result<Self, StoreError> {
        Ok(Self::new(identity, now, store.list().await?))
    }

    /// Adds an item this device just stored.
    pub fn apply_put(&mut self, meta: ItemMeta) {
        if !self.items.iter().any(|item| item.id == meta.id) {
            self.items.push(meta);
            sort_newest_first(&mut self.items);
        }
    }

    /// Drops an item this device just deleted.
    pub fn apply_delete(&mut self, id: &ItemId) {
        self.items.retain(|item| &item.id != id);
    }
}

/// Keeps the cache in `path` current for `serve`: one refresh at start,
/// then one every `interval`, over a store connection of its own. A saved
/// cache of the same store is refreshed rather than read again. The cache
/// is saved under the identity of the key each connection uses, which may
/// differ from `identity`'s after a migration, a rotation, or a join; a
/// cache of another key is read again. After an error the file keeps its
/// last good contents and the store is reopened at the next check; each
/// distinct error is logged once. Returns once `shutdown` turns `true` or
/// its sender is dropped.
pub async fn refresh_loop(
    open_store: StoreOpener,
    path: PathBuf,
    identity: String,
    interval: Duration,
    clock: Arc<dyn Clock>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut cache = ListCache::load(&path).filter(|cache| cache.store == identity);
    let base = ListCache::base_of(&identity).to_owned();
    let mut store: Option<Box<dyn Store>> = None;
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_error: Option<String> = None;
    loop {
        tokio::select! {
            () = stopped(&mut shutdown) => return,
            _ = ticker.tick() => {}
        }
        let refreshed = tokio::select! {
            () = stopped(&mut shutdown) => return,
            refreshed = refresh_once(&open_store, &mut store, &mut cache, &base, clock.now()) => refreshed,
        };
        let outcome = refreshed
            .map_err(|err| format!("cannot read the store: {err}"))
            .and_then(|()| match &cache {
                Some(cache) => cache
                    .save(&path)
                    .map(|()| cache.items.len())
                    .map_err(|err| format!("cannot write {}: {err}", path.display())),
                None => Ok(0),
            });
        match outcome {
            Ok(items) => {
                last_error = None;
                tracing::debug!(items = items as u64, "list cache refreshed");
            }
            Err(message) => {
                if last_error.as_deref() != Some(message.as_str()) {
                    tracing::warn!(
                        error = message.as_str(),
                        "list cache not refreshed; retrying at the next check"
                    );
                    last_error = Some(message);
                }
            }
        }
    }
}

/// One refresh, opening the store first when there is none. The store is
/// kept only when the refresh succeeds, so a failure reconnects next time.
/// `base` is the store's identity without a key.
async fn refresh_once(
    open_store: &StoreOpener,
    store: &mut Option<Box<dyn Store>>,
    cache: &mut Option<ListCache>,
    base: &str,
    now: DateTime<Utc>,
) -> Result<(), StoreError> {
    let connected = match store.take() {
        Some(connected) => connected,
        None => open_store().await?,
    };
    let identity = ListCache::identity_for(base, connected.key_id());
    if cache.as_ref().is_some_and(|cache| cache.store != identity) {
        tracing::info!("the store's key changed; reading the list cache again");
        *cache = None;
    }
    let result = match cache.as_mut() {
        Some(cache) => cache.refresh(connected.as_ref(), now).await,
        None => ListCache::read(connected.as_ref(), identity, now)
            .await
            .map(|fresh| *cache = Some(fresh)),
    };
    if result.is_ok() {
        *store = Some(connected);
    }
    result
}

/// Ids start with their creation time, so the largest id is the newest.
fn sort_newest_first(items: &mut [ItemMeta]) {
    items.sort_by(|a, b| b.id.cmp(&a.id));
}

/// Creates or replaces `path` with `bytes`, readable by its owner only.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::fs::LocalFs;
    use crate::model::NewItem;
    use crate::random::StdRandom;
    use crate::store::{BackendFuture, FsStore};
    use crate::testing::{FaultyFs, FsOp, ManualClock, MapEnv};
    use chrono::TimeDelta;
    use std::io::Cursor;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    const T: &str = "2026-09-14T12:00:00Z";

    fn now() -> DateTime<Utc> {
        T.parse().unwrap()
    }

    fn parse(server: &str) -> Config {
        let env = MapEnv::new().with("HOME", "/home/u");
        let text = format!("[client]\ndevice_name = \"t\"\n\n{server}");
        config::parse(&text, Path::new("/c.toml"), &env).unwrap()
    }

    /// A store in a temporary folder holding `texts`, one second apart,
    /// and its items newest first.
    async fn fixture(texts: &[&str]) -> (TempDir, FsStore<FaultyFs<LocalFs>>, Vec<ItemMeta>) {
        let dir = TempDir::new().unwrap();
        let clock = Arc::new(ManualClock::at(T));
        let store = FsStore::new(
            FaultyFs::new(LocalFs::new(dir.path())),
            clock.clone(),
            Box::new(StdRandom::new()),
        );
        for text in texts {
            store
                .put(
                    NewItem::text("box"),
                    Box::new(Cursor::new(text.as_bytes().to_vec())),
                )
                .await
                .unwrap();
            clock.advance(1);
        }
        let items = store.list().await.unwrap();
        (dir, store, items)
    }

    fn ids(items: &[ItemMeta]) -> Vec<ItemId> {
        items.iter().map(|meta| meta.id.clone()).collect()
    }

    #[test]
    fn only_ssh_stores_have_an_identity() {
        let ssh = parse(
            "[server]\nkind = \"ssh\"\n\n[server.ssh]\nhost = \"nas\"\nport = 2222\nuser = \"pa\"\nhost_key = \"ssh-ed25519 AAAAkey\"\nidentity_file = \"/keys/id\"\nremote_path = \"/srv/passalong\"\n",
        );
        assert_eq!(
            ListCache::store_identity(&ssh).as_deref(),
            Some("ssh pa@nas:2222 /srv/passalong plain")
        );
        let local = parse("[server]\nkind = \"local\"\n\n[server.local]\npath = \"/srv/share\"\n");
        assert_eq!(ListCache::store_identity(&local), None);
    }

    #[test]
    fn the_identity_names_this_device_s_key() {
        let keys = TempDir::new().unwrap();
        let key_file = keys.path().join("store.key");
        let text = format!(
            "[client]\nkey_file = '{}'\n\n[server]\nkind = \"ssh\"\n\n[server.ssh]\nhost = \"nas\"\nuser = \"pa\"\nhost_key = \"ssh-ed25519 AAAAkey\"\nidentity_file = \"/keys/id\"\nremote_path = \"/srv/passalong\"\n",
            key_file.display()
        );
        let env = crate::testing::MapEnv::new().with("HOME", "/home/u");
        let config = crate::config::parse(&text, std::path::Path::new("/c.toml"), &env).unwrap();
        assert!(
            ListCache::store_identity(&config)
                .unwrap()
                .ends_with(" plain")
        );
        let key = crate::crypto::DataKey::generate().unwrap();
        crate::encryption::save_key_file(&key_file, &key, &SystemGit::new()).unwrap();
        assert_eq!(
            ListCache::store_identity(&config).unwrap(),
            format!("ssh pa@nas:22 /srv/passalong key {}", key.key_id())
        );
        std::fs::write(&key_file, "damaged").unwrap();
        assert_eq!(ListCache::store_identity(&config), None);
    }

    #[tokio::test]
    async fn saving_and_loading_round_trips_in_a_private_file() {
        let (dir, _store, items) = fixture(&["a", "b"]).await;
        let path = dir.path().join("state/list-cache.json");
        let cache = ListCache::new("ssh x".into(), now(), items);
        cache.save(&path).unwrap();
        assert_eq!(ListCache::load(&path), Some(cache));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["list-cache.json"], "no temporary file is left");
    }

    #[test]
    fn missing_invalid_or_newer_files_are_no_cache() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("list-cache.json");
        assert_eq!(ListCache::load(&path), None);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(ListCache::load(&path), None);
        let with_version = |version: u32| {
            format!(r#"{{"version":{version},"store":"s","checked_at":"{T}","items":[]}}"#)
        };
        std::fs::write(&path, with_version(CACHE_VERSION + 1)).unwrap();
        assert_eq!(ListCache::load(&path), None);
        std::fs::write(&path, with_version(CACHE_VERSION)).unwrap();
        assert!(ListCache::load(&path).is_some());
    }

    #[test]
    fn a_cache_is_usable_only_for_its_store_and_while_fresh() {
        let cache = ListCache::new("ssh a".into(), now(), Vec::new());
        let limit = max_age(Duration::from_secs(60));
        assert_eq!(limit, Duration::from_secs(120));
        assert!(cache.usable("ssh a", now() + TimeDelta::seconds(120), limit));
        assert!(!cache.usable("ssh a", now() + TimeDelta::seconds(121), limit));
        assert!(!cache.usable("ssh b", now(), limit));
        assert!(
            !cache.usable("ssh a", now() - TimeDelta::seconds(121), limit),
            "checked further in the future than the limit"
        );
    }

    #[tokio::test]
    async fn refresh_reads_one_listing_and_only_new_metadata() {
        let (_dir, store, items) = fixture(&["a", "b", "c", "d"]).await;
        // The cache misses the newest item and still has one that is gone.
        let mut cache = ListCache::new("s".into(), now(), items[1..].to_vec());
        store.delete(&items[3].id).await.unwrap();
        let (dirs, reads) = (
            store.fs().calls(FsOp::ReadDir),
            store.fs().calls(FsOp::OpenRead),
        );
        let later = now() + TimeDelta::seconds(5);
        cache.refresh(&store, later).await.unwrap();
        assert_eq!(store.fs().calls(FsOp::ReadDir) - dirs, 1);
        assert_eq!(
            store.fs().calls(FsOp::OpenRead) - reads,
            1,
            "only the new meta.json"
        );
        assert_eq!(ids(&cache.items), ids(&items[..3]));
        assert_eq!(cache.checked_at, later);
    }

    #[tokio::test]
    async fn a_running_refresh_follows_the_store_to_a_new_key() {
        use crate::crypto::{DataKey, KDF_SALT_LEN, KdfParams, Words};
        use crate::encryption::{migrate, open_with_key, rotate};
        let (dir, _store, items) = fixture(&["a", "b"]).await;
        let root = dir.path().to_path_buf();
        // This device's key file, as `serve` reads it at each reconnection.
        let device_key: Arc<std::sync::Mutex<Option<DataKey>>> =
            Arc::new(std::sync::Mutex::new(None));
        let opener: StoreOpener = {
            let (root, key) = (root.clone(), Arc::clone(&device_key));
            Arc::new(move || -> BackendFuture<'static> {
                let (root, key) = (root.clone(), key.lock().unwrap().clone());
                Box::pin(async move {
                    open_with_key(
                        LocalFs::new(root),
                        key,
                        Arc::new(ManualClock::at(T)),
                        Box::new(StdRandom::new()),
                    )
                    .await
                })
            })
        };
        let kdf = || KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [1; KDF_SALT_LEN],
        };
        let base = "ssh pa@nas:22 /srv/passalong";
        let (mut store, mut cache) = (None, None);
        refresh_once(&opener, &mut store, &mut cache, base, now())
            .await
            .unwrap();
        assert_eq!(cache.as_ref().unwrap().store, format!("{base} plain"));

        let k1 = migrate(
            &LocalFs::new(root.clone()),
            &Words::parse("abacus zoom abdomen abacus zoom abdomen").unwrap(),
            kdf(),
            Arc::new(ManualClock::at(T)),
        )
        .await
        .unwrap();
        *device_key.lock().unwrap() = Some(k1.clone());
        // The kept connection is refused; the next refresh reconnects.
        assert!(
            refresh_once(&opener, &mut store, &mut cache, base, now())
                .await
                .is_err()
        );
        refresh_once(&opener, &mut store, &mut cache, base, now())
            .await
            .unwrap();
        let saved = cache.as_ref().unwrap();
        assert_eq!(
            saved.store,
            ListCache::identity_for(base, Some(k1.key_id()))
        );
        assert_eq!(saved.items.len(), items.len());

        let k2 = rotate(
            &LocalFs::new(root.clone()),
            &k1,
            &Words::parse("zoom zoom zoom zoom zoom zoom").unwrap(),
            kdf(),
            Arc::new(ManualClock::at(T)),
        )
        .await
        .unwrap();
        *device_key.lock().unwrap() = Some(k2.clone());
        assert!(
            refresh_once(&opener, &mut store, &mut cache, base, now())
                .await
                .is_err()
        );
        refresh_once(&opener, &mut store, &mut cache, base, now())
            .await
            .unwrap();
        let saved = cache.unwrap();
        assert_eq!(
            saved.store,
            ListCache::identity_for(base, Some(k2.key_id()))
        );
        assert_eq!(saved.items.len(), items.len());
    }

    #[test]
    fn the_base_of_an_identity_drops_only_its_key() {
        for base in [
            "ssh u@h:22 /srv",
            "ssh u@h:22 /a key b",
            "ssh u@h:22 /x plain",
        ] {
            assert_eq!(ListCache::base_of(&format!("{base} plain")), base);
            let key = crate::crypto::DataKey::generate().unwrap().key_id();
            assert_eq!(
                ListCache::base_of(&ListCache::identity_for(base, Some(key))),
                base
            );
        }
    }

    #[tokio::test]
    async fn a_failed_refresh_leaves_the_cache_as_it_was() {
        let (_dir, store, items) = fixture(&["a", "b"]).await;
        let mut cache = ListCache::new("s".into(), now(), items[1..].to_vec());
        let before = cache.clone();
        store.fs().fail_next(FsOp::ReadDir, 1);
        assert!(
            cache
                .refresh(&store, now() + TimeDelta::seconds(5))
                .await
                .is_err()
        );
        assert_eq!(cache, before);
    }

    #[tokio::test]
    async fn own_sends_and_deletes_apply_without_the_store() {
        let (_dir, _store, items) = fixture(&["a", "b", "c"]).await;
        let mut cache = ListCache::new("s".into(), now(), items[1..].to_vec());
        cache.apply_put(items[0].clone());
        cache.apply_put(items[0].clone());
        assert_eq!(ids(&cache.items), ids(&items), "newest first, no duplicate");
        cache.apply_delete(&items[1].id);
        assert_eq!(
            ids(&cache.items),
            [items[0].id.clone(), items[2].id.clone()]
        );
    }

    /// Opens local stores on `root`, counting the opens.
    fn opener(root: &Path, clock: Arc<ManualClock>, opens: Arc<AtomicUsize>) -> StoreOpener {
        let root = root.to_path_buf();
        Arc::new(move || -> BackendFuture<'static> {
            opens.fetch_add(1, Ordering::SeqCst);
            let store: Box<dyn Store> = Box::new(FsStore::new(
                LocalFs::new(root.clone()),
                clock.clone(),
                Box::new(StdRandom::new()),
            ));
            Box::pin(async move { Ok(store) })
        })
    }

    #[tokio::test(start_paused = true)]
    async fn the_refresh_loop_writes_at_start_and_every_interval_and_keeps_the_file_on_error() {
        let (dir, store, items) = fixture(&["a", "b"]).await;
        let clock = Arc::new(ManualClock::at(T));
        let opens = Arc::new(AtomicUsize::new(0));
        let path = dir.path().join("state/list-cache.json");
        let every = Duration::from_secs(60);
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(refresh_loop(
            opener(dir.path(), clock.clone(), opens.clone()),
            path.clone(),
            "s plain".into(),
            every,
            clock.clone(),
            stopped,
        ));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let first = ListCache::load(&path).expect("written at start");
        assert_eq!((first.store.as_str(), &first.items), ("s plain", &items));
        assert_eq!(first.checked_at, now());

        store
            .put(NewItem::text("box"), Box::new(Cursor::new(b"c".to_vec())))
            .await
            .unwrap();
        clock.advance(60);
        tokio::time::sleep(every).await;
        let second = ListCache::load(&path).expect("written again");
        assert_eq!(second.items.len(), 3, "the new item is listed");
        assert_eq!(second.checked_at, now() + TimeDelta::seconds(60));

        // The items folder turns into a file: listing it fails.
        let items_dir = dir.path().join("items");
        std::fs::rename(&items_dir, dir.path().join("away")).unwrap();
        std::fs::write(&items_dir, b"not a folder").unwrap();
        clock.advance(60);
        tokio::time::sleep(every).await;
        assert_eq!(ListCache::load(&path).as_ref(), Some(&second), "kept");

        std::fs::remove_file(&items_dir).unwrap();
        std::fs::rename(dir.path().join("away"), &items_dir).unwrap();
        clock.advance(60);
        tokio::time::sleep(every).await;
        let recovered = ListCache::load(&path).expect("written after the error");
        assert_eq!(recovered.checked_at, now() + TimeDelta::seconds(180));
        assert_eq!(recovered.items, second.items);
        assert_eq!(opens.load(Ordering::SeqCst), 2, "reopened after the error");

        stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the loop stops")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn the_refresh_loop_replaces_a_cache_of_another_store_and_stops_with_its_sender() {
        let (dir, _store, items) = fixture(&["a"]).await;
        let clock = Arc::new(ManualClock::at(T));
        let path = dir.path().join("list-cache.json");
        ListCache::new("other".into(), now(), Vec::new())
            .save(&path)
            .unwrap();
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(refresh_loop(
            opener(dir.path(), clock.clone(), Arc::default()),
            path.clone(),
            "s plain".into(),
            Duration::from_secs(60),
            clock,
            stopped,
        ));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let cache = ListCache::load(&path).unwrap();
        assert_eq!((cache.store.as_str(), cache.items), ("s plain", items));
        drop(stop);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the loop stops")
            .unwrap();
    }

    #[tokio::test]
    async fn reading_the_store_makes_a_complete_cache() {
        let (_dir, store, items) = fixture(&["a", "b"]).await;
        let cache = ListCache::read(&store, "s".into(), now()).await.unwrap();
        assert_eq!(cache.items, items);
        assert_eq!(cache.version, CACHE_VERSION);
        assert_eq!(cache.store, "s");
    }
}
