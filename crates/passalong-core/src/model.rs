//! Item model: content keys, time-sortable item ids, and item metadata.
//!
//! An item id is `<ts>-<key>`. `ts` is the creation time in whole seconds
//! since the Unix epoch as 8 lowercase hex digits; `key` is the
//! [`ContentKey`], the first 12 lowercase hex digits of the content's
//! SHA-256. The fixed width makes plain string order chronological, so
//! sorting directory names lists the newest items first, while the content
//! key lets a store recognise identical content whenever it was sent.
//!
//! [`ItemId::parse`] is the only way to turn text into an id, and it accepts
//! nothing but hex digits and one `-`, so an id is always safe to use as a
//! path component.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// MIME type recorded for clipboard text.
pub const TEXT_MIME: &str = "text/plain; charset=utf-8";
/// Maximum length of a text preview in characters, ellipsis included.
pub const PREVIEW_CHARS: usize = 80;

const TIMESTAMP_HEX_LEN: usize = 8;
const KEY_HEX_LEN: usize = 12;

/// Errors from building or parsing model values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// Text that is not an `<8 hex>-<12 hex>` item id.
    #[error(
        "invalid item id `{0}`: expected 8 lowercase hex digits, `-`, then 12 lowercase hex digits"
    )]
    InvalidItemId(String),
    /// Text that is not a 12-hex-digit content key.
    #[error("invalid content key `{0}`: expected 12 lowercase hex digits")]
    InvalidContentKey(String),
    /// A creation time that cannot be written as 8 hex digits of seconds.
    #[error("time {0} is outside the item id range 1970-01-01 to 2106-02-07")]
    TimestampOutOfRange(DateTime<Utc>),
    /// A stored file name that leaves nothing usable after sanitising.
    #[error("`{0}` is not a usable file name")]
    InvalidFileName(String),
}

fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

/// Incremental SHA-256 over item content that also counts the bytes, so a
/// stream can be hashed while it is uploaded.
#[derive(Clone, Default)]
pub struct ContentHasher {
    inner: Sha256,
    size: u64,
}

impl ContentHasher {
    /// Starts an empty hash.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds the next chunk of content.
    pub fn update(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
        self.size += bytes.len() as u64;
    }

    /// Finishes the hash.
    pub fn finalize(self) -> ContentDigest {
        ContentDigest {
            sha256: self.inner.finalize().into(),
            size: self.size,
        }
    }
}

impl fmt::Debug for ContentHasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContentHasher")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// SHA-256 and byte count of one item's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentDigest {
    sha256: [u8; 32],
    size: u64,
}

impl ContentDigest {
    /// The full SHA-256 as 64 lowercase hex digits.
    pub fn sha256_hex(&self) -> String {
        hex::encode(self.sha256)
    }

    /// The raw SHA-256 bytes.
    pub fn sha256_bytes(&self) -> &[u8; 32] {
        &self.sha256
    }

    /// The content length in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The content key derived from this digest.
    pub fn content_key(&self) -> ContentKey {
        ContentKey::from_sha256(&self.sha256)
    }
}

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// The first 12 lowercase hex digits (6 bytes) of an item's SHA-256.
///
/// Identical content always has the same key, which is how stores skip
/// re-uploads and how `serve` avoids echoing text that `load` just wrote.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentKey(String);

impl ContentKey {
    /// Derives the key from a full SHA-256 digest.
    pub fn from_sha256(sha256: &[u8; 32]) -> Self {
        Self(hex::encode(&sha256[..KEY_HEX_LEN / 2]))
    }

    /// Parses 12 lowercase hex digits.
    ///
    /// # Errors
    ///
    /// [`ModelError::InvalidContentKey`] for anything else.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        if is_lower_hex(text, KEY_HEX_LEN) {
            Ok(Self(text.to_owned()))
        } else {
            Err(ModelError::InvalidContentKey(text.to_owned()))
        }
    }

    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ContentKey {
    type Err = ModelError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl fmt::Display for ContentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Time-sortable item identifier, `<8 hex seconds>-<content key>`.
///
/// Ordering follows the text, which is chronological, with ties broken by
/// content key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ItemId {
    // `repr` comes first so the derived ordering is the lexical one.
    repr: String,
    secs: u32,
    key: ContentKey,
}

impl ItemId {
    /// Builds the id for content with `key` created at `created_at`.
    /// Sub-second precision is dropped.
    ///
    /// # Errors
    ///
    /// [`ModelError::TimestampOutOfRange`] before 1970 or after
    /// 2106-02-07T06:28:15Z, the limits of 8 hex digits of seconds.
    pub fn new(created_at: DateTime<Utc>, key: ContentKey) -> Result<Self, ModelError> {
        let secs = u32::try_from(created_at.timestamp())
            .map_err(|_| ModelError::TimestampOutOfRange(created_at))?;
        Ok(Self {
            repr: format!("{secs:08x}-{key}"),
            secs,
            key,
        })
    }

    /// Parses an id.
    ///
    /// # Errors
    ///
    /// [`ModelError::InvalidItemId`] unless `text` is exactly 8 lowercase hex
    /// digits, `-`, and 12 lowercase hex digits.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let invalid = || ModelError::InvalidItemId(text.to_owned());
        let (ts, key) = text.split_once('-').ok_or_else(invalid)?;
        if !is_lower_hex(ts, TIMESTAMP_HEX_LEN) || !is_lower_hex(key, KEY_HEX_LEN) {
            return Err(invalid());
        }
        let secs = u32::from_str_radix(ts, 16).map_err(|_| invalid())?;
        Ok(Self {
            repr: text.to_owned(),
            secs,
            key: ContentKey(key.to_owned()),
        })
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.repr
    }

    /// When the item was created, to the second.
    pub fn timestamp(&self) -> DateTime<Utc> {
        // Every u32 second count is within chrono's range, so the default
        // is never used.
        DateTime::from_timestamp(i64::from(self.secs), 0).unwrap_or_default()
    }

    /// The content key part of the id.
    pub fn content_key(&self) -> &ContentKey {
        &self.key
    }
}

impl FromStr for ItemId {
    type Err = ModelError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl TryFrom<String> for ItemId {
    type Error = ModelError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<ItemId> for String {
    fn from(id: ItemId) -> Self {
        id.repr
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.repr)
    }
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

/// What an item holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    /// Clipboard text, stored as UTF-8.
    Text,
    /// A file.
    File,
}

/// Contents of an item's `meta.json`: schema version 1.
///
/// Fields serialise in declaration order. Readers ignore unknown fields so
/// items written by newer clients stay readable; a breaking change must bump
/// [`ItemMeta::SCHEMA_VERSION`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemMeta {
    /// Schema version, currently [`ItemMeta::SCHEMA_VERSION`].
    pub schema: u32,
    /// The item's id.
    pub id: ItemId,
    /// Text or file.
    pub kind: ItemKind,
    /// Original file name for files; `None` for text.
    #[serde(default)]
    pub name: Option<String>,
    /// MIME type: [`TEXT_MIME`] for text, guessed from the name for files.
    pub mime: String,
    /// Content length in bytes.
    pub size: u64,
    /// Full SHA-256 of the content, 64 lowercase hex digits.
    pub sha256: String,
    /// Creation time, equal to the id's timestamp.
    pub created_at: DateTime<Utc>,
    /// Name of the device that sent the item.
    pub device: String,
    /// For text, a one-line preview of at most [`PREVIEW_CHARS`] characters.
    #[serde(default)]
    pub preview: Option<String>,
}

impl ItemMeta {
    /// The schema version written by this release.
    pub const SCHEMA_VERSION: u32 = 1;
}

/// The caller-supplied part of an item's metadata, before its content has
/// been hashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    /// Text or file.
    pub kind: ItemKind,
    /// Original file name for files.
    pub name: Option<String>,
    /// MIME type.
    pub mime: String,
    /// Sending device.
    pub device: String,
}

impl NewItem {
    /// A clipboard text item.
    pub fn text(device: impl Into<String>) -> Self {
        Self {
            kind: ItemKind::Text,
            name: None,
            mime: TEXT_MIME.to_owned(),
            device: device.into(),
        }
    }

    /// A file item; the MIME type is guessed from `name`.
    pub fn file(name: impl Into<String>, device: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            kind: ItemKind::File,
            mime: mime_for_file_name(&name),
            name: Some(name),
            device: device.into(),
        }
    }

    /// Completes the metadata once the content is hashed, assigning the id.
    ///
    /// # Errors
    ///
    /// [`ModelError::TimestampOutOfRange`] when `created_at` cannot be
    /// encoded in an id.
    pub fn finish(
        self,
        created_at: DateTime<Utc>,
        digest: &ContentDigest,
        preview: Option<String>,
    ) -> Result<ItemMeta, ModelError> {
        let id = ItemId::new(created_at, digest.content_key())?;
        Ok(ItemMeta {
            schema: ItemMeta::SCHEMA_VERSION,
            created_at: id.timestamp(),
            id,
            kind: self.kind,
            name: self.name,
            mime: self.mime,
            size: digest.size(),
            sha256: digest.sha256_hex(),
            device: self.device,
            preview,
        })
    }
}

/// One-line preview of clipboard text: whitespace runs collapse to single
/// spaces and text longer than [`PREVIEW_CHARS`] characters ends in `…`.
pub fn preview_of(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= PREVIEW_CHARS {
        return collapsed;
    }
    let mut preview: String = collapsed.chars().take(PREVIEW_CHARS - 1).collect();
    preview.push('…');
    preview
}

/// Reduces a stored file name to something safe to create in a local
/// directory: only the last `/`- or `\\`-separated component is kept, with
/// control characters removed and surrounding spaces trimmed. Names from
/// the server are untrusted, so `../../etc/passwd` becomes `passwd`.
///
/// # Errors
///
/// [`ModelError::InvalidFileName`] when nothing usable remains, for example
/// for `..` or a name ending in a separator.
pub fn sanitise_file_name(name: &str) -> Result<String, ModelError> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err(ModelError::InvalidFileName(name.to_owned()));
    }
    Ok(cleaned.to_owned())
}

/// Guesses a file's MIME type from its extension, falling back to
/// `application/octet-stream`.
pub fn mime_for_file_name(name: &str) -> String {
    mime_guess::from_path(name)
        .first_or_octet_stream()
        .essence_str()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Clock;
    use crate::testing::FixedClock;
    use std::str::FromStr;

    const T: &str = "2026-09-12T09:53:11Z";
    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn hello_digest() -> ContentDigest {
        let mut hasher = ContentHasher::new();
        hasher.update(b"hello");
        hasher.finalize()
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        FixedClock::at(rfc3339).now()
    }

    /// Deterministic xorshift so the ordering property is reproducible.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    // ---------- hashing and content keys ----------

    #[test]
    fn hashes_content_to_the_sha256_vector() {
        let digest = hello_digest();
        assert_eq!(digest.sha256_hex(), HELLO_SHA256);
        assert_eq!(digest.size(), 5);
        assert_eq!(digest.content_key().as_str(), "2cf24dba5fb0");
    }

    #[test]
    fn incremental_hashing_matches_one_shot() {
        let mut hasher = ContentHasher::new();
        hasher.update(b"he");
        hasher.update(b"");
        hasher.update(b"llo");
        assert_eq!(hasher.finalize(), hello_digest());
    }

    #[test]
    fn content_key_is_the_first_six_digest_bytes() {
        let bytes: [u8; 32] = hex::decode(HELLO_SHA256).unwrap().try_into().unwrap();
        assert_eq!(ContentKey::from_sha256(&bytes).as_str(), "2cf24dba5fb0");
    }

    #[test]
    fn content_key_parsing() {
        assert_eq!(
            ContentKey::from_str("2cf24dba5fb0").unwrap().to_string(),
            "2cf24dba5fb0"
        );
        for bad in [
            "2CF24DBA5FB0",
            "2cf24dba5fb",
            "2cf24dba5fb0a",
            "2cf24dba5fbg",
            "",
            "../x/../abc1",
        ] {
            assert!(ContentKey::from_str(bad).is_err(), "{bad:?} accepted");
        }
    }

    // ---------- item ids ----------

    #[test]
    fn item_id_is_hex_seconds_then_content_key() {
        let key = hello_digest().content_key();
        let id = ItemId::new(at(T), key.clone()).unwrap();
        assert_eq!(id.as_str(), "6aa52107-2cf24dba5fb0");
        assert_eq!(id.to_string(), "6aa52107-2cf24dba5fb0");
        assert_eq!(id.timestamp(), at(T));
        assert_eq!(id.content_key(), &key);
    }

    #[test]
    fn item_id_drops_sub_second_precision() {
        let precise = DateTime::parse_from_rfc3339("2026-09-12T09:53:11.987Z")
            .unwrap()
            .with_timezone(&Utc);
        let id = ItemId::new(precise, hello_digest().content_key()).unwrap();
        assert_eq!(id.as_str(), "6aa52107-2cf24dba5fb0");
    }

    #[test]
    fn item_id_parsing_round_trips() {
        let id = ItemId::from_str("6aa52107-2cf24dba5fb0").unwrap();
        assert_eq!(id.timestamp(), at(T));
        assert_eq!(id.content_key().as_str(), "2cf24dba5fb0");
        assert_eq!(
            ItemId::parse("00000000-000000000000").unwrap().timestamp(),
            at("1970-01-01T00:00:00Z")
        );
    }

    #[test]
    fn item_id_parsing_rejects_everything_else() {
        for bad in [
            "6AA52107-2cf24dba5fb0",
            "6aa52107-2CF24DBA5FB0",
            "6aa521072cf24dba5fb0",
            "6aa5210-2cf24dba5fb0",
            "6aa521070-2cf24dba5fb0",
            "6aa52107-2cf24dba5fb",
            "6aa52107-2cf24dba5fb0a",
            "6aa5210g-2cf24dba5fb0",
            "6aa52107_2cf24dba5fb0",
            "../abc123abc1",
            "2cf24dba5fb0",
            "",
            " 6aa52107-2cf24dba5fb0",
        ] {
            let err = ItemId::parse(bad).unwrap_err();
            assert!(err.to_string().contains("item id"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn item_id_rejects_times_outside_the_eight_hex_digit_range() {
        let key = hello_digest().content_key();
        assert!(ItemId::new(at("1969-12-31T23:59:59Z"), key.clone()).is_err());
        assert!(ItemId::new(at("2106-02-07T06:28:16Z"), key.clone()).is_err());
        let last = ItemId::new(at("2106-02-07T06:28:15Z"), key).unwrap();
        assert!(last.as_str().starts_with("ffffffff-"));
    }

    #[test]
    fn lexical_id_order_is_chronological_order() {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for _ in 0..100 {
            let (a, b) = (
                rng.next() % u64::from(u32::MAX),
                rng.next() % u64::from(u32::MAX),
            );
            let key_a =
                ContentKey::from_str(&format!("{:012x}", rng.next() & 0xffff_ffff_ffff)).unwrap();
            let key_b =
                ContentKey::from_str(&format!("{:012x}", rng.next() & 0xffff_ffff_ffff)).unwrap();
            let secs = |s: u64| DateTime::from_timestamp(s as i64, 0).unwrap();
            let id_a = ItemId::new(secs(a), key_a.clone()).unwrap();
            let id_b = ItemId::new(secs(b), key_b.clone()).unwrap();
            let expected = a.cmp(&b).then(key_a.cmp(&key_b));
            assert_eq!(
                id_a.as_str().cmp(id_b.as_str()),
                expected,
                "{id_a} vs {id_b}"
            );
            assert_eq!(
                id_a.cmp(&id_b),
                expected,
                "Ord disagrees for {id_a} vs {id_b}"
            );
        }
    }

    // ---------- metadata ----------

    fn file_meta() -> ItemMeta {
        NewItem::file("a.txt", "box")
            .finish(at(T), &hello_digest(), None)
            .unwrap()
    }

    const FILE_META_JSON: &str = concat!(
        r#"{"schema":1,"id":"6aa52107-2cf24dba5fb0","kind":"file","name":"a.txt","#,
        r#""mime":"text/plain","size":5,"#,
        r#""sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824","#,
        r#""created_at":"2026-09-12T09:53:11Z","device":"box","preview":null}"#
    );

    #[test]
    fn metadata_serialises_to_the_stable_json_layout() {
        let meta = file_meta();
        assert_eq!(serde_json::to_string(&meta).unwrap(), FILE_META_JSON);
        assert_eq!(
            serde_json::from_str::<ItemMeta>(FILE_META_JSON).unwrap(),
            meta
        );
    }

    #[test]
    fn metadata_from_newer_writers_with_extra_fields_still_parses() {
        let newer =
            FILE_META_JSON.replace(r#""preview":null}"#, r#""preview":null,"encrypted":true}"#);
        assert_eq!(
            serde_json::from_str::<ItemMeta>(&newer).unwrap(),
            file_meta()
        );
    }

    #[test]
    fn metadata_with_an_invalid_id_is_rejected() {
        let tampered = FILE_META_JSON.replace("6aa52107-2cf24dba5fb0", "../../etc/passwd");
        let err = serde_json::from_str::<ItemMeta>(&tampered).unwrap_err();
        assert!(err.to_string().contains("item id"), "{err}");
    }

    #[test]
    fn text_items_carry_a_preview_and_the_text_mime() {
        let meta = NewItem::text("box")
            .finish(at(T), &hello_digest(), Some(preview_of("hello")))
            .unwrap();
        assert_eq!(meta.kind, ItemKind::Text);
        assert_eq!(meta.name, None);
        assert_eq!(meta.mime, TEXT_MIME);
        assert_eq!(meta.preview.as_deref(), Some("hello"));
        assert_eq!(meta.schema, ItemMeta::SCHEMA_VERSION);
        assert_eq!(meta.id.as_str(), "6aa52107-2cf24dba5fb0");
        assert_eq!(meta.created_at, at(T));
        assert_eq!((meta.size, meta.sha256.as_str()), (5, HELLO_SHA256));
        assert_eq!(meta.device, "box");
    }

    #[test]
    fn file_items_guess_their_mime_from_the_name() {
        let meta = NewItem::file("photo.PNG", "box");
        assert_eq!(
            (meta.kind, meta.name.as_deref(), meta.mime.as_str()),
            (ItemKind::File, Some("photo.PNG"), "image/png")
        );
    }

    #[test]
    fn finish_fails_for_times_outside_the_id_range() {
        let err = NewItem::text("box")
            .finish(at("1960-01-01T00:00:00Z"), &hello_digest(), None)
            .unwrap_err();
        assert!(matches!(err, ModelError::TimestampOutOfRange(_)), "{err:?}");
    }

    // ---------- previews and mime types ----------

    #[test]
    fn preview_collapses_whitespace() {
        assert_eq!(
            preview_of("  hello\n\n world\t\r\nagain  "),
            "hello world again"
        );
        assert_eq!(preview_of("\n \t"), "");
    }

    #[test]
    fn preview_truncates_to_eighty_characters_with_an_ellipsis() {
        let exact = "a".repeat(80);
        assert_eq!(preview_of(&exact), exact);
        let long = "é".repeat(100);
        let preview = preview_of(&long);
        assert_eq!(preview.chars().count(), 80);
        assert!(preview.ends_with('…'));
        assert_eq!(preview, format!("{}…", "é".repeat(79)));
    }

    #[test]
    fn mime_is_guessed_from_the_extension() {
        assert_eq!(mime_for_file_name("a.png"), "image/png");
        assert_eq!(mime_for_file_name("notes.txt"), "text/plain");
        assert_eq!(
            mime_for_file_name("archive.unknownext"),
            "application/octet-stream"
        );
        assert_eq!(mime_for_file_name("Makefile"), "application/octet-stream");
    }

    #[test]
    fn file_names_are_reduced_to_a_safe_last_component() {
        let cases = [
            ("normal.pdf", "normal.pdf"),
            ("../../etc/passwd", "passwd"),
            ("..\\..\\evil.exe", "evil.exe"),
            (" spaced name.txt ", "spaced name.txt"),
            ("bell\u{7}name", "bellname"),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitise_file_name(input).unwrap(), expected, "{input:?}");
        }
        for bad in ["", "..", ".", "dir/", "/", "\u{0}"] {
            let err = sanitise_file_name(bad).unwrap_err();
            assert!(
                matches!(err, ModelError::InvalidFileName(_)),
                "{bad:?}: {err:?}"
            );
        }
    }
}
