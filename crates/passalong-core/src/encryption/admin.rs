//! Changing a store's encryption: setting it up on an empty store, a fresh
//! start that keeps the earlier items unencrypted in `plain/`, joining it
//! with its words, and changing its words.

use std::sync::Arc;

use super::open::rewrite_started;
use super::{
    EncryptionError, PLAIN_DIR, REWRITE_DIR, STOP_FILE, StoreHeader, create_header, read_header,
    replace_header, write_stop_file,
};
use crate::clock::SystemClock;
use crate::crypto::{DataKey, KdfParams, KeyId, Words, unwrap, wrap};
use crate::fs::{FsError, RemoteFs, RemotePath, SubFs};
use crate::random::StdRandom;
use crate::store::{FsStore, Store, StoreError};

/// What a store's root says about its encryption.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreState {
    /// No header: a plaintext store.
    Plain {
        /// How many items it holds.
        items: usize,
    },
    /// An encrypted store.
    Encrypted {
        /// Id of its data key.
        key_id: KeyId,
        /// Plaintext items a fresh start left in `plain/items/`.
        plain_left: usize,
    },
    /// Its items are being re-encrypted.
    Rewriting {
        /// When that started, if known.
        started: Option<String>,
    },
    /// `items` is a file, but there is no header.
    Broken,
}

fn items_path() -> RemotePath {
    RemotePath::new(STOP_FILE).expect("a valid path")
}

fn plain_path() -> RemotePath {
    RemotePath::new(PLAIN_DIR).expect("a valid path")
}

/// The plaintext items a fresh start kept, as a store of their own, for
/// listing and pruning them.
pub fn plain_store<F: RemoteFs + ?Sized>(fs: &F) -> FsStore<SubFs<&F>> {
    FsStore::new(
        SubFs::new(fs, plain_path()),
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
    )
}

/// Reads what the store's root says about its encryption.
///
/// # Errors
///
/// The filesystem's error, or [`EncryptionError::Header`] for a header
/// that cannot be read.
pub async fn inspect<F: RemoteFs + ?Sized>(fs: &F) -> Result<StoreState, StoreError> {
    if fs.stat(&RemotePath::new(REWRITE_DIR)?).await?.is_some() {
        return Ok(StoreState::Rewriting {
            started: rewrite_started(fs).await,
        });
    }
    if let Some(header) = read_header(fs).await? {
        let plain_left = plain_store(fs).list_ids().await?.len();
        return Ok(StoreState::Encrypted {
            key_id: header.key_id(),
            plain_left,
        });
    }
    if fs
        .stat(&items_path())
        .await?
        .is_some_and(|meta| !meta.is_dir)
    {
        return Ok(StoreState::Broken);
    }
    let plain = FsStore::new(fs, Arc::new(SystemClock), Box::new(StdRandom::new()));
    Ok(StoreState::Plain {
        items: plain.list_ids().await?.len(),
    })
}

/// The error for a store in a state the operation cannot start from.
pub(super) fn refuse(state: StoreState) -> StoreError {
    match state {
        StoreState::Plain { .. } => EncryptionError::NotEncrypted.into(),
        StoreState::Encrypted { .. } => EncryptionError::AlreadyEncrypted.into(),
        StoreState::Rewriting { started } => EncryptionError::Rewriting { started }.into(),
        StoreState::Broken => EncryptionError::HeaderMissing.into(),
    }
}

fn new_key(words: &Words, kdf: KdfParams) -> Result<(DataKey, StoreHeader), StoreError> {
    let key = DataKey::generate()?;
    let header = StoreHeader::new(wrap(&key, words, kdf)?);
    Ok((key, header))
}

/// Encrypts an empty plaintext store under a new data key wrapped with
/// `words`, and returns the key. The stop file comes first, so clients
/// before v0.2.0 are shut out before the header appears.
///
/// # Errors
///
/// [`EncryptionError::Layout`] when the store holds items or other files,
/// the refusal for a store that is not plaintext, and the filesystem's
/// error otherwise.
pub async fn set_up<F: RemoteFs + ?Sized>(
    fs: &F,
    words: &Words,
    kdf: KdfParams,
) -> Result<DataKey, StoreError> {
    match inspect(fs).await? {
        StoreState::Plain { items: 0 } => {}
        StoreState::Plain { items } => {
            return Err(EncryptionError::Layout(format!(
                "the store holds {items} items; encrypt it with a fresh start or by migrating"
            ))
            .into());
        }
        other => return Err(refuse(other)),
    }
    let (key, header) = new_key(words, kdf)?;
    let items = items_path();
    if fs.stat(&items).await?.is_some_and(|meta| meta.is_dir) {
        if !fs.read_dir(&items).await?.is_empty() {
            return Err(EncryptionError::Layout(format!(
                "`{STOP_FILE}` holds files that are not items; move them away first"
            ))
            .into());
        }
        fs.remove_dir_all(&items).await?;
    }
    write_stop_file(fs).await?;
    create_header(fs, &header).await?;
    Ok(key)
}

/// Encrypts a plaintext store under a new data key wrapped with `words`,
/// without re-encrypting its items: they move to `plain/items/`, where the
/// storage can read them until `prune --plain` removes them. Returns the
/// key.
///
/// # Errors
///
/// The refusal for a store that is not plaintext, and the filesystem's
/// error otherwise.
pub async fn fresh_start<F: RemoteFs + ?Sized>(
    fs: &F,
    words: &Words,
    kdf: KdfParams,
) -> Result<DataKey, StoreError> {
    match inspect(fs).await? {
        StoreState::Plain { .. } => {}
        other => return Err(refuse(other)),
    }
    let (key, header) = new_key(words, kdf)?;
    move_items_to_plain(fs).await?;
    if let Err(err) = write_stop_file(fs).await {
        // A client before v0.2.0 recreated `items/` in between: move what
        // it stored as well, once.
        if !matches!(err, StoreError::Encryption(EncryptionError::Layout(_))) {
            return Err(err);
        }
        move_items_to_plain(fs).await?;
        write_stop_file(fs).await?;
    }
    create_header(fs, &header).await?;
    Ok(key)
}

/// Moves `items/` to `plain/items/`: in one rename when there is no
/// `plain/items/` yet, and item by item otherwise.
async fn move_items_to_plain<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    let items = items_path();
    if !fs.stat(&items).await?.is_some_and(|meta| meta.is_dir) {
        return Ok(());
    }
    let target = plain_path().join(STOP_FILE)?;
    fs.create_dir_all(&plain_path()).await?;
    if fs.stat(&target).await?.is_none() {
        fs.rename(&items, &target).await?;
        return Ok(());
    }
    for entry in fs.read_dir(&items).await? {
        let from = items.join(&entry.name)?;
        match fs.rename(&from, &target.join(&entry.name)?).await {
            Ok(()) => {}
            // The same item was moved before.
            Err(FsError::AlreadyExists(_)) => fs.remove_dir_all(&from).await?,
            Err(err) => return Err(err.into()),
        }
    }
    fs.remove_dir_all(&items).await?;
    Ok(())
}

/// This device's key for an encrypted store: its data key, unwrapped with
/// the store's `words`.
///
/// # Errors
///
/// [`EncryptionError::NotEncrypted`] and the other refusals for a store
/// that is not encrypted, and [`CryptoError::WrongWords`] for wrong words.
///
/// [`CryptoError::WrongWords`]: crate::crypto::CryptoError::WrongWords
pub async fn join<F: RemoteFs + ?Sized>(fs: &F, words: &Words) -> Result<DataKey, StoreError> {
    match inspect(fs).await? {
        StoreState::Encrypted { .. } => {}
        other => return Err(refuse(other)),
    }
    let header = read_header(fs)
        .await?
        .ok_or(EncryptionError::HeaderMissing)?;
    Ok(unwrap(header.wrapped(), words)?)
}

/// Replaces an encrypted store's words: its data key is unwrapped with
/// `current` and wrapped again with `new`, so no item is re-encrypted and
/// every device that joined keeps working. Returns the key's id.
///
/// # Errors
///
/// As [`join`], and the filesystem's error while replacing the header.
pub async fn change_words<F: RemoteFs + ?Sized>(
    fs: &F,
    current: &Words,
    new: &Words,
    kdf: KdfParams,
) -> Result<KeyId, StoreError> {
    let key = join(fs, current).await?;
    replace_header(fs, &StoreHeader::new(wrap(&key, new, kdf)?)).await?;
    Ok(key.key_id())
}

/// Removes `plain/` once it holds no items; `true` when it was removed.
///
/// # Errors
///
/// The filesystem's error.
pub async fn remove_plain_if_empty<F: RemoteFs + ?Sized>(fs: &F) -> Result<bool, StoreError> {
    let plain = plain_path();
    if fs.stat(&plain).await?.is_none() || !plain_store(fs).list_ids().await?.is_empty() {
        return Ok(false);
    }
    fs.remove_dir_all(&plain).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{CryptoError, KDF_SALT_LEN};
    use crate::encryption::open_with_key;
    use crate::fs::LocalFs;
    use crate::model::NewItem;
    use crate::testing::ManualClock;
    use tempfile::TempDir;

    const W1: &str = "abacus abdomen abdominal abide abiding ability";
    const W2: &str = "zoom zoom zoom zoom zoom zoom";

    fn words(text: &str) -> Words {
        Words::parse(text).unwrap()
    }

    fn quick() -> KdfParams {
        KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [4; KDF_SALT_LEN],
        }
    }

    fn text(bytes: &[u8]) -> crate::fs::BoxRead {
        Box::new(std::io::Cursor::new(bytes.to_vec()))
    }

    async fn seed(fs: &LocalFs, n: usize) {
        let store = FsStore::new(
            fs.clone(),
            Arc::new(ManualClock::at("2026-09-12T09:53:11Z")),
            Box::new(StdRandom::new()),
        );
        for i in 0..n {
            store
                .put(NewItem::text("box"), text(format!("old {i}").as_bytes()))
                .await
                .unwrap();
        }
    }

    async fn open(fs: &LocalFs, key: &DataKey) -> Box<dyn Store> {
        open_with_key(
            fs.clone(),
            Some(key.clone()),
            Arc::new(SystemClock),
            Box::new(StdRandom::new()),
        )
        .await
        .unwrap()
    }

    fn is(err: StoreError, want: &EncryptionError) -> bool {
        matches!(err, StoreError::Encryption(got) if got == *want)
    }

    #[tokio::test]
    async fn inspect_tells_every_state_apart() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        assert_eq!(inspect(&fs).await.unwrap(), StoreState::Plain { items: 0 });
        seed(&fs, 2).await;
        assert_eq!(inspect(&fs).await.unwrap(), StoreState::Plain { items: 2 });

        let other = TempDir::new().unwrap();
        let fs = LocalFs::new(other.path());
        write_stop_file(&fs).await.unwrap();
        assert_eq!(inspect(&fs).await.unwrap(), StoreState::Broken);
        std::fs::create_dir(other.path().join(".rewrite")).unwrap();
        assert_eq!(
            inspect(&fs).await.unwrap(),
            StoreState::Rewriting { started: None }
        );
    }

    #[tokio::test]
    async fn set_up_encrypts_an_empty_store_once() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        std::fs::create_dir(dir.path().join("items")).unwrap();
        let key = set_up(&fs, &words(W1), quick()).await.unwrap();
        assert_eq!(
            inspect(&fs).await.unwrap(),
            StoreState::Encrypted {
                key_id: key.key_id(),
                plain_left: 0
            }
        );
        assert!(dir.path().join("items").is_file());
        open(&fs, &key)
            .await
            .put(NewItem::text("box"), text(b"sealed"))
            .await
            .unwrap();
        assert!(is(
            set_up(&fs, &words(W1), quick()).await.unwrap_err(),
            &EncryptionError::AlreadyEncrypted
        ));
    }

    #[tokio::test]
    async fn set_up_refuses_a_store_with_items_or_other_files() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        seed(&fs, 1).await;
        assert!(matches!(
            set_up(&fs, &words(W1), quick()).await,
            Err(StoreError::Encryption(EncryptionError::Layout(_)))
        ));
        let other = TempDir::new().unwrap();
        std::fs::create_dir_all(other.path().join("items/not-an-item")).unwrap();
        let fs = LocalFs::new(other.path());
        assert!(matches!(
            set_up(&fs, &words(W1), quick()).await,
            Err(StoreError::Encryption(EncryptionError::Layout(_)))
        ));
        assert!(!other.path().join("encryption").exists());
    }

    #[tokio::test]
    async fn a_fresh_start_keeps_the_old_items_in_plain() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        seed(&fs, 2).await;
        let before = FsStore::new(
            fs.clone(),
            Arc::new(SystemClock),
            Box::new(StdRandom::new()),
        )
        .list_ids()
        .await
        .unwrap();
        let key = fresh_start(&fs, &words(W1), quick()).await.unwrap();
        assert_eq!(
            inspect(&fs).await.unwrap(),
            StoreState::Encrypted {
                key_id: key.key_id(),
                plain_left: 2
            }
        );
        assert_eq!(plain_store(&fs).list_ids().await.unwrap(), before);
        assert!(open(&fs, &key).await.list().await.unwrap().is_empty());

        assert!(!remove_plain_if_empty(&fs).await.unwrap());
        let plain = plain_store(&fs);
        for id in &before {
            plain.delete(id).await.unwrap();
        }
        assert!(remove_plain_if_empty(&fs).await.unwrap());
        assert!(!dir.path().join("plain").exists());
        assert!(!remove_plain_if_empty(&fs).await.unwrap());
    }

    #[tokio::test]
    async fn a_fresh_start_merges_into_an_existing_plain_folder() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        seed(&fs, 2).await;
        let first = FsStore::new(
            fs.clone(),
            Arc::new(SystemClock),
            Box::new(StdRandom::new()),
        )
        .list_ids()
        .await
        .unwrap()[0]
            .clone();
        std::fs::create_dir_all(dir.path().join("plain/items")).unwrap();
        let copy = dir.path().join(format!("plain/items/{first}"));
        std::fs::create_dir(&copy).unwrap();
        fresh_start(&fs, &words(W1), quick()).await.unwrap();
        // The earlier copy of the first item stays; the second one moves in.
        assert_eq!(plain_store(&fs).list_ids().await.unwrap().len(), 2);
        assert!(dir.path().join("items").is_file());
    }

    #[tokio::test]
    async fn joining_needs_the_store_s_words() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        assert!(is(
            join(&fs, &words(W1)).await.unwrap_err(),
            &EncryptionError::NotEncrypted
        ));
        let key = set_up(&fs, &words(W1), quick()).await.unwrap();
        assert_eq!(
            join(&fs, &words(W1)).await.unwrap().as_bytes(),
            key.as_bytes()
        );
        assert!(is(
            join(&fs, &words(W2)).await.unwrap_err(),
            &EncryptionError::Crypto(CryptoError::WrongWords)
        ));
    }

    #[tokio::test]
    async fn new_words_keep_the_key_and_rewrite_no_item() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        let key = set_up(&fs, &words(W1), quick()).await.unwrap();
        let item = open(&fs, &key)
            .await
            .put(NewItem::text("box"), text(b"kept"))
            .await
            .unwrap()
            .meta;
        let content = dir.path().join(format!("v2/items/{}/content", item.id));
        let before = std::fs::read(&content).unwrap();

        assert!(is(
            change_words(&fs, &words(W2), &words(W2), quick())
                .await
                .unwrap_err(),
            &EncryptionError::Crypto(CryptoError::WrongWords)
        ));
        let id = change_words(&fs, &words(W1), &words(W2), quick())
            .await
            .unwrap();
        assert_eq!(id, key.key_id());
        assert_eq!(
            join(&fs, &words(W2)).await.unwrap().as_bytes(),
            key.as_bytes()
        );
        assert!(join(&fs, &words(W1)).await.is_err());
        assert_eq!(std::fs::read(&content).unwrap(), before);
        assert_eq!(open(&fs, &key).await.list().await.unwrap(), vec![item]);
    }
}
