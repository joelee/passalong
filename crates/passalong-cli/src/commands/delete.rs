//! `passalong delete`: remove items from the store.

use std::io::Write;

use anyhow::Context as _;
use passalong_core::store::Store;

use crate::resolve::{Chooser, resolve_item};

/// Deletes the items `inputs` identify and prints each deleted id.
///
/// Every id is resolved before anything is deleted, asking through
/// `chooser` when one is ambiguous, so an unknown id or a cancelled choice
/// deletes nothing. An item named more than once is deleted once.
pub async fn run(
    store: &dyn Store,
    inputs: &[String],
    mut chooser: Option<&mut Chooser<'_>>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let mut ids = Vec::with_capacity(inputs.len());
    for input in inputs {
        let id = resolve_item(store, input, chooser.as_deref_mut()).await?;
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
    use crate::commands::support::{T, TestStore, bytes};
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
        run(&ts.store, &inputs, None, &mut out).await?;
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

    #[tokio::test]
    async fn ambiguous_ids_are_chosen_before_anything_is_deleted() {
        use crate::prompt::ScriptedPrompt;
        use crate::resolve::Chooser;
        use passalong_core::store::StoreError;
        let ts = TestStore::new();
        let keep = put(&ts, "listed first").await;
        let a = ts
            .store
            .put(NewItem::text("box"), bytes(b"same second a"))
            .await
            .unwrap()
            .meta;
        ts.store
            .put(NewItem::text("box"), bytes(b"same second b"))
            .await
            .unwrap();
        let prefix = a.id.as_str()[..9].to_owned();
        let candidates = match ts.store.resolve(&prefix).await {
            Err(StoreError::Ambiguous { candidates, .. }) => candidates,
            other => panic!("{other:?}"),
        };
        let inputs = [keep.id.to_string(), prefix.clone()];

        let mut prompt = ScriptedPrompt::new(true, [""]);
        let mut err = Vec::new();
        let mut out = Vec::new();
        let mut chooser = Chooser {
            prompt: &mut prompt,
            err: &mut err,
            now: T.parse().unwrap(),
        };
        let result = run(&ts.store, &inputs, Some(&mut chooser), &mut out).await;
        assert_eq!(result.unwrap_err().to_string(), "cancelled");
        assert!(out.is_empty());
        assert_eq!(ts.store.list().await.unwrap().len(), 3, "nothing deleted");

        let mut prompt = ScriptedPrompt::new(true, ["1"]);
        let mut err = Vec::new();
        let mut chooser = Chooser {
            prompt: &mut prompt,
            err: &mut err,
            now: T.parse().unwrap(),
        };
        run(&ts.store, &inputs, Some(&mut chooser), &mut out)
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("{}\n{}\n", keep.id, candidates[0])
        );
        let left: Vec<_> = ts
            .store
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(left, [candidates[1].clone()]);
    }
}
