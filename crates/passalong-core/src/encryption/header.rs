//! The store header, `encryption/header.json`: the wrapped data key and how
//! to derive the key that unwraps it. It names no device and no time.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{ENCRYPTION_DIR, EncryptionError, HEADER_FILE, SEALED_TMP_DIR, STOP_FILE, STOP_TEXT};
use crate::crypto::{KdfParams, KeyId, WrappedKey, random_bytes};
use crate::fs::{FsError, RemoteFs, RemotePath};
use crate::store::StoreError;

const FORMAT: u32 = 1;
const KDF_ALG: &str = "argon2id";
const KDF_VERSION: u32 = 0x13;
const MAX_HEADER_BYTES: u64 = 16 * 1024;

/// A store's header: its wrapped data key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreHeader {
    wrapped: WrappedKey,
}

#[derive(Serialize, Deserialize)]
struct HeaderJson {
    format: u32,
    key_id: String,
    kdf: KdfJson,
    wrapped_key: WrappedJson,
}

#[derive(Serialize, Deserialize)]
struct KdfJson {
    alg: String,
    version: u32,
    m_kib: u32,
    t: u32,
    p: u32,
    salt: String,
}

#[derive(Serialize, Deserialize)]
struct WrappedJson {
    nonce: String,
    ciphertext: String,
}

#[derive(Deserialize)]
struct FormatOnly {
    format: u32,
}

fn invalid(reason: impl Into<String>) -> EncryptionError {
    EncryptionError::Header(reason.into())
}

fn decode<const N: usize>(field: &str, text: &str) -> Result<[u8; N], EncryptionError> {
    hex::decode(text)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| invalid(format!("`{field}` is not {N} bytes of hex")))
}

impl StoreHeader {
    /// The header recording `wrapped`.
    pub fn new(wrapped: WrappedKey) -> Self {
        Self { wrapped }
    }

    /// The wrapped data key.
    pub fn wrapped(&self) -> &WrappedKey {
        &self.wrapped
    }

    /// Id of the store's data key.
    pub fn key_id(&self) -> KeyId {
        self.wrapped.key_id
    }

    /// The header as pretty JSON.
    pub fn to_json(&self) -> Vec<u8> {
        let w = &self.wrapped;
        let json = HeaderJson {
            format: FORMAT,
            key_id: w.key_id.to_string(),
            kdf: KdfJson {
                alg: KDF_ALG.to_owned(),
                version: KDF_VERSION,
                m_kib: w.kdf.m_kib,
                t: w.kdf.t,
                p: w.kdf.p,
                salt: hex::encode(w.kdf.salt),
            },
            wrapped_key: WrappedJson {
                nonce: hex::encode(w.nonce),
                ciphertext: hex::encode(&w.ciphertext),
            },
        };
        let mut bytes = serde_json::to_vec_pretty(&json).expect("the header serialises");
        bytes.push(b'\n');
        bytes
    }

    /// Parses a header.
    ///
    /// # Errors
    ///
    /// [`EncryptionError::Header`] for anything but a valid format-1
    /// header.
    pub fn parse(bytes: &[u8]) -> Result<Self, EncryptionError> {
        let format: FormatOnly =
            serde_json::from_slice(bytes).map_err(|err| invalid(err.to_string()))?;
        if format.format != FORMAT {
            return Err(invalid(format!(
                "format {} needs a newer passalong",
                format.format
            )));
        }
        let json: HeaderJson =
            serde_json::from_slice(bytes).map_err(|err| invalid(err.to_string()))?;
        if json.kdf.alg != KDF_ALG || json.kdf.version != KDF_VERSION {
            return Err(invalid(format!(
                "key derivation {} version {} is not supported",
                json.kdf.alg, json.kdf.version
            )));
        }
        let key_id = KeyId::parse(&json.key_id).map_err(|err| invalid(err.to_string()))?;
        let ciphertext = hex::decode(&json.wrapped_key.ciphertext)
            .map_err(|_| invalid("`ciphertext` is not hex"))?;
        Ok(Self::new(WrappedKey {
            key_id,
            kdf: KdfParams {
                m_kib: json.kdf.m_kib,
                t: json.kdf.t,
                p: json.kdf.p,
                salt: decode("salt", &json.kdf.salt)?,
            },
            nonce: decode("nonce", &json.wrapped_key.nonce)?,
            ciphertext,
        }))
    }
}

/// `encryption/`.
pub(crate) fn encryption_dir() -> Result<RemotePath, FsError> {
    RemotePath::new(ENCRYPTION_DIR)
}

/// `encryption/header.json`.
pub(crate) fn header_path() -> Result<RemotePath, FsError> {
    encryption_dir()?.join(HEADER_FILE)
}

/// Reads the store header: `None` when the store has none.
///
/// # Errors
///
/// [`EncryptionError::Header`] for a header that cannot be parsed, and the
/// filesystem's error otherwise.
pub async fn read_header<F: RemoteFs + ?Sized>(fs: &F) -> Result<Option<StoreHeader>, StoreError> {
    let reader = match fs.open_read(&header_path()?).await {
        Ok(reader) => reader,
        Err(FsError::NotFound(_)) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let mut bytes = Vec::new();
    reader
        .take(MAX_HEADER_BYTES)
        .read_to_end(&mut bytes)
        .await
        .map_err(|err| invalid(format!("reading it failed: {err}")))?;
    Ok(Some(StoreHeader::parse(&bytes)?))
}

/// Writes `header` into a new folder under `v2/tmp/`, returning the folder.
async fn stage<F: RemoteFs + ?Sized>(
    fs: &F,
    header: &StoreHeader,
    name: &str,
) -> Result<RemotePath, StoreError> {
    let token: [u8; 8] = random_bytes()?;
    let tmp = RemotePath::new(SEALED_TMP_DIR)?;
    let dir = tmp.join(&format!("{name}-{}", hex::encode(token)))?;
    fs.create_dir_all(&dir).await?;
    let path = dir.join(HEADER_FILE)?;
    let mut writer = fs.open_write(&path).await?;
    writer
        .write_all(&header.to_json())
        .await
        .map_err(|err| FsError::from_io(&path, err))?;
    writer
        .shutdown()
        .await
        .map_err(|err| FsError::from_io(&path, err))?;
    Ok(dir)
}

/// Creates the store header. The header appears in one rename, and never
/// replaces another: a store that already has one is refused.
///
/// # Errors
///
/// [`EncryptionError::AlreadyEncrypted`] when the store has a header, and
/// the filesystem's error otherwise.
pub async fn create_header<F: RemoteFs + ?Sized>(
    fs: &F,
    header: &StoreHeader,
) -> Result<(), StoreError> {
    let target = encryption_dir()?;
    if fs.stat(&target).await?.is_some() {
        return Err(EncryptionError::AlreadyEncrypted.into());
    }
    let staged = stage(fs, header, "header").await?;
    match fs.rename(&staged, &target).await {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs.remove_dir_all(&staged).await;
            Err(match err {
                FsError::AlreadyExists(_) => EncryptionError::AlreadyEncrypted.into(),
                err => err.into(),
            })
        }
    }
}

/// Replaces the store header: the new one is written aside, the old folder
/// moved away, and the new one moved into place. Between the two renames
/// the store has no header, and clients refuse it rather than treat it as
/// plaintext, because of the [`STOP_FILE`].
///
/// # Errors
///
/// The filesystem's error. When the second rename fails, the old header is
/// left in `v2/tmp/old-header-*` for recovery.
pub async fn replace_header<F: RemoteFs + ?Sized>(
    fs: &F,
    header: &StoreHeader,
) -> Result<(), StoreError> {
    let target = encryption_dir()?;
    let staged = stage(fs, header, "header").await?;
    let token: [u8; 8] = random_bytes()?;
    let old =
        RemotePath::new(SEALED_TMP_DIR)?.join(&format!("old-header-{}", hex::encode(token)))?;
    if let Err(err) = fs.rename(&target, &old).await {
        let _ = fs.remove_dir_all(&staged).await;
        return Err(err.into());
    }
    fs.rename(&staged, &target).await?;
    if let Err(err) = fs.remove_dir_all(&old).await {
        tracing::warn!(path = %old, error = %err, "could not remove the previous header");
    }
    Ok(())
}

/// Writes the [`STOP_FILE`] where clients before v0.2.0 look for items.
///
/// # Errors
///
/// [`EncryptionError::Layout`] when `items` is a folder, and the
/// filesystem's error otherwise.
pub async fn write_stop_file<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    let path = RemotePath::new(STOP_FILE)?;
    if fs.stat(&path).await?.is_some_and(|meta| meta.is_dir) {
        return Err(EncryptionError::Layout(format!(
            "`{STOP_FILE}` is a folder; its items must be moved first"
        ))
        .into());
    }
    let mut writer = fs.open_write(&path).await?;
    writer
        .write_all(STOP_TEXT.as_bytes())
        .await
        .map_err(|err| FsError::from_io(&path, err))?;
    writer
        .shutdown()
        .await
        .map_err(|err| FsError::from_io(&path, err))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::crypto::{DataKey, KDF_SALT_LEN, Words, wrap};
    use crate::fs::LocalFs;
    use crate::testing::{FaultyFs, FsOp};
    use tempfile::TempDir;

    pub(crate) fn quick_header(key: &DataKey) -> StoreHeader {
        let kdf = KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [3; KDF_SALT_LEN],
        };
        let words = Words::parse("abacus zoom abdomen abacus zoom abdomen").unwrap();
        StoreHeader::new(wrap(key, &words, kdf).unwrap())
    }

    #[test]
    fn headers_round_trip_and_name_no_device_or_time() {
        let header = quick_header(&DataKey::generate().unwrap());
        let json = header.to_json();
        assert_eq!(StoreHeader::parse(&json).unwrap(), header);
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["format"], 1);
        assert_eq!(value["kdf"]["alg"], "argon2id");
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["format", "kdf", "key_id", "wrapped_key"]);
    }

    #[test]
    fn unknown_formats_and_bad_fields_are_refused() {
        let header = quick_header(&DataKey::generate().unwrap());
        let json = String::from_utf8(header.to_json()).unwrap();
        for bad in [
            json.replace("\"format\": 1", "\"format\": 2"),
            json.replace("argon2id", "scrypt"),
            json.replace(&header.key_id().to_string(), "nothex"),
            "{}".to_owned(),
            "not json".to_owned(),
        ] {
            assert!(
                matches!(
                    StoreHeader::parse(bad.as_bytes()),
                    Err(EncryptionError::Header(_))
                ),
                "{bad}"
            );
        }
        let newer = json.replace("\"format\": 1", "\"format\": 2");
        assert!(
            StoreHeader::parse(newer.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("newer passalong")
        );
    }

    #[tokio::test]
    async fn a_header_is_created_once_and_read_back() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        assert_eq!(read_header(&fs).await.unwrap(), None);
        let header = quick_header(&DataKey::generate().unwrap());
        create_header(&fs, &header).await.unwrap();
        assert_eq!(read_header(&fs).await.unwrap(), Some(header.clone()));
        let other = quick_header(&DataKey::generate().unwrap());
        assert!(matches!(
            create_header(&fs, &other).await,
            Err(StoreError::Encryption(EncryptionError::AlreadyEncrypted))
        ));
        assert_eq!(read_header(&fs).await.unwrap(), Some(header));
        let staging: Vec<_> = std::fs::read_dir(dir.path().join("v2/tmp"))
            .unwrap()
            .collect();
        assert!(staging.is_empty());
    }

    #[tokio::test]
    async fn replacing_a_header_swaps_it_and_a_failed_swap_leaves_none() {
        let dir = TempDir::new().unwrap();
        let fs = FaultyFs::new(LocalFs::new(dir.path()));
        create_header(&fs, &quick_header(&DataKey::generate().unwrap()))
            .await
            .unwrap();
        let second = quick_header(&DataKey::generate().unwrap());
        replace_header(&fs, &second).await.unwrap();
        assert_eq!(read_header(&fs).await.unwrap(), Some(second));
        assert!(
            std::fs::read_dir(dir.path().join("v2/tmp"))
                .unwrap()
                .next()
                .is_none()
        );

        // The old header moves away, then moving the new one in fails.
        let renames = fs.calls(FsOp::Rename);
        fs.fail_nth(FsOp::Rename, renames + 2);
        let third = quick_header(&DataKey::generate().unwrap());
        assert!(replace_header(&fs, &third).await.is_err());
        assert_eq!(read_header(&fs).await.unwrap(), None);
        let kept: Vec<String> = std::fs::read_dir(dir.path().join("v2/tmp"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(
            kept.iter().any(|name| name.starts_with("old-header-")),
            "{kept:?}"
        );
    }

    #[tokio::test]
    async fn the_stop_file_replaces_nothing_but_a_missing_items() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        write_stop_file(&fs).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("items")).unwrap(),
            STOP_TEXT
        );
        std::fs::remove_file(dir.path().join("items")).unwrap();
        std::fs::create_dir(dir.path().join("items")).unwrap();
        assert!(matches!(
            write_stop_file(&fs).await,
            Err(StoreError::Encryption(EncryptionError::Layout(_)))
        ));
    }
}
