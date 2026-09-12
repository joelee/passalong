//! `passalong clipboard`: send the clipboard's text.

use std::io::{Cursor, Read, Write};

use anyhow::Context as _;
use passalong_core::clipboard::Clipboard;
use passalong_core::model::NewItem;
use passalong_core::store::Store;

/// Where the text to send comes from.
pub enum TextSource<'a> {
    /// The clipboard.
    Clipboard(&'a mut dyn Clipboard),
    /// A reader such as standard input (`--stdin`).
    Reader(&'a mut dyn Read),
}

/// Stores the text as a new item and prints its id. Sending text that is
/// already stored prints the existing item's id.
pub async fn run(
    store: &dyn Store,
    source: TextSource<'_>,
    device: &str,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let text = match source {
        TextSource::Clipboard(clipboard) => clipboard.read_text()?.unwrap_or_default(),
        TextSource::Reader(reader) => {
            let mut text = String::new();
            reader
                .read_to_string(&mut text)
                .context("reading standard input as UTF-8 text")?;
            text
        }
    };
    if text.trim().is_empty() {
        anyhow::bail!("clipboard is empty");
    }
    let outcome = store
        .put(
            NewItem::text(device),
            Box::new(Cursor::new(text.into_bytes())),
        )
        .await?;
    writeln!(out, "{}", outcome.meta.id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::TestStore;
    use passalong_core::clipboard::ClipboardError;
    use passalong_core::model::ItemKind;
    use passalong_core::testing::MockClipboard;
    use tokio::io::AsyncReadExt;

    fn printed_id(out: Vec<u8>) -> String {
        let out = String::from_utf8(out).unwrap();
        let id = out.strip_suffix('\n').expect("one line");
        assert!(passalong_core::model::ItemId::parse(id).is_ok(), "{out:?}");
        id.to_owned()
    }

    #[tokio::test]
    async fn sends_the_clipboard_text_and_prints_the_id() {
        let ts = TestStore::new();
        let mut clip = MockClipboard::with_text("hello");
        let mut out = Vec::new();
        run(&ts.store, TextSource::Clipboard(&mut clip), "box", &mut out)
            .await
            .unwrap();
        let id = printed_id(out);
        let items = ts.store.list().await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            (
                items[0].id.as_str(),
                items[0].kind,
                items[0].device.as_str()
            ),
            (id.as_str(), ItemKind::Text, "box")
        );
        let (_, mut stream) = ts.store.get(&items[0].id).await.unwrap();
        let mut body = String::new();
        stream.read_to_string(&mut body).await.unwrap();
        assert_eq!(body, "hello");
    }

    #[tokio::test]
    async fn reads_standard_input_when_asked() {
        let ts = TestStore::new();
        let mut stdin = std::io::Cursor::new(b"from stdin\n".to_vec());
        let mut out = Vec::new();
        run(&ts.store, TextSource::Reader(&mut stdin), "box", &mut out)
            .await
            .unwrap();
        printed_id(out);
        assert_eq!(
            ts.store.list().await.unwrap()[0].preview.as_deref(),
            Some("from stdin")
        );
    }

    #[tokio::test]
    async fn refuses_empty_or_blank_text() {
        let ts = TestStore::new();
        for mut clip in [
            MockClipboard::new(),
            MockClipboard::with_text(""),
            MockClipboard::with_text(" \n\t "),
        ] {
            let err = run(
                &ts.store,
                TextSource::Clipboard(&mut clip),
                "box",
                &mut Vec::new(),
            )
            .await
            .unwrap_err();
            assert_eq!(err.to_string(), "clipboard is empty");
        }
        let mut empty = std::io::Cursor::new(Vec::new());
        let err = run(
            &ts.store,
            TextSource::Reader(&mut empty),
            "box",
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), "clipboard is empty");
        assert!(ts.store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn sending_the_same_text_twice_prints_the_same_id() {
        let ts = TestStore::new();
        let mut clip = MockClipboard::with_text("same");
        let mut first = Vec::new();
        run(
            &ts.store,
            TextSource::Clipboard(&mut clip),
            "box",
            &mut first,
        )
        .await
        .unwrap();
        ts.clock.advance(30);
        let mut second = Vec::new();
        run(
            &ts.store,
            TextSource::Clipboard(&mut clip),
            "box",
            &mut second,
        )
        .await
        .unwrap();
        assert_eq!(printed_id(first), printed_id(second));
        assert_eq!(ts.store.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reports_clipboard_and_input_failures() {
        let ts = TestStore::new();
        let mut clip = MockClipboard::new();
        clip.push_read_error(ClipboardError::Unavailable("no display".into()));
        let err = run(
            &ts.store,
            TextSource::Clipboard(&mut clip),
            "box",
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("clipboard unavailable: no display"),
            "{err:#}"
        );
        let mut binary = std::io::Cursor::new(vec![0xff, 0xfe, 0x00]);
        let err = run(
            &ts.store,
            TextSource::Reader(&mut binary),
            "box",
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("standard input"), "{err:#}");
    }
}
