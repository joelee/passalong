//! `passalong delete`: remove items from the store.

use std::io::Write;

use anyhow::Context as _;
use passalong_core::store::Store;

/// Deletes the items `inputs` identify and prints each deleted id.
///
/// Every id is resolved before anything is deleted, so an unknown or
/// ambiguous id deletes nothing. An item named more than once is deleted
/// once.
pub async fn run(store: &dyn Store, inputs: &[String], out: &mut dyn Write) -> anyhow::Result<()> {
    let mut ids = Vec::with_capacity(inputs.len());
    for input in inputs {
        let id = store.resolve(input).await?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    for id in &ids {
        store
            .delete(id)
            .await
            .with_context(|| format!("deleting {id}"))?;
        writeln!(out, "{id}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{TestStore, bytes};
    use passalong_core::model::{ContentHasher, ItemMeta, NewItem};
    use std::collections::HashMap;

    async fn put(ts: &TestStore, text: &str) -> ItemMeta {
        let meta = ts
            .store
            .put(NewItem::text("box"), bytes(text.as_bytes()))
            .await
            .unwrap()
            .meta;
        ts.clock.advance(1);
        meta
    }

    async fn delete(ts: &TestStore, inputs: &[&str]) -> anyhow::Result<String> {
        let inputs: Vec<String> = inputs.iter().map(|s| (*s).to_owned()).collect();
        let mut out = Vec::new();
        run(&ts.store, &inputs, &mut out).await?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[tokio::test]
    async fn deletes_exactly_the_named_items_and_prints_their_ids() {
        let ts = TestStore::new();
        let a = put(&ts, "alpha").await;
        let b = put(&ts, "bravo").await;
        let c = put(&ts, "charlie").await;
        let out = delete(&ts, &[&a.id.content_key().as_str()[..6], b.id.as_str()])
            .await
            .unwrap();
        assert_eq!(out, format!("{}\n{}\n", a.id, b.id));
        assert_eq!(ts.store.list().await.unwrap(), vec![c]);
    }

    #[tokio::test]
    async fn naming_an_item_twice_deletes_it_once() {
        let ts = TestStore::new();
        let a = put(&ts, "alpha").await;
        let out = delete(&ts, &[a.id.as_str(), &a.id.content_key().as_str()[..4]])
            .await
            .unwrap();
        assert_eq!(out, format!("{}\n", a.id));
    }

    #[tokio::test]
    async fn an_unknown_id_deletes_nothing() {
        let ts = TestStore::new();
        let a = put(&ts, "alpha").await;
        let err = delete(&ts, &[a.id.as_str(), "ffff"]).await.unwrap_err();
        assert!(err.to_string().contains("no item matches `ffff`"), "{err}");
        assert_eq!(ts.store.list().await.unwrap(), vec![a]);
    }

    #[tokio::test]
    async fn an_ambiguous_prefix_deletes_nothing() {
        let ts = TestStore::new();
        let mut seen: HashMap<String, String> = HashMap::new();
        let (x, y) = (0..)
            .find_map(|i| {
                let text = format!("item {i}");
                let mut h = ContentHasher::new();
                h.update(text.as_bytes());
                let prefix = h.finalize().content_key().as_str()[..4].to_owned();
                seen.insert(prefix, text.clone()).map(|prev| (prev, text))
            })
            .unwrap();
        let mx = put(&ts, &x).await;
        put(&ts, &y).await;
        let err = delete(&ts, &[&mx.id.content_key().as_str()[..4]])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("matches 2 items"), "{err}");
        assert_eq!(ts.store.list().await.unwrap().len(), 2);
    }
}
