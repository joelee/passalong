//! `passalong get`: print one item's metadata.

use std::io::Write;

use chrono::FixedOffset;
use passalong_core::store::Store;

use crate::output;
use crate::resolve::Lookup;

/// Prints the metadata of the item `lookup` identifies: one `field: value`
/// line each, with times at `offset`, or with `json` the object `list
/// --json` prints for it. Only the item's metadata is read.
pub async fn run(
    store: &dyn Store,
    lookup: Lookup<'_, '_>,
    json: bool,
    offset: FixedOffset,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let id = lookup.resolve(store).await?;
    let meta = store.get_meta(&id).await?;
    let text = if json {
        output::render_meta_json(&meta)?
    } else {
        output::render_meta(&meta, offset)
    };
    out.write_all(text.as_bytes())?;
    tracing::debug!(id = %id, "metadata printed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{T, TestStore, bytes};
    use passalong_core::model::{ItemMeta, NewItem};

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    async fn put(ts: &TestStore, item: NewItem, data: &[u8]) -> ItemMeta {
        let meta = ts.store.put(item, bytes(data)).await.unwrap().meta;
        ts.clock.advance(1);
        meta
    }

    async fn get(ts: &TestStore, id: &str, json: bool) -> anyhow::Result<String> {
        let mut out = Vec::new();
        run(&ts.store, Lookup::plain(id), json, utc(), &mut out).await?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[tokio::test]
    async fn text_items_show_their_fields_and_preview() {
        let ts = TestStore::new();
        let meta = put(&ts, NewItem::text("laptop"), b"hello").await;
        let out = get(&ts, meta.id.as_str(), false).await.unwrap();
        let expected = format!(
            "id:      {id}\nkind:    text\npreview: hello\nmime:    text/plain; charset=utf-8\nsize:    5 B (5 bytes)\nsha256:  {sha}\ndevice:  laptop\ncreated: 2026-09-12 09:53:11 +00:00 (2026-09-12T09:53:11Z)\n",
            id = meta.id,
            sha = meta.sha256
        );
        assert_eq!(out, expected);
    }

    #[tokio::test]
    async fn files_show_their_name_and_images_their_origin() {
        let ts = TestStore::new();
        let file = put(&ts, NewItem::file("report.pdf", "box"), &[7_u8; 1536]).await;
        let out = get(&ts, file.id.as_str(), false).await.unwrap();
        assert!(out.contains("\nkind:    file\n"), "{out}");
        assert!(out.contains("\nname:    report.pdf\n"), "{out}");
        assert!(out.contains("\nmime:    application/pdf\n"), "{out}");
        assert!(out.contains("\nsize:    1.5 KiB (1536 bytes)\n"), "{out}");
        assert!(
            !out.contains("preview:") && !out.contains("origin:"),
            "{out}"
        );

        let image = put(
            &ts,
            NewItem::clipboard_image("box", T.parse().unwrap()),
            b"png bytes",
        )
        .await;
        let out = get(&ts, image.id.as_str(), false).await.unwrap();
        assert!(out.contains("\nkind:    image\n"), "{out}");
        assert!(out.contains("\norigin:  clipboard\n"), "{out}");
    }

    #[tokio::test]
    async fn times_use_the_given_offset_beside_utc() {
        let ts = TestStore::new();
        let meta = put(&ts, NewItem::text("box"), b"hello").await;
        let mut out = Vec::new();
        run(
            &ts.store,
            Lookup::plain(meta.id.as_str()),
            false,
            FixedOffset::east_opt(2 * 3600).unwrap(),
            &mut out,
        )
        .await
        .unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("created: 2026-09-12 11:53:11 +02:00 (2026-09-12T09:53:11Z)\n"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn json_is_the_items_list_entry() {
        let ts = TestStore::new();
        let meta = put(
            &ts,
            NewItem::clipboard_image("box", T.parse().unwrap()),
            b"png bytes",
        )
        .await;
        let out = get(&ts, meta.id.as_str(), true).await.unwrap();
        assert!(out.ends_with("}\n"), "{out}");
        let back: ItemMeta = serde_json::from_str(&out).unwrap();
        assert_eq!(back, meta);
        assert_eq!(vec![back], ts.store.list().await.unwrap());
    }

    #[tokio::test]
    async fn a_prefix_resolves_and_an_unknown_id_fails() {
        let ts = TestStore::new();
        let meta = put(&ts, NewItem::text("box"), b"hello").await;
        let prefix = &meta.id.content_key().as_str()[..6];
        let out = get(&ts, prefix, false).await.unwrap();
        assert!(out.starts_with(&format!("id:      {}\n", meta.id)), "{out}");
        let err = get(&ts, "ffff", false).await.unwrap_err();
        assert!(err.to_string().contains("no item matches"), "{err}");
    }

    #[tokio::test]
    async fn reads_one_metadata_file_and_never_the_content() {
        use passalong_core::fs::LocalFs;
        use passalong_core::random::StdRandom;
        use passalong_core::store::FsStore;
        use passalong_core::testing::{FaultyFs, FsOp, ManualClock};
        use std::sync::Arc;
        let dir = tempfile::TempDir::new().unwrap();
        let store = FsStore::new(
            FaultyFs::new(LocalFs::new(dir.path())),
            Arc::new(ManualClock::at(T)),
            Box::new(StdRandom::new()),
        );
        let meta = store
            .put(NewItem::file("big.bin", "box"), bytes(&[1_u8; 4096]))
            .await
            .unwrap()
            .meta;
        let before = store.fs().calls(FsOp::OpenRead);
        let mut out = Vec::new();
        run(
            &store,
            Lookup::plain(&meta.id.content_key().as_str()[..8]),
            false,
            utc(),
            &mut out,
        )
        .await
        .unwrap();
        assert_eq!(
            store.fs().calls(FsOp::OpenRead) - before,
            1,
            "only meta.json is opened"
        );
    }
}
