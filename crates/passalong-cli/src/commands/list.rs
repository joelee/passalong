//! `passalong list`: show stored items, newest first, from the list cache
//! when it is fresh.

use std::io::Write;

use chrono::{DateTime, FixedOffset, Utc};
use passalong_core::store::BackendFuture;

use crate::list_cache::CacheFile;
use crate::output;

/// Lists the items as a table, or as JSON when `json` is set. A usable
/// `cache` is printed without calling `open`; otherwise, or with `nocache`,
/// the store is read and the cache written.
pub async fn run<'a>(
    open: impl FnOnce() -> BackendFuture<'a>,
    cache: Option<&CacheFile>,
    nocache: bool,
    now: DateTime<Utc>,
    json: bool,
    offset: FixedOffset,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let items = match cache
        .filter(|_| !nocache)
        .and_then(|cache| cache.usable(now))
    {
        Some(cached) => {
            tracing::debug!(
                age_secs = (now - cached.checked_at).num_seconds(),
                "listing from the list cache"
            );
            cached.items
        }
        None => {
            let store = open().await?;
            match cache {
                Some(cache) => cache.read(store.as_ref(), now, !nocache).await?,
                None => store.list().await?,
            }
        }
    };
    tracing::debug!("listed {} items", items.len());
    let text = if json {
        output::render_json(&items)?
    } else {
        output::render_table(&items, offset)
    };
    out.write_all(text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{T, TestStore, bytes};
    use chrono::{FixedOffset, TimeDelta};
    use passalong_core::cache::ListCache;
    use passalong_core::fs::LocalFs;
    use passalong_core::model::{ItemMeta, NewItem};
    use passalong_core::random::StdRandom;
    use passalong_core::store::{FsStore, Store};
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::TempDir;

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        T.parse().unwrap()
    }

    /// Runs `list` on `ts`'s store, returning its output and whether it
    /// opened the store.
    async fn list(
        ts: &TestStore,
        cache: Option<&CacheFile>,
        nocache: bool,
        json: bool,
    ) -> (String, bool) {
        let opened = Cell::new(false);
        let open = || -> BackendFuture<'static> {
            opened.set(true);
            let store: Box<dyn Store> = Box::new(FsStore::new(
                LocalFs::new(ts.dir.path()),
                ts.clock.clone(),
                Box::new(StdRandom::new()),
            ));
            Box::pin(async move { Ok(store) })
        };
        let mut out = Vec::new();
        run(open, cache, nocache, now(), json, utc(), &mut out)
            .await
            .unwrap();
        (String::from_utf8(out).unwrap(), opened.get())
    }

    /// A cache file in `dir` for the store named `s`, usable for 2 minutes.
    fn cache_file(dir: &Path) -> (PathBuf, CacheFile) {
        let path = dir.join("list-cache.json");
        let file = CacheFile::new(path.clone(), "s".into(), Duration::from_secs(120));
        (path, file)
    }

    #[tokio::test]
    async fn prints_a_table_newest_first() {
        let ts = TestStore::new();
        let old = ts
            .store
            .put(NewItem::text("box"), bytes(b"hello"))
            .await
            .unwrap()
            .meta;
        ts.clock.advance(60);
        let new = ts
            .store
            .put(NewItem::file("a.txt", "box"), bytes(b"file body"))
            .await
            .unwrap()
            .meta;
        let (out, opened) = list(&ts, None, false, false).await;
        assert!(opened);
        let ids: Vec<_> = out
            .lines()
            .skip(1)
            .map(|l| l.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(ids, [new.id.as_str(), old.id.as_str()]);
        assert!(out.starts_with("ID "), "{out}");
    }

    #[tokio::test]
    async fn prints_json_when_asked() {
        let ts = TestStore::new();
        ts.store
            .put(NewItem::text("box"), bytes(b"hello"))
            .await
            .unwrap();
        let (out, _) = list(&ts, None, false, true).await;
        let items: Vec<ItemMeta> = serde_json::from_str(&out).unwrap();
        assert_eq!(items, ts.store.list().await.unwrap());
    }

    #[tokio::test]
    async fn says_so_when_empty() {
        let ts = TestStore::new();
        assert_eq!(list(&ts, None, false, false).await.0, "no items\n");
    }

    #[tokio::test]
    async fn a_fresh_cache_is_printed_without_opening_the_store() {
        let (ts, dir) = (TestStore::new(), TempDir::new().unwrap());
        let meta = ts
            .store
            .put(NewItem::text("box"), bytes(b"hello"))
            .await
            .unwrap()
            .meta;
        let (path, file) = cache_file(dir.path());
        // Checked a minute ago, before the item was stored.
        ListCache::new("s".into(), now() - TimeDelta::seconds(60), Vec::new())
            .save(&path)
            .unwrap();
        assert_eq!(
            list(&ts, Some(&file), false, false).await,
            ("no items\n".to_owned(), false)
        );

        let (out, opened) = list(&ts, Some(&file), true, true).await;
        assert!(opened, "--nocache reads the store");
        let items: Vec<ItemMeta> = serde_json::from_str(&out).unwrap();
        assert_eq!(items, std::slice::from_ref(&meta));
        let cache = ListCache::load(&path).unwrap();
        assert_eq!((cache.items, cache.checked_at), (vec![meta], now()));
    }

    #[tokio::test]
    async fn an_old_or_mismatched_cache_reads_the_store_and_writes_the_cache() {
        let (ts, dir) = (TestStore::new(), TempDir::new().unwrap());
        let meta = ts
            .store
            .put(NewItem::text("box"), bytes(b"hello"))
            .await
            .unwrap()
            .meta;
        let (path, file) = cache_file(dir.path());
        for (store, checked_at) in [("other", now()), ("s", now() - TimeDelta::seconds(121))] {
            ListCache::new(store.into(), checked_at, Vec::new())
                .save(&path)
                .unwrap();
            let (out, opened) = list(&ts, Some(&file), false, false).await;
            assert!(opened, "{store} at {checked_at}");
            assert!(out.contains(meta.id.as_str()), "{out}");
            let cache = ListCache::load(&path).unwrap();
            assert_eq!(cache.store, "s");
            assert_eq!(cache.items, std::slice::from_ref(&meta));
            assert_eq!(cache.checked_at, now());
        }
    }
}
