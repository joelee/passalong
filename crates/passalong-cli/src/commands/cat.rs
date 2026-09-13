//! `passalong cat`: print an item's content to standard output.

use std::io::{ErrorKind, Write};

use anyhow::Context as _;
use passalong_core::model::{ContentHasher, ItemKind, ItemMeta};
use passalong_core::store::Store;
use tokio::io::AsyncReadExt;

use crate::resolve::Lookup;

const CHUNK_SIZE: usize = 64 * 1024;

/// Streams the item `lookup` identifies to `out` exactly as stored, hashing
/// it on the way, and checks the SHA-256 and size after the last byte. With
/// `stdout_is_terminal`, items that are not text are refused unless `force`.
/// A reader that goes away, such as `head`, ends the output quietly.
pub async fn run(
    store: &dyn Store,
    lookup: Lookup<'_, '_>,
    force: bool,
    stdout_is_terminal: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let id = lookup.resolve(store).await?;
    let (meta, mut content) = store.get(&id).await?;
    if stdout_is_terminal && !force && !is_text(&meta) {
        anyhow::bail!(
            "item {id} is binary ({}); redirect the output or use --force",
            meta.mime
        );
    }
    let mut hasher = ContentHasher::new();
    let mut buf = vec![0_u8; CHUNK_SIZE];
    loop {
        let n = content.read(&mut buf).await.context("reading the item")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        if let Err(err) = out.write_all(&buf[..n]).and_then(|()| out.flush()) {
            if err.kind() == ErrorKind::BrokenPipe {
                tracing::debug!(id = %id, "standard output closed early");
                return Ok(());
            }
            return Err(err).context("writing to standard output");
        }
    }
    let digest = hasher.finalize();
    let actual = digest.sha256_hex();
    if actual != meta.sha256 || digest.size() != meta.size {
        anyhow::bail!(
            "item {id} failed verification: expected sha256 {} ({} bytes), got {actual} ({} bytes); the output is damaged",
            meta.sha256,
            meta.size,
            digest.size()
        );
    }
    tracing::info!(id = %id, size = meta.size, "item printed");
    Ok(())
}

/// Text items, and files with a `text/*` MIME type, are safe on a terminal.
fn is_text(meta: &ItemMeta) -> bool {
    meta.kind == ItemKind::Text || meta.mime.starts_with("text/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{TestStore, bytes};
    use passalong_core::model::{ItemMeta, NewItem};

    async fn put(ts: &TestStore, item: NewItem, data: &[u8]) -> ItemMeta {
        let meta = ts.store.put(item, bytes(data)).await.unwrap().meta;
        ts.clock.advance(1);
        meta
    }

    async fn cat(
        ts: &TestStore,
        id: &str,
        force: bool,
        terminal: bool,
    ) -> (anyhow::Result<()>, Vec<u8>) {
        let mut out = Vec::new();
        let result = run(&ts.store, Lookup::plain(id), force, terminal, &mut out).await;
        (result, out)
    }

    #[tokio::test]
    async fn text_is_printed_exactly_with_nothing_added() {
        let ts = TestStore::new();
        let meta = put(&ts, NewItem::text("box"), b"two\nlines").await;
        let (result, out) = cat(&ts, &meta.id.content_key().as_str()[..6], false, true).await;
        result.unwrap();
        assert_eq!(out, b"two\nlines");
    }

    #[tokio::test]
    async fn file_bytes_are_printed_exactly_when_not_on_a_terminal() {
        let ts = TestStore::new();
        let data: Vec<u8> = (0..200_000_u32).map(|i| (i % 251) as u8).collect();
        let meta = put(&ts, NewItem::file("blob.bin", "box"), &data).await;
        let (result, out) = cat(&ts, meta.id.as_str(), false, false).await;
        result.unwrap();
        assert_eq!(out, data);
    }

    #[tokio::test]
    async fn binary_items_are_refused_on_a_terminal_without_force() {
        let ts = TestStore::new();
        let meta = put(&ts, NewItem::file("blob.bin", "box"), &[0, 159, 146, 150]).await;
        let (result, out) = cat(&ts, meta.id.as_str(), false, true).await;
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("is binary (application/octet-stream)"),
            "{message}"
        );
        assert!(message.contains("--force"), "{message}");
        assert!(out.is_empty());
        let (result, out) = cat(&ts, meta.id.as_str(), true, true).await;
        result.unwrap();
        assert_eq!(out, [0, 159, 146, 150]);
    }

    #[tokio::test]
    async fn text_files_print_on_a_terminal() {
        let ts = TestStore::new();
        let meta = put(&ts, NewItem::file("notes.txt", "box"), b"plain notes").await;
        let (result, out) = cat(&ts, meta.id.as_str(), false, true).await;
        result.unwrap();
        assert_eq!(out, b"plain notes");
    }

    #[tokio::test]
    async fn corrupted_content_fails_verification_after_printing() {
        let ts = TestStore::new();
        let meta = put(&ts, NewItem::file("a.txt", "box"), b"original").await;
        let content = ts
            .dir
            .path()
            .join("items")
            .join(meta.id.as_str())
            .join("content");
        std::fs::write(&content, b"tampered").unwrap();
        let (result, out) = cat(&ts, meta.id.as_str(), false, false).await;
        let message = result.unwrap_err().to_string();
        assert!(
            message.starts_with(&format!("item {} failed verification", meta.id)),
            "{message}"
        );
        assert_eq!(out, b"tampered");
    }

    /// A writer whose reader has gone away, like a pipe into `head`.
    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_closed_pipe_ends_the_output_quietly() {
        let ts = TestStore::new();
        let meta = put(&ts, NewItem::text("box"), b"lots of text").await;
        run(
            &ts.store,
            Lookup::plain(meta.id.as_str()),
            false,
            false,
            &mut ClosedPipe,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn unknown_ids_fail_before_printing() {
        let ts = TestStore::new();
        let (result, out) = cat(&ts, "ffff", false, false).await;
        assert!(result.unwrap_err().to_string().contains("no item matches"));
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn images_print_as_png_bytes_but_not_on_a_terminal() {
        use crate::commands::support::T;
        use passalong_core::clipboard::{RgbaImage, encode_png};
        let ts = TestStore::new();
        let png = encode_png(&RgbaImage::new(1, 1, vec![9, 9, 9, 255]).unwrap()).unwrap();
        let meta = put(
            &ts,
            NewItem::clipboard_image("box", T.parse().unwrap()),
            &png,
        )
        .await;
        let (result, out) = cat(&ts, meta.id.as_str(), false, false).await;
        result.unwrap();
        assert_eq!(out, png);
        let (result, _) = cat(&ts, meta.id.as_str(), false, true).await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("is binary (image/png)")
        );
    }
}
