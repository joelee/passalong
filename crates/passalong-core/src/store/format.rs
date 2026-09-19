//! The bytes an item is stored as: its `meta.json` and its `content`.
//!
//! Every store writes items through these functions, so an item has the same
//! bytes in a folder, over SFTP, or on a passalong-server, which stores them
//! as it receives them. A plaintext `meta.json` is the pretty-printed
//! [`ItemMeta`] (schema 1); a sealed one shows only the schema, the id, and
//! the sealed [`SealedMetaBody`] (schema 2). Sealed content is `PAC1`, the
//! content salt, and records of at most [`CHUNK_LEN`] bytes.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::crypto::{
    CHUNK_LEN, CONTENT_SALT_LEN, ContentSealer, CryptoError, SealedMeta, Sealer, read_full,
};
use crate::model::{ContentDigest, ContentHasher, ItemId, ItemKind, ItemMeta, preview_of};

/// Version of a sealed `meta.json`.
pub const SEALED_SCHEMA: u32 = 2;
/// The largest `meta.json` a store reads; it is tiny, and the cap stops a
/// hostile file from exhausting memory.
pub const MAX_META_BYTES: u64 = 64 * 1024;
/// Bytes kept from the start of the content to build a text preview.
pub const PREVIEW_HEAD_BYTES: usize = 4096;
/// How much plaintext content is copied at a time.
const COPY_CHUNK: usize = 64 * 1024;

/// A content salt.
pub type ContentSalt = [u8; CONTENT_SALT_LEN];

/// Why a `meta.json` could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaError {
    /// The file is malformed or describes another item.
    Corrupt(String),
    /// A sealed file that does not parse or open with the key: in a synced
    /// folder it may still be arriving, so the caller decides how to report
    /// it.
    Unopened(String),
}

/// Encodes `meta` as its `meta.json`: plaintext, or with `sealing` sealed
/// together with its content salt.
///
/// # Errors
///
/// A description when encoding or sealing fails.
pub fn encode_meta(
    meta: &ItemMeta,
    sealing: Option<(&Sealer, &ContentSalt)>,
) -> Result<Vec<u8>, EncodeError> {
    let mut json = match sealing {
        Some((sealer, salt)) => {
            let body = serde_json::to_vec(&SealedMetaBody {
                meta: meta.clone(),
                content_salt: hex::encode(salt),
            })?;
            let sealed = sealer.seal_meta(&meta.id, &body)?;
            serde_json::to_vec_pretty(&SealedMetaFile::new(&meta.id, &sealed))?
        }
        None => serde_json::to_vec_pretty(meta)?,
    };
    json.push(b'\n');
    Ok(json)
}

/// Why [`encode_meta`] failed.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// JSON encoding failed.
    #[error("encoding meta.json: {0}")]
    Json(#[from] serde_json::Error),
    /// Sealing failed.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

/// Reads the `meta.json` of item `id`: its metadata, and in a sealed store,
/// with `sealer`, its content salt.
///
/// # Errors
///
/// [`MetaError`] when the bytes are not a valid `meta.json` of `id`.
pub fn decode_meta(
    id: &ItemId,
    bytes: &[u8],
    sealer: Option<&Sealer>,
) -> Result<(ItemMeta, Option<ContentSalt>), MetaError> {
    let Some(sealer) = sealer else {
        let meta: ItemMeta = serde_json::from_slice(bytes)
            .map_err(|err| MetaError::Corrupt(format!("meta.json: {err}")))?;
        if meta.id != *id {
            return Err(MetaError::Corrupt(format!(
                "meta.json describes {}",
                meta.id
            )));
        }
        return Ok((meta, None));
    };
    let file: SealedMetaFile = serde_json::from_slice(bytes)
        .map_err(|err| MetaError::Unopened(format!("meta.json: {err}")))?;
    if file.schema != SEALED_SCHEMA {
        return Err(MetaError::Corrupt(format!(
            "meta.json has schema {}, not {SEALED_SCHEMA}",
            file.schema
        )));
    }
    if file.id != *id {
        return Err(MetaError::Corrupt(format!(
            "meta.json describes {}",
            file.id
        )));
    }
    let sealed = file.sealed().map_err(MetaError::Corrupt)?;
    let plain = sealer.open_meta(id, &sealed).map_err(|_| {
        MetaError::Unopened("meta.json does not open with this store's key".to_owned())
    })?;
    let body: SealedMetaBody = serde_json::from_slice(&plain)
        .map_err(|err| MetaError::Corrupt(format!("sealed meta.json: {err}")))?;
    if body.meta.id != *id {
        return Err(MetaError::Corrupt(format!(
            "meta.json describes {}",
            body.meta.id
        )));
    }
    let salt = decode_array(&body.content_salt)
        .map_err(|reason| MetaError::Corrupt(format!("content salt: {reason}")))?;
    Ok((body.meta, Some(salt)))
}

/// What [`write_content`] learned while copying.
#[derive(Debug)]
pub struct Written {
    /// The plaintext content's digest.
    pub digest: ContentDigest,
    /// The first [`PREVIEW_HEAD_BYTES`] bytes of plaintext, for a preview.
    pub head: Vec<u8>,
    /// The content salt, when the content was sealed.
    pub salt: Option<ContentSalt>,
}

/// Why [`write_content`] failed.
#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    /// Reading the content failed.
    #[error("reading the content failed: {0}")]
    Read(std::io::Error),
    /// Writing the stored bytes failed.
    #[error("writing the content failed: {0}")]
    Write(std::io::Error),
    /// Sealing failed.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

/// Copies `content` into `writer` as it is stored, hashing it and keeping
/// the start for a preview. With `sealer`, it seals it record by record,
/// reading one record ahead to find the last. The writer is shut down at
/// the end.
///
/// # Errors
///
/// [`CopyError`] naming the side that failed.
pub async fn write_content<R, W>(
    content: R,
    writer: &mut W,
    sealer: Option<&Sealer>,
) -> Result<Written, CopyError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let sealing = sealer.map(Sealer::content_sealer).transpose()?;
    copy(content, writer, sealing).await
}

/// [`write_content`] sealing with `sealing`, made beforehand, so that its
/// salt can be recorded before the content is written.
///
/// # Errors
///
/// [`CopyError`] naming the side that failed.
pub async fn seal_content<R, W>(
    content: R,
    writer: &mut W,
    sealing: ContentSealer,
) -> Result<Written, CopyError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    copy(content, writer, Some(sealing)).await
}

async fn copy<R, W>(
    mut content: R,
    writer: &mut W,
    sealing: Option<ContentSealer>,
) -> Result<Written, CopyError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut hasher = ContentHasher::new();
    let mut head = Vec::new();
    let mut keep = |chunk: &[u8]| {
        hasher.update(chunk);
        let room = PREVIEW_HEAD_BYTES.saturating_sub(head.len());
        head.extend_from_slice(&chunk[..room.min(chunk.len())]);
    };
    let salt = match sealing {
        None => {
            let mut buf = vec![0_u8; COPY_CHUNK];
            loop {
                let n = content.read(&mut buf).await.map_err(CopyError::Read)?;
                if n == 0 {
                    break;
                }
                keep(&buf[..n]);
                writer
                    .write_all(&buf[..n])
                    .await
                    .map_err(CopyError::Write)?;
            }
            None
        }
        Some(mut sealing) => {
            writer
                .write_all(&sealing.header())
                .await
                .map_err(CopyError::Write)?;
            let mut current = vec![0_u8; CHUNK_LEN];
            let mut next = vec![0_u8; CHUNK_LEN];
            let mut len = read_full(&mut content, &mut current)
                .await
                .map_err(CopyError::Read)?;
            loop {
                // Only a full record can have more after it.
                let next_len = if len == CHUNK_LEN {
                    read_full(&mut content, &mut next)
                        .await
                        .map_err(CopyError::Read)?
                } else {
                    0
                };
                let chunk = &current[..len];
                keep(chunk);
                let sealed = sealing.seal_chunk(chunk, next_len == 0)?;
                writer.write_all(&sealed).await.map_err(CopyError::Write)?;
                if next_len == 0 {
                    break;
                }
                std::mem::swap(&mut current, &mut next);
                len = next_len;
            }
            Some(sealing.salt())
        }
    };
    writer.shutdown().await.map_err(CopyError::Write)?;
    Ok(Written {
        digest: hasher.finalize(),
        head,
        salt,
    })
}

/// The preview of a text item whose content starts with `head`; other
/// kinds have none.
pub fn preview(kind: ItemKind, head: &[u8]) -> Option<String> {
    (kind == ItemKind::Text).then(|| preview_of(utf8_prefix(head)))
}

/// The longest valid UTF-8 prefix; the preview head may end mid-character.
fn utf8_prefix(bytes: &[u8]) -> &str {
    match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => std::str::from_utf8(&bytes[..err.valid_up_to()]).unwrap_or_default(),
    }
}

/// A sealed store's `meta.json`: only the schema and the id are readable.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SealedMetaFile {
    pub(crate) schema: u32,
    pub(crate) id: ItemId,
    pub(crate) nonce: String,
    pub(crate) sealed: String,
}

impl SealedMetaFile {
    fn new(id: &ItemId, sealed: &SealedMeta) -> Self {
        Self {
            schema: SEALED_SCHEMA,
            id: id.clone(),
            nonce: hex::encode(sealed.nonce),
            sealed: hex::encode(&sealed.ciphertext),
        }
    }

    pub(crate) fn sealed(&self) -> Result<SealedMeta, String> {
        Ok(SealedMeta {
            nonce: decode_array(&self.nonce).map_err(|reason| format!("nonce: {reason}"))?,
            ciphertext: hex::decode(&self.sealed)
                .map_err(|_| "the sealed metadata is not hexadecimal".to_owned())?,
        })
    }
}

/// What a sealed `meta.json` seals: the item's metadata and the salt of its
/// content.
#[derive(Debug, Serialize, Deserialize)]
struct SealedMetaBody {
    meta: ItemMeta,
    content_salt: String,
}

/// `text` as hexadecimal bytes, exactly `N` of them.
pub(crate) fn decode_array<const N: usize>(text: &str) -> Result<[u8; N], String> {
    let bytes = hex::decode(text).map_err(|_| "not hexadecimal".to_owned())?;
    bytes.try_into().map_err(|_| format!("not {N} bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::DataKey;
    use crate::model::NewItem;

    fn meta() -> ItemMeta {
        let mut hasher = ContentHasher::new();
        hasher.update(b"hello");
        NewItem::text("box")
            .finish(
                "2026-09-12T09:53:11Z".parse().unwrap(),
                &hasher.finalize(),
                Some("hello".to_owned()),
            )
            .unwrap()
    }

    #[test]
    fn plaintext_meta_round_trips_and_names_its_item() {
        let meta = meta();
        let bytes = encode_meta(&meta, None).unwrap();
        assert!(bytes.ends_with(b"}\n"));
        assert_eq!(
            decode_meta(&meta.id, &bytes, None).unwrap(),
            (meta.clone(), None)
        );
        let other = ItemId::parse("6aa52107-000000000000").unwrap();
        assert!(matches!(
            decode_meta(&other, &bytes, None),
            Err(MetaError::Corrupt(reason)) if reason.contains("describes")
        ));
    }

    #[test]
    fn sealed_meta_opens_only_with_its_key() {
        let meta = meta();
        let sealer = Sealer::new(DataKey::generate().unwrap());
        let salt = [7_u8; CONTENT_SALT_LEN];
        let bytes = encode_meta(&meta, Some((&sealer, &salt))).unwrap();
        assert_eq!(
            decode_meta(&meta.id, &bytes, Some(&sealer)).unwrap(),
            (meta.clone(), Some(salt))
        );
        let stranger = Sealer::new(DataKey::generate().unwrap());
        assert!(matches!(
            decode_meta(&meta.id, &bytes, Some(&stranger)),
            Err(MetaError::Unopened(_))
        ));
        assert!(matches!(
            decode_meta(&meta.id, b"{\"half", Some(&sealer)),
            Err(MetaError::Unopened(_))
        ));
    }

    #[tokio::test]
    async fn content_is_copied_or_sealed_with_its_digest_and_head() {
        let data: Vec<u8> = (0..CHUNK_LEN + 10).map(|i| (i % 251) as u8).collect();
        let mut plain = Vec::new();
        let written = write_content(std::io::Cursor::new(data.clone()), &mut plain, None)
            .await
            .unwrap();
        assert_eq!(plain, data);
        assert!(written.salt.is_none());
        assert_eq!(written.head, data[..PREVIEW_HEAD_BYTES]);

        let sealer = Sealer::new(DataKey::generate().unwrap());
        let mut sealed = Vec::new();
        let sealed_written = write_content(
            std::io::Cursor::new(data.clone()),
            &mut sealed,
            Some(&sealer),
        )
        .await
        .unwrap();
        assert_eq!(sealed_written.digest, written.digest);
        assert_eq!(
            sealed.len() as u64,
            crate::crypto::sealed_len(data.len() as u64)
        );
        let mut opened = Vec::new();
        sealer
            .open_item_content(std::io::Cursor::new(sealed), sealed_written.salt.unwrap())
            .read_to_end(&mut opened)
            .await
            .unwrap();
        assert_eq!(opened, data);
    }
}
