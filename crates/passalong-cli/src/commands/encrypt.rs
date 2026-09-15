//! `passalong encrypt`: encrypt a store, change its words, or, with
//! `--join`, give this device the key of an encrypted store.
//!
//! Every path needs a terminal: new words are shown there and typed back,
//! and a store's words are typed there without being echoed. Words never
//! reach the results or the logs.

use std::io::Write;
use std::path::Path;

use passalong_core::crypto::{CryptoError, KdfParams, KeyId, Words};
use passalong_core::encryption::{
    self, EncryptionError, GitCheck, StoreState, check_key_location, load_key_file, save_key_file,
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
            "The {items} {} stored now stay unencrypted, in plain/ on the server, until you remove them with `passalong prune --plain`; new items are encrypted.\n",
            items_word(items)
        ));
    }
    prompt.show(&warning)?;
    if !prompt.confirm("Encrypt this store?")? {
        writeln!(out, "nothing was changed")?;
        return Ok(());
    }
    let words = confirm_new_words(keys, prompt)?;
    let kdf = (keys.new_kdf)()?;
    let key = if items == 0 {
        encryption::set_up(fs, &words, kdf).await?
    } else {
        encryption::fresh_start(fs, &words, kdf).await?
    };
    save_key_file(keys.key_file, &key, keys.git)?;
    tracing::info!(key = %key.key_id().short(), "store encrypted");
    writeln!(out, "encrypted the store: key {}", key.key_id().short())?;
    if items > 0 {
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
        async fn run(
            &self,
            join: bool,
            new_words: fn() -> Result<Words, CryptoError>,
            prompt: &mut ScriptedPrompt,
        ) -> anyhow::Result<String> {
            let mut out = Vec::new();
            run(
                &EncryptArgs { join },
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
        let mut prompt = ScriptedPrompt::new(true, ["yes", W1]);
        let out = rig.run(false, w1, &mut prompt).await.unwrap();
        assert!(
            prompt
                .shown()
                .contains("The 2 items stored now stay unencrypted")
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
}
