//! `passalong load`: copy an item to a file, a directory, or the clipboard.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use passalong_core::clipboard::{Clipboard, ClipboardError};
use passalong_core::download::{self, check_integrity, write_verified};
use passalong_core::fs::BoxRead;
use passalong_core::model::{ContentHasher, ItemKind, ItemMeta, sanitise_file_name};
use passalong_core::store::Store;

use crate::resolve::Lookup;
use tokio::io::AsyncReadExt;

#[cfg(test)]
use passalong_core::download::PART_SUFFIX;

/// Opens the clipboard on demand, so only a load to the clipboard needs one.
pub type OpenClipboard<'a> = dyn FnMut() -> Result<Box<dyn Clipboard>, ClipboardError> + 'a;

/// Loads the item `lookup` identifies. Without `dest`, text goes to the
/// clipboard and files are downloaded into `download_dir`, created if
/// missing, under a numbered name if theirs is taken (the exact name with
/// `force`). With `dest`, the item is written to that file, or into that
/// directory under its own name. The written path is printed. Content is
/// checked against the item's SHA-256 before anything is replaced.
pub async fn run(
    store: &dyn Store,
    lookup: Lookup<'_, '_>,
    dest: Option<&Path>,
    force: bool,
    download_dir: &Path,
    open_clipboard: &mut OpenClipboard<'_>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let id = lookup.resolve(store).await?;
    let (meta, content) = store.get(&id).await?;
    let target = match dest {
        None if meta.kind == ItemKind::Text => {
            let bytes = read_verified(content, &meta).await?;
            let text =
                String::from_utf8(bytes).with_context(|| format!("item {id} is not UTF-8 text"))?;
            open_clipboard()?.write_text(&text)?;
            tracing::info!(id = %id, size = meta.size, "item copied to the clipboard");
            return Ok(());
        }
        None => download_target(download_dir, &meta, force).await?,
        Some(dest) => {
            let target = target_path(dest, &meta)?;
            if !force && tokio::fs::try_exists(&target).await.unwrap_or(false) {
                anyhow::bail!(
                    "{} already exists; use --force to overwrite it",
                    target.display()
                );
            }
            target
        }
    };
    write_verified(content, &meta, &target).await?;
    tracing::info!(id = %id, size = meta.size, path = %target.display(), "item written");
    writeln!(out, "{}", target.display())?;
    Ok(())
}

/// Where a download goes: the item's name in `dir`, or its first free
/// numbered variant unless `force`.
async fn download_target(dir: &Path, meta: &ItemMeta, force: bool) -> anyhow::Result<PathBuf> {
    let name = sanitise_file_name(meta.name.as_deref().unwrap_or_default())
        .context("give a destination for this item instead")?;
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("cannot create the download directory {}", dir.display()))?;
    Ok(if force {
        dir.join(name)
    } else {
        download::free_target(dir, &name).await?
    })
}

/// A directory destination gets the item's sanitised name, or `<id>.txt`
/// for text; any other destination is the target file itself.
fn target_path(dest: &Path, meta: &ItemMeta) -> anyhow::Result<PathBuf> {
    if !dest.is_dir() {
        return Ok(dest.to_path_buf());
    }
    let name = match meta.kind {
        ItemKind::Text => format!("{}.txt", meta.id),
        ItemKind::File => sanitise_file_name(meta.name.as_deref().unwrap_or_default())
            .context("give a file path as the destination instead")?,
    };
    Ok(dest.join(name))
}

async fn read_verified(mut content: BoxRead, meta: &ItemMeta) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    content
        .read_to_end(&mut bytes)
        .await
        .context("reading the item")?;
    let mut hasher = ContentHasher::new();
    hasher.update(&bytes);
    check_integrity(meta, hasher)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{TestStore, bytes};
    use passalong_core::clipboard::ClipboardError;
    use passalong_core::model::{ContentHasher, ItemMeta, NewItem};
    use passalong_core::testing::MockClipboard;
    use std::collections::HashMap;
    use tempfile::TempDir;

    struct Env {
        ts: TestStore,
        out_dir: TempDir,
        clip: MockClipboard,
    }

    impl Env {
        fn new() -> Self {
            Self {
                ts: TestStore::new(),
                out_dir: TempDir::new().unwrap(),
                clip: MockClipboard::new(),
            }
        }
        async fn put(&self, item: NewItem, data: &[u8]) -> ItemMeta {
            let meta = self.ts.store.put(item, bytes(data)).await.unwrap().meta;
            self.ts.clock.advance(1);
            meta
        }
        async fn load(&self, id: &str, dest: Option<&Path>, force: bool) -> anyhow::Result<String> {
            let clip = self.clip.clone();
            let mut open = move || -> Result<Box<dyn Clipboard>, ClipboardError> {
                Ok(Box::new(clip.clone()))
            };
            let mut out = Vec::new();
            let downloads = self.downloads();
            run(
                &self.ts.store,
                Lookup::plain(id),
                dest,
                force,
                &downloads,
                &mut open,
                &mut out,
            )
            .await?;
            Ok(String::from_utf8(out).unwrap())
        }
        /// The download directory; it does not exist until a download.
        fn downloads(&self) -> PathBuf {
            self.out_dir.path().join("Downloads")
        }
        fn out(&self, name: &str) -> PathBuf {
            self.out_dir.path().join(name)
        }
        fn leftovers(&self) -> Vec<String> {
            std::fs::read_dir(self.out_dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .filter(|n| n.ends_with(PART_SUFFIX))
                .collect()
        }
    }

    #[tokio::test]
    async fn text_without_destination_goes_to_the_clipboard() {
        let env = Env::new();
        let meta = env.put(NewItem::text("box"), b"hello there").await;
        let printed = env
            .load(&meta.id.content_key().as_str()[..6], None, false)
            .await
            .unwrap();
        assert_eq!(printed, "");
        assert_eq!(env.clip.writes(), ["hello there"]);
    }

    #[tokio::test]
    async fn files_without_a_destination_go_to_the_download_directory() {
        let env = Env::new();
        assert!(!env.downloads().exists());
        let meta = env.put(NewItem::file("report.pdf", "box"), b"%PDF").await;
        let printed = env.load(meta.id.as_str(), None, false).await.unwrap();
        let target = env.downloads().join("report.pdf");
        assert_eq!(printed, format!("{}\n", target.display()));
        assert_eq!(std::fs::read(&target).unwrap(), b"%PDF");
        assert!(env.clip.writes().is_empty());
    }

    #[tokio::test]
    async fn downloads_are_numbered_instead_of_overwritten_unless_forced() {
        let env = Env::new();
        let first = env.put(NewItem::file("report.pdf", "box"), b"one").await;
        let second = env.put(NewItem::file("report.pdf", "box"), b"two").await;
        env.load(first.id.as_str(), None, false).await.unwrap();
        let printed = env.load(second.id.as_str(), None, false).await.unwrap();
        let numbered = env.downloads().join("report (1).pdf");
        assert_eq!(printed, format!("{}\n", numbered.display()));
        assert_eq!(std::fs::read(&numbered).unwrap(), b"two");
        let printed = env.load(second.id.as_str(), None, true).await.unwrap();
        let plain = env.downloads().join("report.pdf");
        assert_eq!(printed, format!("{}\n", plain.display()));
        assert_eq!(std::fs::read(&plain).unwrap(), b"two");
    }

    #[tokio::test]
    async fn a_directory_destination_keeps_the_file_name() {
        let env = Env::new();
        let meta = env.put(NewItem::file("report.pdf", "box"), b"%PDF").await;
        let printed = env
            .load(meta.id.as_str(), Some(env.out_dir.path()), false)
            .await
            .unwrap();
        assert_eq!(printed, format!("{}\n", env.out("report.pdf").display()));
        assert_eq!(std::fs::read(env.out("report.pdf")).unwrap(), b"%PDF");
    }

    #[tokio::test]
    async fn text_into_a_directory_is_named_after_its_id() {
        let env = Env::new();
        let meta = env.put(NewItem::text("box"), b"note").await;
        env.load(meta.id.as_str(), Some(env.out_dir.path()), false)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(env.out(&format!("{}.txt", meta.id))).unwrap(),
            b"note"
        );
    }

    #[tokio::test]
    async fn a_file_destination_is_used_as_is() {
        let env = Env::new();
        let meta = env.put(NewItem::file("a.txt", "box"), b"body").await;
        let target = env.out("renamed.txt");
        env.load(meta.id.as_str(), Some(&target), false)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"body");
    }

    #[tokio::test]
    async fn existing_targets_need_force() {
        let env = Env::new();
        let meta = env.put(NewItem::file("a.txt", "box"), b"new").await;
        let target = env.out("a.txt");
        std::fs::write(&target, b"old").unwrap();
        let err = env
            .load(meta.id.as_str(), Some(&target), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("--force"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        env.load(meta.id.as_str(), Some(&target), true)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }

    #[tokio::test]
    async fn unknown_and_ambiguous_ids_are_errors() {
        let env = Env::new();
        let err = env
            .load("ffff", Some(env.out_dir.path()), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no item matches"), "{err}");
        // two contents whose keys share the first four digits
        let mut seen: HashMap<String, String> = HashMap::new();
        let (a, b) = (0..)
            .find_map(|i| {
                let text = format!("item {i}");
                let mut h = ContentHasher::new();
                h.update(text.as_bytes());
                let prefix = h.finalize().content_key().as_str()[..4].to_owned();
                seen.insert(prefix, text.clone()).map(|prev| (prev, text))
            })
            .unwrap();
        let meta_a = env.put(NewItem::text("box"), a.as_bytes()).await;
        let meta_b = env.put(NewItem::text("box"), b.as_bytes()).await;
        let err = env
            .load(&meta_a.id.content_key().as_str()[..4], None, false)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(meta_a.id.as_str()) && msg.contains(meta_b.id.as_str()),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn corrupted_content_fails_the_integrity_check_and_leaves_nothing() {
        let env = Env::new();
        let meta = env.put(NewItem::file("a.txt", "box"), b"original").await;
        let content = env
            .ts
            .dir
            .path()
            .join("items")
            .join(meta.id.as_str())
            .join("content");
        std::fs::write(&content, b"tampered").unwrap();
        let err = env
            .load(meta.id.as_str(), Some(env.out_dir.path()), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("integrity check failed"), "{err}");
        assert!(!env.out("a.txt").exists());
        assert!(env.leftovers().is_empty(), "{:?}", env.leftovers());

        let text = env.put(NewItem::text("box"), b"clip text").await;
        let content = env
            .ts
            .dir
            .path()
            .join("items")
            .join(text.id.as_str())
            .join("content");
        std::fs::write(&content, b"clip TEXT").unwrap();
        let err = env.load(text.id.as_str(), None, false).await.unwrap_err();
        assert!(err.to_string().contains("integrity check failed"), "{err}");
        assert!(env.clip.writes().is_empty());
    }

    #[tokio::test]
    async fn stored_names_cannot_escape_the_destination_directory() {
        let env = Env::new();
        let evil = env
            .put(NewItem::file("../../etc/passwd", "box"), b"x")
            .await;
        env.load(evil.id.as_str(), Some(env.out_dir.path()), false)
            .await
            .unwrap();
        assert_eq!(std::fs::read(env.out("passwd")).unwrap(), b"x");
        let unusable = env.put(NewItem::file("..", "box"), b"y").await;
        let err = env
            .load(unusable.id.as_str(), Some(env.out_dir.path()), false)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("file name"), "{err:#}");
    }

    #[tokio::test]
    async fn clipboard_and_filesystem_failures_are_reported() {
        let env = Env::new();
        let meta = env.put(NewItem::text("box"), b"text").await;
        let mut open = || -> Result<Box<dyn Clipboard>, ClipboardError> {
            Err(ClipboardError::Unavailable("no display".into()))
        };
        let err = run(
            &env.ts.store,
            Lookup::plain(meta.id.as_str()),
            None,
            false,
            &env.downloads(),
            &mut open,
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("no display"), "{err:#}");
        env.clip.fail_writes(true);
        assert!(env.load(meta.id.as_str(), None, false).await.is_err());
        let nowhere = env.out("missing-dir/file.txt");
        let err = env
            .load(meta.id.as_str(), Some(&nowhere), false)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("missing-dir"), "{err:#}");
    }
}
