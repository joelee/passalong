//! [`Store`] over any [`RemoteFs`], using this layout below the root:
//!
//! ```text
//! items/<id>/content     the item's bytes
//! items/<id>/meta.json   its ItemMeta
//! tmp/<random>/          staging, renamed to items/<id>/ in one step
//! ```
//!
//! An item directory appears only once its content and metadata are
//! complete, so readers never observe a partial item.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::TimeDelta;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::clock::Clock;
use crate::fs::{BoxRead, FsError, RemoteFs, RemotePath};
use crate::model::{
    ContentDigest, ContentHasher, ContentKey, ItemId, ItemKind, ItemMeta, NewItem, preview_of,
};
use crate::random::RandomSource;
use crate::store::{PutOutcome, Store, StoreError};

const ITEMS_DIR: &str = "items";
const TMP_DIR: &str = "tmp";
const CONTENT_FILE: &str = "content";
const META_FILE: &str = "meta.json";
const CHUNK_SIZE: usize = 64 * 1024;
/// Bytes kept from the start of text content to build its preview.
const PREVIEW_HEAD_BYTES: usize = 4096;
/// `meta.json` is tiny; the cap stops a hostile file from exhausting memory.
const MAX_META_BYTES: u64 = 64 * 1024;
const MIN_PREFIX_LEN: usize = 4;

/// A [`Store`] on top of a [`RemoteFs`].
pub struct FsStore<F> {
    fs: F,
    clock: Arc<dyn Clock>,
    rng: Mutex<Box<dyn RandomSource>>,
}

impl<F: RemoteFs> FsStore<F> {
    /// Creates a store over `fs`. `clock` timestamps new items and `rng`
    /// names staging directories.
    pub fn new(fs: F, clock: Arc<dyn Clock>, rng: Box<dyn RandomSource>) -> Self {
        Self {
            fs,
            clock,
            rng: Mutex::new(rng),
        }
    }

    /// The underlying filesystem.
    pub fn fs(&self) -> &F {
        &self.fs
    }

    fn item_dir(id: &ItemId) -> Result<RemotePath, StoreError> {
        Ok(RemotePath::new(ITEMS_DIR)?.join(id.as_str())?)
    }

    /// Every valid item id, newest first, from directory names alone.
    async fn item_ids(&self) -> Result<Vec<ItemId>, StoreError> {
        let entries = match self.fs.read_dir(&RemotePath::new(ITEMS_DIR)?).await {
            Ok(entries) => entries,
            Err(FsError::NotFound(_)) => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut ids: Vec<ItemId> = entries
            .into_iter()
            .filter(|entry| entry.is_dir)
            .filter_map(|entry| ItemId::parse(&entry.name).ok())
            .collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        Ok(ids)
    }

    async fn read_meta(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
        let path = Self::item_dir(id)?.join(META_FILE)?;
        let reader = self.fs.open_read(&path).await?;
        let mut buf = Vec::new();
        reader
            .take(MAX_META_BYTES)
            .read_to_end(&mut buf)
            .await
            .map_err(|err| corrupt(id, format!("reading meta.json: {err}")))?;
        let meta: ItemMeta =
            serde_json::from_slice(&buf).map_err(|err| corrupt(id, format!("meta.json: {err}")))?;
        if meta.id != *id {
            return Err(corrupt(id, format!("meta.json describes {}", meta.id)));
        }
        Ok(meta)
    }

    /// Streams `content` into `path`, hashing it and keeping the first bytes
    /// for a text preview.
    async fn write_content(
        &self,
        mut content: BoxRead,
        path: &RemotePath,
    ) -> Result<(ContentDigest, Vec<u8>), StoreError> {
        let mut writer = self.fs.open_write(path).await?;
        let mut hasher = ContentHasher::new();
        let mut head = Vec::new();
        let mut buf = vec![0_u8; CHUNK_SIZE];
        loop {
            let n = content
                .read(&mut buf)
                .await
                .map_err(|err| StoreError::Content(err.to_string()))?;
            if n == 0 {
                break;
            }
            let chunk = &buf[..n];
            hasher.update(chunk);
            let room = PREVIEW_HEAD_BYTES.saturating_sub(head.len());
            head.extend_from_slice(&chunk[..room.min(n)]);
            writer
                .write_all(chunk)
                .await
                .map_err(|err| FsError::from_io(path, err))?;
        }
        writer
            .shutdown()
            .await
            .map_err(|err| FsError::from_io(path, err))?;
        Ok((hasher.finalize(), head))
    }

    async fn write_meta(&self, path: &RemotePath, meta: &ItemMeta) -> Result<(), StoreError> {
        let mut json = serde_json::to_vec_pretty(meta)
            .map_err(|err| corrupt(&meta.id, format!("encoding meta.json: {err}")))?;
        json.push(b'\n');
        let mut writer = self.fs.open_write(path).await?;
        writer
            .write_all(&json)
            .await
            .map_err(|err| FsError::from_io(path, err))?;
        writer
            .shutdown()
            .await
            .map_err(|err| FsError::from_io(path, err))?;
        Ok(())
    }

    async fn stage_and_publish(
        &self,
        item: NewItem,
        content: BoxRead,
        items: &RemotePath,
        staging: &RemotePath,
    ) -> Result<PutOutcome, StoreError> {
        let (digest, head) = self
            .write_content(content, &staging.join(CONTENT_FILE)?)
            .await?;
        if let Some(existing) = self.find_by_content_key(&digest.content_key()).await? {
            tracing::info!(id = %existing.id, size = existing.size, "item already present");
            return Ok(PutOutcome {
                meta: existing,
                created: false,
            });
        }
        let preview = (item.kind == ItemKind::Text).then(|| preview_of(utf8_prefix(&head)));
        let meta = item.finish(self.clock.now(), &digest, preview)?;
        self.write_meta(&staging.join(META_FILE)?, &meta).await?;
        let target = items.join(meta.id.as_str())?;
        match self.fs.rename(staging, &target).await {
            Ok(()) => {
                tracing::info!(id = %meta.id, size = meta.size, kind = ?meta.kind, "item stored");
                Ok(PutOutcome {
                    meta,
                    created: true,
                })
            }
            // Another client published identical content in the same second.
            Err(FsError::AlreadyExists(_)) => Ok(PutOutcome {
                meta: self.read_meta(&meta.id).await?,
                created: false,
            }),
            Err(err) => Err(err.into()),
        }
    }
}

#[async_trait]
impl<F: RemoteFs> Store for FsStore<F> {
    async fn put(&self, item: NewItem, content: BoxRead) -> Result<PutOutcome, StoreError> {
        let items = RemotePath::new(ITEMS_DIR)?;
        let tmp = RemotePath::new(TMP_DIR)?;
        self.fs.create_dir_all(&items).await?;
        self.fs.create_dir_all(&tmp).await?;
        let token = self
            .rng
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_u64();
        let staging = tmp.join(&format!("{token:016x}"))?;
        self.fs.create_dir_all(&staging).await?;
        let result = self
            .stage_and_publish(item, content, &items, &staging)
            .await;
        // After a successful publish the staging path is gone, so this only
        // clears leftovers from failures and duplicates.
        if let Err(err) = self.fs.remove_dir_all(&staging).await {
            tracing::warn!(path = %staging, error = %err, "could not remove staging directory");
        }
        result
    }

    async fn list(&self) -> Result<Vec<ItemMeta>, StoreError> {
        let mut items = Vec::new();
        for id in self.item_ids().await? {
            match self.read_meta(&id).await {
                Ok(meta) => items.push(meta),
                Err(StoreError::Fs(FsError::NotFound(_))) => {
                    tracing::debug!(id = %id, "skipping item directory without meta.json");
                }
                Err(err @ StoreError::Corrupt { .. }) => {
                    tracing::warn!(id = %id, error = %err, "skipping corrupt item");
                }
                Err(err) => return Err(err),
            }
        }
        Ok(items)
    }

    async fn get(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError> {
        let meta = match self.read_meta(id).await {
            Err(StoreError::Fs(FsError::NotFound(_))) => {
                return Err(StoreError::NotFound(id.to_string()));
            }
            other => other?,
        };
        let content = match self
            .fs
            .open_read(&Self::item_dir(id)?.join(CONTENT_FILE)?)
            .await
        {
            Ok(content) => content,
            Err(FsError::NotFound(_)) => return Err(corrupt(id, "content is missing")),
            Err(err) => return Err(err.into()),
        };
        Ok((meta, content))
    }

    async fn exists(&self, id: &ItemId) -> Result<bool, StoreError> {
        let meta_path = Self::item_dir(id)?.join(META_FILE)?;
        Ok(self.fs.stat(&meta_path).await?.is_some())
    }

    async fn find_by_content_key(&self, key: &ContentKey) -> Result<Option<ItemMeta>, StoreError> {
        let ids = self.item_ids().await?;
        for id in ids.iter().rev().filter(|id| id.content_key() == key) {
            match self.read_meta(id).await {
                Ok(meta) => return Ok(Some(meta)),
                Err(StoreError::Fs(FsError::NotFound(_)) | StoreError::Corrupt { .. }) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(None)
    }

    async fn resolve(&self, input: &str) -> Result<ItemId, StoreError> {
        let input = input.trim();
        let needle = input.to_ascii_lowercase();
        if needle.chars().count() < MIN_PREFIX_LEN {
            return Err(StoreError::InvalidPrefix(input.to_owned()));
        }
        let full_id = needle.contains('-');
        let mut candidates: Vec<ItemId> = self
            .item_ids()
            .await?
            .into_iter()
            .filter(|id| {
                let haystack = if full_id {
                    id.as_str()
                } else {
                    id.content_key().as_str()
                };
                haystack.starts_with(&needle)
            })
            .collect();
        match candidates.len() {
            0 => Err(StoreError::NotFound(input.to_owned())),
            1 => Ok(candidates.remove(0)),
            _ => Err(StoreError::Ambiguous {
                input: input.to_owned(),
                candidates,
            }),
        }
    }

    async fn delete(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
        let meta = match self.read_meta(id).await {
            Err(StoreError::Fs(FsError::NotFound(_))) => {
                return Err(StoreError::NotFound(id.to_string()));
            }
            other => other?,
        };
        let tmp = RemotePath::new(TMP_DIR)?;
        self.fs.create_dir_all(&tmp).await?;
        let token = self
            .rng
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_u64();
        let grave = tmp.join(&format!("deleted-{id}-{token:016x}"))?;
        // Moving the directory out of `items/` first hides the item from
        // every listing in one step; a failed removal leaves only a staging
        // leftover for `clean_staging`.
        match self.fs.rename(&Self::item_dir(id)?, &grave).await {
            Err(FsError::NotFound(_)) => return Err(StoreError::NotFound(id.to_string())),
            other => other?,
        }
        self.fs.remove_dir_all(&grave).await?;
        tracing::info!(id = %id, size = meta.size, "item deleted");
        Ok(meta)
    }

    async fn clean_staging(&self, older_than: Duration) -> Result<usize, StoreError> {
        let tmp = RemotePath::new(TMP_DIR)?;
        let entries = match self.fs.read_dir(&tmp).await {
            Ok(entries) => entries,
            Err(FsError::NotFound(_)) => return Ok(0),
            Err(err) => return Err(err.into()),
        };
        // An age too large to subtract means nothing can be that old.
        let Some(cutoff) = TimeDelta::from_std(older_than)
            .ok()
            .and_then(|age| self.clock.now().checked_sub_signed(age))
        else {
            return Ok(0);
        };
        let mut removed = 0;
        for entry in entries.into_iter().filter(|entry| entry.is_dir) {
            let path = tmp.join(&entry.name)?;
            let stale = matches!(self.fs.stat(&path).await?, Some(meta) if meta.modified.is_some_and(|m| m < cutoff));
            if stale {
                self.fs.remove_dir_all(&path).await?;
                tracing::debug!(path = %path, "removed stale staging directory");
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!("removed {removed} stale staging directories");
        }
        Ok(removed)
    }
}

fn corrupt(id: &ItemId, reason: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt {
        id: id.to_string(),
        reason: reason.to_string(),
    }
}

/// The longest valid UTF-8 prefix; the preview head may end mid-character.
fn utf8_prefix(bytes: &[u8]) -> &str {
    match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => std::str::from_utf8(&bytes[..err.valid_up_to()]).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{FsError, LocalFs, RemoteFs, RemotePath};
    use crate::model::{ContentHasher, ItemKind, ItemMeta, NewItem};
    use crate::random::StdRandom;
    use crate::store::{Store, StoreError};
    use crate::testing::{FaultyFs, FsOp, ManualClock};
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use tempfile::TempDir;
    use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

    const T: &str = "2026-09-12T09:53:11Z";

    struct Fixture {
        dir: TempDir,
        clock: Arc<ManualClock>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: TempDir::new().unwrap(),
                clock: Arc::new(ManualClock::at(T)),
            }
        }
        fn store(&self) -> FsStore<LocalFs> {
            FsStore::new(
                LocalFs::new(self.dir.path()),
                self.clock.clone(),
                Box::new(StdRandom::new()),
            )
        }
        fn faulty(&self) -> FsStore<FaultyFs<LocalFs>> {
            FsStore::new(
                FaultyFs::new(LocalFs::new(self.dir.path())),
                self.clock.clone(),
                Box::new(StdRandom::new()),
            )
        }
        fn path(&self, rel: &str) -> std::path::PathBuf {
            self.dir.path().join(rel)
        }
        fn item_dirs(&self) -> Vec<String> {
            let Ok(rd) = std::fs::read_dir(self.path("items")) else {
                return vec![];
            };
            let mut names: Vec<_> = rd
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }
        fn tmp_is_empty(&self) -> bool {
            std::fs::read_dir(self.path("tmp"))
                .map(|mut rd| rd.next().is_none())
                .unwrap_or(true)
        }
    }

    fn content(bytes: &[u8]) -> BoxRead {
        Box::new(std::io::Cursor::new(bytes.to_vec()))
    }

    async fn read_all(mut r: BoxRead) -> Vec<u8> {
        let mut buf = Vec::new();
        r.read_to_end(&mut buf).await.unwrap();
        buf
    }

    /// Yields `ok` bytes, then fails, like a dropped network stream.
    struct BrokenStream {
        ok: usize,
    }

    impl AsyncRead for BrokenStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.ok == 0 {
                return Poll::Ready(Err(std::io::Error::other("connection reset")));
            }
            let n = self.ok.min(buf.remaining()).min(1000);
            buf.put_slice(&vec![b'x'; n]);
            self.ok -= n;
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn put_publishes_content_and_metadata_atomically() {
        let fx = Fixture::new();
        let out = fx
            .store()
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap();
        assert!(out.created);
        assert_eq!(out.meta.id.as_str(), "6aa52107-2cf24dba5fb0");
        assert_eq!(out.meta.kind, ItemKind::Text);
        assert_eq!(out.meta.preview.as_deref(), Some("hello"));
        assert_eq!(fx.item_dirs(), ["6aa52107-2cf24dba5fb0"]);
        let dir = fx.path("items/6aa52107-2cf24dba5fb0");
        assert_eq!(std::fs::read(dir.join("content")).unwrap(), b"hello");
        let on_disk: ItemMeta =
            serde_json::from_slice(&std::fs::read(dir.join("meta.json")).unwrap()).unwrap();
        assert_eq!(on_disk, out.meta);
        assert!(fx.tmp_is_empty(), "staging directory left behind");
    }

    #[tokio::test]
    async fn put_streams_large_files_exactly() {
        let fx = Fixture::new();
        let data: Vec<u8> = (0..2 * 1024 * 1024)
            .map(|i: u32| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let mut hasher = ContentHasher::new();
        hasher.update(&data);
        let digest = hasher.finalize();
        let out = fx
            .store()
            .put(NewItem::file("big.bin", "box"), content(&data))
            .await
            .unwrap();
        assert_eq!(out.meta.size, data.len() as u64);
        assert_eq!(out.meta.sha256, digest.sha256_hex());
        assert_eq!(out.meta.preview, None, "files get no preview");
        assert_eq!(out.meta.name.as_deref(), Some("big.bin"));
        let (_, stream) = fx.store().get(&out.meta.id).await.unwrap();
        assert_eq!(read_all(stream).await, data);
    }

    #[tokio::test]
    async fn text_preview_survives_a_multibyte_character_cut_at_the_head_limit() {
        let fx = Fixture::new();
        let text = format!("a{}", "é".repeat(5000));
        let out = fx
            .store()
            .put(NewItem::text("box"), content(text.as_bytes()))
            .await
            .unwrap();
        let preview = out.meta.preview.unwrap();
        assert_eq!(preview.chars().count(), 80);
        assert!(!preview.contains('\u{FFFD}'), "{preview}");
    }

    #[tokio::test]
    async fn identical_content_is_stored_once() {
        let fx = Fixture::new();
        let store = fx.store();
        let first = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap();
        let meta_mtime = std::fs::metadata(fx.path("items/6aa52107-2cf24dba5fb0/meta.json"))
            .unwrap()
            .modified()
            .unwrap();
        fx.clock.advance(60);
        let again = store
            .put(NewItem::text("laptop"), content(b"hello"))
            .await
            .unwrap();
        assert!(!again.created);
        assert_eq!(again.meta, first.meta, "the original item is returned");
        assert_eq!(fx.item_dirs().len(), 1);
        assert!(fx.tmp_is_empty());
        let after = std::fs::metadata(fx.path("items/6aa52107-2cf24dba5fb0/meta.json"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(meta_mtime, after, "meta.json was rewritten");
    }

    #[tokio::test]
    async fn list_is_newest_first_and_empty_without_items() {
        let fx = Fixture::new();
        let store = fx.store();
        assert!(store.list().await.unwrap().is_empty());
        let mut ids = Vec::new();
        for text in ["a", "b", "c"] {
            ids.push(
                store
                    .put(NewItem::text("box"), content(text.as_bytes()))
                    .await
                    .unwrap()
                    .meta
                    .id,
            );
            fx.clock.advance(1);
        }
        let listed: Vec<_> = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        ids.reverse();
        assert_eq!(listed, ids);
    }

    #[tokio::test]
    async fn list_skips_foreign_incomplete_and_corrupt_entries() {
        let fx = Fixture::new();
        let store = fx.store();
        let good = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap()
            .meta;
        std::fs::create_dir_all(fx.path("items/not-an-id")).unwrap();
        std::fs::write(fx.path("items/stray-file"), b"").unwrap();
        std::fs::create_dir_all(fx.path("items/6aa52108-000000000000")).unwrap();
        std::fs::create_dir_all(fx.path("items/6aa52109-111111111111")).unwrap();
        std::fs::write(
            fx.path("items/6aa52109-111111111111/meta.json"),
            b"{not json",
        )
        .unwrap();
        // meta.json whose id disagrees with its directory
        std::fs::create_dir_all(fx.path("items/6aa5210a-222222222222")).unwrap();
        std::fs::copy(
            fx.path("items/6aa52107-2cf24dba5fb0/meta.json"),
            fx.path("items/6aa5210a-222222222222/meta.json"),
        )
        .unwrap();
        assert_eq!(store.list().await.unwrap(), vec![good]);
        let err = store
            .get(&ItemId::parse("6aa5210a-222222222222").unwrap())
            .await
            .err()
            .unwrap();
        assert!(matches!(err, StoreError::Corrupt { .. }), "{err:?}");
        let err = store
            .get(&ItemId::parse("6aa52109-111111111111").unwrap())
            .await
            .err()
            .unwrap();
        assert!(matches!(err, StoreError::Corrupt { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn get_and_exists_report_missing_items() {
        let fx = Fixture::new();
        let store = fx.store();
        let id = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap()
            .meta
            .id;
        assert!(store.exists(&id).await.unwrap());
        let (meta, stream) = store.get(&id).await.unwrap();
        assert_eq!(meta.id, id);
        assert_eq!(read_all(stream).await, b"hello");
        let ghost = ItemId::parse("00000001-abcdefabcdef").unwrap();
        assert!(!store.exists(&ghost).await.unwrap());
        assert!(
            matches!(store.get(&ghost).await.err().unwrap(), StoreError::NotFound(s) if s == ghost.as_str())
        );
        std::fs::remove_file(fx.path("items/6aa52107-2cf24dba5fb0/content")).unwrap();
        assert!(matches!(
            store.get(&id).await.err().unwrap(),
            StoreError::Corrupt { .. }
        ));
    }

    #[tokio::test]
    async fn find_by_content_key_matches_the_id_suffix() {
        let fx = Fixture::new();
        let store = fx.store();
        let meta = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap()
            .meta;
        assert_eq!(
            store
                .find_by_content_key(meta.id.content_key())
                .await
                .unwrap(),
            Some(meta)
        );
        let other = crate::model::ContentKey::parse("abcdefabcdef").unwrap();
        assert_eq!(store.find_by_content_key(&other).await.unwrap(), None);
    }

    /// Two distinct contents whose content keys share the first 4 digits.
    fn colliding_pair() -> (String, String) {
        let mut seen: HashMap<String, String> = HashMap::new();
        for i in 0.. {
            let text = format!("item {i}");
            let mut h = ContentHasher::new();
            h.update(text.as_bytes());
            let prefix = h.finalize().content_key().as_str()[..4].to_owned();
            if let Some(prev) = seen.insert(prefix, text.clone()) {
                return (prev, text);
            }
        }
        unreachable!()
    }

    #[tokio::test]
    async fn resolve_accepts_content_key_or_full_id_prefixes() {
        let fx = Fixture::new();
        let store = fx.store();
        let id = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap()
            .meta
            .id;
        for input in [
            "2cf2",
            "2CF24D",
            "2cf24dba5fb0",
            "6aa52107-2c",
            "6aa52107-2cf24dba5fb0",
            " 2cf2 ",
        ] {
            assert_eq!(store.resolve(input).await.unwrap(), id, "{input:?}");
        }
        assert!(matches!(
            store.resolve("2c").await.unwrap_err(),
            StoreError::InvalidPrefix(_)
        ));
        assert!(matches!(
            store.resolve("zzzz").await.unwrap_err(),
            StoreError::NotFound(_)
        ));
        assert!(matches!(
            store.resolve("6aa52108-").await.unwrap_err(),
            StoreError::NotFound(_)
        ));

        let (a, b) = colliding_pair();
        fx.clock.advance(1);
        let id_a = store
            .put(NewItem::text("box"), content(a.as_bytes()))
            .await
            .unwrap()
            .meta
            .id;
        fx.clock.advance(1);
        let id_b = store
            .put(NewItem::text("box"), content(b.as_bytes()))
            .await
            .unwrap()
            .meta
            .id;
        let prefix = &id_a.content_key().as_str()[..4];
        match store.resolve(prefix).await.unwrap_err() {
            StoreError::Ambiguous { input, candidates } => {
                assert_eq!(input, prefix);
                assert_eq!(candidates, vec![id_b.clone(), id_a.clone()]);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            store.resolve(id_a.content_key().as_str()).await.unwrap(),
            id_a
        );
    }

    #[tokio::test]
    async fn failed_publish_leaves_no_item_and_no_staging() {
        let fx = Fixture::new();
        let store = fx.faulty();
        store.fs().fail_next(FsOp::Rename, 1);
        let err = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, StoreError::Fs(FsError::Other { .. })),
            "{err:?}"
        );
        assert!(fx.item_dirs().is_empty());
        assert!(fx.tmp_is_empty());
        // and the store still works afterwards
        assert!(
            store
                .put(NewItem::text("box"), content(b"hello"))
                .await
                .unwrap()
                .created
        );
    }

    #[tokio::test]
    async fn broken_source_stream_leaves_no_item_and_no_staging() {
        let fx = Fixture::new();
        let err = fx
            .store()
            .put(
                NewItem::file("f", "box"),
                Box::new(BrokenStream { ok: 70_000 }),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, StoreError::Content(msg) if msg.contains("connection reset")),
            "{err:?}"
        );
        assert!(fx.item_dirs().is_empty());
        assert!(fx.tmp_is_empty());
    }

    #[tokio::test]
    async fn filesystem_errors_propagate_from_every_read_path() {
        let fx = Fixture::new();
        let store = fx.faulty();
        let id = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap()
            .meta
            .id;
        store.fs().fail_next(FsOp::ReadDir, 1);
        assert!(store.list().await.is_err());
        store.fs().fail_next(FsOp::Stat, 1);
        assert!(store.exists(&id).await.is_err());
        store.fs().fail_next(FsOp::OpenRead, 1);
        assert!(matches!(
            store.get(&id).await.err().unwrap(),
            StoreError::Fs(_)
        ));
        store.fs().fail_next(FsOp::CreateDirAll, 1);
        assert!(
            store
                .put(NewItem::text("box"), content(b"x"))
                .await
                .is_err()
        );
        assert!(fx.tmp_is_empty());
        let _ = RemotePath::root();
        let _: &dyn RemoteFs = store.fs();
    }

    fn tmp_entries(fx: &Fixture) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(fx.path("tmp")) else {
            return vec![];
        };
        let mut names: Vec<_> = rd
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// Sets a staging directory's modification time relative to the fixture clock.
    fn staging_dir_aged(fx: &Fixture, name: &str, age_secs: i64) {
        let dir = fx.path("tmp").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let when = FixedClockAt::minus(T, age_secs);
        std::fs::File::open(&dir)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    struct FixedClockAt;
    impl FixedClockAt {
        fn minus(rfc3339: &str, secs: i64) -> std::time::SystemTime {
            let base = chrono::DateTime::parse_from_rfc3339(rfc3339)
                .unwrap()
                .with_timezone(&chrono::Utc);
            (base - chrono::TimeDelta::seconds(secs)).into()
        }
    }

    #[tokio::test]
    async fn delete_removes_the_item_and_returns_its_metadata() {
        let fx = Fixture::new();
        let store = fx.store();
        let keep = store
            .put(NewItem::text("box"), content(b"keep"))
            .await
            .unwrap()
            .meta;
        fx.clock.advance(1);
        let gone = store
            .put(NewItem::text("box"), content(b"gone"))
            .await
            .unwrap()
            .meta;
        assert_eq!(store.delete(&gone.id).await.unwrap(), gone);
        assert_eq!(store.list().await.unwrap(), vec![keep]);
        assert!(!store.exists(&gone.id).await.unwrap());
        assert!(tmp_entries(&fx).is_empty(), "{:?}", tmp_entries(&fx));
        assert!(
            matches!(store.delete(&gone.id).await.unwrap_err(), StoreError::NotFound(id) if id == gone.id.as_str())
        );
    }

    #[tokio::test]
    async fn a_failed_removal_still_hides_the_item_at_once() {
        let fx = Fixture::new();
        let store = fx.faulty();
        let meta = store
            .put(NewItem::text("box"), content(b"doomed"))
            .await
            .unwrap()
            .meta;
        store.fs().fail_next(FsOp::RemoveDirAll, 1);
        assert!(matches!(
            store.delete(&meta.id).await.unwrap_err(),
            StoreError::Fs(_)
        ));
        assert!(
            store.list().await.unwrap().is_empty(),
            "item must already be gone from listings"
        );
        let left = tmp_entries(&fx);
        assert_eq!(left.len(), 1);
        assert!(
            left[0].starts_with(&format!("deleted-{}-", meta.id)),
            "{left:?}"
        );
    }

    #[tokio::test]
    async fn clean_staging_removes_only_entries_older_than_the_threshold() {
        let fx = Fixture::new();
        let store = fx.store();
        assert_eq!(
            store
                .clean_staging(std::time::Duration::from_secs(3600))
                .await
                .unwrap(),
            0,
            "no tmp yet"
        );
        staging_dir_aged(&fx, "abandoned-upload", 2 * 3600);
        staging_dir_aged(&fx, "deleted-old", 3 * 3600);
        staging_dir_aged(&fx, "upload-in-progress", 10 * 60);
        std::fs::write(fx.path("tmp/stray-file"), b"").unwrap();
        let removed = store
            .clean_staging(std::time::Duration::from_secs(3600))
            .await
            .unwrap();
        assert_eq!(removed, 2);
        assert_eq!(tmp_entries(&fx), ["stray-file", "upload-in-progress"]);
        assert_eq!(
            store.clean_staging(std::time::Duration::MAX).await.unwrap(),
            0,
            "nothing is older than forever"
        );
    }
}
