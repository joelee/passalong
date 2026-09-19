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
use chrono::{DateTime, TimeDelta, Utc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::clock::Clock;
use crate::crypto::{self, CONTENT_SALT_LEN, KeyId, Sealer};
use crate::encryption::{
    ENCRYPTION_DIR, EncryptionError, Leftovers, PLAIN_DIR, REWRITE_DIR, read_header,
};
use crate::fs::{BoxRead, FsError, RemoteFs, RemotePath};
use crate::model::{ContentDigest, ContentKey, ItemId, ItemKind, ItemMeta, NewItem, preview_of};
use crate::random::RandomSource;
use crate::store::format::{self, CopyError, MAX_META_BYTES, MetaError, decode_array};
use crate::store::{PROBE_BYTES, PutOutcome, Store, StoreError, WriteProbe};

const ITEMS_DIR: &str = "items";
const TMP_DIR: &str = "tmp";
/// Where a sealed store keeps its items.
const SEALED_ITEMS_DIR: &str = "v2/items";
/// Where a sealed store stages uploads and deletions.
const SEALED_TMP_DIR: &str = "v2/tmp";
/// How long after its creation a sealed item that does not open counts as
/// still arriving, rather than corrupt.
pub(crate) const INCOMPLETE_GRACE_SECS: i64 = 5 * 60;
const CONTENT_FILE: &str = "content";
const META_FILE: &str = "meta.json";
const MIN_PREFIX_LEN: usize = 4;

/// A [`Store`] on top of a [`RemoteFs`].
///
/// A plaintext store keeps items under `items/` and staging under `tmp/`.
/// A sealed store ([`FsStore::sealed`]) keeps them under `v2/items/` and
/// `v2/tmp/`: every item's metadata and content are sealed by a [`Sealer`],
/// and ids carry keyed content keys.
pub struct FsStore<F> {
    fs: F,
    clock: Arc<dyn Clock>,
    rng: Mutex<Box<dyn RandomSource>>,
    sealer: Option<Sealer>,
    /// Whether a sealed store opened through its header checks that the
    /// header still names its key.
    guard: bool,
    /// Whether a plaintext store checks that it is still plaintext.
    plain_guard: bool,
}

impl<F: RemoteFs> FsStore<F> {
    /// Creates a plaintext store over `fs`. `clock` timestamps new items and
    /// `rng` names staging directories.
    pub fn new(fs: F, clock: Arc<dyn Clock>, rng: Box<dyn RandomSource>) -> Self {
        Self {
            fs,
            clock,
            rng: Mutex::new(rng),
            sealer: None,
            guard: false,
            plain_guard: false,
        }
    }

    /// Creates a store over `fs` that seals every item with `sealer`.
    pub fn sealed(
        fs: F,
        clock: Arc<dyn Clock>,
        rng: Box<dyn RandomSource>,
        sealer: Sealer,
    ) -> Self {
        Self {
            fs,
            clock,
            rng: Mutex::new(rng),
            sealer: Some(sealer),
            guard: false,
            plain_guard: false,
        }
    }

    /// Makes a plaintext store check, before every `put`, `delete`, and
    /// `list_ids`, that it is still plaintext: no lock, no `encryption/`
    /// folder, and no stop file where `items/` goes.
    pub(crate) fn plain_guarded(mut self) -> Self {
        self.plain_guard = true;
        self
    }

    /// The check [`FsStore::plain_guarded`] describes.
    async fn check_plain(&self) -> Result<(), StoreError> {
        if self
            .fs
            .stat(&RemotePath::new(REWRITE_DIR)?)
            .await?
            .is_some()
        {
            return Err(EncryptionError::Rewriting { started: None }.into());
        }
        if self
            .fs
            .stat(&RemotePath::new(ENCRYPTION_DIR)?)
            .await?
            .is_some()
        {
            return Err(EncryptionError::NoKey.into());
        }
        if self
            .fs
            .stat(&RemotePath::new(ITEMS_DIR)?)
            .await?
            .is_some_and(|meta| !meta.is_dir)
        {
            return Err(EncryptionError::HeaderMissing.into());
        }
        Ok(())
    }

    /// Makes a sealed store check, before every `put`, `delete`, and
    /// `list_ids`, and again once `put` has published its item, that no
    /// change of encryption started and that the store header still names
    /// its key.
    pub(crate) fn guarded(mut self) -> Self {
        self.guard = true;
        self
    }

    /// The check [`FsStore::guarded`] or [`FsStore::plain_guarded`]
    /// describes. The header is read every time: its size and modification
    /// time cannot tell two keys apart.
    async fn check_key(&self) -> Result<(), StoreError> {
        if self.plain_guard {
            return self.check_plain().await;
        }
        let (true, Some(sealer)) = (self.guard, &self.sealer) else {
            return Ok(());
        };
        if self
            .fs
            .stat(&RemotePath::new(REWRITE_DIR)?)
            .await?
            .is_some()
        {
            return Err(EncryptionError::Rewriting { started: None }.into());
        }
        let header = read_header(&self.fs)
            .await?
            .ok_or(EncryptionError::HeaderMissing)?;
        if header.key_id() != sealer.key_id() {
            return Err(EncryptionError::KeyChanged.into());
        }
        Ok(())
    }

    /// Takes back the item `id` this client just published, after the
    /// store's encryption changed under it. An item a re-encryption already
    /// took into its source is not there to take back: it is re-encrypted
    /// with the rest.
    async fn retract(&self, id: &ItemId) {
        let token = self
            .rng
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_u64();
        let (Ok(item), Ok(tmp)) = (self.item_dir(id), self.tmp_dir()) else {
            return;
        };
        let Ok(grave) = tmp.join(&format!("retracted-{id}-{token:016x}")) else {
            return;
        };
        match self.fs.rename(&item, &grave).await {
            Ok(()) => {
                tracing::info!(id = %id, "took back an item published while the store's key changed");
                if let Err(err) = self.fs.remove_dir_all(&grave).await {
                    tracing::warn!(path = %grave, error = %err, "could not remove a taken-back item");
                }
            }
            Err(FsError::NotFound(_)) => {
                tracing::debug!(id = %id, "the item went into a re-encryption");
            }
            Err(err) => tracing::warn!(
                id = %id,
                error = %err,
                "could not take back an item published while the store's key changed"
            ),
        }
    }

    /// The id `meta`'s item gets in this store: its creation time, and this
    /// store's content key for its SHA-256.
    pub(crate) fn id_for(&self, meta: &ItemMeta) -> Result<ItemId, StoreError> {
        let sha256 = decode_array(&meta.sha256)
            .map_err(|reason| corrupt(&meta.id, format!("sha256: {reason}")))?;
        let key = self.keyed(&ContentDigest::new(sha256, meta.size));
        Ok(ItemId::new(meta.created_at, key)?)
    }

    /// Stores another store's item, with its metadata and creation time,
    /// under the id [`FsStore::id_for`] gives it, and returns its metadata
    /// here. An item already stored under that id is left as it is. The
    /// content must match `meta`'s SHA-256 and size.
    pub(crate) async fn import(
        &self,
        meta: &ItemMeta,
        content: BoxRead,
    ) -> Result<ItemMeta, StoreError> {
        let id = self.id_for(meta)?;
        let target = self.item_dir(&id)?;
        if self.fs.stat(&target).await?.is_some() {
            return self.read_meta(&id).await;
        }
        let tmp = self.tmp_dir()?;
        self.fs.create_dir_all(&self.items_dir()?).await?;
        self.fs.create_dir_all(&tmp).await?;
        let token = self
            .rng
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_u64();
        let staging = tmp.join(&format!("{token:016x}"))?;
        self.fs.create_dir_all(&staging).await?;
        let result = async {
            let (digest, _, salt) = self
                .write_content(content, &staging.join(CONTENT_FILE)?)
                .await?;
            if digest.sha256_hex() != meta.sha256 || digest.size() != meta.size {
                return Err(corrupt(&meta.id, "its content does not match its SHA-256"));
            }
            let copy = ItemMeta {
                id: id.clone(),
                ..meta.clone()
            };
            self.write_meta(&staging.join(META_FILE)?, &copy, salt)
                .await?;
            match self.fs.rename(&staging, &target).await {
                Ok(()) | Err(FsError::AlreadyExists(_)) => Ok(copy),
                Err(err) => Err(err.into()),
            }
        }
        .await;
        if let Err(err) = self.fs.remove_dir_all(&staging).await {
            tracing::warn!(path = %staging, error = %err, "could not remove staging directory");
        }
        result
    }

    /// Warns, in a sealed store opened through its header, while plaintext
    /// items from before a fresh start remain in `plain/items/`, or uploads
    /// cut short before encryption remain in the plaintext `tmp/`.
    async fn remind_plain_left(&self) {
        if !self.guard {
            return;
        }
        if let Ok(dir) = RemotePath::new(PLAIN_DIR).and_then(|plain| plain.join(ITEMS_DIR))
            && let Ok(entries) = self.fs.read_dir(&dir).await
        {
            let n = entries
                .iter()
                .filter(|entry| entry.is_dir && ItemId::parse(&entry.name).is_ok())
                .count();
            if n > 0 {
                let (items, them) = if n == 1 {
                    ("item remains", "it")
                } else {
                    ("items remain", "them")
                };
                tracing::warn!(
                    "{n} unencrypted {items} from before encryption; remove {them} with `passalong prune --plain`"
                );
            }
        }
        if let Ok(tmp) = RemotePath::new(TMP_DIR)
            && let Ok(entries) = self.fs.read_dir(&tmp).await
            && !entries.is_empty()
        {
            let n = entries.len();
            let (verb, them) = if n == 1 {
                ("remains", "it")
            } else {
                ("remain", "them")
            };
            tracing::warn!(
                "{} {verb} from before encryption; remove {them} with `passalong prune --plain`",
                Leftovers::plaintext(n)
            );
        }
    }

    /// The underlying filesystem.
    pub fn fs(&self) -> &F {
        &self.fs
    }

    /// The sealer of a sealed store.
    pub fn sealer(&self) -> Option<&Sealer> {
        self.sealer.as_ref()
    }

    fn items_dir(&self) -> Result<RemotePath, StoreError> {
        let dir = if self.sealer.is_some() {
            SEALED_ITEMS_DIR
        } else {
            ITEMS_DIR
        };
        Ok(RemotePath::new(dir)?)
    }

    fn tmp_dir(&self) -> Result<RemotePath, StoreError> {
        let dir = if self.sealer.is_some() {
            SEALED_TMP_DIR
        } else {
            TMP_DIR
        };
        Ok(RemotePath::new(dir)?)
    }

    fn item_dir(&self, id: &ItemId) -> Result<RemotePath, StoreError> {
        Ok(self.items_dir()?.join(id.as_str())?)
    }

    fn keyed(&self, digest: &ContentDigest) -> ContentKey {
        match &self.sealer {
            Some(sealer) => sealer.content_key(digest),
            None => digest.content_key(),
        }
    }

    /// The error for a sealed item that does not open: not complete yet
    /// while it is recent, since its files may still be arriving through a
    /// synced folder, and corrupt after that.
    fn unreadable(&self, id: &ItemId, reason: impl std::fmt::Display) -> StoreError {
        let grace = TimeDelta::seconds(INCOMPLETE_GRACE_SECS);
        if id.timestamp() > self.clock.now() - grace {
            tracing::debug!(id = %id, reason = %reason, "sealed item is not complete yet");
            EncryptionError::Incomplete { id: id.to_string() }.into()
        } else {
            corrupt(id, reason)
        }
    }

    /// Every valid item id, newest first, from directory names alone.
    async fn item_ids(&self) -> Result<Vec<ItemId>, StoreError> {
        let entries = match self.fs.read_dir(&self.items_dir()?).await {
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
        Ok(self.read_item(id).await?.0)
    }

    /// An item's metadata and, in a sealed store, its content salt.
    async fn read_item(
        &self,
        id: &ItemId,
    ) -> Result<(ItemMeta, Option<[u8; CONTENT_SALT_LEN]>), StoreError> {
        let path = self.item_dir(id)?.join(META_FILE)?;
        let reader = self.fs.open_read(&path).await?;
        let mut buf = Vec::new();
        reader
            .take(MAX_META_BYTES)
            .read_to_end(&mut buf)
            .await
            .map_err(|err| corrupt(id, format!("reading meta.json: {err}")))?;
        format::decode_meta(id, &buf, self.sealer.as_ref()).map_err(|err| match err {
            MetaError::Corrupt(reason) => corrupt(id, reason),
            MetaError::Unopened(reason) => self.unreadable(id, reason),
        })
    }

    /// Streams `content` into `path`, hashing it and keeping the first bytes
    /// for a text preview. A sealed store seals it chunk by chunk, reading
    /// one chunk ahead to find the last, and also returns the content salt.
    async fn write_content(
        &self,
        content: BoxRead,
        path: &RemotePath,
    ) -> Result<(ContentDigest, Vec<u8>, Option<[u8; CONTENT_SALT_LEN]>), StoreError> {
        let mut writer = self.fs.open_write(path).await?;
        let written = format::write_content(content, &mut writer, self.sealer.as_ref())
            .await
            .map_err(|err| match err {
                CopyError::Read(err) => StoreError::Content(err.to_string()),
                CopyError::Write(err) => FsError::from_io(path, err).into(),
                CopyError::Crypto(err) => err.into(),
            })?;
        Ok((written.digest, written.head, written.salt))
    }

    /// Writes `meta`, sealed with its content salt in a sealed store.
    async fn write_meta(
        &self,
        path: &RemotePath,
        meta: &ItemMeta,
        salt: Option<[u8; CONTENT_SALT_LEN]>,
    ) -> Result<(), StoreError> {
        let sealing = match (&self.sealer, &salt) {
            (Some(sealer), Some(salt)) => Some((sealer, salt)),
            _ => None,
        };
        let json = format::encode_meta(meta, sealing).map_err(|err| match err {
            format::EncodeError::Crypto(err) => err.into(),
            format::EncodeError::Json(err) => {
                corrupt(&meta.id, format!("encoding meta.json: {err}"))
            }
        })?;
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
        let (digest, head, salt) = self
            .write_content(content, &staging.join(CONTENT_FILE)?)
            .await?;
        let key = self.keyed(&digest);
        if let Some(existing) = self.find_by_content_key(&key).await? {
            tracing::info!(id = %existing.id, size = existing.size, "item already present");
            return Ok(PutOutcome {
                meta: existing,
                created: false,
            });
        }
        let preview = (item.kind == ItemKind::Text).then(|| preview_of(utf8_prefix(&head)));
        let meta = item.finish_keyed(self.clock.now(), &digest, key, preview)?;
        self.write_meta(&staging.join(META_FILE)?, &meta, salt)
            .await?;
        let target = items.join(meta.id.as_str())?;
        match self.fs.rename(staging, &target).await {
            Ok(()) => {
                // A change of key that began while the item was staged
                // shows now. The item may be sealed under the old key, so
                // it is taken back and the send fails, to be retried.
                if let Err(err) = self.check_key().await {
                    self.retract(&meta.id).await;
                    return Err(err);
                }
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
    fn key_id(&self) -> Option<KeyId> {
        self.sealer.as_ref().map(Sealer::key_id)
    }

    fn content_key(&self, digest: &ContentDigest) -> ContentKey {
        self.keyed(digest)
    }

    async fn put(&self, item: NewItem, content: BoxRead) -> Result<PutOutcome, StoreError> {
        self.check_key().await?;
        let items = self.items_dir()?;
        let tmp = self.tmp_dir()?;
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
        let items = self.list_after(None).await?;
        self.remind_plain_left().await;
        Ok(items)
    }

    async fn list_ids(&self) -> Result<Vec<ItemId>, StoreError> {
        self.check_key().await?;
        self.item_ids().await
    }

    async fn newest_id(&self) -> Result<Option<ItemId>, StoreError> {
        Ok(self.item_ids().await?.into_iter().next())
    }

    async fn list_after(&self, after: Option<&ItemId>) -> Result<Vec<ItemMeta>, StoreError> {
        let mut items = Vec::new();
        for id in self.item_ids().await? {
            // Ids sort by creation time and come newest first.
            if after.is_some_and(|after| id <= *after) {
                break;
            }
            match self.read_meta(&id).await {
                Ok(meta) => items.push(meta),
                Err(StoreError::Fs(FsError::NotFound(_))) => {
                    tracing::debug!(id = %id, "skipping item directory without meta.json");
                }
                Err(StoreError::Encryption(EncryptionError::Incomplete { .. })) => {
                    tracing::debug!(id = %id, "skipping an item that is not complete yet");
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
        let (meta, salt) = match self.read_item(id).await {
            Err(StoreError::Fs(FsError::NotFound(_))) => {
                return Err(StoreError::NotFound(id.to_string()));
            }
            other => other?,
        };
        let path = self.item_dir(id)?.join(CONTENT_FILE)?;
        if let (Some(sealer), Some(salt)) = (&self.sealer, salt) {
            // Content still arriving through a synced folder is shorter than
            // its metadata promises; checking first tells that apart from
            // damage found while reading.
            let expected = crypto::sealed_len(meta.size);
            match self.fs.stat(&path).await? {
                None => return Err(self.unreadable(id, "content is missing")),
                Some(found) if found.size < expected => {
                    return Err(self.unreadable(
                        id,
                        format!("content has {} of {expected} bytes", found.size),
                    ));
                }
                Some(_) => {}
            }
            let content = self.fs.open_read(&path).await?;
            return Ok((meta, Box::new(sealer.open_item_content(content, salt))));
        }
        let content = match self.fs.open_read(&path).await {
            Ok(content) => content,
            Err(FsError::NotFound(_)) => return Err(corrupt(id, "content is missing")),
            Err(err) => return Err(err.into()),
        };
        Ok((meta, content))
    }

    async fn get_meta(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
        match self.read_meta(id).await {
            Err(StoreError::Fs(FsError::NotFound(_))) => Err(StoreError::NotFound(id.to_string())),
            other => other,
        }
    }

    async fn exists(&self, id: &ItemId) -> Result<bool, StoreError> {
        let meta_path = self.item_dir(id)?.join(META_FILE)?;
        Ok(self.fs.stat(&meta_path).await?.is_some())
    }

    async fn find_by_content_key(&self, key: &ContentKey) -> Result<Option<ItemMeta>, StoreError> {
        let ids = self.item_ids().await?;
        for id in ids.iter().rev().filter(|id| id.content_key() == key) {
            match self.read_meta(id).await {
                Ok(meta) => return Ok(Some(meta)),
                Err(
                    StoreError::Fs(FsError::NotFound(_))
                    | StoreError::Corrupt { .. }
                    | StoreError::Encryption(EncryptionError::Incomplete { .. }),
                ) => {}
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
        self.check_key().await?;
        let meta = match self.read_meta(id).await {
            Err(StoreError::Fs(FsError::NotFound(_))) => {
                return Err(StoreError::NotFound(id.to_string()));
            }
            other => other?,
        };
        let tmp = self.tmp_dir()?;
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
        match self.fs.rename(&self.item_dir(id)?, &grave).await {
            Err(FsError::NotFound(_)) => return Err(StoreError::NotFound(id.to_string())),
            other => other?,
        }
        self.fs.remove_dir_all(&grave).await?;
        tracing::info!(id = %id, size = meta.size, "item deleted");
        Ok(meta)
    }

    async fn probe_write(&self) -> Result<WriteProbe, StoreError> {
        let tmp = self.tmp_dir()?;
        self.fs.create_dir_all(&tmp).await?;
        let token = self
            .rng
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_u64();
        // Under tmp/ like an upload's staging, so listings never see it.
        let probe = tmp.join(&format!("probe-{token:016x}"))?;
        self.fs.create_dir_all(&probe).await?;
        let file = probe.join("probe")?;
        let written = async {
            let mut writer = self.fs.open_write(&file).await?;
            writer
                .write_all(&probe_content())
                .await
                .map_err(|err| FsError::from_io(&file, err))?;
            writer
                .shutdown()
                .await
                .map_err(|err| FsError::from_io(&file, err))?;
            Ok::<(), FsError>(())
        }
        .await;
        let removed = self.fs.remove_dir_all(&probe).await;
        written?;
        removed?;
        Ok(WriteProbe::Verified)
    }

    async fn clean_staging(&self, older_than: Duration) -> Result<usize, StoreError> {
        // An age too large to subtract means nothing can be that old.
        let Some(cutoff) = TimeDelta::from_std(older_than)
            .ok()
            .and_then(|age| self.clock.now().checked_sub_signed(age))
        else {
            return Ok(0);
        };
        let mut removed = self.clean_dir(&self.tmp_dir()?, cutoff).await?;
        // A sealed store never stages in the plaintext `tmp/`; what is
        // there was cut short before encryption.
        if self.sealer.is_some() {
            removed += self.clean_dir(&RemotePath::new(TMP_DIR)?, cutoff).await?;
        }
        if removed > 0 {
            tracing::info!("removed {removed} stale staging directories");
        }
        Ok(removed)
    }
}

impl<F: RemoteFs> FsStore<F> {
    /// Removes the folders in `tmp` last changed before `cutoff`.
    async fn clean_dir(
        &self,
        tmp: &RemotePath,
        cutoff: DateTime<Utc>,
    ) -> Result<usize, StoreError> {
        let entries = match self.fs.read_dir(tmp).await {
            Ok(entries) => entries,
            Err(FsError::NotFound(_)) => return Ok(0),
            Err(err) => return Err(err.into()),
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

/// What [`FsStore::probe_write`] writes: [`PROBE_BYTES`] bytes that start
/// by saying what the file is.
fn probe_content() -> Vec<u8> {
    let mut content = b"passalong write probe\n".to_vec();
    content.resize(PROBE_BYTES, b'.');
    content
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
        crate::testing::set_modified(&dir, when).unwrap();
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

    async fn three_items(fx: &Fixture, store: &impl Store) -> Vec<ItemMeta> {
        let mut metas = Vec::new();
        for text in ["old", "mid", "new"] {
            metas.push(
                store
                    .put(NewItem::text("box"), content(text.as_bytes()))
                    .await
                    .unwrap()
                    .meta,
            );
            fx.clock.advance(1);
        }
        metas
    }

    #[tokio::test]
    async fn list_after_returns_only_newer_items_newest_first() {
        let fx = Fixture::new();
        let store = fx.store();
        let metas = three_items(&fx, &store).await;
        let (old, mid, new) = (&metas[0], &metas[1], &metas[2]);
        assert_eq!(
            store.list_after(Some(&old.id)).await.unwrap(),
            vec![new.clone(), mid.clone()]
        );
        assert!(store.list_after(Some(&new.id)).await.unwrap().is_empty());
        assert_eq!(
            store.list_after(None).await.unwrap(),
            store.list().await.unwrap()
        );
    }

    #[tokio::test]
    async fn list_after_reads_only_the_newer_meta_files() {
        let fx = Fixture::new();
        let store = fx.faulty();
        let metas = three_items(&fx, &store).await;
        let before = store.fs().calls(FsOp::OpenRead);
        assert_eq!(store.list_after(Some(&metas[0].id)).await.unwrap().len(), 2);
        assert_eq!(store.fs().calls(FsOp::OpenRead) - before, 2);
    }

    #[tokio::test]
    async fn list_after_skips_corrupt_items_like_list() {
        let fx = Fixture::new();
        let store = fx.store();
        let metas = three_items(&fx, &store).await;
        std::fs::write(
            fx.path(&format!("items/{}/meta.json", metas[2].id)),
            b"not json",
        )
        .unwrap();
        assert_eq!(
            store.list_after(Some(&metas[0].id)).await.unwrap(),
            vec![metas[1].clone()]
        );
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn newest_id_is_the_latest_item_without_reading_metadata() {
        let fx = Fixture::new();
        let store = fx.faulty();
        assert_eq!(store.newest_id().await.unwrap(), None);
        let metas = three_items(&fx, &store).await;
        let before = store.fs().calls(FsOp::OpenRead);
        assert_eq!(store.newest_id().await.unwrap(), Some(metas[2].id.clone()));
        assert_eq!(store.fs().calls(FsOp::OpenRead), before);
    }

    #[tokio::test]
    async fn get_meta_reads_one_meta_file_and_names_unknown_ids() {
        let fx = Fixture::new();
        let store = fx.faulty();
        let metas = three_items(&fx, &store).await;
        let before = store.fs().calls(FsOp::OpenRead);
        assert_eq!(store.get_meta(&metas[1].id).await.unwrap(), metas[1]);
        assert_eq!(store.fs().calls(FsOp::OpenRead) - before, 1);
        let unknown = ItemId::parse("00000001-000000000000").unwrap();
        assert!(matches!(
            store.get_meta(&unknown).await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn the_default_get_meta_agrees_with_get() {
        /// A store that relies on the trait's default `get_meta`.
        struct DefaultMeta(FsStore<LocalFs>);

        #[async_trait::async_trait]
        impl Store for DefaultMeta {
            async fn put(
                &self,
                item: NewItem,
                content: crate::fs::BoxRead,
            ) -> Result<crate::store::PutOutcome, StoreError> {
                self.0.put(item, content).await
            }
            async fn list(&self) -> Result<Vec<ItemMeta>, StoreError> {
                self.0.list().await
            }
            async fn list_after(
                &self,
                after: Option<&ItemId>,
            ) -> Result<Vec<ItemMeta>, StoreError> {
                self.0.list_after(after).await
            }
            async fn get(&self, id: &ItemId) -> Result<(ItemMeta, crate::fs::BoxRead), StoreError> {
                self.0.get(id).await
            }
            async fn exists(&self, id: &ItemId) -> Result<bool, StoreError> {
                self.0.exists(id).await
            }
            async fn find_by_content_key(
                &self,
                key: &crate::model::ContentKey,
            ) -> Result<Option<ItemMeta>, StoreError> {
                self.0.find_by_content_key(key).await
            }
            async fn resolve(&self, input: &str) -> Result<ItemId, StoreError> {
                self.0.resolve(input).await
            }
            async fn delete(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
                self.0.delete(id).await
            }
            async fn clean_staging(
                &self,
                older_than: std::time::Duration,
            ) -> Result<usize, StoreError> {
                self.0.clean_staging(older_than).await
            }
        }

        let fx = Fixture::new();
        let store = DefaultMeta(fx.store());
        let metas = three_items(&fx, &store).await;
        assert_eq!(
            store.get_meta(&metas[0].id).await.unwrap(),
            store.get(&metas[0].id).await.unwrap().0
        );
        let listed: Vec<ItemId> = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            store.list_ids().await.unwrap(),
            listed,
            "the default list_ids agrees with list"
        );
        let unknown = ItemId::parse("00000001-000000000000").unwrap();
        assert!(matches!(
            store.get_meta(&unknown).await,
            Err(StoreError::NotFound(_))
        ));
        assert_eq!(
            store.probe_write().await.unwrap(),
            crate::store::WriteProbe::NotSupported,
            "the default probe_write writes nothing"
        );
    }

    #[tokio::test]
    async fn probe_write_leaves_items_and_staging_as_they_were() {
        let fx = Fixture::new();
        let store = fx.faulty();
        three_items(&fx, &store).await;
        let (items, tmp) = (fx.item_dirs(), tmp_entries(&fx));
        let writes = store.fs().calls(FsOp::OpenWrite);
        assert_eq!(
            store.probe_write().await.unwrap(),
            crate::store::WriteProbe::Verified
        );
        assert_eq!(
            store.fs().calls(FsOp::OpenWrite) - writes,
            1,
            "one probe file is written"
        );
        assert_eq!(fx.item_dirs(), items);
        assert_eq!(tmp_entries(&fx), tmp);
        assert_eq!(store.list().await.unwrap().len(), 3);
    }

    #[test]
    fn the_probe_is_probe_bytes_long() {
        assert_eq!(probe_content().len(), crate::store::PROBE_BYTES);
        assert!(probe_content().starts_with(b"passalong write probe"));
    }

    #[tokio::test]
    async fn a_failed_probe_is_reported_and_cleaned_up() {
        let fx = Fixture::new();
        let store = fx.faulty();
        store.fs().fail_next(FsOp::OpenWrite, 1);
        assert!(store.probe_write().await.is_err());
        assert!(fx.tmp_is_empty(), "{:?}", tmp_entries(&fx));
        store.fs().fail_next(FsOp::CreateDirAll, 1);
        assert!(store.probe_write().await.is_err());
        store.fs().fail_next(FsOp::RemoveDirAll, 1);
        assert!(
            store.probe_write().await.is_err(),
            "a probe that cannot be removed is an error"
        );
    }

    #[tokio::test]
    async fn list_ids_reads_one_directory_and_no_metadata() {
        let fx = Fixture::new();
        let store = fx.faulty();
        let metas = three_items(&fx, &store).await;
        let (dirs, reads) = (
            store.fs().calls(FsOp::ReadDir),
            store.fs().calls(FsOp::OpenRead),
        );
        let ids = store.list_ids().await.unwrap();
        let newest_first: Vec<ItemId> = metas.iter().rev().map(|m| m.id.clone()).collect();
        assert_eq!(ids, newest_first);
        assert_eq!(store.fs().calls(FsOp::ReadDir) - dirs, 1);
        assert_eq!(store.fs().calls(FsOp::OpenRead) - reads, 0);
    }
}

#[cfg(test)]
mod sealed_tests {
    //! A sealed store behaves as a plaintext one, under `v2/`, and leaves
    //! nothing readable on the storage.

    use super::*;
    use crate::crypto::{CHUNK_LEN, DataKey};
    use crate::fs::LocalFs;
    use crate::model::{ContentHasher, NewItem};
    use crate::random::StdRandom;
    use crate::store::{Store, StoreError};
    use crate::testing::{FaultyFs, FsOp, ManualClock};
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tempfile::TempDir;
    use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

    const T: &str = "2026-09-12T09:53:11Z";
    /// Longer than the grace period for incomplete items.
    const LATER: i64 = 10 * 60;

    struct Fixture {
        dir: TempDir,
        clock: Arc<ManualClock>,
        sealer: Sealer,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: TempDir::new().unwrap(),
                clock: Arc::new(ManualClock::at(T)),
                sealer: Sealer::new(DataKey::generate().unwrap()),
            }
        }
        fn store(&self) -> FsStore<LocalFs> {
            FsStore::sealed(
                LocalFs::new(self.dir.path()),
                self.clock.clone(),
                Box::new(StdRandom::new()),
                self.sealer.clone(),
            )
        }
        fn faulty(&self) -> FsStore<FaultyFs<LocalFs>> {
            FsStore::sealed(
                FaultyFs::new(LocalFs::new(self.dir.path())),
                self.clock.clone(),
                Box::new(StdRandom::new()),
                self.sealer.clone(),
            )
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.dir.path().join(rel)
        }
        fn item_file(&self, id: &ItemId, file: &str) -> PathBuf {
            self.path(&format!("v2/items/{id}/{file}"))
        }
        fn entries(&self, rel: &str) -> Vec<String> {
            let Ok(rd) = std::fs::read_dir(self.path(rel)) else {
                return vec![];
            };
            let mut names: Vec<String> = rd
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }
    }

    fn content(bytes: &[u8]) -> BoxRead {
        Box::new(std::io::Cursor::new(bytes.to_vec()))
    }

    fn digest(bytes: &[u8]) -> ContentDigest {
        let mut hasher = ContentHasher::new();
        hasher.update(bytes);
        hasher.finalize()
    }

    async fn read_all(mut reader: BoxRead) -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).await?;
        Ok(buf)
    }

    async fn get_err(store: &impl Store, id: &ItemId) -> StoreError {
        match store.get(id).await {
            Ok(_) => panic!("{id} opened"),
            Err(err) => err,
        }
    }

    fn is_incomplete(err: &StoreError) -> bool {
        matches!(
            err,
            StoreError::Encryption(EncryptionError::Incomplete { .. })
        )
    }

    /// Every name below `root`, and every file's bytes.
    fn everything(root: &Path) -> (Vec<String>, Vec<Vec<u8>>) {
        let (mut names, mut blobs) = (Vec::new(), Vec::new());
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                names.push(path.file_name().unwrap().to_string_lossy().into_owned());
                if path.is_dir() {
                    pending.push(path);
                } else {
                    blobs.push(std::fs::read(&path).unwrap());
                }
            }
        }
        (names, blobs)
    }

    fn holds(haystack: &[u8], needle: &str) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    /// Yields some bytes, then fails like a dropped connection.
    struct Broken {
        ok: usize,
    }

    impl AsyncRead for Broken {
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
    async fn items_are_sealed_under_v2_with_keyed_ids() {
        let fx = Fixture::new();
        let store = fx.store();
        let out = store
            .put(NewItem::text("box"), content(b"hello sealed"))
            .await
            .unwrap();
        assert!(out.created);
        let id = &out.meta.id;
        assert_eq!(fx.entries(""), ["v2"]);
        assert_eq!(fx.entries("v2/items"), [id.to_string()]);
        let plain_key = digest(b"hello sealed").content_key();
        assert_eq!(
            id.content_key(),
            &fx.sealer.content_key(&digest(b"hello sealed"))
        );
        assert_ne!(id.content_key(), &plain_key);
        assert_eq!(
            &store.content_key(&digest(b"hello sealed")),
            id.content_key()
        );
        assert_eq!(store.key_id(), Some(fx.sealer.key_id()));

        let raw = std::fs::read(fx.item_file(id, "content")).unwrap();
        assert_eq!(&raw[..4], b"PAC1");
        assert_eq!(raw.len() as u64, crypto::sealed_len(12));
        let meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fx.item_file(id, "meta.json")).unwrap()).unwrap();
        assert_eq!(meta["schema"], 2);
        assert_eq!(meta["id"], id.as_str());
        assert_eq!(meta.as_object().unwrap().len(), 4, "{meta}");

        let (got, reader) = store.get(id).await.unwrap();
        assert_eq!(got, out.meta);
        assert_eq!(read_all(reader).await.unwrap(), b"hello sealed");
        assert_eq!(store.get_meta(id).await.unwrap(), out.meta);
        assert_eq!(store.list().await.unwrap(), vec![out.meta.clone()]);
        assert!(fx.entries("v2/tmp").is_empty());
    }

    #[tokio::test]
    async fn nothing_readable_is_left_on_the_storage() {
        let fx = Fixture::new();
        let store = fx.store();
        let device = "secret-device-a1";
        let text = store
            .put(
                NewItem::text(device),
                content(b"marker-text-3f9a in the clipboard"),
            )
            .await
            .unwrap()
            .meta;
        fx.clock.advance(1);
        let file = store
            .put(
                NewItem::file("secret-name-b2.txt", device),
                content(b"marker-file-77c1 contents"),
            )
            .await
            .unwrap()
            .meta;
        fx.clock.advance(1);
        let image = store
            .put(
                NewItem::clipboard_image(device, fx.clock.now()),
                content(b"\x89PNG marker-image-0b2e"),
            )
            .await
            .unwrap()
            .meta;

        let mut needles: Vec<String> = [
            "marker-text-3f9a",
            "marker-file-77c1",
            "marker-image-0b2e",
            "secret-name-b2",
            device,
            "text/plain",
            "image/png",
            "clipboard",
        ]
        .map(str::to_owned)
        .to_vec();
        for meta in [&text, &file, &image] {
            needles.push(meta.sha256.clone());
            needles.push(meta.sha256[..12].to_owned());
            needles.extend(meta.preview.clone());
        }
        let (names, blobs) = everything(fx.dir.path());
        for needle in &needles {
            assert!(
                names.iter().all(|name| !name.contains(needle.as_str())),
                "a name contains {needle}"
            );
            assert!(
                blobs.iter().all(|blob| !holds(blob, needle)),
                "a file contains {needle}"
            );
        }
    }

    #[tokio::test]
    async fn identical_content_is_stored_once() {
        let fx = Fixture::new();
        let store = fx.store();
        let first = store
            .put(NewItem::text("a"), content(b"same"))
            .await
            .unwrap();
        fx.clock.advance(5);
        let second = store
            .put(NewItem::text("b"), content(b"same"))
            .await
            .unwrap();
        assert!(!second.created);
        assert_eq!(second.meta, first.meta);
        assert_eq!(fx.entries("v2/items").len(), 1);
        assert!(fx.entries("v2/tmp").is_empty());
    }

    #[tokio::test]
    async fn lookups_work_on_keyed_ids() {
        let fx = Fixture::new();
        let store = fx.store();
        let mut metas = Vec::new();
        for text in ["one", "two", "three"] {
            metas.push(
                store
                    .put(NewItem::text("box"), content(text.as_bytes()))
                    .await
                    .unwrap()
                    .meta,
            );
            fx.clock.advance(1);
        }
        let ids: Vec<ItemId> = metas.iter().rev().map(|m| m.id.clone()).collect();
        let listed: Vec<ItemId> = store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(listed, ids);
        assert_eq!(store.list_ids().await.unwrap(), ids);
        let newer: Vec<ItemId> = store
            .list_after(Some(&metas[0].id))
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(newer, ids[..2]);

        let first = &metas[0].id;
        assert!(store.exists(first).await.unwrap());
        assert!(
            !store
                .exists(&ItemId::parse("6aa52107-000000000000").unwrap())
                .await
                .unwrap()
        );
        assert_eq!(
            store.resolve(first.content_key().as_str()).await.unwrap(),
            *first
        );
        assert_eq!(store.resolve(first.as_str()).await.unwrap(), *first);
        let keyed = store.content_key(&digest(b"one"));
        assert_eq!(
            store.find_by_content_key(&keyed).await.unwrap(),
            Some(metas[0].clone())
        );
        assert_eq!(
            store
                .find_by_content_key(&digest(b"one").content_key())
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn delete_probe_and_staging_use_v2_tmp() {
        let fx = Fixture::new();
        let store = fx.store();
        let meta = store
            .put(NewItem::text("box"), content(b"gone soon"))
            .await
            .unwrap()
            .meta;
        assert_eq!(store.delete(&meta.id).await.unwrap(), meta);
        assert!(matches!(
            store.get_meta(&meta.id).await,
            Err(StoreError::NotFound(_))
        ));
        assert!(fx.entries("v2/items").is_empty());

        assert_eq!(store.probe_write().await.unwrap(), WriteProbe::Verified);
        assert!(fx.entries("v2/tmp").is_empty());

        std::fs::create_dir_all(fx.path("v2/tmp/stale")).unwrap();
        // Left in the plaintext staging before encryption: cleaned too.
        std::fs::create_dir_all(fx.path("tmp/plain-leftover")).unwrap();
        fx.clock.advance(400 * 24 * 3600);
        assert_eq!(
            store.clean_staging(Duration::from_secs(60)).await.unwrap(),
            2
        );
        assert!(fx.entries("v2/tmp").is_empty());
        assert!(fx.entries("tmp").is_empty());
    }

    #[tokio::test]
    async fn content_swapped_between_items_does_not_open() {
        let fx = Fixture::new();
        let store = fx.store();
        let a = store
            .put(NewItem::text("box"), content(b"aaaa"))
            .await
            .unwrap()
            .meta;
        fx.clock.advance(1);
        let b = store
            .put(NewItem::text("box"), content(b"bbbb"))
            .await
            .unwrap()
            .meta;
        let (pa, pb) = (
            fx.item_file(&a.id, "content"),
            fx.item_file(&b.id, "content"),
        );
        let (ca, cb) = (std::fs::read(&pa).unwrap(), std::fs::read(&pb).unwrap());
        std::fs::write(&pa, cb).unwrap();
        std::fs::write(&pb, ca).unwrap();
        let (_, reader) = store.get(&a.id).await.unwrap();
        let err = read_all(reader).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
    }

    #[tokio::test]
    async fn metadata_moved_or_relabelled_to_another_item_is_refused() {
        let fx = Fixture::new();
        let store = fx.store();
        let a = store
            .put(NewItem::text("box"), content(b"first"))
            .await
            .unwrap()
            .meta;
        fx.clock.advance(1);
        let b = store
            .put(NewItem::text("box"), content(b"second"))
            .await
            .unwrap()
            .meta;
        let b_meta = std::fs::read_to_string(fx.item_file(&b.id, "meta.json")).unwrap();

        std::fs::write(fx.item_file(&a.id, "meta.json"), &b_meta).unwrap();
        assert!(matches!(
            store.get_meta(&a.id).await,
            Err(StoreError::Corrupt { .. })
        ));

        // The id says `a`, but the sealed part was sealed for `b`.
        let relabelled = b_meta.replace(b.id.as_str(), a.id.as_str());
        std::fs::write(fx.item_file(&a.id, "meta.json"), relabelled).unwrap();
        assert!(is_incomplete(&store.get_meta(&a.id).await.unwrap_err()));
        fx.clock.advance(LATER);
        assert!(matches!(
            store.get_meta(&a.id).await,
            Err(StoreError::Corrupt { .. })
        ));
        assert_eq!(store.list().await.unwrap(), vec![b]);
    }

    #[tokio::test]
    async fn another_key_cannot_read_the_items() {
        let fx = Fixture::new();
        let meta = fx
            .store()
            .put(NewItem::text("box"), content(b"private"))
            .await
            .unwrap()
            .meta;
        let stranger = FsStore::sealed(
            LocalFs::new(fx.dir.path()),
            fx.clock.clone(),
            Box::new(StdRandom::new()),
            Sealer::new(DataKey::generate().unwrap()),
        );
        fx.clock.advance(LATER);
        assert!(stranger.list().await.unwrap().is_empty());
        assert!(matches!(
            stranger.get_meta(&meta.id).await,
            Err(StoreError::Corrupt { .. })
        ));
    }

    #[tokio::test]
    async fn recent_items_that_do_not_open_are_not_complete_yet() {
        let fx = Fixture::new();
        let store = fx.store();
        let x = store
            .put(NewItem::text("box"), content(&[b'x'; 100]))
            .await
            .unwrap()
            .meta;
        fx.clock.advance(1);
        let y = store
            .put(NewItem::text("box"), content(&[b'y'; 100]))
            .await
            .unwrap()
            .meta;
        // x's content and y's metadata are still arriving.
        let x_content = std::fs::OpenOptions::new()
            .write(true)
            .open(fx.item_file(&x.id, "content"))
            .unwrap();
        x_content.set_len(50).unwrap();
        let y_meta = std::fs::read(fx.item_file(&y.id, "meta.json")).unwrap();
        std::fs::write(fx.item_file(&y.id, "meta.json"), &y_meta[..20]).unwrap();

        assert!(is_incomplete(&get_err(&store, &x.id).await));
        assert!(is_incomplete(&store.get_meta(&y.id).await.unwrap_err()));
        assert_eq!(store.list().await.unwrap(), vec![x.clone()]);

        fx.clock.advance(LATER);
        assert!(matches!(
            get_err(&store, &x.id).await,
            StoreError::Corrupt { .. }
        ));
        assert!(matches!(
            store.get_meta(&y.id).await,
            Err(StoreError::Corrupt { .. })
        ));
        std::fs::remove_file(fx.item_file(&x.id, "content")).unwrap();
        assert!(matches!(
            get_err(&store, &x.id).await,
            StoreError::Corrupt { .. }
        ));
    }

    #[tokio::test]
    async fn large_content_round_trips_across_chunks() {
        let fx = Fixture::new();
        let store = fx.store();
        let data: Vec<u8> = (0..3 * CHUNK_LEN + 123).map(|i| (i % 251) as u8).collect();
        let meta = store
            .put(NewItem::file("big.bin", "box"), content(&data))
            .await
            .unwrap()
            .meta;
        assert_eq!(meta.size, data.len() as u64);
        let raw = std::fs::read(fx.item_file(&meta.id, "content")).unwrap();
        assert_eq!(raw.len() as u64, crypto::sealed_len(meta.size));
        let (_, reader) = store.get(&meta.id).await.unwrap();
        assert_eq!(read_all(reader).await.unwrap(), data);
    }

    #[tokio::test]
    async fn failed_uploads_leave_no_item_and_no_staging() {
        let fx = Fixture::new();
        let store = fx.faulty();
        store.fs().fail_next(FsOp::Rename, 1);
        assert!(
            store
                .put(NewItem::text("box"), content(b"never"))
                .await
                .is_err()
        );
        assert!(
            store
                .put(NewItem::file("f", "box"), Box::new(Broken { ok: 5000 }))
                .await
                .is_err()
        );
        assert!(fx.entries("v2/items").is_empty());
        assert!(fx.entries("v2/tmp").is_empty());
    }

    #[tokio::test]
    async fn a_plaintext_store_keeps_its_layout_and_plain_keys() {
        let fx = Fixture::new();
        let store = FsStore::new(
            LocalFs::new(fx.dir.path()),
            fx.clock.clone(),
            Box::new(StdRandom::new()),
        );
        let meta = store
            .put(NewItem::text("box"), content(b"plain"))
            .await
            .unwrap()
            .meta;
        assert_eq!(fx.entries(""), ["items", "tmp"]);
        assert_eq!(store.key_id(), None);
        assert!(store.sealer().is_none());
        assert_eq!(
            store.content_key(&digest(b"plain")),
            digest(b"plain").content_key()
        );
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fx.path(&format!("items/{}/meta.json", meta.id))).unwrap(),
        )
        .unwrap();
        assert_eq!(json["schema"], 1);
        assert_eq!(json["preview"], "plain");
    }
}

/// The bytes an item is stored as, pinned before they moved to
/// `store::format` (PLAN-00010 STEP-01), so that moving them, and a second
/// store writing them, cannot change them. Sealing draws fresh salts and
/// nonces, so sealed items are pinned by their shape and by what they open
/// to.
#[cfg(test)]
mod golden_tests {
    use super::*;
    use crate::crypto::{CHUNK_LEN, DataKey, HEADER_LEN};
    use crate::fs::LocalFs;
    use crate::random::StdRandom;
    use crate::store::format::SealedMetaFile;
    use crate::testing::ManualClock;
    use tempfile::TempDir;

    const T: &str = "2026-09-12T09:53:11Z";

    fn content(bytes: &[u8]) -> BoxRead {
        Box::new(std::io::Cursor::new(bytes.to_vec()))
    }

    fn store(dir: &TempDir, sealer: Option<Sealer>) -> FsStore<LocalFs> {
        let fs = LocalFs::new(dir.path());
        let clock = Arc::new(ManualClock::at(T));
        let rng = Box::new(StdRandom::new());
        match sealer {
            Some(sealer) => FsStore::sealed(fs, clock, rng, sealer),
            None => FsStore::new(fs, clock, rng),
        }
    }

    fn stored(dir: &TempDir, items: &str, id: &ItemId, file: &str) -> Vec<u8> {
        std::fs::read(dir.path().join(items).join(id.as_str()).join(file)).unwrap()
    }

    #[tokio::test]
    async fn plaintext_meta_json_is_byte_for_byte_unchanged() {
        let dir = TempDir::new().unwrap();
        let store = store(&dir, None);
        let text = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap()
            .meta;
        let file = store
            .put(
                NewItem::file("report.pdf", "laptop"),
                content(b"%PDF-1.7 x"),
            )
            .await
            .unwrap()
            .meta;
        let text_json = String::from_utf8(stored(&dir, ITEMS_DIR, &text.id, META_FILE)).unwrap();
        let file_json = String::from_utf8(stored(&dir, ITEMS_DIR, &file.id, META_FILE)).unwrap();
        assert_eq!(text_json, GOLDEN_TEXT_META, "{text_json}");
        assert_eq!(file_json, GOLDEN_FILE_META, "{file_json}");
        assert_eq!(stored(&dir, ITEMS_DIR, &text.id, CONTENT_FILE), b"hello");
    }

    const GOLDEN_TEXT_META: &str = r#"{
  "schema": 1,
  "id": "6aa52107-2cf24dba5fb0",
  "kind": "text",
  "name": null,
  "mime": "text/plain; charset=utf-8",
  "size": 5,
  "sha256": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
  "created_at": "2026-09-12T09:53:11Z",
  "device": "box",
  "preview": "hello"
}
"#;
    const GOLDEN_FILE_META: &str = r#"{
  "schema": 1,
  "id": "6aa52107-ec2792f52c44",
  "kind": "file",
  "name": "report.pdf",
  "mime": "application/pdf",
  "size": 10,
  "sha256": "ec2792f52c4416be86b9c3d9f4ce1122e68027a8549c54be5040c913d671aa02",
  "created_at": "2026-09-12T09:53:11Z",
  "device": "laptop",
  "preview": null
}
"#;

    #[tokio::test]
    async fn sealed_meta_json_keeps_its_shape_and_its_sealed_body() {
        let dir = TempDir::new().unwrap();
        let sealer = Sealer::new(DataKey::generate().unwrap());
        let store = store(&dir, Some(sealer.clone()));
        let meta = store
            .put(NewItem::text("box"), content(b"hello"))
            .await
            .unwrap()
            .meta;
        let json = String::from_utf8(stored(&dir, SEALED_ITEMS_DIR, &meta.id, META_FILE)).unwrap();
        let file: SealedMetaFile = serde_json::from_str(&json).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\n  \"schema\": 2,\n  \"id\": \"{}\",\n  \"nonce\": \"{}\",\n  \"sealed\": \"{}\"\n}}\n",
                meta.id, file.nonce, file.sealed
            )
        );
        assert_eq!(file.nonce.len(), 2 * crypto::NONCE_LEN);
        let body = sealer.open_meta(&meta.id, &file.sealed().unwrap()).unwrap();
        let sealed_content = stored(&dir, SEALED_ITEMS_DIR, &meta.id, CONTENT_FILE);
        let salt = hex::encode(&sealed_content[crypto::CONTENT_MAGIC.len()..HEADER_LEN]);
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            format!(
                "{{\"meta\":{},\"content_salt\":\"{salt}\"}}",
                serde_json::to_string(&meta).unwrap()
            )
        );
    }

    #[tokio::test]
    async fn sealed_content_is_framed_as_before() {
        for len in [0, 5, CHUNK_LEN, 2 * CHUNK_LEN + 1] {
            let dir = TempDir::new().unwrap();
            let sealer = Sealer::new(DataKey::generate().unwrap());
            let store = store(&dir, Some(sealer.clone()));
            let plain: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let meta = store
                .put(NewItem::file("f.bin", "box"), content(&plain))
                .await
                .unwrap()
                .meta;
            let sealed = stored(&dir, SEALED_ITEMS_DIR, &meta.id, CONTENT_FILE);
            assert_eq!(sealed.len() as u64, crypto::sealed_len(len as u64), "{len}");
            assert_eq!(&sealed[..4], &crypto::CONTENT_MAGIC);
            let salt: [u8; CONTENT_SALT_LEN] = sealed[4..HEADER_LEN].try_into().unwrap();
            let mut opened = Vec::new();
            sealer
                .open_item_content(std::io::Cursor::new(sealed), salt)
                .read_to_end(&mut opened)
                .await
                .unwrap();
            assert_eq!(opened, plain, "{len}");
        }
    }
}
