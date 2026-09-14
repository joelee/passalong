//! A local copy of the store's item list, so `list` and `choose` can show
//! it without connecting. `serve` keeps it current; see the architecture
//! document.

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::model::{ItemId, ItemMeta};
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
    pub fn store_identity(config: &Config) -> Option<String> {
        match (config.server.kind.as_str(), &config.server.ssh) {
            ("ssh", Some(ssh)) => Some(format!(
                "ssh {}@{}:{} {}",
                ssh.user, ssh.host, ssh.port, ssh.remote_path
            )),
            _ => None,
        }
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
    use crate::store::FsStore;
    use crate::testing::{FaultyFs, FsOp, ManualClock, MapEnv};
    use chrono::TimeDelta;
    use std::io::Cursor;
    use std::path::Path;
    use std::sync::Arc;
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
            Some("ssh pa@nas:2222 /srv/passalong")
        );
        let local = parse("[server]\nkind = \"local\"\n\n[server.local]\npath = \"/srv/share\"\n");
        assert_eq!(ListCache::store_identity(&local), None);
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

    #[tokio::test]
    async fn reading_the_store_makes_a_complete_cache() {
        let (_dir, store, items) = fixture(&["a", "b"]).await;
        let cache = ListCache::read(&store, "s".into(), now()).await.unwrap();
        assert_eq!(cache.items, items);
        assert_eq!(cache.version, CACHE_VERSION);
        assert_eq!(cache.store, "s");
    }
}
