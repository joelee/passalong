//! `passalong init`: write a config file for an SSH server or a
//! passalong-server.
//!
//! The server's host key is never trusted blindly: it is either given with
//! `--host-key`, or fetched and then confirmed by the user or matched
//! against `--fingerprint`. A passalong-server's certificate is likewise
//! trusted only through the operating system or a pin the user confirmed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use async_trait::async_trait;
use chrono::Utc;
use passalong_core::api_key::{ApiKey, load_api_key, save_api_key};
use passalong_core::config::{
    self, API_KEY_FILE_NAME, Config, EnvProvider, HttpsConfig, HttpsInitAnswers, InitAnswers,
    TlsPin,
};
use passalong_core::crypto::{CryptoError, KdfParams, Words};
use passalong_core::encryption::{
    EncryptionAdmin, EncryptionError, GitCheck, StoreState, load_key_file,
};
use passalong_core::store::BackendRegistry;
use passalong_https::api::{EncryptionState, Viewer, Workspace};
use passalong_https::tls::Presented;
use passalong_ssh::DiscoveredKey;
use passalong_ssh::error::SshError;
use passalong_ssh::host_key::PinnedHostKey;

use crate::cli::{Backend, InitArgs};
use crate::commands::check::key_summary;
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

/// Reaches a passalong-server; the network version is [`NetworkServer`].
#[async_trait]
pub trait ServerSource: Send + Sync {
    /// The certificate the server at `url` presents, untrusted, found
    /// without sending a request.
    async fn presented(&self, url: &str) -> anyhow::Result<Presented>;

    /// What the server says about `key` and its workspace.
    async fn inspect(
        &self,
        https: &HttpsConfig,
        key: ApiKey,
    ) -> anyhow::Result<(Viewer, Workspace)>;
}

/// [`ServerSource`] over the network.
pub struct NetworkServer;

#[async_trait]
impl ServerSource for NetworkServer {
    async fn presented(&self, url: &str) -> anyhow::Result<Presented> {
        Ok(passalong_https::tls::presented(url).await?)
    }

    async fn inspect(
        &self,
        https: &HttpsConfig,
        key: ApiKey,
    ) -> anyhow::Result<(Viewer, Workspace)> {
        let client = passalong_https::Client::new(https, key)?;
        Ok((client.viewer().await?, client.workspace().await?))
    }
}

/// How many tries typing a pin or an API key gets.
const ATTEMPTS: usize = 3;

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
    /// A passalong-server's certificate and answers.
    pub server: &'a dyn ServerSource,
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
    let interactive = !args.yes && deps.prompt.is_interactive();
    if !interactive && !args.yes {
        anyhow::bail!(
            "init needs a terminal to ask questions; without one, pass --yes with --host and --host-key or --fingerprint, or with --url"
        );
    }
    let ssh_given = args.host.is_some()
        || args.port.is_some()
        || args.user.is_some()
        || args.identity_file.is_some()
        || args.remote_path.is_some()
        || args.host_key.is_some()
        || args.fingerprint.is_some();
    let https_given = args.url.is_some() || args.tls_pin.is_some() || args.api_key_file.is_some();
    let backend = match args.backend {
        Some(backend) => backend,
        None if https_given => Backend::Https,
        None if ssh_given || !interactive => Backend::Ssh,
        None => loop {
            match deps
                .prompt
                .ask(
                    "Server kind: ssh, or https for a passalong-server",
                    Some("ssh"),
                )?
                .trim()
            {
                "ssh" => break Backend::Ssh,
                "https" => break Backend::Https,
                other => deps
                    .prompt
                    .show(&format!("`{other}` is neither ssh nor https.\n"))?,
            }
        },
    };
    match backend {
        Backend::Ssh if https_given => {
            anyhow::bail!("--url, --tls-pin, and --api-key-file are for --backend https")
        }
        Backend::Https if ssh_given => anyhow::bail!(
            "--host, --port, --user, --identity-file, --remote-path, --host-key, and --fingerprint are for --backend ssh"
        ),
        Backend::Https => return run_https(args, target, deps, interactive, out).await,
        Backend::Ssh => {}
    }
    if args.yes && args.host_key.is_none() && args.fingerprint.is_none() {
        anyhow::bail!(
            "--yes needs --host-key or --fingerprint, so the server's key is never trusted blindly"
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
        ..
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
    let keys = parsed.client.key_file.as_deref().map(|key_file| Keys {
        key_file,
        git,
        new_words,
        new_kdf,
    });
    offer_encryption(&parsed, check, keys, interactive, prompt, out).await
}

/// Opens the store the written config names and offers what fits: to
/// encrypt an empty store, to join an encrypted one, or the command to run.
async fn offer_encryption(
    parsed: &Config,
    check: &dyn ConnectionCheck,
    keys: Option<Keys<'_>>,
    interactive: bool,
    prompt: &mut dyn Prompt,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let admin = check
        .open(parsed)
        .await
        .context("wrote the config, but connecting with it failed")?;
    let state = admin
        .inspect()
        .await
        .context("wrote the config, but reading the store failed")?;
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

/// Shows `text` before a question, where questions appear when `--quiet`
/// hides the results, and with the results otherwise.
fn tell(
    text: &str,
    asks: bool,
    quiet: bool,
    prompt: &mut dyn Prompt,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if quiet && asks {
        prompt.show(text)?;
    } else {
        out.write_all(text.as_bytes())?;
    }
    Ok(())
}

/// `init` for a passalong-server: the URL, the certificate, the API key,
/// what the server says, and then the config.
async fn run_https(
    args: &InitArgs,
    target: &Path,
    deps: InitDeps<'_>,
    interactive: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let InitDeps {
        env,
        prompt,
        server,
        check,
        quiet,
        git,
        new_words,
        new_kdf,
        ..
    } = deps;
    let url = match &args.url {
        Some(url) => url.clone(),
        None if interactive => prompt.ask("Server URL, such as https://box.example:8443", None)?,
        None => String::new(),
    };
    if url.trim().is_empty() {
        anyhow::bail!("a server URL is required (--url)");
    }
    let hostname = env.hostname();
    let device_name = match &args.device_name {
        Some(name) => name.clone(),
        None if interactive => prompt.ask("Name for this device", Some(&hostname))?,
        None => hostname,
    };
    let api_key_file = match &args.api_key_file {
        Some(path) => PathBuf::from(path),
        None => std::path::absolute(
            target
                .parent()
                .unwrap_or(Path::new("."))
                .join(API_KEY_FILE_NAME),
        )
        .with_context(|| format!("cannot place the API key beside {}", target.display()))?,
    };
    // Validate the URL and the key file's place before any connection.
    let draft = HttpsInitAnswers {
        device_name,
        url: url.trim().to_owned(),
        tls_pin: None,
        api_key_file,
    };
    let checked = config::parse(&config::render_https(&draft), target, env)?;
    let https = checked
        .server
        .https
        .clone()
        .context("the rendered config has no server.https section")?;

    let presented = server.presented(&https.url).await?;
    let pin = presented.pin;
    let about = format!(
        "The server at {} presented a certificate whose public key has the pin\n  {pin}\n",
        https.url
    );
    let tls_pin = match &args.tls_pin {
        Some(given) => {
            let given = TlsPin::parse(given).map_err(|err| anyhow::anyhow!("--tls-pin: {err}"))?;
            tell(&about, false, quiet, prompt, out)?;
            if given != pin {
                anyhow::bail!(
                    "TLS pin mismatch: expected {given}, but the server presented {pin}; nothing was written"
                );
            }
            Some(given)
        }
        None if presented.trusted => {
            let about = format!("{about}The operating system trusts its certificate.\n");
            tell(&about, interactive, quiet, prompt, out)?;
            if !interactive
                || prompt.confirm("Trust it through the operating system, without a pin?")?
            {
                None
            } else {
                Some(confirm_pin(pin, prompt)?)
            }
        }
        None if !interactive => anyhow::bail!(
            "the operating system does not trust the server's certificate; pass --tls-pin with the pin `passalong-server tls fingerprint` prints on the server; nothing was written"
        ),
        None => {
            let about = format!(
                "{about}The operating system does not trust it, so it is pinned. Run this on the server and paste what it prints:\n  passalong-server tls fingerprint\n"
            );
            tell(&about, true, quiet, prompt, out)?;
            Some(confirm_pin(pin, prompt)?)
        }
    };
    let mut https = https;
    https.tls_pin = tls_pin;

    let saved = load_api_key(&https.api_key_file, git).map_err(EncryptionError::from)?;
    let (api_key, typed) = match saved {
        Some(key) => {
            writeln!(out, "using the API key in {}", https.api_key_file.display())?;
            (key, false)
        }
        None if interactive => (ask_api_key(prompt)?, true),
        None => anyhow::bail!(
            "no API key in {}: put the key the server's operator gave you there, readable by you only, or run init in a terminal; nothing was written",
            https.api_key_file.display()
        ),
    };

    let answers = HttpsInitAnswers {
        tls_pin: https.tls_pin,
        api_key_file: https.api_key_file.clone(),
        ..draft
    };
    let text = config::render_https(&answers);
    let parsed = config::parse(&text, target, env)?;
    if !args.no_test {
        let (viewer, workspace) = server
            .inspect(&https, api_key.clone())
            .await
            .context("cannot use the server; nothing was written")?;
        let (_, key) = key_summary(&viewer.key, Utc::now());
        writeln!(
            out,
            "connected to passalong-server {}: API key {key}",
            viewer.server.version
        )?;
        let device_key = match &parsed.client.key_file {
            Some(path) => load_key_file(path, git).map_err(EncryptionError::from)?,
            None => None,
        };
        // A server could claim "not encrypted" to have a new device send
        // plaintext; only the user can tell whether that is expected.
        if workspace.encryption.state == EncryptionState::Plaintext && device_key.is_none() {
            let n = workspace.item_count;
            let about = format!(
                "The server says workspace {} is not encrypted ({n} {}). If other devices use it encrypted, stop here: the server is not telling the truth.\n",
                workspace.name,
                if n == 1 { "item" } else { "items" }
            );
            tell(&about, interactive, quiet, prompt, out)?;
            if interactive && !prompt.confirm("Is it meant to be unencrypted for now?")? {
                anyhow::bail!(
                    "the server's claim was not confirmed; nothing was written. Ask the server's operator"
                );
            }
        }
    }

    if typed {
        save_api_key(&https.api_key_file, &api_key, git).map_err(EncryptionError::from)?;
        writeln!(out, "wrote {}", https.api_key_file.display())?;
    }
    write_config(target, &text)?;
    writeln!(out, "wrote {}", target.display())?;
    tracing::info!(path = %target.display(), "config written");
    if args.no_test {
        writeln!(
            out,
            "To encrypt the store, or to join an encrypted one, run `passalong encrypt` or `passalong encrypt --join`."
        )?;
        return Ok(());
    }
    let keys = parsed.client.key_file.as_deref().map(|key_file| Keys {
        key_file,
        git,
        new_words,
        new_kdf,
    });
    offer_encryption(&parsed, check, keys, interactive, prompt, out).await
}

/// Asks for the pin until it is `presented`'s, three times at most.
fn confirm_pin(presented: TlsPin, prompt: &mut dyn Prompt) -> anyhow::Result<TlsPin> {
    for _ in 0..ATTEMPTS {
        let typed = prompt.ask("Pin", None)?;
        match TlsPin::parse(typed.trim()) {
            Ok(pin) if pin == presented => return Ok(pin),
            Ok(_) => prompt.show("That is not the pin the server presented.\n")?,
            Err(err) => prompt.show(&format!("{err}\n"))?,
        }
    }
    anyhow::bail!("the server's pin was not confirmed; nothing was written")
}

/// Asks for the API key, without echo, until it has the right shape,
/// three times at most.
fn ask_api_key(prompt: &mut dyn Prompt) -> anyhow::Result<ApiKey> {
    for _ in 0..ATTEMPTS {
        let typed = prompt.ask_secret("API key from the server's operator (pal_...)")?;
        match ApiKey::parse(&typed) {
            Ok(key) => return Ok(key),
            Err(err) => prompt.show(&format!("{err}\n"))?,
        }
    }
    anyhow::bail!("no usable API key was typed; nothing was written")
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

    /// A self-signed passalong-server, or one the system trusts, that
    /// answers with `workspace`.
    struct FakeServer {
        trusted: bool,
        workspace: serde_json::Value,
        inspected: AtomicUsize,
    }

    const PIN: &str = "sha256/Zmh6rfhivXdsj8GLjp+OIAiXFIVu4jOzkCpZHQ1fKSU=";
    const OTHER_PIN: &str = "sha256/2oM6f3DD8sKJtGwCB8Ao+uGyGJ3O4xzETkqW4HfaEDk=";

    fn api_key() -> String {
        format!("pal_{}_{}", "k1d".repeat(4), "0".repeat(64))
    }

    #[async_trait]
    impl ServerSource for FakeServer {
        async fn presented(&self, _url: &str) -> anyhow::Result<Presented> {
            Ok(Presented {
                pin: TlsPin::parse(PIN).unwrap(),
                trusted: self.trusted,
            })
        }

        async fn inspect(
            &self,
            _https: &HttpsConfig,
            key: ApiKey,
        ) -> anyhow::Result<(Viewer, Workspace)> {
            self.inspected.fetch_add(1, Ordering::SeqCst);
            let viewer = serde_json::json!({
                "key": {"id": key.id(), "label": "laptop", "role": "readWrite", "expiresAt": "2099-01-02T00:00:00Z"},
                "server": {"version": "0.1.0", "apiVersion": 1},
            });
            Ok((
                serde_json::from_value(viewer)?,
                serde_json::from_value(self.workspace.clone())?,
            ))
        }
    }

    fn workspace(state: &str, items: u64) -> serde_json::Value {
        serde_json::json!({
            "name": "home",
            "quotaBytes": "1000",
            "usedBytes": "0",
            "itemCount": items,
            "encryption": {"state": state},
        })
    }

    struct Rig {
        dir: TempDir,
        env: MapEnv,
        keys: FakeKeys,
        server: FakeServer,
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
                server: FakeServer {
                    trusted: false,
                    workspace: workspace("plaintext", 3),
                    inspected: AtomicUsize::new(0),
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
                server: &self.server,
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
        let mut prompt = ScriptedPrompt::new(true, ["", "nas.local", "", "", "", "", "", "yes"]);
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
            8,
            "the kind of server, six values, and one fingerprint confirmation"
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
        let mut prompt = ScriptedPrompt::new(true, ["", "nas.local", "", "", "", "", "", "yes"]);
        let mut out = Vec::new();
        let deps = InitDeps {
            env: &rig.env,
            prompt: &mut prompt,
            keys: &rig.keys,
            server: &rig.server,
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
        let mut prompt = ScriptedPrompt::new(true, ["", "nas.local", "", "", "", "", "", "no"]);
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
        let mut bad_port = ScriptedPrompt::new(true, ["", "nas", "http"]);
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
        let mut prompt = ScriptedPrompt::new(
            true,
            ["", "nas.local", "", "", "", "", "", "yes", "yes", W1],
        );
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
        let mut prompt =
            ScriptedPrompt::new(true, ["", "nas.local", "", "", "", "", "", "yes", W1]);
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
        let mut prompt = ScriptedPrompt::new(true, ["", "nas.local", "", "", "", "", "", "yes"]);
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

    fn https_args() -> InitArgs {
        InitArgs {
            url: Some("https://box.example:8443/".into()),
            ..InitArgs::default()
        }
    }

    impl Rig {
        fn api_key_file(&self) -> PathBuf {
            self.dir.path().join("cfg/api.key")
        }
        fn put_api_key(&self) {
            save_api_key(
                &self.api_key_file(),
                &ApiKey::parse(&api_key()).unwrap(),
                &self.git,
            )
            .unwrap();
        }
    }

    #[tokio::test]
    async fn a_self_signed_server_is_pinned_once_its_pin_is_pasted() {
        let rig = Rig::new();
        let key = api_key();
        // The kind, the URL, the device, a wrong pin, the right one, the
        // key, and yes, the workspace is meant to be unencrypted.
        let mut prompt = ScriptedPrompt::new(
            true,
            [
                "https",
                "https://box.example:8443/",
                "",
                OTHER_PIN,
                PIN,
                &key,
                "yes",
            ],
        );
        let out = rig.run(&InitArgs::default(), &mut prompt).await.unwrap();
        assert!(
            prompt.shown().contains("not the pin the server presented"),
            "{}",
            prompt.shown()
        );
        let cfg = rig.written();
        assert_eq!(cfg.server.kind, "https");
        let https = cfg.server.https.clone().unwrap();
        assert_eq!(https.url, "https://box.example:8443");
        assert_eq!(https.tls_pin, Some(TlsPin::parse(PIN).unwrap()));
        assert_eq!(https.api_key_file, rig.api_key_file());
        let saved = load_api_key(&rig.api_key_file(), &rig.git)
            .unwrap()
            .unwrap();
        assert_eq!(saved.expose(), key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(rig.api_key_file())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(out.contains(PIN), "{out}");
        assert!(out.contains("passalong-server tls fingerprint"), "{out}");
        assert!(
            out.contains("API key laptop (read-write), expires 2099-01-02"),
            "{out}"
        );
        assert!(
            out.contains("says workspace home is not encrypted (3 items)"),
            "{out}"
        );
        assert!(out.contains("connected: 3 items on the server"), "{out}");
        assert!(!out.contains(&key), "the key is never shown");
        assert!(
            prompt.questions().iter().any(|q| q.ends_with("(hidden)")),
            "the key is typed without echo"
        );
    }

    #[tokio::test]
    async fn a_pin_typed_wrong_three_times_writes_nothing() {
        let rig = Rig::new();
        let mut prompt = ScriptedPrompt::new(true, ["", OTHER_PIN, "nonsense", OTHER_PIN]);
        let err = rig.run(&https_args(), &mut prompt).await.unwrap_err();
        assert!(err.to_string().contains("not confirmed"), "{err}");
        assert!(!rig.target().exists());
        assert!(!rig.api_key_file().exists());
    }

    #[tokio::test]
    async fn a_trusted_certificate_needs_no_pin() {
        let mut rig = Rig::new();
        rig.server.trusted = true;
        rig.put_api_key();
        let mut prompt = ScriptedPrompt::new(true, ["", "yes", "yes"]);
        let out = rig.run(&https_args(), &mut prompt).await.unwrap();
        assert_eq!(rig.written().server.https.unwrap().tls_pin, None);
        assert!(
            out.contains("The operating system trusts its certificate"),
            "{out}"
        );
        assert!(out.contains("using the API key in"), "{out}");

        // Declining the system's word pins the key instead.
        let rig2 = Rig::new();
        let mut trusted = rig2;
        trusted.server.trusted = true;
        trusted.put_api_key();
        let mut prompt = ScriptedPrompt::new(true, ["", "no", PIN, "yes"]);
        trusted.run(&https_args(), &mut prompt).await.unwrap();
        assert_eq!(
            trusted.written().server.https.unwrap().tls_pin,
            Some(TlsPin::parse(PIN).unwrap())
        );
    }

    #[tokio::test]
    async fn declining_the_server_s_plaintext_claim_writes_nothing() {
        let rig = Rig::new();
        let key = api_key();
        let mut prompt = ScriptedPrompt::new(true, ["", PIN, &key, "no"]);
        let err = rig.run(&https_args(), &mut prompt).await.unwrap_err();
        assert!(err.to_string().contains("not confirmed"), "{err}");
        assert!(!rig.target().exists());
        assert!(
            !rig.api_key_file().exists(),
            "the key is saved only with the config"
        );
    }

    #[tokio::test]
    async fn a_device_with_a_key_is_not_asked_about_a_plaintext_claim() {
        let rig = Rig::new();
        rig.put_api_key();
        let key_file = rig.home().join(".config/passalong/store.key");
        let args = InitArgs {
            tls_pin: Some(PIN.into()),
            yes: true,
            ..https_args()
        };
        encryption::save_key_file(
            &key_file,
            &passalong_core::crypto::DataKey::generate().unwrap(),
            &rig.git,
        )
        .unwrap();
        let mut prompt = ScriptedPrompt::new(false, Vec::<&str>::new());
        let result = rig.run(&args, &mut prompt).await;
        let written = rig.target().exists();
        // With its key, the device refuses the plaintext store when it is
        // used; init asks nothing and writes the config.
        assert!(written, "{result:?}");
        assert!(prompt.questions().is_empty());
    }

    #[tokio::test]
    async fn an_encrypted_workspace_is_joined() {
        let mut rig = Rig::new();
        rig.check.items = 0;
        let fs = LocalFs::new(rig.check.store.path());
        let key = encryption::set_up(&fs, &w1().unwrap(), quick().unwrap())
            .await
            .unwrap();
        rig.server.workspace = serde_json::json!({
            "name": "home", "quotaBytes": "1000", "usedBytes": "0", "itemCount": 0,
            "encryption": {"state": "sealed", "keyId": key.key_id().to_string()},
        });
        rig.put_api_key();
        let mut prompt = ScriptedPrompt::new(true, ["", PIN, W1]);
        let out = rig.run(&https_args(), &mut prompt).await.unwrap();
        assert!(out.contains("connected: the store is encrypted"), "{out}");
        let cfg = rig.written();
        let joined = load_key_file(cfg.client.key_file.as_ref().unwrap(), &rig.git)
            .unwrap()
            .unwrap();
        assert_eq!(joined.key_id(), key.key_id());
    }

    #[tokio::test]
    async fn an_empty_workspace_is_offered_encryption() {
        let mut rig = Rig::new();
        rig.check.items = 0;
        rig.server.workspace = workspace("plaintext", 0);
        rig.put_api_key();
        let mut prompt = ScriptedPrompt::new(true, ["", PIN, "yes", "yes", W1]);
        let out = rig.run(&https_args(), &mut prompt).await.unwrap();
        assert!(out.contains("connected: 0 items on the server"), "{out}");
        let cfg = rig.written();
        assert!(
            load_key_file(cfg.client.key_file.as_ref().unwrap(), &rig.git)
                .unwrap()
                .is_some(),
            "{out}"
        );
    }

    #[tokio::test]
    async fn yes_needs_a_pin_for_an_untrusted_server_and_the_key_on_disk() {
        let rig = Rig::new();
        let yes = InitArgs {
            yes: true,
            ..https_args()
        };
        let mut prompt = ScriptedPrompt::new(false, Vec::<&str>::new());
        let err = rig.run(&yes, &mut prompt).await.unwrap_err();
        assert!(err.to_string().contains("--tls-pin"), "{err}");

        let pinned = InitArgs {
            tls_pin: Some(PIN.into()),
            ..yes.clone()
        };
        let err = rig.run(&pinned, &mut prompt).await.unwrap_err();
        assert!(err.to_string().contains("no API key in"), "{err}");

        let wrong = InitArgs {
            tls_pin: Some(OTHER_PIN.into()),
            ..yes
        };
        let err = rig.run(&wrong, &mut prompt).await.unwrap_err();
        assert!(err.to_string().contains("mismatch"), "{err}");
        assert!(!rig.target().exists());

        rig.put_api_key();
        let out = rig.run(&pinned, &mut prompt).await.unwrap();
        assert!(
            out.contains("says workspace home is not encrypted"),
            "{out}"
        );
        assert!(rig.target().exists());
    }

    #[tokio::test]
    async fn no_test_writes_without_asking_the_server() {
        let rig = Rig::new();
        rig.put_api_key();
        let args = InitArgs {
            tls_pin: Some(PIN.into()),
            yes: true,
            no_test: true,
            ..https_args()
        };
        rig.run(&args, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap();
        assert!(rig.target().exists());
        assert_eq!(rig.server.inspected.load(Ordering::SeqCst), 0);
        assert!(rig.check.seen.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn ssh_and_https_flags_do_not_mix() {
        let rig = Rig::new();
        let args = InitArgs {
            backend: Some(Backend::Ssh),
            yes: true,
            ..https_args()
        };
        let err = rig
            .run(&args, &mut ScriptedPrompt::new(false, Vec::<&str>::new()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("--backend https"), "{err}");
    }
}
