//! Client-side encryption: the key hierarchy, key wrapping, the keyed
//! content key, and sealed metadata and content.
//!
//! # Keys
//!
//! An encrypted store has one random 256-bit [`DataKey`]. Its [`KeyId`] is
//! the first 8 bytes of `HKDF-SHA256(data key, info "passalong key id v1")`.
//! The storage holds the data key only wrapped ([`WrappedKey`]):
//! AES-256-GCM under a key that Argon2id derives from six generated
//! [`Words`] and a random 16-byte salt, with `passalong key v1` and the key
//! id as associated data.
//!
//! Subkeys come from HKDF-SHA256 over the data key:
//!
//! - `passalong id v1`: the HMAC-SHA256 key of the keyed content key,
//!   `HMAC(id key, SHA-256 of the content)`, of which the first 6 bytes
//!   replace the plain content key in item ids;
//! - `passalong meta v1`: the AES-256-GCM key that seals metadata, with a
//!   random 12-byte nonce and `passalong meta v1` plus the item id as
//!   associated data;
//! - `passalong content v1`, salted with 32 random bytes per item: the
//!   AES-256-GCM key of that item's content, in the chunked format of
//!   [`ContentSealer`] and [`OpenReader`].
//!
//! Every secret is zeroised when dropped, and `Debug` never shows one.

mod stream;
mod words;

use std::fmt;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::io::AsyncRead;
use zeroize::{Zeroize, Zeroizing};

use crate::model::{ContentDigest, ContentKey, ItemId};

pub use stream::{
    CHUNK_LEN, CONTENT_MAGIC, ContentSealer, HEADER_LEN, OpenReader, TAG_LEN, read_full, sealed_len,
};
pub use words::{WORD_COUNT, Words, word_list};

/// Length of a data key in bytes.
pub const KEY_LEN: usize = 32;
/// Length of an AES-GCM nonce in bytes.
pub const NONCE_LEN: usize = 12;
/// Length of an Argon2 salt in bytes.
pub const KDF_SALT_LEN: usize = 16;
/// Length of a content salt in bytes.
pub const CONTENT_SALT_LEN: usize = 32;

const INFO_KEY_ID: &[u8] = b"passalong key id v1";
const INFO_ID: &[u8] = b"passalong id v1";
const INFO_META: &[u8] = b"passalong meta v1";
const INFO_CONTENT: &[u8] = b"passalong content v1";
const AAD_KEY: &[u8] = b"passalong key v1";
const AAD_META: &[u8] = b"passalong meta v1";

/// Errors from encryption and decryption. Messages never contain secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// The operating system's random number generator failed.
    #[error("the system random number generator failed: {0}")]
    Random(String),
    /// Key derivation settings outside the accepted bounds.
    #[error("key derivation settings are not accepted: {0}")]
    KdfParams(String),
    /// The words do not unwrap the key.
    #[error("these words do not unlock the store's key")]
    WrongWords,
    /// The unwrapped key's id differs from the recorded one.
    #[error("the unlocked key does not match key id {0}")]
    KeyIdMismatch(String),
    /// Sealed data failed authentication: it was changed, truncated inside
    /// a record, or belongs to another item or key.
    #[error("authentication failed: the data was changed or belongs elsewhere")]
    Authentication,
    /// Sealed content ends before its final record.
    #[error("the sealed content is truncated")]
    Truncated,
    /// Data that is not in the expected format.
    #[error("{0}")]
    Format(String),
    /// A passphrase with the wrong number of words.
    #[error("expected {expected} words, got {got}")]
    WordCount {
        /// Words required.
        expected: usize,
        /// Words given.
        got: usize,
    },
    /// A word that is not in the word list; the word itself is not shown.
    #[error("word {0} is not in the word list")]
    UnknownWord(usize),
}

/// Fills a fixed-size array from the operating system's random number
/// generator.
pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|err| CryptoError::Random(err.to_string()))?;
    Ok(bytes)
}

fn cipher(key: &[u8; KEY_LEN]) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(key).expect("AES-256-GCM takes a 32-byte key")
}

fn nonce(bytes: [u8; NONCE_LEN]) -> aes_gcm::aead::Nonce<Aes256Gcm> {
    bytes.into()
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// A store's 256-bit data key. Zeroised on drop; `Debug` shows only its id.
#[derive(Clone)]
pub struct DataKey([u8; KEY_LEN]);

impl DataKey {
    /// A new random key.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Random`] when the system generator fails.
    pub fn generate() -> Result<Self, CryptoError> {
        Ok(Self(random_bytes()?))
    }

    /// The key with these bytes.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw key, for writing the key file.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// The key's public identifier.
    pub fn key_id(&self) -> KeyId {
        let okm = self.derive(None, INFO_KEY_ID);
        let mut id = [0_u8; KeyId::LEN];
        id.copy_from_slice(&okm[..KeyId::LEN]);
        KeyId(id)
    }

    /// `HKDF-SHA256(self, salt, info)`, 32 bytes.
    fn derive(&self, salt: Option<&[u8]>, info: &[u8]) -> Zeroizing<[u8; KEY_LEN]> {
        let mut okm = Zeroizing::new([0_u8; KEY_LEN]);
        Hkdf::<Sha256>::new(salt, &self.0)
            .expand(info, okm.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        okm
    }

    /// The AES-256-GCM cipher for the content salted with `salt`.
    pub(crate) fn content_cipher(&self, salt: &[u8; CONTENT_SALT_LEN]) -> Aes256Gcm {
        cipher(&self.derive(Some(salt), INFO_CONTENT))
    }
}

impl Drop for DataKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DataKey(id {})", self.key_id())
    }
}

/// The public identifier of a [`DataKey`]: 8 bytes, shown as 16 lowercase
/// hex digits. It names a key without revealing it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyId([u8; KeyId::LEN]);

impl KeyId {
    /// Length in bytes.
    pub const LEN: usize = 8;

    /// Parses 16 lowercase hex digits.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Format`] for anything else.
    pub fn parse(text: &str) -> Result<Self, CryptoError> {
        let bad =
            || CryptoError::Format(format!("invalid key id `{text}`: expected 16 hex digits"));
        if text.len() != 2 * Self::LEN || text.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(bad());
        }
        let bytes = hex::decode(text).map_err(|_| bad())?;
        let mut id = [0_u8; Self::LEN];
        id.copy_from_slice(&bytes);
        Ok(Self(id))
    }

    /// The raw bytes.
    pub fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }

    /// The first 8 hex digits, for messages.
    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyId({self})")
    }
}

// ---------------------------------------------------------------------------
// Key wrapping
// ---------------------------------------------------------------------------

/// Argon2id settings and salt for deriving the key that wraps a data key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    /// Iterations.
    pub t: u32,
    /// Lanes.
    pub p: u32,
    /// Random salt.
    pub salt: [u8; KDF_SALT_LEN],
}

impl KdfParams {
    /// Memory for new keys: 64 MiB, RFC 9106's second recommended option.
    pub const M_KIB: u32 = 65_536;
    /// Iterations for new keys.
    pub const T: u32 = 3;
    /// Lanes for new keys.
    pub const P: u32 = 4;
    /// Most memory accepted from a header: 1 GiB.
    pub const MAX_M_KIB: u32 = 1_048_576;
    /// Most iterations accepted from a header.
    pub const MAX_T: u32 = 10;
    /// Most lanes accepted from a header.
    pub const MAX_P: u32 = 16;

    /// The settings for a new key, with a random salt.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Random`] when the system generator fails.
    pub fn generate() -> Result<Self, CryptoError> {
        Ok(Self {
            m_kib: Self::M_KIB,
            t: Self::T,
            p: Self::P,
            salt: random_bytes()?,
        })
    }

    /// Refuses settings above the bounds, so a hostile header cannot make
    /// a client exhaust its memory or time, and settings Argon2 rejects.
    ///
    /// # Errors
    ///
    /// [`CryptoError::KdfParams`] naming the offending setting.
    pub fn check(&self) -> Result<(), CryptoError> {
        if self.m_kib > Self::MAX_M_KIB {
            return Err(CryptoError::KdfParams(format!(
                "memory {} KiB is above {} KiB",
                self.m_kib,
                Self::MAX_M_KIB
            )));
        }
        if self.t > Self::MAX_T {
            return Err(CryptoError::KdfParams(format!(
                "{} iterations is above {}",
                self.t,
                Self::MAX_T
            )));
        }
        if self.p > Self::MAX_P {
            return Err(CryptoError::KdfParams(format!(
                "{} lanes is above {}",
                self.p,
                Self::MAX_P
            )));
        }
        self.argon2().map(|_| ())
    }

    fn argon2(&self) -> Result<Argon2<'static>, CryptoError> {
        let params = Params::new(self.m_kib, self.t, self.p, Some(KEY_LEN))
            .map_err(|err| CryptoError::KdfParams(err.to_string()))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }

    /// The wrapping key for `words`.
    fn derive(&self, words: &Words) -> Result<Zeroizing<[u8; KEY_LEN]>, CryptoError> {
        self.check()?;
        let mut out = Zeroizing::new([0_u8; KEY_LEN]);
        self.argon2()?
            .hash_password_into(words.as_str().as_bytes(), &self.salt, out.as_mut())
            .map_err(|err| CryptoError::KdfParams(err.to_string()))?;
        Ok(out)
    }
}

/// A data key sealed under a key derived from words, as the store header
/// records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKey {
    /// Id of the wrapped key.
    pub key_id: KeyId,
    /// How the wrapping key is derived.
    pub kdf: KdfParams,
    /// AES-GCM nonce.
    pub nonce: [u8; NONCE_LEN],
    /// The sealed key and its tag.
    pub ciphertext: Vec<u8>,
}

fn key_aad(key_id: &KeyId) -> Vec<u8> {
    [AAD_KEY, key_id.as_bytes()].concat()
}

/// Wraps `key` under `words` with the settings and salt in `kdf`.
///
/// # Errors
///
/// [`CryptoError::KdfParams`] for settings outside the bounds, and
/// [`CryptoError::Random`] when the system generator fails.
pub fn wrap(key: &DataKey, words: &Words, kdf: KdfParams) -> Result<WrappedKey, CryptoError> {
    let wrapping = kdf.derive(words)?;
    let key_id = key.key_id();
    let nonce_bytes = random_bytes()?;
    let ciphertext = cipher(&wrapping)
        .encrypt(
            &nonce(nonce_bytes),
            Payload {
                msg: key.as_bytes(),
                aad: &key_aad(&key_id),
            },
        )
        .map_err(|_| CryptoError::Format("sealing the key failed".to_owned()))?;
    Ok(WrappedKey {
        key_id,
        kdf,
        nonce: nonce_bytes,
        ciphertext,
    })
}

/// Unwraps the data key with `words`.
///
/// # Errors
///
/// [`CryptoError::KdfParams`] for settings outside the bounds, checked
/// before any work; [`CryptoError::WrongWords`] when the words, or anything
/// in the wrapped key, are wrong.
pub fn unwrap(wrapped: &WrappedKey, words: &Words) -> Result<DataKey, CryptoError> {
    let wrapping = wrapped.kdf.derive(words)?;
    let plain = Zeroizing::new(
        cipher(&wrapping)
            .decrypt(
                &nonce(wrapped.nonce),
                Payload {
                    msg: &wrapped.ciphertext,
                    aad: &key_aad(&wrapped.key_id),
                },
            )
            .map_err(|_| CryptoError::WrongWords)?,
    );
    let bytes: [u8; KEY_LEN] = plain
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Format("the wrapped key has the wrong length".to_owned()))?;
    let key = DataKey::from_bytes(bytes);
    if key.key_id() != wrapped.key_id {
        return Err(CryptoError::KeyIdMismatch(wrapped.key_id.to_string()));
    }
    Ok(key)
}

// ---------------------------------------------------------------------------
// Sealing
// ---------------------------------------------------------------------------

/// Metadata sealed for one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedMeta {
    /// AES-GCM nonce.
    pub nonce: [u8; NONCE_LEN],
    /// The sealed metadata and its tag.
    pub ciphertext: Vec<u8>,
}

/// Seals and opens one store's items with the subkeys of its data key.
#[derive(Clone)]
pub struct Sealer {
    key: DataKey,
    key_id: KeyId,
    id_key: Zeroizing<[u8; KEY_LEN]>,
    meta: Aes256Gcm,
}

impl Sealer {
    /// A sealer for `key`.
    pub fn new(key: DataKey) -> Self {
        let key_id = key.key_id();
        let id_key = key.derive(None, INFO_ID);
        let meta = cipher(&key.derive(None, INFO_META));
        Self {
            key,
            key_id,
            id_key,
            meta,
        }
    }

    /// Id of the data key.
    pub fn key_id(&self) -> KeyId {
        self.key_id
    }

    /// The keyed content key: the first 6 bytes of
    /// `HMAC-SHA256(id key, SHA-256 of the content)`. Equal content gives
    /// the same key under one data key, so deduplication and typed prefixes
    /// still work, but the storage cannot confirm guesses about content.
    pub fn content_key(&self, digest: &ContentDigest) -> ContentKey {
        let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(self.id_key.as_ref())
            .expect("HMAC takes a key of any length");
        mac.update(digest.sha256_bytes());
        let mut tag = [0_u8; 32];
        tag.copy_from_slice(&mac.finalize().into_bytes());
        ContentKey::from_sha256(&tag)
    }

    /// Seals an item's metadata, bound to its id.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Random`] when the system generator fails.
    pub fn seal_meta(&self, id: &ItemId, plain: &[u8]) -> Result<SealedMeta, CryptoError> {
        let nonce_bytes = random_bytes()?;
        let ciphertext = self
            .meta
            .encrypt(
                &nonce(nonce_bytes),
                Payload {
                    msg: plain,
                    aad: &meta_aad(id),
                },
            )
            .map_err(|_| CryptoError::Format("sealing the metadata failed".to_owned()))?;
        Ok(SealedMeta {
            nonce: nonce_bytes,
            ciphertext,
        })
    }

    /// Opens an item's metadata.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Authentication`] when it was changed, sealed for
    /// another id, or sealed under another key.
    pub fn open_meta(
        &self,
        id: &ItemId,
        sealed: &SealedMeta,
    ) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        self.meta
            .decrypt(
                &nonce(sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: &meta_aad(id),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| CryptoError::Authentication)
    }

    /// A sealer for one item's content, with a fresh random salt.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Random`] when the system generator fails.
    pub fn content_sealer(&self) -> Result<ContentSealer, CryptoError> {
        let salt = random_bytes()?;
        Ok(ContentSealer::new(self.key.content_cipher(&salt), salt))
    }

    /// A reader that opens sealed content from `sealed`, failing with
    /// [`std::io::ErrorKind::InvalidData`] on any tampering.
    pub fn open_content<R: AsyncRead + Unpin>(&self, sealed: R) -> OpenReader<R> {
        OpenReader::new(sealed, self.key.clone(), None)
    }

    /// Like [`Sealer::open_content`], and also refuses content whose salt
    /// differs from `salt`, the one its item's metadata records, so content
    /// moved from another item does not open.
    pub fn open_item_content<R: AsyncRead + Unpin>(
        &self,
        sealed: R,
        salt: [u8; CONTENT_SALT_LEN],
    ) -> OpenReader<R> {
        OpenReader::new(sealed, self.key.clone(), Some(salt))
    }
}

impl fmt::Debug for Sealer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sealer(key {})", self.key_id)
    }
}

fn meta_aad(id: &ItemId) -> Vec<u8> {
    [AAD_META, id.as_str().as_bytes()].concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ContentHasher;
    use chrono::{TimeZone, Utc};

    /// Small settings so tests run fast; production uses `KdfParams::generate`.
    fn quick_kdf() -> KdfParams {
        KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [7; KDF_SALT_LEN],
        }
    }

    fn words() -> Words {
        Words::parse("abacus zoom abdomen abacus zoom abdomen").unwrap()
    }

    fn digest(bytes: &[u8]) -> ContentDigest {
        let mut hasher = ContentHasher::new();
        hasher.update(bytes);
        hasher.finalize()
    }

    fn id(key: &ContentKey) -> ItemId {
        ItemId::new(
            Utc.with_ymd_and_hms(2026, 9, 15, 8, 0, 0).unwrap(),
            key.clone(),
        )
        .unwrap()
    }

    #[test]
    fn generated_keys_differ_and_have_stable_ids() {
        let a = DataKey::generate().unwrap();
        let b = DataKey::generate().unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
        assert_eq!(a.key_id(), a.clone().key_id());
        assert_ne!(a.key_id(), b.key_id());
        let shown = a.key_id().to_string();
        assert_eq!(shown.len(), 16);
        assert_eq!(KeyId::parse(&shown).unwrap(), a.key_id());
        assert_eq!(a.key_id().short(), shown[..8]);
    }

    #[test]
    fn key_ids_parse_only_lowercase_hex_of_the_right_length() {
        for bad in [
            "",
            "0123456789abcde",
            "0123456789abcdef0",
            "0123456789ABCDEF",
            "0123456789abcdeg",
        ] {
            assert!(KeyId::parse(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn debug_output_never_shows_secrets() {
        let key = DataKey::from_bytes([0xab; KEY_LEN]);
        let shown = format!("{key:?} {:?}", Sealer::new(key.clone()));
        assert!(!shown.contains(&hex::encode(key.as_bytes())), "{shown}");
        assert!(!shown.contains("abab"), "{shown}");
        assert!(shown.contains(&key.key_id().to_string()));
        let w = words();
        assert!(!format!("{w:?}").contains("abacus"));
    }

    #[test]
    fn a_wrapped_key_unwraps_with_the_same_words_only() {
        let key = DataKey::generate().unwrap();
        let wrapped = wrap(&key, &words(), quick_kdf()).unwrap();
        assert_eq!(wrapped.key_id, key.key_id());
        assert_eq!(
            unwrap(&wrapped, &words()).unwrap().as_bytes(),
            key.as_bytes()
        );

        let other = Words::parse("zoom zoom zoom zoom zoom zoom").unwrap();
        assert_eq!(
            unwrap(&wrapped, &other).unwrap_err(),
            CryptoError::WrongWords
        );
    }

    #[test]
    fn any_change_to_a_wrapped_key_is_refused() {
        let key = DataKey::generate().unwrap();
        let wrapped = wrap(&key, &words(), quick_kdf()).unwrap();

        let mut flipped = wrapped.clone();
        flipped.ciphertext[3] ^= 1;
        assert_eq!(
            unwrap(&flipped, &words()).unwrap_err(),
            CryptoError::WrongWords
        );

        let mut renamed = wrapped.clone();
        renamed.key_id = DataKey::generate().unwrap().key_id();
        assert_eq!(
            unwrap(&renamed, &words()).unwrap_err(),
            CryptoError::WrongWords
        );

        let mut salted = wrapped.clone();
        salted.kdf.salt[0] ^= 1;
        assert_eq!(
            unwrap(&salted, &words()).unwrap_err(),
            CryptoError::WrongWords
        );

        let mut nonce = wrapped;
        nonce.nonce[0] ^= 1;
        assert_eq!(
            unwrap(&nonce, &words()).unwrap_err(),
            CryptoError::WrongWords
        );
    }

    #[test]
    fn settings_above_the_bounds_are_refused_before_deriving() {
        let key = DataKey::generate().unwrap();
        let wrapped = wrap(&key, &words(), quick_kdf()).unwrap();
        for (m_kib, t, p) in [
            (KdfParams::MAX_M_KIB + 1, 1, 1),
            (64, KdfParams::MAX_T + 1, 1),
            (64, 1, KdfParams::MAX_P + 1),
        ] {
            let mut hostile = wrapped.clone();
            hostile.kdf.m_kib = m_kib;
            hostile.kdf.t = t;
            hostile.kdf.p = p;
            assert!(
                matches!(unwrap(&hostile, &words()), Err(CryptoError::KdfParams(_))),
                "{m_kib} {t} {p}"
            );
        }
        assert!(matches!(
            KdfParams {
                m_kib: 1,
                ..quick_kdf()
            }
            .check(),
            Err(CryptoError::KdfParams(_))
        ));
    }

    #[test]
    fn new_keys_use_the_chosen_argon2id_settings_and_a_random_salt() {
        let a = KdfParams::generate().unwrap();
        let b = KdfParams::generate().unwrap();
        assert_eq!((a.m_kib, a.t, a.p), (65_536, 3, 4));
        assert_ne!(a.salt, b.salt);
        a.check().unwrap();
    }

    #[test]
    fn keyed_content_keys_hide_the_plain_hash_but_keep_equality() {
        let a = Sealer::new(DataKey::generate().unwrap());
        let b = Sealer::new(DataKey::generate().unwrap());
        let hello = digest(b"hello");
        assert_ne!(a.content_key(&hello), hello.content_key());
        assert_eq!(a.content_key(&hello), a.content_key(&digest(b"hello")));
        assert_ne!(a.content_key(&hello), a.content_key(&digest(b"hellp")));
        assert_ne!(a.content_key(&hello), b.content_key(&hello));
    }

    #[test]
    fn metadata_opens_only_for_its_own_id_and_key() {
        let sealer = Sealer::new(DataKey::generate().unwrap());
        let key = sealer.content_key(&digest(b"x"));
        let item = id(&key);
        let sealed = sealer.seal_meta(&item, b"{\"name\":\"a.txt\"}").unwrap();
        assert!(!sealed.ciphertext.windows(5).any(|w| w == b"a.txt"));
        assert_eq!(
            sealer.open_meta(&item, &sealed).unwrap().as_slice(),
            b"{\"name\":\"a.txt\"}"
        );

        let other = id(&sealer.content_key(&digest(b"y")));
        assert_eq!(
            sealer.open_meta(&other, &sealed).unwrap_err(),
            CryptoError::Authentication
        );
        let stranger = Sealer::new(DataKey::generate().unwrap());
        assert_eq!(
            stranger.open_meta(&item, &sealed).unwrap_err(),
            CryptoError::Authentication
        );
        for i in 0..sealed.ciphertext.len() {
            let mut flipped = sealed.clone();
            flipped.ciphertext[i] ^= 0x01;
            assert!(sealer.open_meta(&item, &flipped).is_err(), "byte {i}");
        }
        let mut nonce = sealed;
        nonce.nonce[11] ^= 1;
        assert!(sealer.open_meta(&item, &nonce).is_err());
    }

    #[test]
    fn metadata_nonces_are_fresh() {
        let sealer = Sealer::new(DataKey::generate().unwrap());
        let item = id(&sealer.content_key(&digest(b"x")));
        let a = sealer.seal_meta(&item, b"same").unwrap();
        let b = sealer.seal_meta(&item, b"same").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    /// Records how long unwrapping takes with the production settings. Run
    /// with `cargo test --release -p passalong-core timing_ -- --ignored
    /// --nocapture`; the 3 s limit applies to optimised builds only.
    #[test]
    #[ignore = "timing measurement; run by hand"]
    fn timing_unwrap_with_the_production_settings() {
        let key = DataKey::generate().unwrap();
        let wrapped = wrap(&key, &words(), KdfParams::generate().unwrap()).unwrap();
        let started = std::time::Instant::now();
        unwrap(&wrapped, &words()).unwrap();
        let elapsed = started.elapsed();
        eprintln!(
            "argon2id 64 MiB, t=3, p=4 unwrap: {} ms",
            elapsed.as_millis()
        );
        if !cfg!(debug_assertions) {
            assert!(elapsed.as_secs_f64() < 3.0, "{elapsed:?}");
        }
    }
}
