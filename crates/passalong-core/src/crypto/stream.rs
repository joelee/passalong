//! The sealed content format.
//!
//! A sealed item's content is [`CONTENT_MAGIC`], a 32-byte salt, then
//! records. Each record is at most [`CHUNK_LEN`] bytes of plaintext sealed
//! with AES-256-GCM, followed by its [`TAG_LEN`]-byte tag. The key is
//! `HKDF-SHA256(data key, salt, info "passalong content v1")`, fresh for
//! every item, and the nonce is three zero bytes, the record number as a
//! big-endian `u64`, and `1` on the final record or `0` otherwise. Every
//! record but the final one holds a full chunk; the final one holds the
//! rest, and is empty only for empty content.
//!
//! Because the record number and the final flag are part of the nonce, a
//! reader rejects reordered, repeated, or missing records, content cut
//! after any record, and data after the final one. Binding content to its
//! item is the metadata's job: it records the salt and the plaintext's
//! SHA-256.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::Aead;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use zeroize::Zeroizing;

use super::{CONTENT_SALT_LEN, CryptoError, DataKey, NONCE_LEN, nonce};

/// First bytes of sealed content.
pub const CONTENT_MAGIC: [u8; 4] = *b"PAC1";
/// Plaintext bytes per record, except the final one.
pub const CHUNK_LEN: usize = 64 * 1024;
/// Length of an AES-GCM tag.
pub const TAG_LEN: usize = 16;
/// Length of the magic and the salt.
pub const HEADER_LEN: usize = CONTENT_MAGIC.len() + CONTENT_SALT_LEN;
const RECORD_LEN: usize = CHUNK_LEN + TAG_LEN;

fn record_nonce(counter: u64, last: bool) -> aes_gcm::aead::Nonce<Aes256Gcm> {
    let mut bytes = [0_u8; NONCE_LEN];
    bytes[3..11].copy_from_slice(&counter.to_be_bytes());
    bytes[11] = u8::from(last);
    nonce(bytes)
}

fn invalid(err: CryptoError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

/// Reads until `buf` is full or the stream ends, returning the bytes read.
///
/// # Errors
///
/// The reader's error.
pub async fn read_full<R: AsyncRead + Unpin + ?Sized>(
    reader: &mut R,
    buf: &mut [u8],
) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = reader.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

/// Seals one item's content, record by record. The caller writes
/// [`ContentSealer::header`] first, then each [`ContentSealer::seal_chunk`]
/// result in order; it must know which chunk is the last, for example by
/// reading one chunk ahead.
pub struct ContentSealer {
    cipher: Aes256Gcm,
    salt: [u8; CONTENT_SALT_LEN],
    counter: u64,
    finished: bool,
}

impl ContentSealer {
    pub(crate) fn new(cipher: Aes256Gcm, salt: [u8; CONTENT_SALT_LEN]) -> Self {
        Self {
            cipher,
            salt,
            counter: 0,
            finished: false,
        }
    }

    /// The item's content salt, which its metadata records.
    pub fn salt(&self) -> [u8; CONTENT_SALT_LEN] {
        self.salt
    }

    /// The magic and the salt.
    pub fn header(&self) -> [u8; HEADER_LEN] {
        let mut header = [0_u8; HEADER_LEN];
        header[..CONTENT_MAGIC.len()].copy_from_slice(&CONTENT_MAGIC);
        header[CONTENT_MAGIC.len()..].copy_from_slice(&self.salt);
        header
    }

    /// Seals the next chunk. Every chunk but the last must be exactly
    /// [`CHUNK_LEN`] bytes; the last may be shorter or empty.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Format`] for a chunk of the wrong length or one after
    /// the last.
    pub fn seal_chunk(&mut self, plain: &[u8], last: bool) -> Result<Vec<u8>, CryptoError> {
        if self.finished {
            return Err(CryptoError::Format(
                "the content is already finished".to_owned(),
            ));
        }
        if plain.len() > CHUNK_LEN || (!last && plain.len() != CHUNK_LEN) {
            return Err(CryptoError::Format(format!(
                "a {}chunk of {} bytes",
                if last { "final " } else { "" },
                plain.len()
            )));
        }
        let sealed = self
            .cipher
            .encrypt(&record_nonce(self.counter, last), plain)
            .map_err(|_| CryptoError::Format("sealing the content failed".to_owned()))?;
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| CryptoError::Format("too many records".to_owned()))?;
        self.finished = last;
        Ok(sealed)
    }
}

/// Reads sealed content and yields the plaintext, record by record. Fails
/// with [`io::ErrorKind::InvalidData`], wrapping a [`CryptoError`], when the
/// content was changed, cut short, extended, or sealed under another key.
/// Plaintext is released only after its record is authenticated.
pub struct OpenReader<R> {
    inner: R,
    key: DataKey,
    cipher: Option<Aes256Gcm>,
    /// One record plus one byte of look-ahead, to tell the final record.
    buf: Box<[u8]>,
    filled: usize,
    eof: bool,
    counter: u64,
    out: Zeroizing<Vec<u8>>,
    out_pos: usize,
    done: bool,
}

impl<R> OpenReader<R> {
    pub(crate) fn new(inner: R, key: DataKey) -> Self {
        Self {
            inner,
            key,
            cipher: None,
            buf: vec![0_u8; RECORD_LEN + 1].into_boxed_slice(),
            filled: 0,
            eof: false,
            counter: 0,
            out: Zeroizing::new(Vec::new()),
            out_pos: 0,
            done: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for OpenReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        dst: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.out_pos < this.out.len() {
                let n = dst.remaining().min(this.out.len() - this.out_pos);
                dst.put_slice(&this.out[this.out_pos..this.out_pos + n]);
                this.out_pos += n;
                return Poll::Ready(Ok(()));
            }
            if this.done {
                return Poll::Ready(Ok(()));
            }
            let want = if this.cipher.is_none() {
                HEADER_LEN
            } else {
                RECORD_LEN + 1
            };
            while !this.eof && this.filled < want {
                let mut chunk = ReadBuf::new(&mut this.buf[this.filled..want]);
                ready!(Pin::new(&mut this.inner).poll_read(cx, &mut chunk))?;
                let n = chunk.filled().len();
                if n == 0 {
                    this.eof = true;
                } else {
                    this.filled += n;
                }
            }
            if this.cipher.is_none() {
                if this.filled < HEADER_LEN || this.buf[..CONTENT_MAGIC.len()] != CONTENT_MAGIC {
                    return Poll::Ready(Err(invalid(CryptoError::Format(
                        "this is not sealed content".to_owned(),
                    ))));
                }
                let mut salt = [0_u8; CONTENT_SALT_LEN];
                salt.copy_from_slice(&this.buf[CONTENT_MAGIC.len()..HEADER_LEN]);
                this.cipher = Some(this.key.content_cipher(&salt));
                this.buf.copy_within(HEADER_LEN..this.filled, 0);
                this.filled -= HEADER_LEN;
                continue;
            }
            // More than a record buffered means another record follows.
            let (len, last) = if this.filled > RECORD_LEN {
                (RECORD_LEN, false)
            } else {
                (this.filled, true)
            };
            if len < TAG_LEN {
                return Poll::Ready(Err(invalid(CryptoError::Truncated)));
            }
            let cipher = this.cipher.as_ref().expect("set after the header");
            let record = &this.buf[..len];
            let plain = match cipher.decrypt(&record_nonce(this.counter, last), record) {
                Ok(plain) => plain,
                // A whole record that is valid, but not as the final one,
                // means the content was cut after it.
                Err(_)
                    if last
                        && len == RECORD_LEN
                        && cipher
                            .decrypt(&record_nonce(this.counter, false), record)
                            .is_ok() =>
                {
                    return Poll::Ready(Err(invalid(CryptoError::Truncated)));
                }
                Err(_) => return Poll::Ready(Err(invalid(CryptoError::Authentication))),
            };
            this.counter += 1;
            this.buf.copy_within(len..this.filled, 0);
            this.filled -= len;
            this.out = Zeroizing::new(plain);
            this.out_pos = 0;
            this.done = last;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Sealer;
    use super::*;
    use std::io::Cursor;

    fn sealer() -> Sealer {
        Sealer::new(DataKey::generate().unwrap())
    }

    /// Seals `plain` the way a store does: whole chunks, with the last one
    /// found by looking one chunk ahead.
    fn seal(sealer: &Sealer, plain: &[u8]) -> Vec<u8> {
        let mut content = sealer.content_sealer().unwrap();
        let mut out = content.header().to_vec();
        let chunks: Vec<&[u8]> = if plain.is_empty() {
            vec![&[]]
        } else {
            plain.chunks(CHUNK_LEN).collect()
        };
        for (i, chunk) in chunks.iter().enumerate() {
            out.extend(content.seal_chunk(chunk, i + 1 == chunks.len()).unwrap());
        }
        out
    }

    async fn open(sealer: &Sealer, sealed: Vec<u8>) -> io::Result<Vec<u8>> {
        let mut reader = sealer.open_content(Cursor::new(sealed));
        let mut plain = Vec::new();
        reader.read_to_end(&mut plain).await?;
        Ok(plain)
    }

    fn crypto_error(err: &io::Error) -> CryptoError {
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
        err.get_ref()
            .and_then(|inner| inner.downcast_ref::<CryptoError>())
            .cloned()
            .unwrap_or_else(|| panic!("not a crypto error: {err}"))
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    /// Byte offset of record `n` in sealed content with full records.
    fn record_at(n: usize) -> usize {
        HEADER_LEN + n * RECORD_LEN
    }

    #[tokio::test]
    async fn content_round_trips_at_every_boundary() {
        let sealer = sealer();
        for len in [
            0,
            1,
            CHUNK_LEN - 1,
            CHUNK_LEN,
            CHUNK_LEN + 1,
            3 * 1024 * 1024,
        ] {
            let plain = pattern(len);
            let sealed = seal(&sealer, &plain);
            let records = len.div_ceil(CHUNK_LEN).max(1);
            assert_eq!(sealed.len(), HEADER_LEN + len + records * TAG_LEN, "{len}");
            assert_eq!(open(&sealer, sealed).await.unwrap(), plain, "{len}");
        }
    }

    #[tokio::test]
    async fn reads_in_small_pieces_give_the_same_plaintext() {
        let sealer = sealer();
        let plain = pattern(2 * CHUNK_LEN + 17);
        let mut reader = sealer.open_content(Cursor::new(seal(&sealer, &plain)));
        let mut got = Vec::new();
        let mut piece = [0_u8; 1000];
        loop {
            let n = reader.read(&mut piece).await.unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&piece[..n]);
        }
        assert_eq!(got, plain);
    }

    #[tokio::test]
    async fn content_cut_after_a_record_is_truncated() {
        let sealer = sealer();
        let mut sealed = seal(&sealer, &pattern(2 * CHUNK_LEN + 10));
        sealed.truncate(record_at(2));
        assert_eq!(
            crypto_error(&open(&sealer, sealed).await.unwrap_err()),
            CryptoError::Truncated
        );

        let mut header_only = seal(&sealer, b"abc");
        header_only.truncate(HEADER_LEN);
        assert_eq!(
            crypto_error(&open(&sealer, header_only).await.unwrap_err()),
            CryptoError::Truncated
        );
    }

    #[tokio::test]
    async fn content_cut_inside_a_record_fails_authentication() {
        let sealer = sealer();
        let mut sealed = seal(&sealer, &pattern(2 * CHUNK_LEN + 10));
        sealed.truncate(record_at(1) + 100);
        assert_eq!(
            crypto_error(&open(&sealer, sealed).await.unwrap_err()),
            CryptoError::Authentication
        );
    }

    #[tokio::test]
    async fn reordered_duplicated_or_extended_records_fail() {
        let sealer = sealer();
        let plain = pattern(3 * CHUNK_LEN);
        let sealed = seal(&sealer, &plain);
        let rec = |n: usize| sealed[record_at(n)..record_at(n + 1)].to_vec();

        let mut swapped = sealed[..HEADER_LEN].to_vec();
        for n in [1, 0, 2] {
            swapped.extend(rec(n));
        }
        let mut duplicated = sealed[..HEADER_LEN].to_vec();
        for n in [0, 0, 1, 2] {
            duplicated.extend(rec(n));
        }
        let mut extended = sealed.clone();
        extended.push(0);
        let mut appended = sealed.clone();
        appended.extend(rec(2));

        for (what, bad) in [
            ("swapped", swapped),
            ("duplicated", duplicated),
            ("extended", extended),
            ("appended", appended),
        ] {
            let err = open(&sealer, bad).await.unwrap_err();
            assert_eq!(crypto_error(&err), CryptoError::Authentication, "{what}");
        }
    }

    #[tokio::test]
    async fn a_flipped_bit_anywhere_is_refused() {
        let sealer = sealer();
        let plain = pattern(CHUNK_LEN + 50);
        let sealed = seal(&sealer, &plain);
        // The magic, the salt, the first record, its tag, the final record.
        for at in [0, 5, HEADER_LEN + 3, record_at(1) - 1, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[at] ^= 0x10;
            assert!(open(&sealer, bad).await.is_err(), "byte {at}");
        }
    }

    #[tokio::test]
    async fn content_opens_only_under_its_own_key_and_salt() {
        let a = sealer();
        let b = sealer();
        let sealed = seal(&a, b"hello");
        assert_eq!(
            crypto_error(&open(&b, sealed.clone()).await.unwrap_err()),
            CryptoError::Authentication
        );
        let mut other_salt = sealed;
        let from = seal(&a, b"hello");
        other_salt[CONTENT_MAGIC.len()..HEADER_LEN]
            .copy_from_slice(&from[CONTENT_MAGIC.len()..HEADER_LEN]);
        assert!(open(&a, other_salt).await.is_err());
        let x = a.content_sealer().unwrap();
        let y = a.content_sealer().unwrap();
        assert_ne!(x.salt(), y.salt());
    }

    #[tokio::test]
    async fn plaintext_is_not_sealed_content() {
        let sealer = sealer();
        let err = open(
            &sealer,
            b"just some plain text here, not sealed at all".to_vec(),
        )
        .await
        .unwrap_err();
        assert!(matches!(crypto_error(&err), CryptoError::Format(_)));
    }

    #[test]
    fn chunks_must_be_whole_until_the_last_and_nothing_follows_it() {
        let sealer = sealer();
        let mut content = sealer.content_sealer().unwrap();
        assert!(content.seal_chunk(&[0; 10], false).is_err());
        assert!(content.seal_chunk(&vec![0; CHUNK_LEN + 1], true).is_err());
        content.seal_chunk(&vec![0; CHUNK_LEN], false).unwrap();
        content.seal_chunk(&[0; 10], true).unwrap();
        assert!(content.seal_chunk(&[], true).is_err());
    }

    #[tokio::test]
    async fn read_full_fills_across_short_reads() {
        let data = pattern(100);
        let mut reader = Cursor::new(data.clone()).chain(Cursor::new(vec![9_u8; 5]));
        let mut buf = [0_u8; 103];
        assert_eq!(read_full(&mut reader, &mut buf).await.unwrap(), 103);
        assert_eq!(&buf[..100], &data[..]);
        let mut rest = [0_u8; 10];
        assert_eq!(read_full(&mut reader, &mut rest).await.unwrap(), 2);
    }
}
