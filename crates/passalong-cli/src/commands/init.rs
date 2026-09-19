//! `passalong init`: write a config file for an SSH server.
//!
//! The server's host key is never trusted blindly: it is either given with
//! `--host-key`, or fetched and then confirmed by the user or matched
//! against `--fingerprint`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use async_trait::async_trait;
use passalong_core::config::{self, Config, EnvProvider, InitAnswers};
use passalong_core::crypto::{CryptoError, KdfParams, Words};
use passalong_core::encryption::{EncryptionAdmin, GitCheck, StoreState};
use passalong_core::store::BackendRegistry;
use passalong_ssh::DiscoveredKey;
use passalong_ssh::error::SshError;
use passalong_ssh::host_key::PinnedHostKey;

use crate::cli::InitArgs;
use crate::commands::encrypt::{self, Keys};
use crate::prompt::Prompt;

const DEFAULT_PORT: &str = "22";
const DEFAULT_USER: &str = "passalong";
const DEFAULT_IDENTITY: &str = "~/.ssh/id_ed25519";
const DEFAULT_REMOTE_PATH: &str = "/srv/passalong";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetches a server's host key; the network version is [`NetworkHostKeys`].
#[async_trait]
pub trait HostKeySource: Send + Sync {
    /// The key `host:port` presents, untrusted.
    async fn fetch(&self, host: &str, port: u16) -> Result<DiscoveredKey, SshError>;
}

/// Tests a written config; the real version is [`StoreCheck`].
#[async_trait]
pub trait ConnectionCheck: Send + Sync {
    /// Connects with `config` and returns what changes the store's
    /// encryption.
    async fn open(&self, config: &Config) -> anyhow::Result<Box<dyn EncryptionAdmin>>;
}

/// [`HostKeySource`] over SSH, with a 10-second timeout.
pub struct NetworkHostKeys;

#[async_trait]
impl HostKeySource for NetworkHostKeys {
    async fn fetch(&self, host: &str, port: u16) -> Result<DiscoveredKey, SshError> {
        passalong_ssh::fetch_host_key(host, port, DISCOVERY_TIMEOUT).await
    }
}

/// [`ConnectionCheck`] that opens the store for changing its encryption.
pub struct StoreCheck(pub BackendRegistry);

#[async_trait]
impl ConnectionCheck for StoreCheck {
    async fn open(&self, config: &Config) -> anyhow::Result<Box<dyn EncryptionAdmin>> {
        Ok(self.0.open_admin(config).await?)
    }
}

/// What `init` talks to besides the file system.
pub struct InitDeps<'a> {
    /// Environment, for `~` expansion and the default device name.
    pub env: &'a dyn EnvProvider,
    /// Questions to the user.
    pub prompt: &'a mut dyn Prompt,
    /// Host-key discovery.
    pub keys: &'a dyn HostKeySource,
    /// Connection test after writing.
    pub check: &'a dyn ConnectionCheck,
    /// `--quiet`: results are hidden, so what a question is about is shown
    /// with the question instead.
    pub quiet: bool,
    /// Asks git whether the key file would be committed.
    pub git: &'a dyn GitCheck,
    /// New words for a store encrypted during `init`.
    pub new_words: fn() -> Result<Words, CryptoError>,
    /// Settings for wrapping a new key.
    pub new_kdf: fn() -> Result<KdfParams, CryptoError>,
}

/// Collects the settings, establishes the host key, writes `target`, and
/// tests the connection unless `--no-test`.
pub async fn run(
    args: &InitArgs,
    target: &Path,
    deps: InitDeps<'_>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if target.exists() && !args.force {
        anyhow::bail!(
            "{} already exists; use --force to replace it",
            target.display()
        );
    }
    if args.yes && args.host_key.is_none() && args.fingerprint.is_none() {
        anyhow::bail!(
            "--yes needs --host-key or --fingerprint, so the server's key is never trusted blindly"
        );
    }
    let interactive = !args.yes && deps.prompt.is_interactive();
    if !interactive && !args.yes {
        anyhow::bail!(
            "init needs a terminal to ask questions; without one, pass --yes with --host and --host-key or --fingerprint"
        );
    }
    let InitDeps {
        env,
        prompt,
        keys,
        check,
        quiet,
        git,
        new_words,
        new_kdf,
    } = deps;
    let mut value =
        |question: &str, flag: Option<String>, default: Option<&str>| -> anyhow::Result<String> {
            Ok(match flag {
                Some(value) => value,
                None if interactive => prompt.ask(question, default)?,
                None => default.unwrap_or_default().to_owned(),
            })
        };

    let host = value("Server host name or address", args.host.clone(), None)?;
    if host.trim().is_empty() {
        anyhow::bail!("a server host is required (--host)");
    }
    let port_text = value(
        "SSH port",
        args.port.map(|port| port.to_string()),
        Some(DEFAULT_PORT),
    )?;
    let port: u16 = port_text
        .parse()
        .ok()
        .filter(|port| *port != 0)
        .with_context(|| format!("invalid port `{port_text}`: expected 1 to 65535"))?;
    let user = value("User on the server", args.user.clone(), Some(DEFAULT_USER))?;
    let identity_file = value(
        "Private key for logging in",
        args.identity_file.clone(),
        Some(DEFAULT_IDENTITY),
    )?;
    let remote_path = value(
        "Storage directory on the server",
        args.remote_path.clone(),
        Some(DEFAULT_REMOTE_PATH),
    )?;
    let hostname = env.hostname();
    let device_name = value(
        "Name for this device",
        args.device_name.clone(),
        Some(&hostname),
    )?;

    let host_key = match &args.host_key {
        Some(line) => {
            PinnedHostKey::parse(line)?;
            line.trim().to_owned()
        }
        None => {
            let found = keys.fetch(&host, port).await?;
            let asks = args.fingerprint.is_none();
            let mut about = format!(
                "The server at {host}:{port} presented this {} host key:\n  {}\n",
                found.algorithm, found.fingerprint
            );
            if asks {
                about.push_str(
                    "Compare it with the server's own key, for example by running there:\n  ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub\n",
                );
            }
            if quiet && asks {
                prompt.show(&about)?;
            } else {
                out.write_all(about.as_bytes())?;
            }
            match &args.fingerprint {
                Some(expected) if expected.trim() == found.fingerprint => {}
                Some(expected) => anyhow::bail!(
                    "host key fingerprint mismatch: expected {}, but the server presented {}; nothing was written",
                    expected.trim(),
                    found.fingerprint
                ),
                None => {
                    if !prompt.confirm("Does the fingerprint match?")? {
                        anyhow::bail!("host key not confirmed; nothing was written");
                    }
                }
            }
            found.openssh_line
        }
    };

    let text = config::render(&InitAnswers {
        device_name,
        host,
        port,
        user: user.clone(),
        host_key,
        identity_file,
        remote_path,
    });
    // Validate exactly what will be written, including `~` expansion.
    let parsed = config::parse(&text, target, env)?;
    write_config(target, &text)?;
    writeln!(out, "wrote {}", target.display())?;
    tracing::info!(path = %target.display(), "config written");

    let identity = parsed
        .server
        .ssh
        .as_ref()
        .map(|ssh| ssh.identity_file.clone())
        .unwrap_or_default();
    let public = PathBuf::from(format!("{}.pub", identity.display()));
    if public.exists() {
        writeln!(
            out,
            "Add this device's public key to ~{user}/.ssh/authorized_keys on the server:"
        )?;
        writeln!(out, "  {}", public.display())?;
    } else {
        writeln!(
            out,
            "No public key at {}; create a key pair with: ssh-keygen -t ed25519",
            public.display()
        )?;
    }
    if args.no_test {
        writeln!(
            out,
            "To encrypt the store, or to join an encrypted one, run `passalong encrypt` or `passalong encrypt --join`."
        )?;
        return Ok(());
    }
    let admin = check
        .open(&parsed)
        .await
        .context("wrote the config, but connecting with it failed")?;
    let state = admin
        .inspect()
        .await
        .context("wrote the config, but reading the store failed")?;
    let keys = parsed.client.key_file.as_deref().map(|key_file| Keys {
        key_file,
        git,
        new_words,
        new_kdf,
    });
    match state {
        StoreState::Plain { items } => {
            let word = if items == 1 { "item" } else { "items" };
            writeln!(out, "connected: {items} {word} on the server")?;
            match (&keys, items) {
                (Some(keys), 0) if interactive => {
                    encrypt::set_up(admin.as_ref(), 0, keys, prompt, out).await?;
                }
                (_, 0) => writeln!(out, "To encrypt this store, run `passalong encrypt`.")?,
                _ => writeln!(
                    out,
                    "To encrypt the {items} stored {word}, run `passalong encrypt`."
                )?,
            }
        }
        StoreState::Encrypted { .. } => {
            writeln!(out, "connected: the store is encrypted")?;
            match &keys {
                Some(keys) if interactive => {
                    encrypt::join(admin.as_ref(), keys, prompt, out).await?
                }
                _ => writeln!(out, "To join it, run `passalong encrypt --join`.")?,
            }
        }
        _ => writeln!(
            out,
            "connected: the store's encryption is being changed or needs recovery; run `passalong check`"
        )?,
    }
    Ok(())
}

/// Writes through a temporary file and a rename, so a failure never leaves
/// a half-written config.
fn write_config(target: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut temp = target.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, text).with_context(|| format!("cannot write {}", temp.display()))?;
    std::fs::rename(&temp, target).with_context(|| format!("cannot write {}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::ScriptedPrompt;
    use passalong_core::crypto::KDF_SALT_LEN;
    use passalong_core::encryption::{self, FsEncryptionAdmin};
    use passalong_core::encryption::{SystemGit, load_key_file};
    use passalong_core::fs::LocalFs;
    use passalong_core::model::NewItem;
    use passalong_core::random::StdRandom;
    use passalong_core::store::{FsStore, Store};
    use passalong_core::testing::{ManualClock, MapEnv};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF2M9DqIpW9GMebpvjNg+bobwAbQKRBqPVMatyvyI4gq";
    const KEY_A_FP: &str = "SHA256:5Si4lWKPwa0+I2wCQf3eOtcF8jWo30BWybHoXLTxABo";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMKy9BQGg0B6NYvYwyrJzGCOHCXKQBj7E/5jvWJSEDCi";

    struct FakeKeys {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl HostKeySource for FakeKeys {
        async fn fetch(&self, _host: &str, _port: u16) -> Result<DiscoveredKey, SshError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(DiscoveredKey {
                openssh_line: KEY_A.into(),
                fingerprint: KEY_A_FP.into(),
                algorithm: "ssh-ed25519".into(),
            })
        }
    }

    /// A store in a temporary folder, seeded with `items` items when opened.
    struct FakeCheck {
        fail: bool,
        seen: Mutex<Option<Config>>,
        store: TempDir,
        items: usize,
    }

    #[async_trait]
    impl ConnectionCheck for FakeCheck {
        async fn open(&self, config: &Config) -> anyhow::Result<Box<dyn EncryptionAdmin>> {
            *self.seen.lock().unwrap() = Some(config.clone());
            if self.fail {
                anyhow::bail!("connection refused")
            }
            let fs = LocalFs::new(self.store.path());
            if self.items > 0 {
                let store = FsStore::new(
                    fs.clone(),
                    Arc::new(ManualClock::at("2026-09-12T09:53:11Z")),
                    Box::new(StdRandom::new()),
                );
                for i in store.list_ids().await?.len()..self.items {
                    let text = format!("item {i}").into_bytes();
                    store
                        .put(NewItem::text("seed"), Box::new(std::io::Cursor::new(text)))
                        .await?;
                }
            }
            Ok(Box::new(FsEncryptionAdmin::new(fs)))
        }
    }

    const W1: &str = "abacus abdomen abdominal abide abiding ability";

    fn w1() -> Result<Words, CryptoError> {
        Words::parse(W1)
    }

    fn quick() -> Result<KdfParams, CryptoError> {
        Ok(KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [8; KDF_SALT_LEN],
        })
    }

    struct Rig {
        dir: TempDir,
        env: MapEnv,
        keys: FakeKeys,
        check: FakeCheck,
        git: SystemGit,
    }

    impl Rig {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let home = dir.path().join("home");
            std::fs::create_dir_all(&home).unwrap();
            let env = MapEnv::new()
                .with("HOME", home.to_str().unwrap())
                .with_hostname("box");
            Self {
                dir,
                env,
                keys: FakeKeys {
                    calls: AtomicUsize::new(0),
                },
                check: FakeCheck {
                    fail: false,
                    seen: Mutex::new(None),
                    store: TempDir::new().unwrap(),
                    items: 3,
                },
                git: SystemGit::new(),
            }
        }
        fn target(&self) -> PathBuf {
            self.dir.path().join("cfg/config.toml")
        }
        fn home(&self) -> PathBuf {
            self.dir.path().join("home")
        }
        async fn run(
            &self,
            args: &InitArgs,
            prompt: &mut ScriptedPrompt,
        ) -> anyhow::Result<String> {
            let mut out = Vec::new();
            let deps = InitDeps {
                env: &self.env,
                prompt,
                keys: &self.keys,
                check: &self.check,
                quiet: false,
                git: &self.git,
                new_words: w1,
                new_kdf: quick,
            };
            run(args, &self.target(), deps, &mut out).await?;
            Ok(String::from_utf8(out).unwrap())
        }
        fn written(&self) -> Config {
            config::load(&self.target(), &self.env).unwrap()
        }
    }

    fn scripted(args: &InitArgs) -> InitArgs {
        args.clone()
    }

    fn yes_with(host: &str) -> InitArgs {
        InitArgs {
            host: Some(host.into()),
            yes: true,
            ..InitArgs::default()
        }
    }

    #[tokio::test]
    async fn interactive_answers_with_defaults_write_a_working_config() {
        let rig = Rig::new();
        let mut prompt = ScriptedPrompt::new(true, ["nas.local", "", "", "", "", "", "yes"]);
        let out = rig.run(&InitArgs::default(), &mut prompt).await.unwrap();
        let cfg = rig.written();
        let ssh = cfg.server.ssh.clone().unwrap();
        assert_eq!(
            (ssh.host.as_str(), ssh.port, ssh.user.as_str()),
            ("nas.local", 22, "passalong")
        );
        assert_eq!(ssh.identity_file, rig.home().join(".ssh/id_ed25519"));
        assert_eq!(ssh.remote_path, "/srv/passalong");
        assert_eq!(ssh.host_key, KEY_A);
        assert_eq!(cfg.client.device_name, "box");
        assert_eq!(
            prompt.questions().len(),
            7,
            "six values and one fingerprint confirmation"
        );
        assert!(
            out.contains(KEY_A_FP) && out.contains(&format!("wrote {}", rig.target().display())),
            "{out}"
        );
        assert!(out.contains("connected: 3 items on the server\n"), "{out}");
        assert_eq!(rig.check.seen.lock().unwrap().as_ref(), Some(&cfg));
    }

    #[tokio::test]
    async fn quiet_still_shows_the_fingerprint_before_asking() {
        let rig = Rig::new();
        let mut prompt = ScriptedPrompt::new(true, ["nas.local", "", "", "", "", "", "yes"]);
        let mut out = Vec::new();
        let deps = InitDeps {
            env: &rig.env,
            prompt: &mut prompt,
            keys: &rig.keys,
            check: &rig.check,
            quiet: true,
            git: &rig.git,
            new_words: w1,
            new_kdf: quick,
        };
        run(&InitArgs::default(), &rig.target(), deps, &mut out)
            .await
            .unwrap();
        let shown = prompt.shown();
        assert!(shown.contains(KEY_A_FP), "{shown}");
        assert!(shown.contains("ssh-keygen -lf"), "{shown}");
        assert!(
            !String::from_utf8(out).unwrap().contains(KEY_A_FP),
            "the fingerprint goes to the prompt, not the results"
        );
    }

    #[tokio::test]
    async fn declining_the_fingerprint_writes_nothing() {
        let rig = Rig::new();
        let mut prompt = ScriptedPrompt::new(true, ["nas.local", "", "", "", "", "", "no"]);
        let err = rig
            .run(&InitArgs::default(), &mut prompt)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not confirmed"), "{err}");
        assert!(!rig.target().exists());
    }

    #[tokio::test]
    async fn a_fingerprint_mismatch_is_refused() {
        let rig = Rig::new();
        let args = InitArgs {
            fingerprint: Some("SHA256:somethingelse".into()),
            ..yes_with("nas")
        };
        let err = rig
            .run(&args, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("mismatch") && err.to_string().contains(KEY_A_FP),
            "{err}"
        );
        assert!(!rig.target().exists());
    }

    #[tokio::test]
    async fn yes_alone_never_trusts_a_key() {
        let rig = Rig::new();
        let err = rig
            .run(
                &yes_with("nas"),
                &mut ScriptedPrompt::new(true, Vec::<&str>::new()),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("--host-key or --fingerprint"),
            "{err}"
        );
        assert!(!rig.target().exists());
        assert_eq!(rig.keys.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn yes_with_a_matching_fingerprint_asks_nothing() {
        let rig = Rig::new();
        let mut prompt = ScriptedPrompt::new(false, Vec::<&str>::new());
        let args = InitArgs {
            fingerprint: Some(KEY_A_FP.into()),
            ..yes_with("nas")
        };
        rig.run(&scripted(&args), &mut prompt).await.unwrap();
        assert!(prompt.questions().is_empty());
        assert_eq!(rig.written().server.ssh.unwrap().host_key, KEY_A);
    }

    #[tokio::test]
    async fn a_given_host_key_skips_discovery() {
        let rig = Rig::new();
        let args = InitArgs {
            host_key: Some(KEY_B.into()),
            ..yes_with("nas")
        };
        rig.run(&args, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap();
        assert_eq!(rig.keys.calls.load(Ordering::SeqCst), 0);
        assert_eq!(rig.written().server.ssh.unwrap().host_key, KEY_B);
    }

    #[tokio::test]
    async fn invalid_input_is_refused_before_writing() {
        let rig = Rig::new();
        let bad_key = InitArgs {
            host_key: Some("not a key".into()),
            ..yes_with("nas")
        };
        assert!(
            rig.run(
                &bad_key,
                &mut ScriptedPrompt::new(false, Vec::<&str>::new())
            )
            .await
            .is_err()
        );
        let no_host = InitArgs {
            host_key: Some(KEY_B.into()),
            yes: true,
            ..InitArgs::default()
        };
        let err = rig
            .run(
                &no_host,
                &mut ScriptedPrompt::new(false, Vec::<&str>::new()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("--host"), "{err}");
        let mut bad_port = ScriptedPrompt::new(true, ["nas", "http"]);
        let err = rig
            .run(&InitArgs::default(), &mut bad_port)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("port"), "{err}");
        assert!(!rig.target().exists());
    }

    #[tokio::test]
    async fn an_existing_config_needs_force() {
        let rig = Rig::new();
        std::fs::create_dir_all(rig.target().parent().unwrap()).unwrap();
        std::fs::write(rig.target(), "keep me").unwrap();
        let args = InitArgs {
            host_key: Some(KEY_B.into()),
            ..yes_with("nas")
        };
        let err = rig
            .run(&args, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("--force"), "{err}");
        assert_eq!(std::fs::read_to_string(rig.target()).unwrap(), "keep me");
        rig.run(
            &InitArgs {
                force: true,
                ..args
            },
            &mut ScriptedPrompt::new(false, Vec::<&str>::new()),
        )
        .await
        .unwrap();
        assert_eq!(rig.written().server.ssh.unwrap().host, "nas");
    }

    #[tokio::test]
    async fn without_a_terminal_it_needs_yes() {
        let rig = Rig::new();
        let err = rig
            .run(
                &InitArgs {
                    host: Some("nas".into()),
                    ..InitArgs::default()
                },
                &mut ScriptedPrompt::new(false, Vec::<&str>::new()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("terminal"), "{err}");
    }

    #[tokio::test]
    async fn a_failed_connection_test_is_reported_after_writing() {
        let mut rig = Rig::new();
        rig.check.fail = true;
        let args = InitArgs {
            host_key: Some(KEY_B.into()),
            ..yes_with("nas")
        };
        let err = rig
            .run(&args, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("wrote the config")
                && format!("{err:#}").contains("connection refused"),
            "{err:#}"
        );
        assert!(rig.target().exists());
        let skipped = Rig::new();
        let out = skipped
            .run(
                &InitArgs {
                    no_test: true,
                    ..args
                },
                &mut ScriptedPrompt::new(false, Vec::<&str>::new()),
            )
            .await
            .unwrap();
        assert!(skipped.check.seen.lock().unwrap().is_none());
        assert!(!out.contains("connected"), "{out}");
    }

    fn key_file(rig: &Rig) -> PathBuf {
        rig.home().join(".config/passalong/store.key")
    }

    #[tokio::test]
    async fn an_empty_store_can_be_encrypted_during_init() {
        let mut rig = Rig::new();
        rig.check.items = 0;
        let mut prompt =
            ScriptedPrompt::new(true, ["nas.local", "", "", "", "", "", "yes", "yes", W1]);
        let out = rig.run(&InitArgs::default(), &mut prompt).await.unwrap();
        assert!(out.contains("connected: 0 items on the server\n"), "{out}");
        let key = load_key_file(&key_file(&rig), &rig.git).unwrap().unwrap();
        assert!(
            out.contains(&format!(
                "encrypted the store: key {}",
                key.key_id().short()
            )),
            "{out}"
        );
        assert!(
            rig.check
                .store
                .path()
                .join("encryption/header.json")
                .is_file()
        );
        assert!(!out.contains("abacus"));
    }

    #[tokio::test]
    async fn an_encrypted_store_is_joined_during_init() {
        let mut rig = Rig::new();
        rig.check.items = 0;
        let key = encryption::set_up(
            &LocalFs::new(rig.check.store.path()),
            &w1().unwrap(),
            quick().unwrap(),
        )
        .await
        .unwrap();
        let mut prompt = ScriptedPrompt::new(true, ["nas.local", "", "", "", "", "", "yes", W1]);
        let out = rig.run(&InitArgs::default(), &mut prompt).await.unwrap();
        assert!(out.contains("connected: the store is encrypted\n"), "{out}");
        assert!(
            out.contains(&format!(
                "joined the encrypted store: key {}",
                key.key_id().short()
            )),
            "{out}"
        );
        let saved = load_key_file(&key_file(&rig), &rig.git).unwrap().unwrap();
        assert_eq!(saved.key_id(), key.key_id());
    }

    #[tokio::test]
    async fn yes_and_stores_with_items_only_say_how_to_encrypt_or_join() {
        let mut rig = Rig::new();
        rig.check.items = 0;
        let args = InitArgs {
            host_key: Some(KEY_B.into()),
            ..yes_with("nas")
        };
        let out = rig
            .run(&args, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap();
        assert!(
            out.contains("To encrypt this store, run `passalong encrypt`."),
            "{out}"
        );
        assert!(!key_file(&rig).exists());
        encryption::set_up(
            &LocalFs::new(rig.check.store.path()),
            &w1().unwrap(),
            quick().unwrap(),
        )
        .await
        .unwrap();
        let again = InitArgs {
            force: true,
            ..args.clone()
        };
        let out = rig
            .run(&again, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap();
        assert!(
            out.contains("To join it, run `passalong encrypt --join`."),
            "{out}"
        );

        let busy = Rig::new();
        let mut prompt = ScriptedPrompt::new(true, ["nas.local", "", "", "", "", "", "yes"]);
        let out = busy.run(&InitArgs::default(), &mut prompt).await.unwrap();
        assert!(
            out.contains("To encrypt the 3 stored items, run `passalong encrypt`."),
            "{out}"
        );
    }

    #[tokio::test]
    async fn it_points_at_the_public_key_to_install() {
        let rig = Rig::new();
        let args = InitArgs {
            host_key: Some(KEY_B.into()),
            no_test: true,
            ..yes_with("nas")
        };
        let out = rig
            .run(&args, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap();
        assert!(out.contains("ssh-keygen -t ed25519"), "{out}");
        std::fs::create_dir_all(rig.home().join(".ssh")).unwrap();
        std::fs::write(
            rig.home().join(".ssh/id_ed25519.pub"),
            "ssh-ed25519 AAAA me",
        )
        .unwrap();
        let out = rig
            .run(
                &InitArgs {
                    force: true,
                    ..args
                },
                &mut ScriptedPrompt::new(false, Vec::<&str>::new()),
            )
            .await
            .unwrap();
        assert!(
            out.contains(&rig.home().join(".ssh/id_ed25519.pub").display().to_string()),
            "{out}"
        );
    }
}
