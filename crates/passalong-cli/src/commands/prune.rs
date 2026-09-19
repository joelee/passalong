//! `passalong prune`: delete items by age or count.

use std::io::Write;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, FixedOffset, Utc};
use passalong_core::encryption::{self, EncryptionAdmin, EncryptionError, StoreState};
use passalong_core::model::ItemMeta;
use passalong_core::retention;
use passalong_core::store::Store;

use crate::output;
use crate::prompt::Prompt;

/// Staging directories older than this belong to no live upload.
const STALE_STAGING: Duration = Duration::from_secs(3_600);

/// What `prune` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneOptions {
    /// Delete items created at least this long ago.
    pub older_than: Option<Duration>,
    /// Always keep this many of the newest items.
    pub keep: Option<usize>,
    /// Show what would be deleted, then stop.
    pub dry_run: bool,
    /// Do not ask for confirmation.
    pub yes: bool,
    /// `--quiet`: results are hidden, so the list is shown with the
    /// confirmation question instead.
    pub quiet: bool,
}

fn items_word(n: usize) -> &'static str {
    if n == 1 { "item" } else { "items" }
}

/// Selects items with [`retention::select`], shows them, asks unless `yes`,
/// deletes them, and removes stale staging directories. `now` decides ages.
pub async fn run(
    store: &dyn Store,
    options: &PruneOptions,
    now: DateTime<Utc>,
    prompt: &mut dyn Prompt,
    offset: FixedOffset,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if options.older_than.is_none() && options.keep.is_none() {
        anyhow::bail!("prune needs --older-than <AGE>, --keep <N>, or both");
    }
    let items = store.list().await?;
    let ids = retention::select(&items, now, options.older_than, options.keep);
    let selected: Vec<ItemMeta> = items
        .into_iter()
        .filter(|meta| ids.contains(&meta.id))
        .collect();

    if selected.is_empty() {
        writeln!(out, "nothing to prune")?;
    } else {
        let n = selected.len();
        let listing = format!(
            "{n} {} to delete:\n{}",
            items_word(n),
            output::render_table(&selected, offset)
        );
        out.write_all(listing.as_bytes())?;
        if options.dry_run {
            writeln!(out, "dry run: nothing deleted")?;
            return Ok(());
        }
        if !options.yes {
            if !prompt.is_interactive() {
                anyhow::bail!(
                    "refusing to delete {n} {} without --yes when not running in a terminal",
                    items_word(n)
                );
            }
            if options.quiet {
                prompt.show(&listing)?;
            }
            if !prompt.confirm(&format!("Delete {n} {}?", items_word(n)))? {
                writeln!(out, "nothing deleted")?;
                return Ok(());
            }
        }
        for meta in &selected {
            store
                .delete(&meta.id)
                .await
                .with_context(|| format!("deleting {}", meta.id))?;
        }
        writeln!(out, "deleted {n} {}", items_word(n))?;
    }
    if !options.dry_run {
        let removed = store.clean_staging(STALE_STAGING).await?;
        if removed > 0 {
            let dirs = if removed == 1 {
                "directory"
            } else {
                "directories"
            };
            writeln!(out, "removed {removed} stale staging {dirs}")?;
        }
    }
    Ok(())
}

/// `prune --plain`: prunes the unencrypted items a fresh start left in
/// `plain/`, as [`run`] prunes a store's items, and removes `plain/` once it
/// is empty.
///
/// # Errors
///
/// For a store that is not encrypted or cannot be used, and as [`run`].
pub async fn run_plain(
    admin: &dyn EncryptionAdmin,
    options: &PruneOptions,
    now: DateTime<Utc>,
    prompt: &mut dyn Prompt,
    offset: FixedOffset,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let plain_left = match admin.inspect().await? {
        StoreState::Encrypted { plain_left, .. } => plain_left,
        StoreState::Plain { .. } => anyhow::bail!(
            "the store is not encrypted: `--plain` prunes the unencrypted items an encrypted store kept from before; use `passalong prune` without it"
        ),
        StoreState::Rewriting { started } => {
            return Err(EncryptionError::Rewriting { started }.into());
        }
        StoreState::Broken => return Err(EncryptionError::HeaderMissing.into()),
        _ => anyhow::bail!("this store's state is not known to this version of passalong"),
    };
    // Only a filesystem can hold what passalong 0.2.0 left.
    if let Some(fs) = admin.fs() {
        let left = encryption::leftovers(fs).await?;
        if !left.is_empty() {
            if options.dry_run {
                writeln!(out, "would remove {left}")?;
            } else {
                encryption::remove_leftovers(fs).await?;
                writeln!(out, "removed {left}")?;
            }
        }
    }
    if plain_left == 0 {
        writeln!(out, "no unencrypted items remain")?;
        return Ok(());
    }
    run(
        admin.plain_store().as_ref(),
        options,
        now,
        prompt,
        offset,
        out,
    )
    .await?;
    if !options.dry_run && admin.remove_plain_if_empty().await? {
        writeln!(out, "removed plain/: no unencrypted items remain")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{T, TestStore, bytes};
    use crate::prompt::ScriptedPrompt;
    use chrono::{FixedOffset, TimeDelta};
    use passalong_core::clock::Clock;
    use passalong_core::model::{ItemMeta, NewItem};

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    /// Three items created 3, 2, and 1 days before the fixture clock.
    async fn seeded() -> (TestStore, Vec<ItemMeta>) {
        let ts = TestStore::new();
        ts.clock.advance(-3 * 86_400);
        let mut metas = Vec::new();
        for text in ["oldest", "middle", "newest"] {
            metas.push(
                ts.store
                    .put(NewItem::text("box"), bytes(text.as_bytes()))
                    .await
                    .unwrap()
                    .meta,
            );
            ts.clock.advance(86_400);
        }
        (ts, metas)
    }

    fn opts(
        older_than: Option<u64>,
        keep: Option<usize>,
        dry_run: bool,
        yes: bool,
    ) -> PruneOptions {
        PruneOptions {
            older_than: older_than.map(std::time::Duration::from_secs),
            keep,
            dry_run,
            yes,
            quiet: false,
        }
    }

    async fn prune(
        ts: &TestStore,
        o: &PruneOptions,
        prompt: &mut ScriptedPrompt,
    ) -> anyhow::Result<String> {
        let mut out = Vec::new();
        run(&ts.store, o, ts.clock.now(), prompt, utc(), &mut out).await?;
        Ok(String::from_utf8(out).unwrap())
    }

    fn remaining(metas: &[ItemMeta], keep: &[usize]) -> Vec<ItemMeta> {
        let mut v: Vec<ItemMeta> = keep.iter().map(|&i| metas[i].clone()).collect();
        v.reverse();
        v
    }

    #[tokio::test]
    async fn needs_an_age_or_a_count() {
        let (ts, _) = seeded().await;
        let err = prune(
            &ts,
            &opts(None, None, false, true),
            &mut ScriptedPrompt::new(true, Vec::<&str>::new()),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("--older-than"), "{err}");
    }

    #[tokio::test]
    async fn dry_run_lists_candidates_and_deletes_nothing() {
        let (ts, metas) = seeded().await;
        let out = prune(
            &ts,
            &opts(None, Some(1), true, false),
            &mut ScriptedPrompt::new(false, Vec::<&str>::new()),
        )
        .await
        .unwrap();
        assert!(out.starts_with("2 items to delete:\n"), "{out}");
        assert!(
            out.contains(metas[0].id.as_str())
                && out.contains(metas[1].id.as_str())
                && !out.contains(metas[2].id.as_str()),
            "{out}"
        );
        assert!(out.ends_with("dry run: nothing deleted\n"), "{out}");
        assert_eq!(ts.store.list().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn declining_the_prompt_deletes_nothing() {
        let (ts, _) = seeded().await;
        let mut prompt = ScriptedPrompt::new(true, ["n"]);
        let out = prune(&ts, &opts(None, Some(1), false, false), &mut prompt)
            .await
            .unwrap();
        assert!(out.ends_with("nothing deleted\n"), "{out}");
        assert_eq!(prompt.questions(), ["Delete 2 items?"]);
        assert_eq!(ts.store.list().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn confirming_deletes_the_selection() {
        let (ts, metas) = seeded().await;
        let out = prune(
            &ts,
            &opts(Some(36 * 3600), None, false, false),
            &mut ScriptedPrompt::new(true, ["y"]),
        )
        .await
        .unwrap();
        assert!(out.ends_with("deleted 2 items\n"), "{out}");
        assert_eq!(ts.store.list().await.unwrap(), remaining(&metas, &[2]));
    }

    #[tokio::test]
    async fn quiet_still_shows_the_list_before_asking() {
        let (ts, metas) = seeded().await;
        let mut prompt = ScriptedPrompt::new(true, ["y"]);
        let quiet = PruneOptions {
            quiet: true,
            ..opts(None, Some(1), false, false)
        };
        prune(&ts, &quiet, &mut prompt).await.unwrap();
        let shown = prompt.shown();
        assert!(shown.starts_with("2 items to delete:\n"), "{shown}");
        assert!(shown.contains(metas[0].id.as_str()), "{shown}");
        let (ts, _) = seeded().await;
        let mut prompt = ScriptedPrompt::new(false, Vec::<&str>::new());
        let quiet_yes = PruneOptions {
            quiet: true,
            ..opts(None, Some(1), false, true)
        };
        prune(&ts, &quiet_yes, &mut prompt).await.unwrap();
        assert_eq!(prompt.shown(), "", "nothing to confirm, nothing shown");
    }

    #[tokio::test]
    async fn refuses_without_yes_when_not_interactive() {
        let (ts, _) = seeded().await;
        let err = prune(
            &ts,
            &opts(None, Some(1), false, false),
            &mut ScriptedPrompt::new(false, Vec::<&str>::new()),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("--yes"), "{err}");
        assert_eq!(ts.store.list().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn yes_skips_the_prompt() {
        let (ts, metas) = seeded().await;
        let mut prompt = ScriptedPrompt::new(false, Vec::<&str>::new());
        let out = prune(&ts, &opts(None, Some(2), false, true), &mut prompt)
            .await
            .unwrap();
        assert!(out.ends_with("deleted 1 item\n"), "{out}");
        assert!(prompt.questions().is_empty());
        assert_eq!(ts.store.list().await.unwrap(), remaining(&metas, &[1, 2]));
    }

    #[tokio::test]
    async fn nothing_selected_says_so() {
        let (ts, _) = seeded().await;
        let out = prune(
            &ts,
            &opts(Some(30 * 86_400), None, false, false),
            &mut ScriptedPrompt::new(true, Vec::<&str>::new()),
        )
        .await
        .unwrap();
        assert_eq!(out, "nothing to prune\n");
    }

    #[tokio::test]
    async fn stale_staging_is_cleaned_except_in_dry_runs() {
        let (ts, _) = seeded().await;
        let dir = ts.dir.path().join("tmp/abandoned");
        std::fs::create_dir_all(&dir).unwrap();
        let old: std::time::SystemTime = (ts.clock.now() - TimeDelta::hours(2)).into();
        passalong_core::testing::set_modified(&dir, old).unwrap();
        let out = prune(
            &ts,
            &opts(Some(30 * 86_400), None, true, false),
            &mut ScriptedPrompt::new(true, Vec::<&str>::new()),
        )
        .await
        .unwrap();
        assert_eq!(out, "nothing to prune\n");
        assert!(dir.exists(), "dry run must not clean staging");
        let out = prune(
            &ts,
            &opts(Some(30 * 86_400), None, false, false),
            &mut ScriptedPrompt::new(true, Vec::<&str>::new()),
        )
        .await
        .unwrap();
        assert_eq!(out, "nothing to prune\nremoved 1 stale staging directory\n");
        assert!(!dir.exists());
        let _ = T;
    }
}

#[cfg(test)]
mod plain_tests {
    use super::*;
    use crate::commands::support::{T, TestStore, bytes};
    use crate::prompt::ScriptedPrompt;
    use passalong_core::crypto::{KDF_SALT_LEN, KdfParams, Words};
    use passalong_core::fs::LocalFs;
    use passalong_core::model::NewItem;

    fn keep(n: usize) -> PruneOptions {
        PruneOptions {
            older_than: None,
            keep: Some(n),
            dry_run: false,
            yes: true,
            quiet: false,
        }
    }

    async fn prune(fs: &LocalFs, options: &PruneOptions) -> anyhow::Result<String> {
        let mut out = Vec::new();
        let mut prompt = ScriptedPrompt::new(false, Vec::<&str>::new());
        let utc = FixedOffset::east_opt(0).unwrap();
        let admin = passalong_core::encryption::FsEncryptionAdmin::new(fs);
        run_plain(
            &admin,
            options,
            T.parse().unwrap(),
            &mut prompt,
            utc,
            &mut out,
        )
        .await?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[tokio::test]
    async fn plain_items_are_pruned_and_the_folder_removed_once_empty() {
        let ts = TestStore::new();
        for text in ["a", "b", "c"] {
            ts.store
                .put(NewItem::text("box"), bytes(text.as_bytes()))
                .await
                .unwrap();
            ts.clock.advance(1);
        }
        let fs = LocalFs::new(ts.dir.path());
        let kdf = KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [2; KDF_SALT_LEN],
        };
        encryption::fresh_start(
            &fs,
            &Words::parse("zoom zoom zoom zoom zoom zoom").unwrap(),
            kdf,
        )
        .await
        .unwrap();
        let out = prune(&fs, &keep(1)).await.unwrap();
        assert!(out.contains("deleted 2 items"), "{out}");
        assert!(ts.dir.path().join("plain").exists());
        let out = prune(&fs, &keep(0)).await.unwrap();
        assert!(out.contains("deleted 1 item"), "{out}");
        assert!(out.contains("removed plain/"), "{out}");
        assert!(!ts.dir.path().join("plain").exists());
        assert_eq!(
            prune(&fs, &keep(0)).await.unwrap(),
            "no unencrypted items remain\n"
        );
    }

    #[tokio::test]
    async fn leftovers_of_cut_short_uploads_go_with_prune_plain() {
        let ts = TestStore::new();
        let fs = LocalFs::new(ts.dir.path());
        let kdf = KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [2; KDF_SALT_LEN],
        };
        encryption::set_up(
            &fs,
            &Words::parse("zoom zoom zoom zoom zoom zoom").unwrap(),
            kdf,
        )
        .await
        .unwrap();
        std::fs::create_dir_all(ts.dir.path().join("tmp/cut-short")).unwrap();
        let dry = PruneOptions {
            dry_run: true,
            ..keep(0)
        };
        assert_eq!(
            prune(&fs, &dry).await.unwrap(),
            "would remove 1 unencrypted leftover of cut-short uploads\nno unencrypted items remain\n"
        );
        assert!(ts.dir.path().join("tmp").exists());
        assert_eq!(
            prune(&fs, &keep(0)).await.unwrap(),
            "removed 1 unencrypted leftover of cut-short uploads\nno unencrypted items remain\n"
        );
        assert!(!ts.dir.path().join("tmp").exists());
    }

    #[tokio::test]
    async fn a_plaintext_store_has_nothing_for_plain() {
        let ts = TestStore::new();
        let err = prune(&LocalFs::new(ts.dir.path()), &keep(0))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not encrypted"), "{err}");
    }
}
