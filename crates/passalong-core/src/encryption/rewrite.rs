//! Re-encrypting every item: migrating a plaintext store, and rotating an
//! encrypted store's data key. One journalled engine does both:
//!
//! 1. `.rewrite/` appears, with `plan.json` saying what runs, in one rename
//!    of a whole journal; it is the lock (see the `journal` module).
//! 2. The new header goes to `.rewrite/header/`; a rotation also copies the
//!    current one to `.rewrite/old-header/`.
//! 3. The source moves into `.rewrite/source/` in one rename: `items/` for
//!    a migration, which then writes the stop file, or `v2/items/` for a
//!    rotation.
//! 4. Each source item, oldest first, is sealed under the new key into
//!    `v2/items/`, keeping its metadata and creation time. Its new id
//!    follows from its SHA-256, so an item already there is skipped and a
//!    resumed run repeats nothing.
//! 5. Every new item is read back and its SHA-256 and size compared.
//! 6. The new header replaces the old; then `.rewrite/source/`, and last
//!    `.rewrite/` itself, are removed.
//!
//! [`finish`] resumes an interrupted run; [`undo`] puts the source back as
//! long as the new header is not yet in place. Each claims
//! `.rewrite/recovery/` while it runs, so two recoveries never run at once.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

use super::admin::refuse;
use super::header::{encryption_dir, read_header_in, write_header_in};
use super::journal::{
    Journal, PLAN_FILE, claim_recovery, publish, read_journal, release_unstarted, sweep_staged,
    unclaim, under_rewrite, unsupported,
};
use super::{
    EncryptionError, HEADER_FILE, REWRITE_DIR, SEALED_TMP_DIR, STOP_FILE, StoreHeader, StoreState,
    inspect, read_header, write_stop_file,
};
use crate::clock::Clock;
use crate::crypto::{DataKey, KdfParams, Sealer, Words, random_bytes, unwrap, wrap};
use crate::fs::{FsError, RemoteFs, RemotePath, SubFs};
use crate::model::ContentHasher;
use crate::random::StdRandom;
use crate::store::{FsStore, Store, StoreError};

const NEW_HEADER: &str = "header";
const OLD_HEADER: &str = "old-header";
const SOURCE: &str = "source";

/// What a re-encryption does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RewriteKind {
    /// A plaintext store's items are encrypted.
    Migrate,
    /// An encrypted store's items move to a new data key.
    Rotate,
}

/// `.rewrite/plan.json`: what an interrupted re-encryption was doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewritePlan {
    /// Migration or rotation.
    pub kind: RewriteKind,
    /// When it started, RFC 3339.
    pub started_at: String,
    /// The old key's id, for a rotation.
    pub from_key: Option<String>,
    /// The new key's id.
    pub to_key: String,
    /// How many items the source held.
    pub items: usize,
}

/// Where the source's items sit inside `.rewrite/source/`, and where they
/// came from.
fn source_items(kind: RewriteKind) -> &'static str {
    match kind {
        RewriteKind::Migrate => STOP_FILE,
        RewriteKind::Rotate => "v2/items",
    }
}

/// Reads the plan of the re-encryption in progress, if any.
///
/// # Errors
///
/// The filesystem's error, and [`EncryptionError::Layout`] for a journal
/// that cannot be read or records another change; [`read_journal`] tells
/// every journal apart.
pub async fn read_plan<F: RemoteFs + ?Sized>(fs: &F) -> Result<Option<RewritePlan>, StoreError> {
    match read_journal(fs).await? {
        None => Ok(None),
        Some(Journal::Rewrite(plan)) => Ok(Some(plan)),
        Some(Journal::Unreadable { reason }) => {
            Err(EncryptionError::Layout(format!("{PLAN_FILE}: {reason}")).into())
        }
        Some(other) => Err(unsupported(&other)),
    }
}

/// The plan the lock's journal records, for resuming it.
async fn rewrite_journal<F: RemoteFs + ?Sized>(fs: &F) -> Result<RewritePlan, StoreError> {
    match read_journal(fs).await? {
        Some(Journal::Rewrite(plan)) => Ok(plan),
        None | Some(Journal::Unreadable { .. }) => Err(EncryptionError::Layout(
            "the re-encryption stopped before its plan was saved; undo it instead".to_owned(),
        )
        .into()),
        Some(other) => Err(unsupported(&other)),
    }
}

fn started_at(clock: &dyn Clock) -> String {
    clock
        .now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Encrypts every item of a plaintext store under a new data key wrapped
/// with `words`, and returns the key.
///
/// # Errors
///
/// The refusal for a store that is not plaintext, and any failure on the
/// way, after which [`finish`] or [`undo`] recovers.
pub async fn migrate<F: RemoteFs + ?Sized>(
    fs: &F,
    words: &Words,
    kdf: KdfParams,
    clock: Arc<dyn Clock>,
) -> Result<DataKey, StoreError> {
    let items = match inspect(fs).await? {
        StoreState::Plain { items } => items,
        other => return Err(refuse(other)),
    };
    let key = DataKey::generate()?;
    let header = StoreHeader::new(wrap(&key, words, kdf)?);
    let plan = RewritePlan {
        kind: RewriteKind::Migrate,
        started_at: started_at(clock.as_ref()),
        from_key: None,
        to_key: key.key_id().to_string(),
        items,
    };
    publish(fs, &plan).await?;
    write_header_in(fs, &under_rewrite(NEW_HEADER)?, &header).await?;
    run(fs, RewriteKind::Migrate, &key, None, clock).await?;
    Ok(key)
}

/// Moves every item of an encrypted store from `old` to a new data key
/// wrapped with `words`, and returns the new key. Devices holding the old
/// key are refused afterwards until they join again.
///
/// # Errors
///
/// [`EncryptionError::KeyMismatch`] when `old` is not the store's key, the
/// refusal for a store that is not encrypted, and any failure on the way,
/// after which [`finish`] or [`undo`] recovers.
pub async fn rotate<F: RemoteFs + ?Sized>(
    fs: &F,
    old: &DataKey,
    words: &Words,
    kdf: KdfParams,
    clock: Arc<dyn Clock>,
) -> Result<DataKey, StoreError> {
    match inspect(fs).await? {
        StoreState::Encrypted { key_id, .. } if key_id == old.key_id() => {}
        StoreState::Encrypted { key_id, .. } => {
            return Err(EncryptionError::KeyMismatch {
                device: old.key_id().short(),
                store: key_id.short(),
            }
            .into());
        }
        other => return Err(refuse(other)),
    }
    let current = read_header(fs)
        .await?
        .ok_or(EncryptionError::HeaderMissing)?;
    let old_store = FsStore::sealed(
        fs,
        clock.clone(),
        Box::new(StdRandom::new()),
        Sealer::new(old.clone()),
    );
    let items = old_store.list_ids().await?.len();
    let key = DataKey::generate()?;
    let header = StoreHeader::new(wrap(&key, words, kdf)?);
    let plan = RewritePlan {
        kind: RewriteKind::Rotate,
        started_at: started_at(clock.as_ref()),
        from_key: Some(old.key_id().to_string()),
        to_key: key.key_id().to_string(),
        items,
    };
    publish(fs, &plan).await?;
    write_header_in(fs, &under_rewrite(OLD_HEADER)?, &current).await?;
    write_header_in(fs, &under_rewrite(NEW_HEADER)?, &header).await?;
    run(fs, RewriteKind::Rotate, &key, Some(old), clock).await?;
    Ok(key)
}

/// Resumes an interrupted re-encryption. `new_words` unlock the new key;
/// a rotation whose items are not all moved yet also needs the old key,
/// from `old_key` or unlocked with `old_words`. Returns the new key.
///
/// # Errors
///
/// [`EncryptionError::Layout`] when nothing is to be recovered,
/// [`EncryptionError::Recovering`] while another recovery runs, wrong
/// words, and any failure on the way.
pub async fn finish<F: RemoteFs + ?Sized>(
    fs: &F,
    new_words: &Words,
    old_key: Option<&DataKey>,
    old_words: Option<&Words>,
    clock: Arc<dyn Clock>,
) -> Result<DataKey, StoreError> {
    claim_recovery(fs).await?;
    let result = finish_claimed(fs, new_words, old_key, old_words, clock).await;
    unclaim(fs, result).await
}

async fn finish_claimed<F: RemoteFs + ?Sized>(
    fs: &F,
    new_words: &Words,
    old_key: Option<&DataKey>,
    old_words: Option<&Words>,
    clock: Arc<dyn Clock>,
) -> Result<DataKey, StoreError> {
    let plan = rewrite_journal(fs).await?;
    let pending = read_header_in(fs, &under_rewrite(NEW_HEADER)?).await?;
    let current = read_header(fs).await?;
    let (header, swapped) = match (pending, current) {
        (Some(header), _) => (header, false),
        (None, Some(current)) if current.key_id().to_string() == plan.to_key => (current, true),
        _ => {
            return Err(EncryptionError::Layout(
                "the re-encryption stopped before its new key was saved; undo it instead"
                    .to_owned(),
            )
            .into());
        }
    };
    let key = unwrap(header.wrapped(), new_words)?;
    if swapped {
        cleanup(fs).await?;
        return Ok(key);
    }
    let old = match plan.kind {
        RewriteKind::Migrate => None,
        RewriteKind::Rotate => Some(old_key_for(fs, old_key, old_words).await?),
    };
    run(fs, plan.kind, &key, old.as_ref(), clock).await?;
    Ok(key)
}

/// The old key of a rotation: `old_key` when it is the one recorded, or
/// the one `old_words` unlock.
async fn old_key_for<F: RemoteFs + ?Sized>(
    fs: &F,
    old_key: Option<&DataKey>,
    old_words: Option<&Words>,
) -> Result<DataKey, StoreError> {
    let old_header = read_header_in(fs, &under_rewrite(OLD_HEADER)?)
        .await?
        .ok_or(EncryptionError::HeaderMissing)?;
    if let Some(key) = old_key.filter(|key| key.key_id() == old_header.key_id()) {
        return Ok(key.clone());
    }
    match old_words {
        Some(words) => Ok(unwrap(old_header.wrapped(), words)?),
        None => Err(EncryptionError::KeyMismatch {
            device: old_key.map(|key| key.key_id().short()).unwrap_or_default(),
            store: old_header.key_id().short(),
        }
        .into()),
    }
}

/// Puts an interrupted re-encryption's source back and releases the lock.
///
/// # Errors
///
/// [`EncryptionError::Layout`] when nothing is to be recovered, when the
/// new header is already in place and only [`finish`] can complete it, and
/// when a lock whose journal is missing or unreadable holds anything else;
/// [`EncryptionError::Recovering`] while another recovery runs.
pub async fn undo<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    claim_recovery(fs).await?;
    let result = undo_claimed(fs).await;
    unclaim(fs, result).await
}

async fn undo_claimed<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    let plan = match read_journal(fs).await? {
        Some(Journal::Rewrite(plan)) => plan,
        // Locked, but stopped before its plan was whole.
        None | Some(Journal::Unreadable { .. }) => return release_unstarted(fs).await,
        Some(other) => return Err(unsupported(&other)),
    };
    let pending = under_rewrite(&format!("{NEW_HEADER}/{HEADER_FILE}"))?;
    let swapped = fs.stat(&pending).await?.is_none()
        && read_header(fs)
            .await?
            .is_some_and(|header| header.key_id().to_string() == plan.to_key);
    if swapped {
        return Err(EncryptionError::Layout(
            "the new key is already in place; finish the re-encryption instead".to_owned(),
        )
        .into());
    }
    let moved = under_rewrite(&format!("{SOURCE}/{}", source_items(plan.kind)))?;
    match plan.kind {
        RewriteKind::Migrate => {
            fs.remove_dir_all(&RemotePath::new("v2")?).await?;
            let items = RemotePath::new(STOP_FILE)?;
            if fs.stat(&items).await?.is_some_and(|meta| !meta.is_dir) {
                fs.remove_file(&items).await?;
            }
            if fs.stat(&moved).await?.is_some() {
                fold(fs, &moved, &items).await?;
            }
        }
        RewriteKind::Rotate => {
            let items = RemotePath::new("v2/items")?;
            if fs.stat(&moved).await?.is_some() {
                fs.remove_dir_all(&items).await?;
                fs.rename(&moved, &items).await?;
            }
            // A swap cut short leaves no header: the old one comes back.
            if fs.stat(&encryption_dir()?).await?.is_none() {
                let old = read_header_in(fs, &under_rewrite(OLD_HEADER)?)
                    .await?
                    .ok_or(EncryptionError::HeaderMissing)?;
                write_header_in(fs, &encryption_dir()?, &old).await?;
            }
        }
    }
    sweep_staged(fs).await?;
    fs.remove_dir_all(&RemotePath::new(REWRITE_DIR)?).await?;
    Ok(())
}

/// Moves the folder `from` to `to`: in one rename when `to` does not exist,
/// entry by entry otherwise, dropping entries `to` already has.
async fn fold<F: RemoteFs + ?Sized>(
    fs: &F,
    from: &RemotePath,
    to: &RemotePath,
) -> Result<(), StoreError> {
    if fs.stat(to).await?.is_none() {
        if let Some(parent) = to.parent() {
            fs.create_dir_all(&parent).await?;
        }
        fs.rename(from, to).await?;
        return Ok(());
    }
    for entry in fs.read_dir(from).await? {
        let source = from.join(&entry.name)?;
        match fs.rename(&source, &to.join(&entry.name)?).await {
            Ok(()) => {}
            Err(FsError::AlreadyExists(_)) => fs.remove_dir_all(&source).await?,
            Err(err) => return Err(err.into()),
        }
    }
    fs.remove_dir_all(from).await?;
    Ok(())
}

/// Steps 3 to 6, from wherever an earlier run stopped.
async fn run<F: RemoteFs + ?Sized>(
    fs: &F,
    kind: RewriteKind,
    key: &DataKey,
    old: Option<&DataKey>,
    clock: Arc<dyn Clock>,
) -> Result<(), StoreError> {
    let moved = under_rewrite(&format!("{SOURCE}/{}", source_items(kind)))?;
    let original = RemotePath::new(source_items(kind))?;
    if fs.stat(&moved).await?.is_none() {
        fs.create_dir_all(&moved.parent().expect("under .rewrite"))
            .await?;
        if fs.stat(&original).await?.is_some_and(|meta| meta.is_dir) {
            fs.rename(&original, &moved).await?;
        } else {
            fs.create_dir_all(&moved).await?;
        }
    }
    if kind == RewriteKind::Migrate
        && let Err(err) = write_stop_file(fs).await
    {
        // A client before v0.2.0 recreated `items/`: its items join the
        // source, once.
        if !matches!(err, StoreError::Encryption(EncryptionError::Layout(_))) {
            return Err(err);
        }
        fold(fs, &original, &moved).await?;
        write_stop_file(fs).await?;
    }

    let source_fs = SubFs::new(fs, under_rewrite(SOURCE)?);
    let rng = || Box::new(StdRandom::new());
    let source = match old {
        Some(old) => FsStore::sealed(source_fs, clock.clone(), rng(), Sealer::new(old.clone())),
        None => FsStore::new(source_fs, clock.clone(), rng()),
    };
    let target = FsStore::sealed(fs, clock, rng(), Sealer::new(key.clone()));
    let mut ids = source.list_ids().await?;
    ids.reverse();
    let mut metas = Vec::with_capacity(ids.len());
    for id in &ids {
        let meta = source.get_meta(id).await?;
        if !target.exists(&target.id_for(&meta)?).await? {
            let (meta, content) = source.get(id).await?;
            target.import(&meta, content).await?;
        }
        metas.push(meta);
    }
    for meta in &metas {
        verify(&target, meta).await?;
    }
    tracing::info!(items = metas.len(), key = %key.key_id().short(), "items re-encrypted and verified");
    swap_header(fs, kind).await?;
    cleanup(fs).await
}

/// Reads back the new copy of `meta`'s item and compares its SHA-256 and
/// size.
async fn verify<F: RemoteFs>(
    target: &FsStore<F>,
    meta: &crate::model::ItemMeta,
) -> Result<(), StoreError> {
    let id = target.id_for(meta)?;
    let (_, mut content) = target.get(&id).await?;
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

/// Puts the new header in place, unless an earlier run already did.
async fn swap_header<F: RemoteFs + ?Sized>(fs: &F, kind: RewriteKind) -> Result<(), StoreError> {
    let new = under_rewrite(NEW_HEADER)?;
    if fs.stat(&new).await?.is_none() {
        return Ok(());
    }
    let current = encryption_dir()?;
    if kind == RewriteKind::Rotate && fs.stat(&current).await?.is_some() {
        let token: [u8; 8] = random_bytes()?;
        let old =
            RemotePath::new(SEALED_TMP_DIR)?.join(&format!("old-header-{}", hex::encode(token)))?;
        fs.create_dir_all(&RemotePath::new(SEALED_TMP_DIR)?).await?;
        fs.rename(&current, &old).await?;
        fs.rename(&new, &current).await?;
        if let Err(err) = fs.remove_dir_all(&old).await {
            tracing::warn!(path = %old, error = %err, "could not remove the previous header");
        }
        return Ok(());
    }
    fs.rename(&new, &current).await?;
    Ok(())
}

/// Removes the source, journals staged by locks never taken, and then the
/// lock.
async fn cleanup<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    fs.remove_dir_all(&under_rewrite(SOURCE)?).await?;
    sweep_staged(fs).await?;
    fs.remove_dir_all(&RemotePath::new(REWRITE_DIR)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KDF_SALT_LEN;
    use crate::encryption::{open_with_key, set_up};
    use crate::fs::LocalFs;
    use crate::model::{ItemMeta, NewItem};
    use crate::testing::{FaultyFs, FsOp, ManualClock};
    use std::collections::BTreeMap;
    use std::path::Path;
    use tempfile::TempDir;

    const W1: &str = "abacus abdomen abdominal abide abiding ability";
    const W2: &str = "zoom zoom zoom zoom zoom zoom";
    const OPS: [FsOp; 9] = [
        FsOp::CreateDirAll,
        FsOp::ReadDir,
        FsOp::OpenRead,
        FsOp::OpenWrite,
        FsOp::Rename,
        FsOp::RemoveDirAll,
        FsOp::Stat,
        FsOp::CreateDir,
        FsOp::RemoveFile,
    ];

    fn words(text: &str) -> Words {
        Words::parse(text).unwrap()
    }

    fn quick() -> KdfParams {
        KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [7; KDF_SALT_LEN],
        }
    }

    fn clock() -> Arc<ManualClock> {
        Arc::new(ManualClock::at("2026-09-15T08:00:00Z"))
    }

    fn text(bytes: &[u8]) -> crate::fs::BoxRead {
        Box::new(std::io::Cursor::new(bytes.to_vec()))
    }

    /// A plaintext store with a text and a file, a second apart.
    async fn plain_items(dir: &Path) -> Vec<ItemMeta> {
        let clock = clock();
        let store = FsStore::new(LocalFs::new(dir), clock.clone(), Box::new(StdRandom::new()));
        let first = store
            .put(NewItem::text("box"), text(b"first item"))
            .await
            .unwrap()
            .meta;
        clock.advance(1);
        let second = store
            .put(NewItem::file("b.txt", "lap"), text(b"second item"))
            .await
            .unwrap()
            .meta;
        vec![first, second]
    }

    async fn open(dir: &Path, key: &DataKey) -> Result<Box<dyn Store>, StoreError> {
        open_with_key(
            LocalFs::new(dir),
            Some(key.clone()),
            clock(),
            Box::new(StdRandom::new()),
        )
        .await
    }

    /// An encrypted store with two items, and its key.
    async fn sealed_items(dir: &Path) -> (DataKey, Vec<ItemMeta>) {
        let key = set_up(&LocalFs::new(dir), &words(W1), quick())
            .await
            .unwrap();
        let store = open(dir, &key).await.unwrap();
        let mut metas = Vec::new();
        for body in ["first sealed", "second sealed"] {
            metas.push(
                store
                    .put(NewItem::text("box"), text(body.as_bytes()))
                    .await
                    .unwrap()
                    .meta,
            );
        }
        (key, metas)
    }

    /// Asserts the store under `key` holds `originals`, with their times,
    /// metadata, and contents.
    async fn assert_holds(dir: &Path, key: &DataKey, originals: &[ItemMeta]) {
        let store = open(dir, key).await.unwrap();
        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), originals.len(), "{listed:?}");
        for original in originals {
            let copy = listed
                .iter()
                .find(|meta| meta.sha256 == original.sha256)
                .expect("every item is kept");
            assert_eq!(
                (
                    copy.created_at,
                    &copy.name,
                    &copy.device,
                    copy.kind,
                    copy.size
                ),
                (
                    original.created_at,
                    &original.name,
                    &original.device,
                    original.kind,
                    original.size
                )
            );
            let (_, mut content) = store.get(&copy.id).await.unwrap();
            let mut bytes = Vec::new();
            content.read_to_end(&mut bytes).await.unwrap();
            let mut hasher = ContentHasher::new();
            hasher.update(&bytes);
            assert_eq!(hasher.finalize().sha256_hex(), original.sha256);
        }
    }

    /// Every file and folder below `root`, except staging folders.
    fn snapshot(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
        let mut found = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                if rel == "tmp" || rel.starts_with("tmp/") || rel.starts_with("v2/tmp") {
                    continue;
                }
                if path.is_dir() {
                    found.insert(rel, None);
                    pending.push(path);
                } else {
                    found.insert(rel, Some(std::fs::read(&path).unwrap()));
                }
            }
        }
        found
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn a_migration_encrypts_every_item_keeping_its_time_and_metadata() {
        let dir = TempDir::new().unwrap();
        let originals = plain_items(dir.path()).await;
        let fs = LocalFs::new(dir.path());
        let key = migrate(&fs, &words(W1), quick(), clock()).await.unwrap();
        assert_eq!(
            inspect(&fs).await.unwrap(),
            StoreState::Encrypted {
                key_id: key.key_id(),
                plain_left: 0
            }
        );
        assert!(dir.path().join("items").is_file());
        assert!(!dir.path().join(".rewrite").exists());
        assert_eq!(read_plan(&fs).await.unwrap(), None);
        assert_holds(dir.path(), &key, &originals).await;
    }

    #[tokio::test]
    async fn a_rotation_moves_every_item_to_a_new_key_and_shuts_out_the_old() {
        let dir = TempDir::new().unwrap();
        let (old, originals) = sealed_items(dir.path()).await;
        let fs = LocalFs::new(dir.path());
        let stranger = DataKey::generate().unwrap();
        assert!(matches!(
            rotate(&fs, &stranger, &words(W2), quick(), clock()).await,
            Err(StoreError::Encryption(EncryptionError::KeyMismatch { .. }))
        ));
        let new = rotate(&fs, &old, &words(W2), quick(), clock())
            .await
            .unwrap();
        assert_ne!(new.key_id(), old.key_id());
        assert_holds(dir.path(), &new, &originals).await;
        let ids = open(dir.path(), &new)
            .await
            .unwrap()
            .list_ids()
            .await
            .unwrap();
        assert!(originals.iter().all(|meta| !ids.contains(&meta.id)));
        assert!(matches!(
            open(dir.path(), &old).await,
            Err(StoreError::Encryption(EncryptionError::KeyMismatch { .. }))
        ));
    }

    #[tokio::test]
    async fn a_second_rewrite_waits_for_the_first() {
        let dir = TempDir::new().unwrap();
        plain_items(dir.path()).await;
        std::fs::create_dir(dir.path().join(".rewrite")).unwrap();
        let fs = LocalFs::new(dir.path());
        assert!(matches!(
            migrate(&fs, &words(W1), quick(), clock()).await,
            Err(StoreError::Encryption(EncryptionError::Rewriting { .. }))
        ));
        assert!(!dir.path().join("v2").exists());
        // A lock without a plan is released by undo.
        undo(&fs).await.unwrap();
        assert!(!dir.path().join(".rewrite").exists());
        assert!(undo(&fs).await.is_err());
        assert!(finish(&fs, &words(W1), None, None, clock()).await.is_err());
    }

    /// How many calls of each kind a whole migration of `template` makes.
    async fn migration_calls(template: &Path) -> [usize; 9] {
        let scratch = TempDir::new().unwrap();
        copy_tree(template, scratch.path());
        let fs = FaultyFs::new(LocalFs::new(scratch.path()));
        migrate(&fs, &words(W1), quick(), clock()).await.unwrap();
        OPS.map(|op| fs.calls(op))
    }

    #[tokio::test]
    async fn resuming_uploads_nothing_already_published() {
        let template = TempDir::new().unwrap();
        let originals = plain_items(template.path()).await;
        let reads = migration_calls(template.path()).await[2];
        let dir = TempDir::new().unwrap();
        copy_tree(template.path(), dir.path());
        // The last read is the check of the last re-encrypted item.
        let fs = FaultyFs::new(LocalFs::new(dir.path()));
        fs.fail_nth(FsOp::OpenRead, reads);
        assert!(migrate(&fs, &words(W1), quick(), clock()).await.is_err());
        let fs = FaultyFs::new(LocalFs::new(dir.path()));
        let key = finish(&fs, &words(W1), None, None, clock()).await.unwrap();
        assert_eq!(
            fs.calls(FsOp::OpenWrite),
            1,
            "only the stop file is written again"
        );
        assert_holds(dir.path(), &key, &originals).await;
    }

    /// Recovers a store left locked by a cut run, by finishing it at `cut`
    /// and undoing it in a copy: each must either restore `before` or leave
    /// every item under the new key.
    async fn recover_both_ways(
        cut: &Path,
        before: &BTreeMap<String, Option<Vec<u8>>>,
        originals: &[ItemMeta],
        new_words: &Words,
        old_key: Option<&DataKey>,
        at: &str,
    ) {
        let copy = TempDir::new().unwrap();
        copy_tree(cut, copy.path());
        let local = LocalFs::new(cut);
        match finish(&local, new_words, old_key, None, clock()).await {
            Ok(key) => assert_holds(cut, &key, originals).await,
            Err(_) => {
                undo(&local)
                    .await
                    .unwrap_or_else(|err| panic!("{at}: {err}"));
                assert_eq!(&snapshot(cut), before, "{at}: undo after a refused finish");
            }
        }
        let local = LocalFs::new(copy.path());
        match undo(&local).await {
            Ok(()) => assert_eq!(&snapshot(copy.path()), before, "{at}: undo"),
            Err(_) => {
                let key = finish(&local, new_words, old_key, None, clock())
                    .await
                    .unwrap_or_else(|err| panic!("{at}: {err}"));
                assert_holds(copy.path(), &key, originals).await;
            }
        }
    }

    #[tokio::test]
    async fn a_migration_cut_at_any_call_is_finished_or_undone_without_loss() {
        let template = TempDir::new().unwrap();
        let originals = plain_items(template.path()).await;
        let before = snapshot(template.path());
        let calls = migration_calls(template.path()).await;
        let mut recovered = 0;
        for (op, count) in OPS.into_iter().zip(calls) {
            for nth in 1..=count {
                let at = format!("{op:?} call {nth}");
                let cut = TempDir::new().unwrap();
                copy_tree(template.path(), cut.path());
                let fs = FaultyFs::new(LocalFs::new(cut.path()));
                fs.fail_nth(op, nth);
                let result = migrate(&fs, &words(W1), quick(), clock()).await;
                match (result, inspect(&LocalFs::new(cut.path())).await.unwrap()) {
                    (Ok(key), _) => assert_holds(cut.path(), &key, &originals).await,
                    (Err(_), StoreState::Plain { .. }) => {
                        assert_eq!(snapshot(cut.path()), before, "{at}");
                    }
                    (Err(_), StoreState::Rewriting { .. }) => {
                        recovered += 1;
                        recover_both_ways(cut.path(), &before, &originals, &words(W1), None, &at)
                            .await;
                    }
                    (Err(err), state) => panic!("{at}: {err} left {state:?}"),
                }
            }
        }
        assert!(recovered > 10, "only {recovered} cuts needed recovery");
    }

    /// No staged journal is left in `dir`, and `.rewrite/` is absent or
    /// holds a whole journal.
    async fn lock_is_whole_or_absent(dir: &Path) {
        if dir.join(".rewrite").exists() {
            assert!(matches!(
                read_journal(&LocalFs::new(dir)).await.unwrap(),
                Some(Journal::Rewrite(_))
            ));
        }
        let staged: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".rewrite-"))
            .collect();
        assert!(staged.is_empty(), "{staged:?}");
    }

    #[tokio::test]
    async fn the_lock_appears_only_with_a_whole_journal() {
        let template = TempDir::new().unwrap();
        plain_items(template.path()).await;
        let before = snapshot(template.path());
        for after in [0, 1, 40, usize::MAX] {
            let dir = TempDir::new().unwrap();
            copy_tree(template.path(), dir.path());
            let fs = FaultyFs::new(LocalFs::new(dir.path()));
            // The journal is the migration's first write.
            fs.cut_nth_write(1, after);
            assert!(
                migrate(&fs, &words(W1), quick(), clock()).await.is_err(),
                "cut after {after}"
            );
            lock_is_whole_or_absent(dir.path()).await;
            assert_eq!(snapshot(dir.path()), before, "cut after {after}");
        }
    }

    #[tokio::test]
    async fn a_lock_without_a_whole_journal_is_released_only_when_nothing_moved() {
        let dir = TempDir::new().unwrap();
        plain_items(dir.path()).await;
        let fs = LocalFs::new(dir.path());
        let lock = dir.path().join(".rewrite");
        // passalong 0.2.0 stopped after creating its plan file.
        std::fs::create_dir(&lock).unwrap();
        std::fs::write(lock.join("plan.json"), "").unwrap();
        assert!(matches!(
            read_journal(&fs).await.unwrap(),
            Some(Journal::Unreadable { .. })
        ));
        assert!(read_plan(&fs).await.is_err());
        assert!(finish(&fs, &words(W1), None, None, clock()).await.is_err());
        undo(&fs).await.unwrap();
        assert!(!lock.exists());

        // Half a plan, and the source already moved: nothing is touched.
        std::fs::create_dir(&lock).unwrap();
        std::fs::write(lock.join("plan.json"), r#"{"kind":"mig"#).unwrap();
        std::fs::create_dir(lock.join("source")).unwrap();
        std::fs::rename(dir.path().join("items"), lock.join("source/items")).unwrap();
        let err = undo(&fs).await.unwrap_err();
        assert!(err.to_string().contains("source"), "{err}");
        assert!(lock.join("source/items").is_dir());
        assert!(
            !lock.join("recovery").exists(),
            "a failed recovery gives up its claim"
        );
    }

    #[tokio::test]
    async fn two_recoveries_never_run_at_once() {
        use crate::encryption::{recovery_in_progress, release_recovery};
        let template = TempDir::new().unwrap();
        let originals = plain_items(template.path()).await;
        let renames = migration_calls(template.path()).await[4];
        let dir = TempDir::new().unwrap();
        copy_tree(template.path(), dir.path());
        let fs = FaultyFs::new(LocalFs::new(dir.path()));
        // The last rename puts the new header in place.
        fs.fail_nth(FsOp::Rename, renames);
        assert!(migrate(&fs, &words(W1), quick(), clock()).await.is_err());
        let fs = LocalFs::new(dir.path());
        assert_eq!(recovery_in_progress(&fs).await.unwrap(), None);

        // Wrong words: the failed finish gives up its claim.
        assert!(finish(&fs, &words(W2), None, None, clock()).await.is_err());
        assert_eq!(recovery_in_progress(&fs).await.unwrap(), None);

        // A recovery that is running shuts out a second.
        std::fs::create_dir(dir.path().join(".rewrite/recovery")).unwrap();
        let marker = recovery_in_progress(&fs).await.unwrap().unwrap();
        assert!(!marker.is_stale(chrono::Utc::now()));
        for err in [
            undo(&fs).await.err(),
            finish(&fs, &words(W1), None, None, clock()).await.err(),
        ] {
            assert!(
                matches!(
                    err,
                    Some(StoreError::Encryption(EncryptionError::Recovering {
                        started: Some(_)
                    }))
                ),
                "{err:?}"
            );
        }

        // Taken over, it finishes.
        release_recovery(&fs).await.unwrap();
        let key = finish(&fs, &words(W1), None, None, clock()).await.unwrap();
        assert_holds(dir.path(), &key, &originals).await;
        assert!(!dir.path().join(".rewrite").exists());
    }

    #[tokio::test]
    async fn a_rotation_cut_at_any_call_is_finished_or_undone_without_loss() {
        let template = TempDir::new().unwrap();
        let (old, originals) = sealed_items(template.path()).await;
        let before = snapshot(template.path());
        let calls = {
            let scratch = TempDir::new().unwrap();
            copy_tree(template.path(), scratch.path());
            let fs = FaultyFs::new(LocalFs::new(scratch.path()));
            rotate(&fs, &old, &words(W2), quick(), clock())
                .await
                .unwrap();
            OPS.map(|op| fs.calls(op))
        };
        let mut recovered = 0;
        for (op, count) in OPS.into_iter().zip(calls) {
            for nth in 1..=count {
                let at = format!("{op:?} call {nth}");
                let cut = TempDir::new().unwrap();
                copy_tree(template.path(), cut.path());
                let fs = FaultyFs::new(LocalFs::new(cut.path()));
                fs.fail_nth(op, nth);
                let result = rotate(&fs, &old, &words(W2), quick(), clock()).await;
                match (result, inspect(&LocalFs::new(cut.path())).await.unwrap()) {
                    (Ok(key), _) => assert_holds(cut.path(), &key, &originals).await,
                    (Err(_), StoreState::Encrypted { key_id, .. }) if key_id == old.key_id() => {
                        assert_eq!(snapshot(cut.path()), before, "{at}");
                    }
                    (Err(_), StoreState::Rewriting { .. }) => {
                        recovered += 1;
                        recover_both_ways(
                            cut.path(),
                            &before,
                            &originals,
                            &words(W2),
                            Some(&old),
                            &at,
                        )
                        .await;
                    }
                    (Err(err), state) => panic!("{at}: {err} left {state:?}"),
                }
            }
        }
        assert!(recovered > 10, "only {recovered} cuts needed recovery");
    }
}
