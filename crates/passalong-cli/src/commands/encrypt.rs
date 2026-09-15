//! `passalong encrypt`: encrypt a store, change its words, or, with
//! `--join`, give this device the key of an encrypted store.
//!
//! Every path needs a terminal: new words are shown there and typed back,
//! and a store's words are typed there without being echoed. Words never
//! reach the results or the logs.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use passalong_core::clock::SystemClock;

use passalong_core::crypto::{CryptoError, KdfParams, KeyId, Words};
use passalong_core::encryption::{
    self, EncryptionError, GitCheck, RewriteKind, StoreState, check_key_location, load_key_file,
    save_key_file,
};
use passalong_core::fs::RemoteFs;

use crate::cli::EncryptArgs;
use crate::prompt::Prompt;

/// How many tries typing words gets.
const ATTEMPTS: usize = 3;

/// The warning `encrypt` shows before it changes anything.
const WARNING: &str = "Encrypting keeps what the server stores unreadable without this store's six words.\n\
Every device then needs passalong 0.2.0 or later, and joins once with `passalong encrypt --join`; older versions stop working with this store.\n\
If the words and every device's key file are lost, the items cannot be recovered.\n";

/// Where this device's key goes, and how new words and key-derivation
/// settings are made.
pub struct Keys<'a> {
    /// The key file, `client.key_file`.
    pub key_file: &'a Path,
    /// Asks git whether the key file would be committed.
    pub git: &'a dyn GitCheck,
    /// New words: [`Words::generate`] outside tests.
    pub new_words: fn() -> Result<Words, CryptoError>,
    /// Settings for wrapping the key: [`KdfParams::generate`] outside tests.
    pub new_kdf: fn() -> Result<KdfParams, CryptoError>,
}

fn items_word(n: usize) -> &'static str {
    if n == 1 { "item" } else { "items" }
}

/// Encrypts a plaintext store, changes an encrypted store's words, or joins
/// an encrypted store with `--join`.
///
/// # Errors
///
/// Without a terminal, for words typed wrong three times, and when the
/// store refuses the change.
pub async fn run(
    args: &EncryptArgs,
    fs: &dyn RemoteFs,
    keys: &Keys<'_>,
    prompt: &mut dyn Prompt,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if !prompt.is_interactive() {
        anyhow::bail!(
            "encrypt needs a terminal: new words are shown there, and words are typed there without being echoed"
        );
    }
    if args.join {
        return join(fs, keys, prompt, out).await;
    }
    if args.recover {
        return recover(fs, keys, prompt, out).await;
    }
    if args.rotate {
        return rotate(fs, keys, prompt, out).await;
    }
    match encryption::inspect(fs).await? {
        StoreState::Plain { items } => set_up(fs, items, keys, prompt, out).await,
        StoreState::Encrypted { key_id, .. } => change_words(fs, key_id, keys, prompt, out).await,
        StoreState::Rewriting { started } => Err(EncryptionError::Rewriting { started }.into()),
        StoreState::Broken => Err(EncryptionError::HeaderMissing.into()),
        _ => anyhow::bail!("this store's state is not known to this version of passalong"),
    }
}

/// Encrypts a plaintext store holding `items` items, after a warning and a
/// yes: shows new words, has them typed back, and saves this device's key.
/// Items already stored stay unencrypted in `plain/` (a fresh start).
///
/// # Errors
///
/// When the key file's place is refused, the words are not confirmed, or
/// the store refuses.
pub async fn set_up(
    fs: &dyn RemoteFs,
    items: usize,
    keys: &Keys<'_>,
    prompt: &mut dyn Prompt,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    check_key_location(keys.key_file, keys.git)?;
    let mut warning = WARNING.to_owned();
    if items > 0 {
        warning.push_str(&format!(
            "The {items} {} stored now can be migrated, which re-encrypts each one by downloading and uploading it again, or left unencrypted in plain/ on the server until you remove them with `passalong prune --plain` (a fresh start).\n",
            items_word(items)
        ));
    }
    prompt.show(&warning)?;
    if !prompt.confirm("Encrypt this store?")? {
        writeln!(out, "nothing was changed")?;
        return Ok(());
    }
    let migrate = items > 0 && ask_migrate(prompt, items)?;
    let words = confirm_new_words(keys, prompt)?;
    let kdf = (keys.new_kdf)()?;
    let key = if migrate {
        encryption::migrate(fs, &words, kdf, Arc::new(SystemClock))
            .await
            .context(STOPPED)?
    } else if items == 0 {
        encryption::set_up(fs, &words, kdf).await?
    } else {
        encryption::fresh_start(fs, &words, kdf).await?
    };
    save_key_file(keys.key_file, &key, keys.git)?;
    tracing::info!(key = %key.key_id().short(), "store encrypted");
    writeln!(out, "encrypted the store: key {}", key.key_id().short())?;
    if migrate {
        writeln!(out, "migrated {items} {}", items_word(items))?;
    } else if items > 0 {
        writeln!(
            out,
            "{items} unencrypted {} remain in plain/; remove them with: passalong prune --plain",
            items_word(items)
        )?;
    }
    Ok(())
}

/// Gives this device the key of an encrypted store: asks for its words and
/// saves the key they unlock.
///
/// # Errors
///
/// When the store is not encrypted, the key file's place is refused, or
/// the words are wrong.
pub async fn join(
    fs: &dyn RemoteFs,
    keys: &Keys<'_>,
    prompt: &mut dyn Prompt,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    match encryption::inspect(fs).await? {
        StoreState::Encrypted { .. } => {}
        StoreState::Plain { .. } => return Err(EncryptionError::NotEncrypted.into()),
        StoreState::Rewriting { started } => {
            return Err(EncryptionError::Rewriting { started }.into());
        }
        StoreState::Broken => return Err(EncryptionError::HeaderMissing.into()),
        _ => anyhow::bail!("this store's state is not known to this version of passalong"),
    }
    check_key_location(keys.key_file, keys.git)?;
    let words = ask_words(prompt, "The store's six words")?;
    let key = encryption::join(fs, &words).await?;
    save_key_file(keys.key_file, &key, keys.git)?;
    tracing::info!(key = %key.key_id().short(), "joined an encrypted store");
    writeln!(
        out,
        "joined the encrypted store: key {}",
        key.key_id().short()
    )?;
    Ok(())
}

/// What a failed re-encryption says to do next.
const STOPPED: &str =
    "the re-encryption stopped; run `passalong encrypt --recover` to finish or undo it";

/// Asks whether to migrate a store's items or start fresh.
fn ask_migrate(prompt: &mut dyn Prompt, items: usize) -> anyhow::Result<bool> {
    let question = format!(
        "Migrate the {items} {} or start fresh? [migrate/fresh]",
        items_word(items)
    );
    for _ in 0..ATTEMPTS {
        match prompt
            .ask(&question, Some("migrate"))?
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "m" | "migrate" => return Ok(true),
            "f" | "fresh" => return Ok(false),
            _ => prompt.show("Answer migrate or fresh.\n")?,
        }
    }
    anyhow::bail!("no choice between migrate and fresh; nothing was changed")
}

/// Replaces an encrypted store's data key and words, re-encrypting every
/// item, with this device's key as the old one.
async fn rotate(
    fs: &dyn RemoteFs,
    keys: &Keys<'_>,
    prompt: &mut dyn Prompt,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let store_key = match encryption::inspect(fs).await? {
        StoreState::Encrypted { key_id, .. } => key_id,
        StoreState::Plain { .. } => return Err(EncryptionError::NotEncrypted.into()),
        StoreState::Rewriting { started } => {
            return Err(EncryptionError::Rewriting { started }.into());
        }
        StoreState::Broken => return Err(EncryptionError::HeaderMissing.into()),
        _ => anyhow::bail!("this store's state is not known to this version of passalong"),
    };
    let old = match load_key_file(keys.key_file, keys.git)? {
        Some(key) if key.key_id() == store_key => key,
        Some(key) => {
            return Err(EncryptionError::KeyMismatch {
                device: key.key_id().short(),
                store: store_key.short(),
            }
            .into());
        }
        None => return Err(EncryptionError::NoKey.into()),
    };
    prompt.show(
        "Rotating replaces the store's key and words and re-encrypts every item. Every other device then stops working with this store until it joins again with the new words.\n",
    )?;
    if !prompt.confirm("Rotate the store's key?")? {
        writeln!(out, "nothing was changed")?;
        return Ok(());
    }
    let words = confirm_new_words(keys, prompt)?;
    let key = encryption::rotate(fs, &old, &words, (keys.new_kdf)()?, Arc::new(SystemClock))
        .await
        .context(STOPPED)?;
    save_key_file(keys.key_file, &key, keys.git)?;
    tracing::info!(key = %key.key_id().short(), "store key rotated");
    writeln!(
        out,
        "rotated the store's key: {} replaces {}; every other device must run `passalong encrypt --join` with the new words",
        key.key_id().short(),
        old.key_id().short()
    )?;
    Ok(())
}

/// Finishes or undoes an interrupted re-encryption.
async fn recover(
    fs: &dyn RemoteFs,
    keys: &Keys<'_>,
    prompt: &mut dyn Prompt,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let Some(plan) = encryption::read_plan(fs).await? else {
        return match encryption::undo(fs).await {
            Ok(()) => {
                writeln!(out, "released a re-encryption lock that had not started")?;
                Ok(())
            }
            Err(_) => {
                writeln!(out, "no re-encryption to recover")?;
                Ok(())
            }
        };
    };
    let what = match plan.kind {
        RewriteKind::Migrate => "encrypting",
        RewriteKind::Rotate => "moving to a new key",
    };
    prompt.show(&format!(
        "A re-encryption started at {}: {what} the store's {} {}.\n",
        plan.started_at,
        plan.items,
        items_word(plan.items)
    ))?;
    let answer = prompt.ask("Finish it, or undo it? [finish/undo]", Some("finish"))?;
    match answer.trim().to_ascii_lowercase().as_str() {
        "f" | "finish" => {
            let new = ask_words(prompt, "The new six words shown when it started")?;
            let device = load_key_file(keys.key_file, keys.git)?;
            let old_words = if plan.kind == RewriteKind::Rotate
                && device.as_ref().map(|key| key.key_id().to_string()) != plan.from_key
            {
                Some(ask_words(prompt, "The store's old six words")?)
            } else {
                None
            };
            let key = encryption::finish(
                fs,
                &new,
                device.as_ref(),
                old_words.as_ref(),
                Arc::new(SystemClock),
            )
            .await?;
            save_key_file(keys.key_file, &key, keys.git)?;
            writeln!(out, "finished: the store's key is {}", key.key_id().short())?;
        }
        "u" | "undo" => {
            encryption::undo(fs).await?;
            writeln!(out, "undone: the store is as it was before")?;
        }
        _ => anyhow::bail!("answer finish or undo; nothing was changed"),
    }
    Ok(())
}

/// Changes an encrypted store's words, keeping its key.
async fn change_words(
    fs: &dyn RemoteFs,
    store_key: KeyId,
    keys: &Keys<'_>,
    prompt: &mut dyn Prompt,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    match load_key_file(keys.key_file, keys.git)? {
        Some(key) if key.key_id() == store_key => {}
        Some(key) => {
            return Err(EncryptionError::KeyMismatch {
                device: key.key_id().short(),
                store: store_key.short(),
            }
            .into());
        }
        None => return Err(EncryptionError::NoKey.into()),
    }
    prompt.show(
        "This store is encrypted. New words keep its key: nothing is re-encrypted, and every device that joined keeps working.\n",
    )?;
    let current = ask_words(prompt, "The store's current six words")?;
    let new = confirm_new_words(keys, prompt)?;
    let key_id = encryption::change_words(fs, &current, &new, (keys.new_kdf)()?).await?;
    tracing::info!(key = %key_id.short(), "store words changed");
    writeln!(
        out,
        "changed the store's words; its key stays {}",
        key_id.short()
    )?;
    Ok(())
}

/// Asks for six words, without echoing them, until they parse.
fn ask_words(prompt: &mut dyn Prompt, question: &str) -> anyhow::Result<Words> {
    let mut attempt = 1;
    loop {
        let typed = prompt.ask_secret(question)?;
        match Words::parse(&typed) {
            Ok(words) => return Ok(words),
            Err(err) if attempt < ATTEMPTS => prompt.show(&format!("{err}; try again.\n"))?,
            Err(err) => return Err(err.into()),
        }
        attempt += 1;
    }
}

/// Shows new words, then has them typed back.
fn confirm_new_words(keys: &Keys<'_>, prompt: &mut dyn Prompt) -> anyhow::Result<Words> {
    let words = (keys.new_words)()?;
    prompt.show(&format!(
        "\nThe store's six words:\n\n    {}\n\nWrite them down or keep them in a password manager. Every device types them to join, and nothing else can recover the store.\n\n",
        words.as_str()
    ))?;
    for attempt in 1..=ATTEMPTS {
        let typed = prompt.ask_secret("Type the six words to confirm")?;
        if Words::parse(&typed).is_ok_and(|typed| typed == words) {
            return Ok(words);
        }
        if attempt < ATTEMPTS {
            prompt.show("Those are not the words shown; try again.\n")?;
        }
    }
    anyhow::bail!("the words were not confirmed; nothing was changed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{TestStore, bytes};
    use crate::prompt::ScriptedPrompt;
    use passalong_core::crypto::{DataKey, KDF_SALT_LEN};
    use passalong_core::encryption::SystemGit;
    use passalong_core::fs::LocalFs;
    use passalong_core::model::NewItem;
    use passalong_core::store::Store;
    use tempfile::TempDir;

    const W1: &str = "abacus abdomen abdominal abide abiding ability";
    const W2: &str = "zoom zoom zoom zoom zoom zoom";

    fn w1() -> Result<Words, CryptoError> {
        Words::parse(W1)
    }

    fn w2() -> Result<Words, CryptoError> {
        Words::parse(W2)
    }

    fn quick() -> Result<KdfParams, CryptoError> {
        Ok(KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [9; KDF_SALT_LEN],
        })
    }

    struct Rig {
        store: TestStore,
        _home: TempDir,
        key_file: std::path::PathBuf,
        git: SystemGit,
    }

    impl Rig {
        fn new() -> Self {
            let home = TempDir::new().unwrap();
            let key_file = home.path().join("keys/store.key");
            Self {
                store: TestStore::new(),
                _home: home,
                key_file,
                git: SystemGit::new(),
            }
        }
        fn fs(&self) -> LocalFs {
            LocalFs::new(self.store.dir.path())
        }
        fn key_file(&self) -> std::path::PathBuf {
            self.key_file.clone()
        }
        fn keys(&self, new_words: fn() -> Result<Words, CryptoError>) -> Keys<'_> {
            Keys {
                key_file: &self.key_file,
                git: &self.git,
                new_words,
                new_kdf: quick,
            }
        }
        async fn run_with(
            &self,
            args: EncryptArgs,
            new_words: fn() -> Result<Words, CryptoError>,
            prompt: &mut ScriptedPrompt,
        ) -> anyhow::Result<String> {
            let mut out = Vec::new();
            run(&args, &self.fs(), &self.keys(new_words), prompt, &mut out).await?;
            Ok(String::from_utf8(out).unwrap())
        }
        async fn run(
            &self,
            join: bool,
            new_words: fn() -> Result<Words, CryptoError>,
            prompt: &mut ScriptedPrompt,
        ) -> anyhow::Result<String> {
            let mut out = Vec::new();
            run(
                &EncryptArgs {
                    join,
                    ..EncryptArgs::default()
                },
                &self.fs(),
                &self.keys(new_words),
                prompt,
                &mut out,
            )
            .await?;
            Ok(String::from_utf8(out).unwrap())
        }
        fn saved_key(&self) -> Option<DataKey> {
            load_key_file(&self.key_file(), &self.git).unwrap()
        }
    }

    #[tokio::test]
    async fn an_empty_store_is_encrypted_after_the_words_are_typed_back() {
        let rig = Rig::new();
        let mut prompt = ScriptedPrompt::new(true, ["yes", W1]);
        let out = rig.run(false, w1, &mut prompt).await.unwrap();
        let key = rig.saved_key().unwrap();
        assert!(
            out.contains(&format!(
                "encrypted the store: key {}",
                key.key_id().short()
            )),
            "{out}"
        );
        assert!(!out.contains("abacus"), "words never reach the results");
        assert!(prompt.shown().contains(W1), "the words are shown once");
        assert!(prompt.shown().contains("0.2.0"), "{}", prompt.shown());
        assert_eq!(
            prompt.questions(),
            [
                "Encrypt this store?",
                "Type the six words to confirm (hidden)"
            ]
        );
        assert_eq!(
            encryption::inspect(&rig.fs()).await.unwrap(),
            StoreState::Encrypted {
                key_id: key.key_id(),
                plain_left: 0
            }
        );
    }

    #[tokio::test]
    async fn declining_or_mistyping_the_words_changes_nothing() {
        let rig = Rig::new();
        let out = rig
            .run(false, w1, &mut ScriptedPrompt::new(true, ["no"]))
            .await
            .unwrap();
        assert_eq!(out, "nothing was changed\n");
        let mut prompt = ScriptedPrompt::new(true, ["yes", W2, "abacus", "not words at all"]);
        let err = rig.run(false, w1, &mut prompt).await.unwrap_err();
        assert!(err.to_string().contains("not confirmed"), "{err}");
        assert!(rig.saved_key().is_none());
        assert_eq!(
            encryption::inspect(&rig.fs()).await.unwrap(),
            StoreState::Plain { items: 0 }
        );
    }

    #[tokio::test]
    async fn without_a_terminal_nothing_is_asked_or_changed() {
        let rig = Rig::new();
        let err = rig
            .run(false, w1, &mut ScriptedPrompt::new(false, ["yes", W1]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("needs a terminal"), "{err}");
        assert!(rig.saved_key().is_none());
    }

    #[tokio::test]
    async fn a_store_with_items_gets_a_fresh_start() {
        let rig = Rig::new();
        for text in ["one", "two"] {
            rig.store
                .store
                .put(NewItem::text("box"), bytes(text.as_bytes()))
                .await
                .unwrap();
            rig.store.clock.advance(1);
        }
        let mut prompt = ScriptedPrompt::new(true, ["yes", "fresh", W1]);
        let out = rig.run(false, w1, &mut prompt).await.unwrap();
        assert!(
            prompt
                .shown()
                .contains("The 2 items stored now can be migrated")
        );
        assert!(
            out.contains("2 unencrypted items remain in plain/"),
            "{out}"
        );
        let key = rig.saved_key().unwrap();
        assert_eq!(
            encryption::inspect(&rig.fs()).await.unwrap(),
            StoreState::Encrypted {
                key_id: key.key_id(),
                plain_left: 2
            }
        );
    }

    #[tokio::test]
    async fn new_words_ask_for_the_current_ones_and_keep_the_key() {
        let rig = Rig::new();
        let key = encryption::set_up(&rig.fs(), &w1().unwrap(), quick().unwrap())
            .await
            .unwrap();
        save_key_file(&rig.key_file(), &key, &rig.git).unwrap();

        let mut prompt = ScriptedPrompt::new(true, [W1, W2]);
        let out = rig.run(false, w2, &mut prompt).await.unwrap();
        assert_eq!(
            out,
            format!(
                "changed the store's words; its key stays {}\n",
                key.key_id().short()
            )
        );
        assert_eq!(
            prompt.questions(),
            [
                "The store's current six words (hidden)",
                "Type the six words to confirm (hidden)"
            ]
        );
        let joined = encryption::join(&rig.fs(), &w2().unwrap()).await.unwrap();
        assert_eq!(joined.key_id(), key.key_id());

        let err = rig
            .run(false, w1, &mut ScriptedPrompt::new(true, [W1, W1]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("do not unlock"), "{err}");
    }

    #[tokio::test]
    async fn changing_words_needs_this_device_to_hold_the_key() {
        let rig = Rig::new();
        encryption::set_up(&rig.fs(), &w1().unwrap(), quick().unwrap())
            .await
            .unwrap();
        let err = rig
            .run(false, w2, &mut ScriptedPrompt::new(true, [W1, W2]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("encrypt --join"), "{err}");
        save_key_file(&rig.key_file(), &DataKey::generate().unwrap(), &rig.git).unwrap();
        let err = rig
            .run(false, w2, &mut ScriptedPrompt::new(true, [W1, W2]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is not the store's key"), "{err}");
    }

    #[tokio::test]
    async fn joining_saves_the_key_the_words_unlock() {
        let rig = Rig::new();
        let err = rig
            .run(true, w1, &mut ScriptedPrompt::new(true, [W1]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not encrypted"), "{err}");

        let key = encryption::set_up(&rig.fs(), &w1().unwrap(), quick().unwrap())
            .await
            .unwrap();
        let err = rig
            .run(true, w1, &mut ScriptedPrompt::new(true, [W2]))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("do not unlock"), "{err}");
        assert!(rig.saved_key().is_none());

        let mut prompt = ScriptedPrompt::new(true, ["abacus", W1]);
        let out = rig.run(true, w1, &mut prompt).await.unwrap();
        assert!(
            prompt.shown().contains("expected 6 words"),
            "{}",
            prompt.shown()
        );
        assert_eq!(
            out,
            format!("joined the encrypted store: key {}\n", key.key_id().short())
        );
        assert_eq!(rig.saved_key().unwrap().key_id(), key.key_id());
    }

    #[tokio::test]
    async fn a_store_being_rewritten_or_broken_is_refused() {
        let rig = Rig::new();
        std::fs::write(rig.store.dir.path().join("items"), "stop").unwrap();
        let err = rig
            .run(
                false,
                w1,
                &mut ScriptedPrompt::new(true, Vec::<&str>::new()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("encrypt --recover"), "{err}");
        std::fs::create_dir(rig.store.dir.path().join(".rewrite")).unwrap();
        for join in [false, true] {
            let err = rig
                .run(join, w1, &mut ScriptedPrompt::new(true, Vec::<&str>::new()))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("re-encrypted"), "{err}");
        }
    }

    async fn seed(rig: &Rig, texts: &[&str]) {
        for text in texts {
            rig.store
                .store
                .put(NewItem::text("box"), bytes(text.as_bytes()))
                .await
                .unwrap();
            rig.store.clock.advance(1);
        }
    }

    fn args(rotate: bool, recover: bool) -> EncryptArgs {
        EncryptArgs {
            rotate,
            recover,
            ..EncryptArgs::default()
        }
    }

    #[tokio::test]
    async fn a_store_with_items_is_migrated_by_default() {
        let rig = Rig::new();
        seed(&rig, &["one", "two"]).await;
        let mut prompt = ScriptedPrompt::new(true, ["yes", "", W1]);
        let out = rig.run(false, w1, &mut prompt).await.unwrap();
        assert!(out.contains("migrated 2 items"), "{out}");
        let key = rig.saved_key().unwrap();
        assert_eq!(
            encryption::inspect(&rig.fs()).await.unwrap(),
            StoreState::Encrypted {
                key_id: key.key_id(),
                plain_left: 0
            }
        );
        let store = encryption::open_with_key(
            rig.fs(),
            Some(key),
            rig.store.clock.clone(),
            Box::new(passalong_core::random::StdRandom::new()),
        )
        .await
        .unwrap();
        assert_eq!(store.list().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn rotating_replaces_this_device_s_key() {
        let rig = Rig::new();
        let err = rig
            .run_with(
                args(true, false),
                w2,
                &mut ScriptedPrompt::new(true, ["yes", W2]),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not encrypted"), "{err}");
        let old = encryption::set_up(&rig.fs(), &w1().unwrap(), quick().unwrap())
            .await
            .unwrap();
        let err = rig
            .run_with(
                args(true, false),
                w2,
                &mut ScriptedPrompt::new(true, ["yes", W2]),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("encrypt --join"), "{err}");
        save_key_file(&rig.key_file(), &old, &rig.git).unwrap();
        let out = rig
            .run_with(
                args(true, false),
                w2,
                &mut ScriptedPrompt::new(true, ["no"]),
            )
            .await
            .unwrap();
        assert_eq!(out, "nothing was changed\n");
        let mut prompt = ScriptedPrompt::new(true, ["yes", W2]);
        let out = rig
            .run_with(args(true, false), w2, &mut prompt)
            .await
            .unwrap();
        let new = rig.saved_key().unwrap();
        assert_ne!(new.key_id(), old.key_id());
        assert!(out.contains("encrypt --join"), "{out}");
        assert_eq!(
            encryption::join(&rig.fs(), &w2().unwrap())
                .await
                .unwrap()
                .key_id(),
            new.key_id()
        );
    }

    /// A migration of one item cut before its new header was put in place.
    async fn cut_migration(rig: &Rig) {
        seed(rig, &["kept"]).await;
        let fs = passalong_core::testing::FaultyFs::new(rig.fs());
        fs.fail_nth(passalong_core::testing::FsOp::Rename, 3);
        assert!(
            encryption::migrate(
                &fs,
                &w1().unwrap(),
                quick().unwrap(),
                rig.store.clock.clone()
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn an_interrupted_migration_is_finished_or_undone() {
        let rig = Rig::new();
        let out = rig
            .run_with(
                args(false, true),
                w1,
                &mut ScriptedPrompt::new(true, Vec::<&str>::new()),
            )
            .await
            .unwrap();
        assert_eq!(out, "no re-encryption to recover\n");
        cut_migration(&rig).await;
        let err = rig
            .run(
                false,
                w1,
                &mut ScriptedPrompt::new(true, Vec::<&str>::new()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("encrypt --recover"), "{err}");
        let mut prompt = ScriptedPrompt::new(true, ["", W1]);
        let out = rig
            .run_with(args(false, true), w1, &mut prompt)
            .await
            .unwrap();
        assert!(
            prompt.shown().contains("encrypting the store's 1 item"),
            "{}",
            prompt.shown()
        );
        let key = rig.saved_key().unwrap();
        assert_eq!(
            out,
            format!("finished: the store's key is {}\n", key.key_id().short())
        );
        assert!(matches!(
            encryption::inspect(&rig.fs()).await.unwrap(),
            StoreState::Encrypted { .. }
        ));

        let other = Rig::new();
        cut_migration(&other).await;
        let out = other
            .run_with(
                args(false, true),
                w1,
                &mut ScriptedPrompt::new(true, ["undo"]),
            )
            .await
            .unwrap();
        assert_eq!(out, "undone: the store is as it was before\n");
        assert_eq!(
            encryption::inspect(&other.fs()).await.unwrap(),
            StoreState::Plain { items: 1 }
        );
    }
}
