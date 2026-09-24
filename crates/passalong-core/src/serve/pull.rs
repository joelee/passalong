//! Pull mode: applying items that other devices send.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

use crate::clipboard::{RgbaImage, decode_png};
use crate::crypto::{CryptoError, KeyId};
use crate::download::{self, DownloadError};
use crate::encryption::EncryptionError;
use crate::model::{ItemId, ItemKind, ItemMeta, sanitise_file_name};
use crate::serve::{StoreOpener, stopped};
use crate::store::fs_store::INCOMPLETE_GRACE_SECS;
use crate::store::{Store, StoreError};

/// Content for the clipboard task to put on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardWrite {
    /// Text.
    Text(String),
    /// An image.
    Image(RgbaImage),
}

/// Applies new items from other devices, remembering which item ids it has
/// already handled. Ids decide nothing about order across devices, so an
/// item from a device whose clock runs behind is still applied.
pub struct Puller {
    device: String,
    download_dir: PathBuf,
    seen: HashSet<ItemId>,
    /// The store's key when `seen` was taken; a new key means new ids.
    key_id: Option<KeyId>,
    clipboard: Option<mpsc::Sender<ClipboardWrite>>,
    warned_no_dir: bool,
    warned_no_clipboard: bool,
}

impl Puller {
    /// A puller for `device` that ignores everything already stored. Files
    /// go to `download_dir` when it exists; text and images go to
    /// `clipboard`, or are skipped without one.
    ///
    /// # Errors
    ///
    /// The store's error when its items cannot be listed.
    pub async fn start(
        store: &dyn Store,
        device: String,
        download_dir: PathBuf,
        clipboard: Option<mpsc::Sender<ClipboardWrite>>,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            device,
            download_dir,
            seen: store.list_ids().await?.into_iter().collect(),
            key_id: store.key_id(),
            clipboard,
            warned_no_dir: false,
            warned_no_clipboard: false,
        })
    }

    /// Handles every item not seen before, oldest id first: files from
    /// other devices are downloaded, and the newest text or clipboard image
    /// among them goes to the clipboard. One directory listing finds the
    /// new ids; only their metadata is read. Ids that left the store are
    /// forgotten. Local problems are logged and the item is skipped.
    ///
    /// # Errors
    ///
    /// A store error. Items not yet handled stay unseen, so the next poll
    /// picks them up without repeating anything already done.
    pub async fn poll(&mut self, store: &dyn Store) -> Result<(), StoreError> {
        let ids = store.list_ids().await?;
        if store.key_id() != self.key_id {
            // The store was migrated or its key rotated, so every id is new;
            // starting from its current items avoids applying them all again.
            tracing::info!(
                items = ids.len(),
                "the store's key changed; pull mode starts from its current items"
            );
            self.key_id = store.key_id();
            self.seen = ids.into_iter().collect();
            return Ok(());
        }
        let present: HashSet<&ItemId> = ids.iter().collect();
        self.seen.retain(|id| present.contains(id));
        let unseen: Vec<ItemId> = ids
            .iter()
            .filter(|id| !self.seen.contains(*id))
            .cloned()
            .collect();
        let mut new = Vec::new();
        for id in &unseen {
            match store.get_meta(id).await {
                Ok(meta) => new.push(meta),
                // Deleted or damaged since it was listed: nothing to apply.
                Err(err @ (StoreError::NotFound(_) | StoreError::Corrupt { .. })) => {
                    tracing::warn!(id = %id, error = %err, "skipping an unreadable item");
                    self.seen.insert(id.clone());
                }
                // Still arriving through a synced folder: tried again at the
                // next poll, without holding up the items after it.
                Err(StoreError::Encryption(EncryptionError::Incomplete { .. })) => {
                    tracing::debug!(id = %id, "item not complete yet; trying again at the next poll");
                }
                Err(err) => return Err(err),
            }
        }
        // `new` is newest first, like the listing.
        let newest_for_clipboard = new
            .iter()
            .find(|meta| self.is_foreign(meta) && for_clipboard(meta))
            .map(|meta| meta.id.clone());
        for meta in new.iter().rev() {
            let handled = if !self.is_foreign(meta) {
                Handled::Done
            } else if !for_clipboard(meta) {
                self.download(store, meta).await?
            } else if newest_for_clipboard.as_ref() == Some(&meta.id) {
                self.apply(store, meta).await?
            } else {
                Handled::Done
            };
            if handled == Handled::Done {
                self.seen.insert(meta.id.clone());
            }
        }
        Ok(())
    }

    /// How many ids are remembered.
    #[cfg(test)]
    pub(crate) fn seen_len(&self) -> usize {
        self.seen.len()
    }

    fn is_foreign(&self, meta: &ItemMeta) -> bool {
        meta.device != self.device
    }

    async fn apply(&mut self, store: &dyn Store, meta: &ItemMeta) -> Result<Handled, StoreError> {
        let Some(clipboard) = self.clipboard.clone() else {
            if !self.warned_no_clipboard {
                tracing::warn!("no clipboard available; pulled text and images are skipped");
                self.warned_no_clipboard = true;
            }
            return Ok(Handled::Done);
        };
        let bytes = match fetch(store, meta).await? {
            Fetched::Bytes(bytes) => bytes,
            Fetched::Not(handled) => return Ok(handled),
        };
        let write = if meta.kind == ItemKind::Text {
            match String::from_utf8(bytes) {
                Ok(text) => ClipboardWrite::Text(text),
                Err(_) => {
                    tracing::warn!(id = %meta.id, "pulled text is not UTF-8; skipped");
                    return Ok(Handled::Done);
                }
            }
        } else {
            match decode_png(&bytes) {
                Ok(image) => ClipboardWrite::Image(image),
                Err(err) => {
                    tracing::warn!(id = %meta.id, error = %err, "pulled image is unusable; skipped");
                    return Ok(Handled::Done);
                }
            }
        };
        tracing::info!(id = %meta.id, device = meta.device.as_str(), "pulled to the clipboard");
        // Only fails when serve is stopping.
        let _ = clipboard.send(write).await;
        Ok(Handled::Done)
    }

    async fn download(
        &mut self,
        store: &dyn Store,
        meta: &ItemMeta,
    ) -> Result<Handled, StoreError> {
        let exists = tokio::fs::metadata(&self.download_dir)
            .await
            .is_ok_and(|metadata| metadata.is_dir());
        if !exists {
            if !self.warned_no_dir {
                tracing::warn!(
                    path = %self.download_dir.display(),
                    "the download directory does not exist; pulled files are skipped"
                );
                self.warned_no_dir = true;
            }
            return Ok(Handled::Done);
        }
        self.warned_no_dir = false;
        let name = match sanitise_file_name(meta.name.as_deref().unwrap_or_default()) {
            Ok(name) => name,
            Err(err) => {
                tracing::warn!(id = %meta.id, error = %err, "pulled file has no usable name; skipped");
                return Ok(Handled::Done);
            }
        };
        let content = match store.get(&meta.id).await {
            Ok((_, content)) => content,
            Err(err) => return unopened(meta, err),
        };
        match download::download_into(content, meta, &self.download_dir, &name).await {
            Ok(target) => tracing::info!(id = %meta.id, path = %target.display(), "pulled file"),
            Err(DownloadError::Read(err)) => return read_failed(meta, &err),
            Err(err) => {
                tracing::warn!(id = %meta.id, error = %err, "cannot save the pulled file; skipped");
            }
        }
        Ok(Handled::Done)
    }
}

/// Whether an item was dealt with, or is to be tried again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handled {
    /// Applied, or skipped for good.
    Done,
    /// Not complete yet: tried again at the next poll.
    Later,
}

/// An item's verified content, or what became of the item instead.
enum Fetched {
    Bytes(Vec<u8>),
    Not(Handled),
}

/// Text and clipboard images go to the clipboard; everything else is a file.
fn for_clipboard(meta: &ItemMeta) -> bool {
    meta.kind == ItemKind::Text || meta.is_clipboard_image()
}

/// The item's content, verified; after a warning, [`Handled::Done`] when
/// it is damaged, and [`Handled::Later`] while it is still arriving.
async fn fetch(store: &dyn Store, meta: &ItemMeta) -> Result<Fetched, StoreError> {
    let content = match store.get(&meta.id).await {
        Ok((_, content)) => content,
        Err(err) => return unopened(meta, err).map(Fetched::Not),
    };
    match download::read_verified(content, meta).await {
        Ok(bytes) => Ok(Fetched::Bytes(bytes)),
        Err(DownloadError::Read(err)) => read_failed(meta, &err).map(Fetched::Not),
        Err(err) => {
            tracing::warn!(id = %meta.id, error = %err, "pulled item is damaged; skipped");
            Ok(Fetched::Not(Handled::Done))
        }
    }
}

/// An item whose content did not open: still arriving, gone or damaged, or
/// a store error, retried later.
fn unopened(meta: &ItemMeta, err: StoreError) -> Result<Handled, StoreError> {
    match err {
        StoreError::Encryption(EncryptionError::Incomplete { .. }) => {
            tracing::debug!(id = %meta.id, "item not complete yet; trying again at the next poll");
            Ok(Handled::Later)
        }
        StoreError::NotFound(_) | StoreError::Corrupt { .. } => {
            tracing::warn!(id = %meta.id, error = %err, "skipping an unreadable item");
            Ok(Handled::Done)
        }
        err => Err(err),
    }
}

/// Reading an item failed part-way. Sealed content that does not open is
/// still arriving while the item is recent, and damaged after that; any
/// other failure is a store problem, retried later.
fn read_failed(meta: &ItemMeta, err: &std::io::Error) -> Result<Handled, StoreError> {
    let sealed = err
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<CryptoError>().is_some());
    if !sealed {
        return Err(StoreError::Backend(format!(
            "reading item {}: {err}",
            meta.id
        )));
    }
    if meta.id.timestamp() > Utc::now() - TimeDelta::seconds(INCOMPLETE_GRACE_SECS) {
        tracing::debug!(id = %meta.id, error = %err, "item not complete yet; trying again at the next poll");
        Ok(Handled::Later)
    } else {
        tracing::warn!(id = %meta.id, error = %err, "pulled item is damaged; skipped");
        Ok(Handled::Done)
    }
}

/// Polls every `interval` until stopped, reopening the store after a
/// failure. Each distinct failure is logged once.
pub(crate) async fn pull_loop(
    mut puller: Puller,
    mut store: Box<dyn Store>,
    open_store: StoreOpener,
    interval: Duration,
    mut shutdown: watch::Receiver<bool>,
    denied: mpsc::UnboundedSender<String>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_error: Option<String> = None;
    loop {
        tokio::select! {
            () = stopped(&mut shutdown) => return,
            _ = ticker.tick() => {}
        }
        match puller.poll(store.as_ref()).await {
            Ok(()) => last_error = None,
            Err(StoreError::Denied(message)) => {
                let _ = denied.send(message);
                return;
            }
            Err(err) => {
                let message = err.to_string();
                if last_error.as_deref() != Some(message.as_str()) {
                    tracing::warn!(
                        error = message.as_str(),
                        "pull failed; retrying at the next check"
                    );
                    last_error = Some(message);
                }
                match open_store().await {
                    Ok(fresh) => store = fresh,
                    Err(StoreError::Denied(message)) => {
                        let _ = denied.send(message);
                        return;
                    }
                    Err(err) => tracing::debug!(error = %err, "reopening the store failed"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{RgbaImage, encode_png};
    use crate::crypto::{DataKey, Sealer};
    use crate::fs::LocalFs;
    use crate::model::{ItemMeta, NewItem};
    use crate::random::StdRandom;
    use crate::store::FsStore;
    use crate::testing::{FaultyFs, FsOp, ManualClock};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    struct Rig {
        dir: TempDir,
        clock: Arc<ManualClock>,
        store: FsStore<FaultyFs<LocalFs>>,
    }

    impl Rig {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            std::fs::create_dir_all(dir.path().join("store")).unwrap();
            std::fs::create_dir_all(dir.path().join("downloads")).unwrap();
            let clock = Arc::new(ManualClock::at("2026-09-13T08:00:00Z"));
            let store = FsStore::new(
                FaultyFs::new(LocalFs::new(dir.path().join("store"))),
                clock.clone(),
                Box::new(StdRandom::new()),
            );
            Self { dir, clock, store }
        }
        fn downloads(&self) -> PathBuf {
            self.dir.path().join("downloads")
        }
        async fn put(&self, item: NewItem, data: &[u8]) -> ItemMeta {
            let content = Box::new(std::io::Cursor::new(data.to_vec()));
            let meta = self.store.put(item, content).await.unwrap().meta;
            self.clock.advance(1);
            meta
        }
        async fn puller(&self, clipboard: Option<mpsc::Sender<ClipboardWrite>>) -> Puller {
            Puller::start(&self.store, "me".into(), self.downloads(), clipboard)
                .await
                .unwrap()
        }
        fn downloaded(&self) -> Vec<String> {
            let Ok(entries) = std::fs::read_dir(self.downloads()) else {
                return Vec::new();
            };
            let mut names: Vec<String> = entries
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }
    }

    fn drain(rx: &mut mpsc::Receiver<ClipboardWrite>) -> Vec<ClipboardWrite> {
        let mut writes = Vec::new();
        while let Ok(write) = rx.try_recv() {
            writes.push(write);
        }
        writes
    }

    #[tokio::test]
    async fn a_new_key_starts_pull_mode_afresh_instead_of_applying_everything() {
        let rig = Rig::new();
        let mut puller = rig.puller(None).await;
        let sealed = FsStore::sealed(
            LocalFs::new(rig.dir.path().join("sealed")),
            rig.clock.clone(),
            Box::new(StdRandom::new()),
            Sealer::new(DataKey::generate().unwrap()),
        );
        let file = |data: &[u8]| Box::new(std::io::Cursor::new(data.to_vec()));
        sealed
            .put(NewItem::file("migrated.txt", "phone"), file(b"old"))
            .await
            .unwrap();
        rig.clock.advance(1);
        puller.poll(&sealed).await.unwrap();
        assert!(
            rig.downloaded().is_empty(),
            "migrated items are not applied again"
        );
        assert_eq!(puller.seen_len(), 1);
        sealed
            .put(NewItem::file("new.txt", "phone"), file(b"new"))
            .await
            .unwrap();
        puller.poll(&sealed).await.unwrap();
        assert_eq!(rig.downloaded(), ["new.txt"]);
    }

    fn flip_a_byte(path: &std::path::Path) {
        let mut bytes = std::fs::read(path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 1;
        std::fs::write(path, bytes).unwrap();
    }

    fn content(data: &[u8]) -> crate::fs::BoxRead {
        Box::new(std::io::Cursor::new(data.to_vec()))
    }

    #[tokio::test]
    async fn a_damaged_encrypted_item_is_skipped_and_the_items_after_it_still_arrive() {
        let rig = Rig::new();
        let root = rig.dir.path().join("sealed");
        // Created long enough ago that damage is not still-arriving data.
        let sealed = FsStore::sealed(
            LocalFs::new(root.clone()),
            rig.clock.clone(),
            Box::new(StdRandom::new()),
            Sealer::new(DataKey::generate().unwrap()),
        );
        let (tx, mut rx) = mpsc::channel(8);
        let mut puller = Puller::start(&sealed, "me".into(), rig.downloads(), Some(tx))
            .await
            .unwrap();
        let damaged = sealed
            .put(NewItem::file("damaged.txt", "phone"), content(b"flipped"))
            .await
            .unwrap()
            .meta;
        rig.clock.advance(1);
        sealed
            .put(NewItem::file("fine.txt", "phone"), content(b"fine"))
            .await
            .unwrap();
        rig.clock.advance(1);
        sealed
            .put(NewItem::text("phone"), content(b"words"))
            .await
            .unwrap();
        flip_a_byte(&root.join(format!("v2/items/{}/content", damaged.id)));
        puller.poll(&sealed).await.unwrap();
        assert_eq!(rig.downloaded(), ["fine.txt"]);
        assert_eq!(drain(&mut rx), [ClipboardWrite::Text("words".into())]);
        assert_eq!(puller.seen_len(), 3, "the damaged item is not tried again");
    }

    #[tokio::test]
    async fn a_recent_encrypted_item_that_does_not_open_is_tried_again_later() {
        let rig = Rig::new();
        let root = rig.dir.path().join("sealed");
        // Created now, so a failure may still be data arriving.
        let sealed = FsStore::sealed(
            LocalFs::new(root.clone()),
            Arc::new(crate::clock::SystemClock),
            Box::new(StdRandom::new()),
            Sealer::new(DataKey::generate().unwrap()),
        );
        let mut puller = Puller::start(&sealed, "me".into(), rig.downloads(), None)
            .await
            .unwrap();
        let arriving = sealed
            .put(NewItem::file("arriving.txt", "phone"), content(b"arriving"))
            .await
            .unwrap()
            .meta;
        sealed
            .put(NewItem::file("fine.txt", "phone"), content(b"fine"))
            .await
            .unwrap();
        let path = root.join(format!("v2/items/{}/content", arriving.id));
        flip_a_byte(&path);
        puller.poll(&sealed).await.unwrap();
        assert_eq!(rig.downloaded(), ["fine.txt"]);
        assert_eq!(puller.seen_len(), 1, "the arriving item stays unseen");
        // The rest of it arrives.
        flip_a_byte(&path);
        puller.poll(&sealed).await.unwrap();
        assert_eq!(rig.downloaded(), ["arriving.txt", "fine.txt"]);
    }

    #[tokio::test]
    async fn items_from_before_the_start_and_from_this_device_are_ignored() {
        let rig = Rig::new();
        rig.put(NewItem::text("phone"), b"before start").await;
        rig.put(NewItem::file("old.txt", "phone"), b"before start")
            .await;
        let (tx, mut rx) = mpsc::channel(8);
        let mut puller = rig.puller(Some(tx)).await;
        rig.put(NewItem::text("me"), b"mine").await;
        rig.put(NewItem::file("mine.txt", "me"), b"mine").await;
        puller.poll(&rig.store).await.unwrap();
        assert!(drain(&mut rx).is_empty());
        assert!(rig.downloaded().is_empty());
    }

    #[tokio::test]
    async fn only_the_newest_text_from_another_device_goes_to_the_clipboard() {
        let rig = Rig::new();
        let (tx, mut rx) = mpsc::channel(8);
        let mut puller = rig.puller(Some(tx)).await;
        rig.put(NewItem::text("phone"), b"first").await;
        rig.put(NewItem::text("phone"), b"second").await;
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(drain(&mut rx), [ClipboardWrite::Text("second".into())]);
        puller.poll(&rig.store).await.unwrap();
        assert!(drain(&mut rx).is_empty(), "nothing is applied twice");
    }

    #[tokio::test]
    async fn clipboard_images_are_decoded_for_the_clipboard() {
        let rig = Rig::new();
        let (tx, mut rx) = mpsc::channel(8);
        let mut puller = rig.puller(Some(tx)).await;
        let image = RgbaImage::new(1, 1, vec![1, 2, 3, 255]).unwrap();
        let at = "2026-09-13T08:00:00Z".parse().unwrap();
        rig.put(
            NewItem::clipboard_image("phone", at),
            &encode_png(&image).unwrap(),
        )
        .await;
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(drain(&mut rx), [ClipboardWrite::Image(image)]);
        assert!(rig.downloaded().is_empty());
    }

    #[tokio::test]
    async fn files_are_downloaded_into_an_existing_directory_without_overwriting() {
        let rig = Rig::new();
        std::fs::write(rig.downloads().join("report.pdf"), b"mine").unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let mut puller = rig.puller(Some(tx)).await;
        rig.put(NewItem::file("report.pdf", "phone"), b"theirs")
            .await;
        let png = encode_png(&RgbaImage::new(1, 1, vec![9, 9, 9, 255]).unwrap()).unwrap();
        rig.put(NewItem::file("photo.png", "phone"), &png).await;
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(
            rig.downloaded(),
            ["photo.png", "report (1).pdf", "report.pdf"]
        );
        assert_eq!(
            std::fs::read(rig.downloads().join("report (1).pdf")).unwrap(),
            b"theirs"
        );
        assert_eq!(
            std::fs::read(rig.downloads().join("report.pdf")).unwrap(),
            b"mine"
        );
        assert_eq!(
            std::fs::read(rig.downloads().join("photo.png")).unwrap(),
            png
        );
        assert!(drain(&mut rx).is_empty(), "files never go to the clipboard");
    }

    #[tokio::test]
    async fn without_a_download_directory_files_are_skipped_and_nothing_is_created() {
        let rig = Rig::new();
        std::fs::remove_dir(rig.downloads()).unwrap();
        let mut puller = rig.puller(None).await;
        rig.put(NewItem::file("a.txt", "phone"), b"a").await;
        puller.poll(&rig.store).await.unwrap();
        assert!(!rig.downloads().exists());
        std::fs::create_dir(rig.downloads()).unwrap();
        puller.poll(&rig.store).await.unwrap();
        assert!(
            rig.downloaded().is_empty(),
            "a skipped file is not fetched later"
        );
        rig.put(NewItem::file("b.txt", "phone"), b"b").await;
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(rig.downloaded(), ["b.txt"]);
    }

    #[tokio::test]
    async fn a_store_error_keeps_the_position_for_the_next_poll() {
        let rig = Rig::new();
        let (tx, mut rx) = mpsc::channel(8);
        let mut puller = rig.puller(Some(tx)).await;
        rig.put(NewItem::text("phone"), b"hello").await;
        rig.store.fs().fail_next(FsOp::ReadDir, 1);
        assert!(puller.poll(&rig.store).await.is_err());
        assert!(drain(&mut rx).is_empty());
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(drain(&mut rx), [ClipboardWrite::Text("hello".into())]);
    }

    #[tokio::test]
    async fn without_a_clipboard_text_is_skipped_and_files_still_arrive() {
        let rig = Rig::new();
        let mut puller = rig.puller(None).await;
        rig.put(NewItem::text("phone"), b"words").await;
        rig.put(NewItem::file("a.txt", "phone"), b"a").await;
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(rig.downloaded(), ["a.txt"]);
    }

    #[tokio::test]
    async fn an_item_from_a_device_with_a_slow_clock_is_applied_once() {
        let rig = Rig::new();
        let (tx, mut rx) = mpsc::channel(8);
        let mut puller = rig.puller(Some(tx)).await;
        rig.clock.advance(600);
        rig.put(NewItem::text("phone"), b"on time").await;
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(drain(&mut rx), [ClipboardWrite::Text("on time".into())]);
        // A device whose clock is an hour behind writes an older id.
        let slow = FsStore::new(
            LocalFs::new(rig.dir.path().join("store")),
            Arc::new(ManualClock::at("2026-09-13T07:00:00Z")),
            Box::new(StdRandom::new()),
        );
        slow.put(
            NewItem::text("tablet"),
            Box::new(std::io::Cursor::new(b"late clock".to_vec())),
        )
        .await
        .unwrap();
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(drain(&mut rx), [ClipboardWrite::Text("late clock".into())]);
        puller.poll(&rig.store).await.unwrap();
        assert!(drain(&mut rx).is_empty(), "applied once");
    }

    #[tokio::test]
    async fn a_poll_reads_one_listing_and_only_the_new_metadata() {
        let rig = Rig::new();
        for i in 0..5 {
            rig.put(NewItem::text("phone"), format!("old {i}").as_bytes())
                .await;
        }
        let mut puller = rig.puller(None).await;
        for i in 0..3 {
            rig.put(NewItem::text("me"), format!("mine {i}").as_bytes())
                .await;
        }
        let (dirs, reads) = (
            rig.store.fs().calls(FsOp::ReadDir),
            rig.store.fs().calls(FsOp::OpenRead),
        );
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(rig.store.fs().calls(FsOp::ReadDir) - dirs, 1, "one listing");
        assert_eq!(
            rig.store.fs().calls(FsOp::OpenRead) - reads,
            3,
            "new metadata only"
        );
    }

    #[tokio::test]
    async fn ids_that_leave_the_store_are_forgotten() {
        let rig = Rig::new();
        let old = rig.put(NewItem::text("phone"), b"old").await;
        let mut puller = rig.puller(None).await;
        let new = rig.put(NewItem::text("phone"), b"new").await;
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(puller.seen_len(), 2);
        rig.store.delete(&old.id).await.unwrap();
        rig.store.delete(&new.id).await.unwrap();
        puller.poll(&rig.store).await.unwrap();
        assert_eq!(puller.seen_len(), 0);
    }
}
