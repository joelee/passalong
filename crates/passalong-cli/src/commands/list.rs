//! `passalong list`: show stored items, newest first.

use std::io::Write;

use chrono::FixedOffset;
use passalong_core::store::Store;

use crate::output;

/// Lists the store's items as a table, or as JSON when `json` is set.
pub async fn run(
    store: &dyn Store,
    json: bool,
    offset: FixedOffset,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let items = store.list().await?;
    tracing::debug!("listed {} items", items.len());
    let text = if json {
        output::render_json(&items)?
    } else {
        output::render_table(&items, offset)
    };
    out.write_all(text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{TestStore, bytes};
    use chrono::FixedOffset;
    use passalong_core::model::{ItemMeta, NewItem};

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    #[tokio::test]
    async fn prints_a_table_newest_first() {
        let ts = TestStore::new();
        let old = ts
            .store
            .put(NewItem::text("box"), bytes(b"hello"))
            .await
            .unwrap()
            .meta;
        ts.clock.advance(60);
        let new = ts
            .store
            .put(NewItem::file("a.txt", "box"), bytes(b"file body"))
            .await
            .unwrap()
            .meta;
        let mut out = Vec::new();
        run(&ts.store, false, utc(), &mut out).await.unwrap();
        let out = String::from_utf8(out).unwrap();
        let ids: Vec<_> = out
            .lines()
            .skip(1)
            .map(|l| l.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(ids, [new.id.as_str(), old.id.as_str()]);
        assert!(out.starts_with("ID "), "{out}");
    }

    #[tokio::test]
    async fn prints_json_when_asked() {
        let ts = TestStore::new();
        ts.store
            .put(NewItem::text("box"), bytes(b"hello"))
            .await
            .unwrap();
        let mut out = Vec::new();
        run(&ts.store, true, utc(), &mut out).await.unwrap();
        let items: Vec<ItemMeta> = serde_json::from_slice(&out).unwrap();
        assert_eq!(items, ts.store.list().await.unwrap());
    }

    #[tokio::test]
    async fn says_so_when_empty() {
        let ts = TestStore::new();
        let mut out = Vec::new();
        run(&ts.store, false, utc(), &mut out).await.unwrap();
        assert_eq!(out, b"no items\n");
    }
}
