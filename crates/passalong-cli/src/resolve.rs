//! Choosing an item when an id prefix matches several.

use std::collections::HashMap;
use std::io::Write;

use chrono::{DateTime, Utc};
use passalong_core::model::{ItemId, ItemMeta};
use passalong_core::store::{Store, StoreError};

use crate::output::{display_name, kind_label};
use crate::prompt::Prompt;

/// How many matching items are offered at most.
pub const MAX_CHOICES: usize = 9;
/// How many answers are accepted before giving up.
const MAX_ATTEMPTS: usize = 3;

/// What is needed to ask the user which item they mean.
pub struct Chooser<'a> {
    /// Asks the question; nothing is asked unless it is interactive.
    pub prompt: &'a mut dyn Prompt,
    /// Where the matching items are listed: standard error.
    pub err: &'a mut dyn Write,
    /// The current time, for "5 min ago".
    pub now: DateTime<Utc>,
}

/// An id or prefix typed by the user, and optionally a way to ask which
/// item they mean when it matches several.
pub struct Lookup<'a, 'b> {
    /// What the user typed.
    pub input: &'a str,
    /// How to ask; `None` keeps the ambiguity error.
    pub chooser: Option<&'a mut Chooser<'b>>,
}

impl<'a> Lookup<'a, '_> {
    /// A lookup that never asks, for tests.
    #[cfg(test)]
    pub fn plain(input: &'a str) -> Self {
        Self {
            input,
            chooser: None,
        }
    }

    /// Resolves the input, asking when it is ambiguous; see [`resolve_item`].
    pub async fn resolve(self, store: &dyn Store) -> anyhow::Result<ItemId> {
        resolve_item(store, self.input, self.chooser).await
    }
}

/// Resolves `input` to one item. When it matches several and `chooser` is
/// interactive, lists up to [`MAX_CHOICES`] of them, newest first, and asks
/// for a number; an empty answer, or three answers that are not a listed
/// number, cancel. Otherwise the store's error is returned unchanged.
pub async fn resolve_item(
    store: &dyn Store,
    input: &str,
    chooser: Option<&mut Chooser<'_>>,
) -> anyhow::Result<ItemId> {
    let (candidates, chooser) = match store.resolve(input).await {
        Err(StoreError::Ambiguous { input, candidates }) => match chooser {
            Some(chooser) if chooser.prompt.is_interactive() => (candidates, chooser),
            _ => return Err(StoreError::Ambiguous { input, candidates }.into()),
        },
        other => return Ok(other?),
    };
    let metas: HashMap<ItemId, ItemMeta> = store
        .list()
        .await?
        .into_iter()
        .map(|meta| (meta.id.clone(), meta))
        .collect();
    let shown = &candidates[..candidates.len().min(MAX_CHOICES)];
    writeln!(chooser.err, "`{input}` matches {} items:", candidates.len())?;
    let rows: Vec<[String; 5]> = shown
        .iter()
        .map(|id| match metas.get(id) {
            Some(meta) => [
                id.to_string(),
                kind_label(meta).to_owned(),
                display_name(meta),
                meta.device.clone(),
                age(meta.created_at, chooser.now),
            ],
            None => [
                id.to_string(),
                "?".into(),
                "-".into(),
                "-".into(),
                "-".into(),
            ],
        })
        .collect();
    let widths: [usize; 5] = std::array::from_fn(|column| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    });
    for (number, row) in rows.iter().enumerate() {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        writeln!(
            chooser.err,
            "  {}  {}",
            number + 1,
            cells.join("  ").trim_end()
        )?;
    }
    if candidates.len() > shown.len() {
        writeln!(
            chooser.err,
            "  and {} more; type more characters of the id",
            candidates.len() - shown.len()
        )?;
    }
    let question = format!("Choose 1-{}, or press Enter to cancel", shown.len());
    for _ in 0..MAX_ATTEMPTS {
        let answer = chooser.prompt.ask(&question, None)?;
        if answer.is_empty() {
            anyhow::bail!("cancelled");
        }
        match answer.parse::<usize>() {
            Ok(number) if (1..=shown.len()).contains(&number) => {
                return Ok(shown[number - 1].clone());
            }
            _ => writeln!(
                chooser.err,
                "please answer with a number from 1 to {}",
                shown.len()
            )?,
        }
    }
    anyhow::bail!("cancelled: no valid choice")
}

/// How long ago `created` was, in the largest whole unit.
fn age(created: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - created).num_seconds();
    match secs {
        ..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{TestStore, bytes};
    use crate::prompt::ScriptedPrompt;
    use passalong_core::model::{ItemId, NewItem};
    use passalong_core::store::StoreError;

    /// Stores `n` texts in the same second, so they all share the
    /// timestamp part of their ids, `xxxxxxxx-`. Returns that prefix.
    async fn same_second(ts: &TestStore, n: usize) -> String {
        let mut prefix = String::new();
        for i in 0..n {
            let text = format!("text number {i}");
            let meta = ts
                .store
                .put(NewItem::text("box"), bytes(text.as_bytes()))
                .await
                .unwrap()
                .meta;
            prefix = meta.id.as_str()[..9].to_owned();
        }
        prefix
    }

    async fn candidates(ts: &TestStore, input: &str) -> Vec<ItemId> {
        match ts.store.resolve(input).await {
            Err(StoreError::Ambiguous { candidates, .. }) => candidates,
            other => panic!("expected an ambiguous prefix, got {other:?}"),
        }
    }

    async fn choose(
        ts: &TestStore,
        input: &str,
        prompt: &mut ScriptedPrompt,
    ) -> (anyhow::Result<ItemId>, String) {
        let mut err = Vec::new();
        let result = {
            let mut chooser = Chooser {
                prompt,
                err: &mut err,
                now: "2026-09-12T10:00:00Z".parse().unwrap(),
            };
            resolve_item(&ts.store, input, Some(&mut chooser)).await
        };
        (result, String::from_utf8(err).unwrap())
    }

    #[tokio::test]
    async fn a_unique_prefix_needs_no_question() {
        let ts = TestStore::new();
        let meta = ts
            .store
            .put(NewItem::text("box"), bytes(b"only"))
            .await
            .unwrap()
            .meta;
        let mut prompt = ScriptedPrompt::new(true, []);
        let (result, err) = choose(&ts, &meta.id.content_key().as_str()[..6], &mut prompt).await;
        assert_eq!(result.unwrap(), meta.id);
        assert!(prompt.questions().is_empty());
        assert!(err.is_empty(), "{err}");
    }

    #[tokio::test]
    async fn the_chosen_candidate_is_returned() {
        let ts = TestStore::new();
        let prefix = same_second(&ts, 2).await;
        let expected = candidates(&ts, &prefix).await;
        let mut prompt = ScriptedPrompt::new(true, ["2"]);
        let (result, err) = choose(&ts, &prefix, &mut prompt).await;
        assert_eq!(result.unwrap(), expected[1]);
        assert_eq!(prompt.questions(), ["Choose 1-2, or press Enter to cancel"]);
        assert!(
            err.starts_with(&format!("`{prefix}` matches 2 items:\n")),
            "{err}"
        );
        assert!(
            err.contains(&format!("  1  {}  text", expected[0])),
            "{err}"
        );
        assert!(
            err.contains(&format!("  2  {}  text", expected[1])),
            "{err}"
        );
        assert!(err.contains("text number"), "{err}");
        assert!(err.contains("box"), "{err}");
        assert!(err.contains("6 min ago"), "{err}");
    }

    #[tokio::test]
    async fn an_empty_answer_cancels() {
        let ts = TestStore::new();
        let prefix = same_second(&ts, 2).await;
        let mut prompt = ScriptedPrompt::new(true, [""]);
        let (result, _) = choose(&ts, &prefix, &mut prompt).await;
        assert_eq!(result.unwrap_err().to_string(), "cancelled");
    }

    #[tokio::test]
    async fn an_invalid_answer_is_asked_again_up_to_three_times() {
        let ts = TestStore::new();
        let prefix = same_second(&ts, 2).await;
        let expected = candidates(&ts, &prefix).await;
        let mut prompt = ScriptedPrompt::new(true, ["9", "1"]);
        let (result, err) = choose(&ts, &prefix, &mut prompt).await;
        assert_eq!(result.unwrap(), expected[0]);
        assert_eq!(
            err.matches("answer with a number from 1 to 2").count(),
            1,
            "{err}"
        );

        let mut prompt = ScriptedPrompt::new(true, ["x", "0", "3"]);
        let (result, err) = choose(&ts, &prefix, &mut prompt).await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "cancelled: no valid choice"
        );
        assert_eq!(prompt.questions().len(), 3);
        assert_eq!(
            err.matches("answer with a number from 1 to 2").count(),
            3,
            "{err}"
        );
    }

    #[tokio::test]
    async fn at_most_nine_candidates_are_offered() {
        let ts = TestStore::new();
        let prefix = same_second(&ts, 11).await;
        let expected = candidates(&ts, &prefix).await;
        let mut prompt = ScriptedPrompt::new(true, ["9"]);
        let (result, err) = choose(&ts, &prefix, &mut prompt).await;
        assert_eq!(result.unwrap(), expected[8]);
        assert!(
            err.starts_with(&format!("`{prefix}` matches 11 items:\n")),
            "{err}"
        );
        assert!(err.contains(&format!("  9  {}", expected[8])), "{err}");
        assert!(!err.contains(&expected[9].to_string()), "{err}");
        assert!(
            err.contains("and 2 more; type more characters of the id"),
            "{err}"
        );
        assert_eq!(prompt.questions(), ["Choose 1-9, or press Enter to cancel"]);
    }

    #[tokio::test]
    async fn without_a_terminal_the_ambiguity_error_stands() {
        let ts = TestStore::new();
        let prefix = same_second(&ts, 2).await;
        let expected = candidates(&ts, &prefix).await;
        let mut prompt = ScriptedPrompt::new(false, []);
        let (result, err) = choose(&ts, &prefix, &mut prompt).await;
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains(expected[0].as_str()) && message.contains(expected[1].as_str()),
            "{message}"
        );
        assert!(prompt.questions().is_empty());
        assert!(err.is_empty(), "{err}");
        let message = resolve_item(&ts.store, &prefix, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(message.contains("matches 2 items"), "{message}");
    }

    #[test]
    fn ages_read_like_a_person_would_say_them() {
        let now: chrono::DateTime<chrono::Utc> = "2026-09-12T10:00:00Z".parse().unwrap();
        let ago = |secs: i64| age(now - chrono::Duration::seconds(secs), now);
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(3599), "59 min ago");
        assert_eq!(ago(3600), "1 h ago");
        assert_eq!(ago(86_399), "23 h ago");
        assert_eq!(ago(86_400), "1 d ago");
        assert_eq!(ago(-30), "just now");
    }
}
