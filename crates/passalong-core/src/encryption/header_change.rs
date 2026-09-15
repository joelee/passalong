//! Changes to the store header alone, journalled like re-encryption:
//! setting up encryption, a fresh start, and new words.
//!
//! 1. The lock appears with its journal and the new header in
//!    `.rewrite/header/`; for new words the current header is kept in
//!    `.rewrite/old-header/` too.
//! 2. For a set-up or a fresh start, `items/` makes way for the stop file:
//!    an empty one is removed, and one holding items moves to
//!    `plain/items/`.
//! 3. The current header, if any, moves into `.rewrite/previous/`, and the
//!    new one takes its place.
//! 4. The lock is removed.
//!
//! Each step can run again, so [`finish`](super::finish) completes an
//! interrupted change, and [`undo`](super::undo) puts the old header and
//! items back.

use std::sync::Arc;

use super::admin::{refuse, restore_items, stop_old_clients};
use super::header::{encryption_dir, read_header_in};
use super::journal::{
    HeaderChange, HeaderChangeKind, NEW_HEADER, OLD_HEADER, PREVIOUS, publish, release_lock,
    under_rewrite,
};
use super::{EncryptionError, SEALED_TMP_DIR, StoreHeader, StoreState, inspect, read_header};
use crate::clock::{Clock, SystemClock};
use crate::crypto::{DataKey, Sealer, Words, random_bytes, unwrap};
use crate::fs::{FsError, RemoteFs, RemotePath};
use crate::random::StdRandom;
use crate::store::{FsStore, Store, StoreError};

/// How many items [`restore_header`] tries to open with a candidate key.
const PROBE_ITEMS: usize = 8;

/// Runs the header change `kind`, putting `header` in place; `old` is the
/// header it replaces, if any.
pub(super) async fn run_change<F: RemoteFs + ?Sized>(
    fs: &F,
    kind: HeaderChangeKind,
    header: &StoreHeader,
    old: Option<&StoreHeader>,
) -> Result<(), StoreError> {
    let started_at = SystemClock
        .now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let journal = HeaderChange::new(kind, started_at, header.key_id().to_string());
    let mut staged = vec![(NEW_HEADER, header)];
    staged.extend(old.map(|old| (OLD_HEADER, old)));
    publish(fs, &journal, &staged).await?;
    complete(fs, kind).await
}

/// Steps 2 to 4, from wherever an earlier run stopped.
async fn complete<F: RemoteFs + ?Sized>(fs: &F, kind: HeaderChangeKind) -> Result<(), StoreError> {
    if kind != HeaderChangeKind::Words {
        stop_old_clients(fs).await?;
    }
    install(fs).await?;
    release_lock(fs).await
}

/// Puts the lock's new header in place, moving the current one into the
/// lock; does nothing once the new header is in place.
pub(super) async fn install<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    let pending = under_rewrite(NEW_HEADER)?;
    if fs.stat(&pending).await?.is_none() {
        return Ok(());
    }
    let current = encryption_dir()?;
    if fs.stat(&current).await?.is_some() {
        let previous = under_rewrite(PREVIOUS)?;
        if fs.stat(&previous).await?.is_some() {
            return Err(EncryptionError::Layout(
                "the lock holds a replaced header, yet another header is in place; nothing was changed"
                    .to_owned(),
            )
            .into());
        }
        fs.rename(&current, &previous).await?;
    }
    fs.rename(&pending, &current).await?;
    Ok(())
}

/// Completes the interrupted header `change`, once `new_words` unlock the
/// header it puts in place. Returns the key.
pub(super) async fn finish_change<F: RemoteFs + ?Sized>(
    fs: &F,
    change: &HeaderChange,
    new_words: &Words,
) -> Result<DataKey, StoreError> {
    let header = match read_header_in(fs, &under_rewrite(NEW_HEADER)?).await? {
        Some(header) => header,
        None => read_header(fs)
            .await?
            .ok_or(EncryptionError::HeaderMissing)?,
    };
    if header.key_id().to_string() != change.key {
        return Err(EncryptionError::Layout(
            "the header in place is not the one this change saved; undo it instead".to_owned(),
        )
        .into());
    }
    let key = unwrap(header.wrapped(), new_words)?;
    complete(fs, change.kind).await?;
    Ok(key)
}

/// Undoes the interrupted header `change`: the header and items are put
/// back as they were, and the lock removed.
pub(super) async fn revert<F: RemoteFs + ?Sized>(
    fs: &F,
    change: &HeaderChange,
) -> Result<(), StoreError> {
    let current = encryption_dir()?;
    let pending = under_rewrite(NEW_HEADER)?;
    let previous = under_rewrite(PREVIOUS)?;
    // The new header, if it was put in place, goes back into the lock.
    if fs.stat(&pending).await?.is_none() && fs.stat(&current).await?.is_some() {
        fs.rename(&current, &pending).await?;
    }
    if fs.stat(&current).await?.is_none() && fs.stat(&previous).await?.is_some() {
        fs.rename(&previous, &current).await?;
    }
    match change.kind {
        HeaderChangeKind::Words => {
            if fs.stat(&current).await?.is_none() {
                fs.rename(&under_rewrite(OLD_HEADER)?, &current).await?;
            }
        }
        HeaderChangeKind::SetUp | HeaderChangeKind::FreshStart => restore_items(fs).await?,
    }
    release_lock(fs).await
}

/// Repairs a broken store with its `words`, and returns its key.
///
/// - A header beside an `items/` folder: the folder's items, stored by a
///   client that never saw the stop file, move to `plain/items/`, and the
///   stop file is written, once `words` unlock the header.
/// - No readable header: passalong 0.2.0 may have left one under `v2/tmp/`
///   when a change of words stopped between its two renames. The first one
///   `words` unlock whose key opens the store's items is put in place; the
///   store's words are then the ones given.
///
/// # Errors
///
/// The refusal for a store that is not broken, wrong words, and
/// [`EncryptionError::Layout`], changing nothing, when no saved header fits.
pub async fn restore_header<F: RemoteFs + ?Sized>(
    fs: &F,
    words: &Words,
) -> Result<DataKey, StoreError> {
    match inspect(fs).await? {
        StoreState::Broken => {}
        other => return Err(refuse(other)),
    }
    if let Some(header) = read_header(fs).await? {
        let key = unwrap(header.wrapped(), words)?;
        stop_old_clients(fs).await?;
        tracing::info!(key = %key.key_id().short(), "moved an items folder beside the header to plain/");
        return Ok(key);
    }
    let tmp = RemotePath::new(SEALED_TMP_DIR)?;
    let entries = match fs.read_dir(&tmp).await {
        Ok(entries) => entries,
        Err(FsError::NotFound(_)) => Vec::new(),
        Err(err) => return Err(err.into()),
    };
    let candidates = entries.into_iter().filter(|entry| {
        entry.is_dir && (entry.name.starts_with("header-") || entry.name.starts_with("old-header-"))
    });
    for entry in candidates {
        let dir = tmp.join(&entry.name)?;
        let Ok(Some(header)) = read_header_in(fs, &dir).await else {
            continue;
        };
        let Ok(key) = unwrap(header.wrapped(), words) else {
            continue;
        };
        if !opens_items(fs, &key).await? {
            continue;
        }
        // Whatever `encryption/` holds without a readable header moves
        // aside rather than being deleted.
        let current = encryption_dir()?;
        if fs.stat(&current).await?.is_some() {
            let token: [u8; 8] = random_bytes()?;
            let aside = tmp.join(&format!("broken-header-{}", hex::encode(token)))?;
            fs.rename(&current, &aside).await?;
        }
        fs.rename(&dir, &current).await?;
        tracing::info!(key = %key.key_id().short(), "store header restored");
        return Ok(key);
    }
    Err(EncryptionError::Layout(
        "no header saved in the store opens with these words and the store's items; nothing was changed"
            .to_owned(),
    )
    .into())
}

/// Whether `key` opens the metadata of one of the store's first items, or
/// the store has none.
async fn opens_items<F: RemoteFs + ?Sized>(fs: &F, key: &DataKey) -> Result<bool, StoreError> {
    let store = FsStore::sealed(
        fs,
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
        Sealer::new(key.clone()),
    );
    let ids = store.list_ids().await?;
    if ids.is_empty() {
        return Ok(true);
    }
    for id in ids.iter().take(PROBE_ITEMS) {
        if store.get_meta(id).await.is_ok() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::wrap;
    use crate::encryption::header::write_header_in;
    use crate::encryption::rewrite::tests::{
        OPS, W1, W2, assert_holds, clock, copy_tree, plain_items, quick, sealed_items, snapshot,
        words,
    };
    use crate::encryption::{
        change_words, finish, fresh_start, join, plain_store, replace_header, set_up, undo,
    };
    use crate::fs::LocalFs;
    use crate::model::ItemMeta;
    use crate::testing::{FaultyFs, FsOp};
    use std::path::Path;
    use tempfile::TempDir;

    #[derive(Debug, Clone, Copy)]
    enum Change {
        SetUp,
        Fresh,
        Words,
    }

    impl Change {
        /// The words the store has once the change is done.
        fn new_words(self) -> Words {
            match self {
                Change::Words => words(W2),
                Change::SetUp | Change::Fresh => words(W1),
            }
        }

        async fn apply<F: RemoteFs>(self, fs: &F) -> Result<(), StoreError> {
            match self {
                Change::SetUp => set_up(fs, &words(W1), quick()).await.map(drop),
                Change::Fresh => fresh_start(fs, &words(W1), quick()).await.map(drop),
                Change::Words => change_words(fs, &words(W1), &words(W2), quick())
                    .await
                    .map(drop),
            }
        }

        /// Fills `dir` with the store the change starts from; returns its
        /// items and, for new words, its key.
        async fn template(self, dir: &Path) -> (Vec<ItemMeta>, Option<DataKey>) {
            match self {
                Change::SetUp => (Vec::new(), None),
                Change::Fresh => (plain_items(dir).await, None),
                Change::Words => {
                    let (key, items) = sealed_items(dir).await;
                    (items, Some(key))
                }
            }
        }
    }

    /// Asserts `dir` holds the store `change` leaves, under `key`.
    async fn assert_done(
        change: Change,
        dir: &Path,
        key: &DataKey,
        items: &[ItemMeta],
        old: Option<&DataKey>,
    ) {
        let fs = LocalFs::new(dir);
        assert!(!dir.join(".rewrite").exists());
        assert_eq!(
            join(&fs, &change.new_words()).await.unwrap().key_id(),
            key.key_id()
        );
        match change {
            Change::SetUp => assert_eq!(
                inspect(&fs).await.unwrap(),
                StoreState::Encrypted {
                    key_id: key.key_id(),
                    plain_left: 0
                }
            ),
            Change::Fresh => {
                assert_eq!(
                    inspect(&fs).await.unwrap(),
                    StoreState::Encrypted {
                        key_id: key.key_id(),
                        plain_left: items.len()
                    }
                );
                let kept = plain_store(&fs).list_ids().await.unwrap();
                assert!(items.iter().all(|meta| kept.contains(&meta.id)));
            }
            Change::Words => {
                assert_eq!(Some(key.key_id()), old.map(DataKey::key_id));
                assert!(join(&fs, &words(W1)).await.is_err());
                assert_holds(dir, key, items).await;
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum Fault {
        /// The `nth` call of an operation fails.
        Fail(FsOp, usize),
        /// The `nth` write is cut short after a few bytes.
        Cut(usize),
    }

    /// Cuts `change` at every filesystem call and write. Each cut store is
    /// as before, or has a lock that both `finish` and `undo` recover from.
    async fn cut_everywhere(change: Change) {
        let template = TempDir::new().unwrap();
        let (items, old) = change.template(template.path()).await;
        let before = snapshot(template.path());
        let calls = {
            let scratch = TempDir::new().unwrap();
            copy_tree(template.path(), scratch.path());
            let fs = FaultyFs::new(LocalFs::new(scratch.path()));
            change.apply(&fs).await.unwrap();
            OPS.map(|op| fs.calls(op))
        };
        let writes = calls[3];
        let faults = OPS
            .into_iter()
            .zip(calls)
            .flat_map(|(op, count)| (1..=count).map(move |nth| Fault::Fail(op, nth)))
            .chain((1..=writes).map(Fault::Cut));
        let mut recovered = 0;
        for fault in faults {
            let at = format!("{change:?}: {fault:?}");
            let cut = TempDir::new().unwrap();
            copy_tree(template.path(), cut.path());
            let fs = FaultyFs::new(LocalFs::new(cut.path()));
            match fault {
                Fault::Fail(op, nth) => fs.fail_nth(op, nth),
                Fault::Cut(nth) => fs.cut_nth_write(nth, 10),
            }
            if change.apply(&fs).await.is_ok() {
                let key = join(&LocalFs::new(cut.path()), &change.new_words())
                    .await
                    .unwrap();
                assert_done(change, cut.path(), &key, &items, old.as_ref()).await;
                continue;
            }
            if !cut.path().join(".rewrite").exists() {
                assert_eq!(snapshot(cut.path()), before, "{at}");
                continue;
            }
            recovered += 1;
            let copy = TempDir::new().unwrap();
            copy_tree(cut.path(), copy.path());
            let key = finish(
                &LocalFs::new(cut.path()),
                &change.new_words(),
                None,
                None,
                clock(),
            )
            .await
            .unwrap_or_else(|err| panic!("{at}: finish: {err}"));
            assert_done(change, cut.path(), &key, &items, old.as_ref()).await;
            undo(&LocalFs::new(copy.path()))
                .await
                .unwrap_or_else(|err| panic!("{at}: undo: {err}"));
            assert_eq!(snapshot(copy.path()), before, "{at}: undo");
        }
        assert!(
            recovered >= 3,
            "{change:?}: only {recovered} cuts needed recovery"
        );
    }

    #[tokio::test]
    async fn a_set_up_cut_anywhere_is_finished_or_undone() {
        cut_everywhere(Change::SetUp).await;
    }

    #[tokio::test]
    async fn a_fresh_start_cut_anywhere_is_finished_or_undone() {
        cut_everywhere(Change::Fresh).await;
    }

    #[tokio::test]
    async fn a_change_of_words_cut_anywhere_is_finished_or_undone() {
        cut_everywhere(Change::Words).await;
    }

    #[tokio::test]
    async fn a_set_up_or_fresh_start_leaves_no_plaintext_staging() {
        use crate::encryption::rewrite::tests::{holds_marker, seed_staging};
        for change in [Change::SetUp, Change::Fresh] {
            let dir = TempDir::new().unwrap();
            change.template(dir.path()).await;
            seed_staging(dir.path());
            change.apply(&LocalFs::new(dir.path())).await.unwrap();
            assert!(!dir.path().join("tmp").exists(), "{change:?}");
            assert!(!holds_marker(dir.path()), "{change:?}");
        }
    }

    #[tokio::test]
    async fn a_change_of_words_waits_for_a_rewrite() {
        let dir = TempDir::new().unwrap();
        sealed_items(dir.path()).await;
        std::fs::create_dir(dir.path().join(".rewrite")).unwrap();
        let fs = LocalFs::new(dir.path());
        assert!(matches!(
            change_words(&fs, &words(W1), &words(W2), quick()).await,
            Err(StoreError::Encryption(EncryptionError::Rewriting { .. }))
        ));
    }

    #[tokio::test]
    async fn a_header_0_2_0_left_aside_is_restored_with_the_store_s_words() {
        let dir = TempDir::new().unwrap();
        let (key, items) = sealed_items(dir.path()).await;
        // passalong 0.2.0 changed words with `replace_header`: the old
        // header moved aside, then moving the new one in failed.
        let faulty = FaultyFs::new(LocalFs::new(dir.path()));
        faulty.fail_nth(FsOp::Rename, 2);
        let new_header = StoreHeader::new(wrap(&key, &words(W2), quick()).unwrap());
        assert!(replace_header(&faulty, &new_header).await.is_err());
        let fs = LocalFs::new(dir.path());
        assert_eq!(inspect(&fs).await.unwrap(), StoreState::Broken);
        // A header another key wraps under the same words opens no item.
        let stranger =
            StoreHeader::new(wrap(&DataKey::generate().unwrap(), &words(W1), quick()).unwrap());
        write_header_in(
            &fs,
            &RemotePath::new("v2/tmp/header-0000").unwrap(),
            &stranger,
        )
        .await
        .unwrap();

        let err = restore_header(&fs, &words("zoom abacus zoom abacus zoom abacus"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nothing was changed"), "{err}");
        assert_eq!(inspect(&fs).await.unwrap(), StoreState::Broken);

        let restored = restore_header(&fs, &words(W1)).await.unwrap();
        assert_eq!(restored.key_id(), key.key_id());
        assert_eq!(join(&fs, &words(W1)).await.unwrap().key_id(), key.key_id());
        assert_holds(dir.path(), &key, &items).await;
        assert!(matches!(
            restore_header(&fs, &words(W1)).await,
            Err(StoreError::Encryption(EncryptionError::AlreadyEncrypted))
        ));
    }

    #[tokio::test]
    async fn a_header_beside_an_items_folder_is_repaired_with_the_store_s_words() {
        let dir = TempDir::new().unwrap();
        let (key, items) = sealed_items(dir.path()).await;
        // A client before v0.2.0 on a synced copy that never received the
        // stop file stored items in an `items/` folder.
        std::fs::remove_file(dir.path().join("items")).unwrap();
        let strays = plain_items(dir.path()).await;
        let fs = LocalFs::new(dir.path());
        assert_eq!(inspect(&fs).await.unwrap(), StoreState::Broken);
        assert!(restore_header(&fs, &words(W2)).await.is_err());
        assert_eq!(inspect(&fs).await.unwrap(), StoreState::Broken);

        let restored = restore_header(&fs, &words(W1)).await.unwrap();
        assert_eq!(restored.key_id(), key.key_id());
        assert_eq!(
            inspect(&fs).await.unwrap(),
            StoreState::Encrypted {
                key_id: key.key_id(),
                plain_left: strays.len()
            }
        );
        assert_holds(dir.path(), &key, &items).await;
    }
}
