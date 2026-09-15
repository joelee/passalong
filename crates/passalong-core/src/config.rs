//! Configuration: discovery, parsing, defaults, and validation.
//!
//! Non-secret settings live in `config.toml`, found through the lookup order
//! in [`locate`]. The only secret, the SSH key passphrase, is read from
//! [`PASSPHRASE_ENV`] (normally loaded from `.env`) and cannot be set in the
//! file. Every key and default is documented in `docs/configuration.md`.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::telemetry::{LogLevel, ParseLogLevelError};

/// Application name used in configuration paths.
pub const APP_NAME: &str = "passalong";
/// Environment variable naming the config file (lookup position 2).
pub const CONFIG_FILE_ENV: &str = "PASSALONG_CONFIG_FILE";
/// Environment variable holding the SSH private key passphrase.
pub const PASSPHRASE_ENV: &str = "PASSALONG_SSH_KEY_PASSPHRASE";
/// Environment variable overriding `client.log_level`.
pub const LOG_LEVEL_ENV: &str = "PASSALONG_LOG_LEVEL";

const CONFIG_FILE_NAME: &str = "config.toml";
/// Name of the key file in the default config folder.
pub const KEY_FILE_NAME: &str = "store.key";
const DEFAULT_SSH_PORT: i64 = 22;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_DROP_FOLDER: &str = "~/PassAlong";
const DEFAULT_DOWNLOAD_DIR: &str = "~/Downloads";
const DEFAULT_PULL_INTERVAL_MS: u64 = 5000;
const DEFAULT_LIST_CACHE_CHECK_SECS: u64 = 60;
const MIN_LIST_CACHE_CHECK_SECS: u64 = 10;
const MAX_LIST_CACHE_CHECK_SECS: u64 = 86_400;
const MIN_PULL_INTERVAL_MS: u64 = 1000;
const DEFAULT_CLIPBOARD_POLL_INTERVAL_MS: u64 = 750;
const DEFAULT_FILE_STABLE_WAIT_MS: u64 = 1000;
const MAX_CONNECT_TIMEOUT_SECS: u64 = 3600;
const MAX_INTERVAL_MS: u64 = 3_600_000;

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

/// Read access to the process environment, injectable so tests never touch
/// the real one.
pub trait EnvProvider {
    /// Returns the raw value of `key`, or `None` when it is unset. Callers
    /// treat an empty value as unset.
    fn var(&self, key: &str) -> Option<String>;

    /// Returns this machine's host name.
    fn hostname(&self) -> String;
}

/// [`EnvProvider`] backed by the real process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdEnv;

impl EnvProvider for StdEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn hostname(&self) -> String {
        gethostname::gethostname().to_string_lossy().into_owned()
    }
}

/// Returns the value of `key` when it is set and not empty.
fn non_empty(env: &dyn EnvProvider, key: &str) -> Option<String> {
    env.var(key).filter(|value| !value.is_empty())
}

/// Whose conventions decide the standard file locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    /// Linux, macOS, and other Unix-like systems: XDG and `$HOME`.
    Unix,
    /// Windows: `%APPDATA%` and `%USERPROFILE%`.
    Windows,
}

impl Platform {
    /// The platform this binary was built for.
    pub(crate) fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// On Windows, the folder the environment variable `key` names. `None`
/// elsewhere, or when it is unset; then the Unix rules apply, which on a
/// real Windows system, where these variables are always set, never
/// happens.
fn windows_dir(env: &dyn EnvProvider, platform: Platform, key: &str) -> Option<PathBuf> {
    if platform == Platform::Windows {
        non_empty(env, key).map(PathBuf::from)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Where a configuration file was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// The `--config` command-line argument.
    Explicit,
    /// The [`CONFIG_FILE_ENV`] environment variable.
    EnvVar,
    /// `$XDG_CONFIG_HOME/passalong/config.toml`.
    XdgConfigHome,
    /// `$HOME/.config/passalong/config.toml`; on Windows,
    /// `%APPDATA%\passalong\config.toml`.
    HomeConfig,
    /// `/etc/passalong/config.toml`; on Windows,
    /// `%ProgramData%\passalong\config.toml`.
    System,
    /// `./config.toml`.
    WorkingDir,
}

impl fmt::Display for ConfigOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Explicit => "--config",
            Self::EnvVar => CONFIG_FILE_ENV,
            Self::XdgConfigHome => "$XDG_CONFIG_HOME",
            Self::HomeConfig if cfg!(windows) => "%APPDATA%",
            Self::HomeConfig => "$HOME/.config",
            Self::System => "the system config directory",
            Self::WorkingDir => "the working directory",
        })
    }
}

/// A configuration file chosen by [`locate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedConfig {
    /// Absolute path of the file.
    pub path: PathBuf,
    /// Which lookup position produced it.
    pub origin: ConfigOrigin,
}

/// Filesystem roots for the lookup positions that do not come from the
/// environment; injectable so tests can use temporary directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRoots {
    /// Directory holding `passalong/config.toml` system-wide (`/etc`, or
    /// `%ProgramData%` on Windows).
    pub system_config_dir: PathBuf,
    /// Directory holding `./config.toml`, and the base for relative paths.
    pub working_dir: PathBuf,
}

impl SearchRoots {
    /// Returns `/etc`, or `%ProgramData%` on Windows, and the process's
    /// current directory.
    ///
    /// # Errors
    ///
    /// Fails when the current directory cannot be determined.
    pub fn from_system() -> io::Result<Self> {
        let system_config_dir = if cfg!(windows) {
            std::env::var_os("ProgramData")
                .filter(|dir| !dir.is_empty())
                .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
        } else {
            PathBuf::from("/etc")
        };
        Ok(Self {
            system_config_dir,
            working_dir: std::env::current_dir()?,
        })
    }
}

/// Finds the configuration file. The first match wins:
///
/// 1. `explicit` (the `--config` argument)
/// 2. [`CONFIG_FILE_ENV`]
/// 3. `$XDG_CONFIG_HOME/passalong/config.toml`
/// 4. `$HOME/.config/passalong/config.toml`
/// 5. `/etc/passalong/config.toml`
/// 6. `./config.toml`
///
/// On Windows, 3 and 4 are replaced by `%APPDATA%\passalong\config.toml`,
/// and 5 is under `%ProgramData%`; `XDG_CONFIG_HOME` and `HOME` are read
/// only when `APPDATA` is unset.
///
/// A path named by 1 or 2 must exist: a typo there is reported rather than
/// silently falling back to another file. Relative paths resolve against
/// the working directory; an empty variable counts as unset; a relative
/// `XDG_CONFIG_HOME` is ignored, as the XDG specification requires.
///
/// # Errors
///
/// [`ConfigError::Missing`] when 1 or 2 names a file that does not exist,
/// and [`ConfigError::NotFound`] listing every probed path otherwise.
pub fn locate(
    explicit: Option<&Path>,
    env: &dyn EnvProvider,
    roots: &SearchRoots,
) -> Result<LocatedConfig, ConfigError> {
    locate_on(explicit, env, roots, Platform::current())
}

/// [`locate`] with `platform`'s locations.
pub(crate) fn locate_on(
    explicit: Option<&Path>,
    env: &dyn EnvProvider,
    roots: &SearchRoots,
    platform: Platform,
) -> Result<LocatedConfig, ConfigError> {
    let absolute = |path: &Path| {
        if path.has_root() {
            path.to_path_buf()
        } else {
            roots.working_dir.join(path)
        }
    };

    let pinned = explicit
        .map(|path| (absolute(path), ConfigOrigin::Explicit))
        .or_else(|| {
            non_empty(env, CONFIG_FILE_ENV)
                .map(|path| (absolute(Path::new(&path)), ConfigOrigin::EnvVar))
        });
    if let Some((path, origin)) = pinned {
        return if probe(&path)? {
            tracing::debug!(path = %path.display(), "using config file from {origin}");
            Ok(LocatedConfig { path, origin })
        } else {
            Err(ConfigError::Missing { path, origin })
        };
    }

    let mut candidates = Vec::with_capacity(4);
    if let Some(appdata) = windows_dir(env, platform, "APPDATA") {
        candidates.push((
            appdata.join(APP_NAME).join(CONFIG_FILE_NAME),
            ConfigOrigin::HomeConfig,
        ));
    } else {
        if let Some(xdg) = non_empty(env, "XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.has_root())
        {
            candidates.push((
                xdg.join(APP_NAME).join(CONFIG_FILE_NAME),
                ConfigOrigin::XdgConfigHome,
            ));
        }
        if let Some(home) = non_empty(env, "HOME") {
            candidates.push((
                Path::new(&home)
                    .join(".config")
                    .join(APP_NAME)
                    .join(CONFIG_FILE_NAME),
                ConfigOrigin::HomeConfig,
            ));
        }
    }
    candidates.push((
        roots
            .system_config_dir
            .join(APP_NAME)
            .join(CONFIG_FILE_NAME),
        ConfigOrigin::System,
    ));
    candidates.push((
        roots.working_dir.join(CONFIG_FILE_NAME),
        ConfigOrigin::WorkingDir,
    ));

    for (path, origin) in &candidates {
        if probe(path)? {
            tracing::debug!(path = %path.display(), "using config file from {origin}");
            return Ok(LocatedConfig {
                path: path.clone(),
                origin: *origin,
            });
        }
    }
    Err(ConfigError::NotFound {
        searched: candidates.into_iter().map(|(path, _)| path).collect(),
    })
}

/// Whether a config file exists at `path`. A missing file, or a path running
/// through a regular file, counts as absent; anything that stops us looking,
/// such as a directory we may not enter, is an error rather than a silent
/// skip to the next location.
fn probe(path: &Path) -> Result<bool, ConfigError> {
    match fs::metadata(path) {
        Ok(meta) => Ok(meta.is_file()),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(err) => Err(ConfigError::Unreadable {
            path: path.to_path_buf(),
            reason: if err.kind() == io::ErrorKind::PermissionDenied {
                "permission denied".to_owned()
            } else {
                err.to_string()
            },
        }),
    }
}

// ---------------------------------------------------------------------------
// Validated configuration
// ---------------------------------------------------------------------------

/// Complete, validated configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// `[client]`: settings about this device.
    pub client: ClientConfig,
    /// `[server]`: the storage backend.
    pub server: ServerConfig,
    /// `[serve]`: the long-running `passalong serve` process.
    pub serve: ServeConfig,
}

/// `[client]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientConfig {
    /// Name recorded on every item this device sends. Default: host name.
    pub device_name: String,
    /// Log verbosity. Default: `info`.
    pub log_level: LogLevel,
    /// Where `load` puts file items when no destination is given, and where
    /// pull mode writes files; absolute, `~` expanded. Default: `~/Downloads`.
    pub download_dir: PathBuf,
    /// Where this device keeps an encrypted store's data key; absolute, `~`
    /// expanded. Default: [`KEY_FILE_NAME`] beside the default config file
    /// ([`default_key_path`]); `None` only when neither `XDG_CONFIG_HOME`
    /// nor `HOME` gives one, and then encryption needs it set.
    pub key_file: Option<PathBuf>,
}

/// `[server]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerConfig {
    /// Backend name. `ssh` and `local` are validated here; any other value
    /// is passed through for the store factory to accept or reject.
    pub kind: String,
    /// `[server.ssh]`, present when `kind = "ssh"`.
    pub ssh: Option<SshConfig>,
    /// `[server.local]`, present when `kind = "local"`.
    pub local: Option<LocalConfig>,
}

/// `[server.ssh]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SshConfig {
    /// Server host name or IP address.
    pub host: String,
    /// Server port. Default: 22.
    pub port: u16,
    /// Login user.
    pub user: String,
    /// The server's pinned public host key, as an OpenSSH `type base64` line.
    pub host_key: String,
    /// Private key used to authenticate; `~` is expanded.
    pub identity_file: PathBuf,
    /// Storage directory on the server: absolute, or relative to the login
    /// home. Never `~`-expanded because it is a remote path.
    pub remote_path: String,
    /// TCP connect timeout in seconds. Default: 10.
    pub connect_timeout_secs: u64,
    /// Key passphrase from [`PASSPHRASE_ENV`]; never read from the file.
    pub passphrase: Option<Passphrase>,
}

/// `[server.local]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocalConfig {
    /// Storage directory, for example a mounted network share; `~` is expanded.
    pub path: PathBuf,
}

/// `[serve]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServeConfig {
    /// Folder watched for files to send; `~` is expanded. Default: `~/PassAlong`.
    pub drop_folder: PathBuf,
    /// How often the clipboard is checked. Default: 750 ms.
    pub clipboard_poll_interval_ms: u64,
    /// How long a dropped file must stay unchanged before it is sent.
    /// Default: 1000 ms.
    pub file_stable_wait_ms: u64,
    /// What happens to a dropped file once sent. Default: [`AfterSend::Move`].
    pub after_send: AfterSend,
    /// Whether clipboard images are sent too. Default: `true`.
    pub clipboard_images: bool,
    /// Whether items sent by other devices are applied here. Default: `false`.
    pub pull: bool,
    /// How often pull mode checks for new items. Default: 5000 ms.
    pub pull_interval_ms: u64,
    /// Whether `serve` keeps a local copy of the item list that `list` and
    /// `choose` read without connecting; `ssh` backend only. Default: `true`.
    pub list_cache: bool,
    /// How often `serve` compares that copy with the store. Default: 60 s.
    pub list_cache_check_secs: u64,
}

/// What `serve` does with a dropped file after sending it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AfterSend {
    /// Move it into `<drop_folder>/sent/`.
    #[default]
    Move,
    /// Delete it.
    Delete,
}

/// SSH private-key passphrase.
///
/// `Debug` output is redacted and there is no `Display`, so the value cannot
/// reach logs or error messages by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Passphrase(String);

impl Passphrase {
    /// Wraps a passphrase.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the secret, for handing to the SSH key decoder only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from finding, reading, or validating configuration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// No file exists at any probed location.
    #[error("no config file found; searched: {}", join_paths(searched))]
    NotFound {
        /// Every path that was probed, in lookup order.
        searched: Vec<PathBuf>,
    },
    /// A path given by `--config` or [`CONFIG_FILE_ENV`] does not exist.
    #[error("config file {} from {origin} does not exist", path.display())]
    Missing {
        /// The missing path.
        path: PathBuf,
        /// Which of the two explicit positions named it.
        origin: ConfigOrigin,
    },
    /// A lookup location exists but cannot be inspected, for example
    /// because a directory on its path may not be entered.
    #[error("cannot read config file {}: {reason}", path.display())]
    Unreadable {
        /// The config file path.
        path: PathBuf,
        /// Why it cannot be read.
        reason: String,
    },
    /// The file exists but cannot be read.
    #[error("cannot read config file {}: {source}", path.display())]
    Read {
        /// The unreadable path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// The file is not valid TOML or contains an unknown key or wrong type.
    #[error("invalid config file {}: {message}", path.display())]
    Parse {
        /// The file being parsed.
        path: PathBuf,
        /// One-line description including the offending line.
        message: String,
    },
    /// A required key is absent.
    #[error("missing required config key `{0}`")]
    MissingKey(String),
    /// A key has a value outside its allowed set or range.
    #[error("invalid value for `{key}`: {reason}")]
    InvalidValue {
        /// Dotted key name, or the environment variable name.
        key: String,
        /// What is wrong and what is allowed.
        reason: String,
    },
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn invalid(key: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        key: key.to_owned(),
        reason: reason.into(),
    }
}

// ---------------------------------------------------------------------------
// Loading and parsing
// ---------------------------------------------------------------------------

/// Reads and validates the configuration file at `path`.
///
/// # Errors
///
/// [`ConfigError::Read`] when the file cannot be read, otherwise as [`parse`].
pub fn load(path: &Path, env: &dyn EnvProvider) -> Result<Config, ConfigError> {
    let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let config = parse(&text, path, env)?;
    tracing::debug!(path = %path.display(), kind = %config.server.kind, "configuration loaded");
    Ok(config)
}

/// Parses and validates configuration text; `path` is used only in errors.
///
/// # Errors
///
/// [`ConfigError::Parse`] for invalid TOML, unknown keys, or wrong types;
/// [`ConfigError::MissingKey`] and [`ConfigError::InvalidValue`] name the
/// offending dotted key.
pub fn parse(text: &str, path: &Path, env: &dyn EnvProvider) -> Result<Config, ConfigError> {
    let raw: RawConfig = toml::from_str(text).map_err(|err| ConfigError::Parse {
        path: path.to_path_buf(),
        message: describe_toml_error(text, &err),
    })?;
    raw.validate(env)
}

/// Resolves the log level: `flag`, then [`LOG_LEVEL_ENV`], then
/// `client.log_level`.
///
/// # Errors
///
/// [`ConfigError::InvalidValue`] naming [`LOG_LEVEL_ENV`] when it holds an
/// unknown level.
pub fn effective_log_level(
    flag: Option<LogLevel>,
    env: &dyn EnvProvider,
    config: &Config,
) -> Result<LogLevel, ConfigError> {
    if let Some(level) = flag {
        return Ok(level);
    }
    match non_empty(env, LOG_LEVEL_ENV) {
        Some(value) => value
            .parse()
            .map_err(|err: ParseLogLevelError| invalid(LOG_LEVEL_ENV, err.to_string())),
        None => Ok(config.client.log_level),
    }
}

/// Expands a leading `~` or `~/` to `$HOME`. Anything else, including
/// `~user`, is returned unchanged.
///
/// # Errors
///
/// [`ConfigError::InvalidValue`] for `key` when the path starts with `~` and
/// `HOME` is unset or empty.
pub fn expand_tilde(path: &str, key: &str, env: &dyn EnvProvider) -> Result<PathBuf, ConfigError> {
    expand_tilde_on(path, key, env, Platform::current())
}

/// [`expand_tilde`] with `platform`'s home folder: `%USERPROFILE%` on
/// Windows, where `~\` is accepted too, and `$HOME` elsewhere.
pub(crate) fn expand_tilde_on(
    path: &str,
    key: &str,
    env: &dyn EnvProvider,
    platform: Platform,
) -> Result<PathBuf, ConfigError> {
    let rest = if path == "~" {
        Some("")
    } else if platform == Platform::Windows {
        path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
    } else {
        path.strip_prefix("~/")
    };
    let Some(rest) = rest else {
        return Ok(PathBuf::from(path));
    };
    let home = windows_dir(env, platform, "USERPROFILE")
        .or_else(|| non_empty(env, "HOME").map(PathBuf::from))
        .ok_or_else(|| invalid(key, "starts with `~` but HOME is not set"))?;
    Ok(if rest.is_empty() {
        home
    } else {
        home.join(rest)
    })
}

/// Turns a TOML error into one line that names the offending source line,
/// so the CLI can print it as a single `error:` line.
fn describe_toml_error(text: &str, err: &toml::de::Error) -> String {
    let message = err.message().trim().replace('\n', " ");
    let Some(span) = err.span() else {
        return message;
    };
    let before = text.get(..span.start).unwrap_or(text);
    let line_number = before.matches('\n').count() + 1;
    let line = text.lines().nth(line_number - 1).unwrap_or_default().trim();
    format!("line {line_number} (`{line}`): {message}")
}

fn required(value: Option<String>, key: &str) -> Result<String, ConfigError> {
    let value = value.ok_or_else(|| ConfigError::MissingKey(key.to_owned()))?;
    if value.trim().is_empty() {
        return Err(invalid(key, "must not be empty"));
    }
    Ok(value)
}

fn bounded(
    value: Option<i64>,
    default: u64,
    min: u64,
    max: u64,
    key: &str,
) -> Result<u64, ConfigError> {
    let Some(value) = value else {
        return Ok(default);
    };
    u64::try_from(value)
        .ok()
        .filter(|value| (min..=max).contains(value))
        .ok_or_else(|| invalid(key, format!("must be between {min} and {max}")))
}

// The raw structs mirror the file exactly. Every field is optional so that
// validation, not serde, reports missing keys with their dotted names.

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    client: RawClient,
    server: Option<RawServer>,
    #[serde(default)]
    serve: RawServe,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClient {
    device_name: Option<String>,
    log_level: Option<String>,
    download_dir: Option<String>,
    key_file: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServer {
    kind: Option<String>,
    ssh: Option<RawSsh>,
    local: Option<RawLocal>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSsh {
    host: Option<String>,
    port: Option<i64>,
    user: Option<String>,
    host_key: Option<String>,
    identity_file: Option<String>,
    remote_path: Option<String>,
    connect_timeout_secs: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLocal {
    path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServe {
    drop_folder: Option<String>,
    clipboard_poll_interval_ms: Option<i64>,
    file_stable_wait_ms: Option<i64>,
    after_send: Option<String>,
    clipboard_images: Option<bool>,
    pull: Option<bool>,
    pull_interval_ms: Option<i64>,
    list_cache: Option<bool>,
    list_cache_check_secs: Option<i64>,
}

impl RawConfig {
    fn validate(self, env: &dyn EnvProvider) -> Result<Config, ConfigError> {
        // Server first: it holds the required keys users most often miss.
        let server = self.server.unwrap_or_default();
        let kind = required(server.kind, "server.kind")?;
        let (ssh, local) = match kind.as_str() {
            "ssh" => (Some(server.ssh.unwrap_or_default().validate(env)?), None),
            "local" => (None, Some(server.local.unwrap_or_default().validate(env)?)),
            _ => (None, None),
        };
        let server = ServerConfig { kind, ssh, local };
        let client = self.client.validate(env)?;
        let serve = self.serve.validate(env)?;
        // A pulled file written into the drop folder would be sent back.
        if serve.pull && client.download_dir.starts_with(&serve.drop_folder) {
            return Err(invalid(
                "client.download_dir",
                "must not be serve.drop_folder or inside it when serve.pull is true",
            ));
        }
        Ok(Config {
            client,
            server,
            serve,
        })
    }
}

impl RawClient {
    fn validate(self, env: &dyn EnvProvider) -> Result<ClientConfig, ConfigError> {
        let device_name = match self.device_name {
            Some(name) => required(Some(name), "client.device_name")?,
            None => env.hostname(),
        };
        let log_level = match self.log_level {
            Some(level) => level
                .parse()
                .map_err(|err: ParseLogLevelError| invalid("client.log_level", err.to_string()))?,
            None => LogLevel::default(),
        };
        let download_dir = match self.download_dir {
            Some(dir) => required(Some(dir), "client.download_dir")?,
            None => DEFAULT_DOWNLOAD_DIR.to_owned(),
        };
        let download_dir = expand_tilde(&download_dir, "client.download_dir", env)?;
        if !download_dir.has_root() {
            return Err(invalid(
                "client.download_dir",
                "must be an absolute path or start with `~/`",
            ));
        }
        let key_file = match self.key_file {
            Some(path) => {
                let path = required(Some(path), "client.key_file")?;
                let path = expand_tilde(&path, "client.key_file", env)?;
                if !path.has_root() {
                    return Err(invalid(
                        "client.key_file",
                        "must be an absolute path or start with `~/`",
                    ));
                }
                Some(path)
            }
            None => default_key_path(env),
        };
        Ok(ClientConfig {
            device_name,
            log_level,
            download_dir,
            key_file,
        })
    }
}

impl RawSsh {
    fn validate(self, env: &dyn EnvProvider) -> Result<SshConfig, ConfigError> {
        let host = required(self.host, "server.ssh.host")?;
        let user = required(self.user, "server.ssh.user")?;
        let host_key = required(self.host_key, "server.ssh.host_key")?;
        let identity_file = required(self.identity_file, "server.ssh.identity_file")?;
        let identity_file = expand_tilde(&identity_file, "server.ssh.identity_file", env)?;
        let remote_path = required(self.remote_path, "server.ssh.remote_path")?;
        let port = u16::try_from(self.port.unwrap_or(DEFAULT_SSH_PORT))
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| invalid("server.ssh.port", "must be between 1 and 65535"))?;
        let connect_timeout_secs = bounded(
            self.connect_timeout_secs,
            DEFAULT_CONNECT_TIMEOUT_SECS,
            1,
            MAX_CONNECT_TIMEOUT_SECS,
            "server.ssh.connect_timeout_secs",
        )?;
        Ok(SshConfig {
            host,
            port,
            user,
            host_key,
            identity_file,
            remote_path,
            connect_timeout_secs,
            passphrase: non_empty(env, PASSPHRASE_ENV).map(Passphrase::new),
        })
    }
}

impl RawLocal {
    fn validate(self, env: &dyn EnvProvider) -> Result<LocalConfig, ConfigError> {
        let path = required(self.path, "server.local.path")?;
        Ok(LocalConfig {
            path: expand_tilde(&path, "server.local.path", env)?,
        })
    }
}

impl RawServe {
    fn validate(self, env: &dyn EnvProvider) -> Result<ServeConfig, ConfigError> {
        let drop_folder = match self.drop_folder {
            Some(folder) => required(Some(folder), "serve.drop_folder")?,
            None => DEFAULT_DROP_FOLDER.to_owned(),
        };
        let after_send = match self.after_send.as_deref() {
            None | Some("move") => AfterSend::Move,
            Some("delete") => AfterSend::Delete,
            Some(_) => return Err(invalid("serve.after_send", "expected `move` or `delete`")),
        };
        Ok(ServeConfig {
            drop_folder: expand_tilde(&drop_folder, "serve.drop_folder", env)?,
            clipboard_poll_interval_ms: bounded(
                self.clipboard_poll_interval_ms,
                DEFAULT_CLIPBOARD_POLL_INTERVAL_MS,
                1,
                MAX_INTERVAL_MS,
                "serve.clipboard_poll_interval_ms",
            )?,
            file_stable_wait_ms: bounded(
                self.file_stable_wait_ms,
                DEFAULT_FILE_STABLE_WAIT_MS,
                0,
                MAX_INTERVAL_MS,
                "serve.file_stable_wait_ms",
            )?,
            after_send,
            clipboard_images: self.clipboard_images.unwrap_or(true),
            pull: self.pull.unwrap_or(false),
            pull_interval_ms: bounded(
                self.pull_interval_ms,
                DEFAULT_PULL_INTERVAL_MS,
                MIN_PULL_INTERVAL_MS,
                MAX_INTERVAL_MS,
                "serve.pull_interval_ms",
            )?,
            list_cache: self.list_cache.unwrap_or(true),
            list_cache_check_secs: bounded(
                self.list_cache_check_secs,
                DEFAULT_LIST_CACHE_CHECK_SECS,
                MIN_LIST_CACHE_CHECK_SECS,
                MAX_LIST_CACHE_CHECK_SECS,
                "serve.list_cache_check_secs",
            )?,
        })
    }
}

// ---------------------------------------------------------------------------
// Writing a config (`passalong init`)
// ---------------------------------------------------------------------------

/// The values `passalong init` collects for an SSH server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitAnswers {
    /// `client.device_name`.
    pub device_name: String,
    /// `server.ssh.host`.
    pub host: String,
    /// `server.ssh.port`.
    pub port: u16,
    /// `server.ssh.user`.
    pub user: String,
    /// `server.ssh.host_key`, the confirmed OpenSSH key line.
    pub host_key: String,
    /// `server.ssh.identity_file` as typed; `~` is kept for readability.
    pub identity_file: String,
    /// `server.ssh.remote_path`.
    pub remote_path: String,
}

/// Renders a complete, commented config file for an SSH server. Every value
/// is TOML-escaped, so the result always parses back to the same answers.
pub fn render(answers: &InitAnswers) -> String {
    let q = |value: &str| toml::Value::String(value.to_owned()).to_string();
    format!(
        r#"# passalong configuration, written by `passalong init`.
# Every key is documented in docs/configuration.md. Secrets never go here:
# the SSH key passphrase comes from PASSALONG_SSH_KEY_PASSPHRASE (see .env.sample).

[client]
# Name recorded on every item you send.
device_name = {device}
# error | warning | info | verbose | debug
log_level = "info"

[server]
kind = "ssh"

[server.ssh]
host = {host}
port = {port}
user = {user}
# The server's public host key, pinned after you confirmed its fingerprint.
host_key = {host_key}
identity_file = {identity}
# Absolute, or relative to the SSH user's home directory.
remote_path = {remote}
connect_timeout_secs = 10

[serve]
drop_folder = "~/PassAlong"
clipboard_poll_interval_ms = 750
file_stable_wait_ms = 1000
# What to do with a dropped file after it is sent: "move" (into drop_folder/sent/) or "delete".
after_send = "move"
"#,
        device = q(&answers.device_name),
        host = q(&answers.host),
        port = answers.port,
        user = q(&answers.user),
        host_key = q(&answers.host_key),
        identity = q(&answers.identity_file),
        remote = q(&answers.remote_path),
    )
}

/// Where `passalong init` writes without `--config`:
/// `$XDG_CONFIG_HOME/passalong/config.toml` when that variable is absolute,
/// otherwise `$HOME/.config/passalong/config.toml`; on Windows,
/// `%APPDATA%\passalong\config.toml`. These are lookup positions, so the
/// written file is found again.
pub fn default_config_path(env: &dyn EnvProvider) -> Option<PathBuf> {
    default_config_path_on(env, Platform::current())
}

/// [`default_config_path`] with `platform`'s locations.
pub(crate) fn default_config_path_on(env: &dyn EnvProvider, platform: Platform) -> Option<PathBuf> {
    if let Some(appdata) = windows_dir(env, platform, "APPDATA") {
        return Some(appdata.join(APP_NAME).join(CONFIG_FILE_NAME));
    }
    if let Some(xdg) = non_empty(env, "XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.has_root())
    {
        return Some(xdg.join(APP_NAME).join(CONFIG_FILE_NAME));
    }
    non_empty(env, "HOME").map(|home| {
        Path::new(&home)
            .join(".config")
            .join(APP_NAME)
            .join(CONFIG_FILE_NAME)
    })
}

/// Where the key file is by default: [`KEY_FILE_NAME`] in the folder of
/// [`default_config_path`].
pub fn default_key_path(env: &dyn EnvProvider) -> Option<PathBuf> {
    default_config_path(env).map(|config| config.with_file_name(KEY_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::LogLevel;
    use crate::testing::MapEnv;
    use std::fs;
    use tempfile::TempDir;

    // The tests below describe the Unix locations on every platform; the
    // Windows ones are tested by name.
    fn locate(
        explicit: Option<&Path>,
        env: &dyn EnvProvider,
        roots: &SearchRoots,
    ) -> Result<LocatedConfig, ConfigError> {
        locate_on(explicit, env, roots, Platform::Unix)
    }

    fn default_config_path(env: &dyn EnvProvider) -> Option<PathBuf> {
        default_config_path_on(env, Platform::Unix)
    }

    fn expand_tilde(path: &str, key: &str, env: &dyn EnvProvider) -> Result<PathBuf, ConfigError> {
        expand_tilde_on(path, key, env, Platform::Unix)
    }

    #[test]
    fn windows_looks_in_appdata_and_not_in_home_or_xdg() {
        let sb = Sandbox::new();
        sb.touch_all_standard();
        let appdata = sb.home.join("AppData");
        let env = sb.env().with("APPDATA", appdata.to_str().unwrap());
        // Nothing in %APPDATA%: the system file comes next, never $HOME's.
        let found = locate_on(None, &env, &sb.roots(), Platform::Windows).unwrap();
        assert_eq!(found.origin, ConfigOrigin::System);
        let wanted = sb.touch(&appdata.join("passalong").join("config.toml"));
        let found = locate_on(None, &env, &sb.roots(), Platform::Windows).unwrap();
        assert_eq!(
            (found.path, found.origin),
            (wanted, ConfigOrigin::HomeConfig)
        );
    }

    #[test]
    fn windows_writes_to_appdata_and_expands_tilde_from_userprofile() {
        let env = MapEnv::new()
            .with("APPDATA", "/appdata")
            .with("USERPROFILE", "/profile")
            .with("HOME", "/home/u")
            .with("XDG_CONFIG_HOME", "/x");
        assert_eq!(
            default_config_path_on(&env, Platform::Windows),
            Some(Path::new("/appdata").join("passalong").join("config.toml"))
        );
        for input in ["~/docs", "~\\docs"] {
            assert_eq!(
                expand_tilde_on(input, "k", &env, Platform::Windows).unwrap(),
                Path::new("/profile").join("docs"),
                "{input}"
            );
        }
        // `~\` is a plain name elsewhere.
        assert_eq!(
            expand_tilde_on("~\\docs", "k", &env, Platform::Unix).unwrap(),
            PathBuf::from("~\\docs")
        );
        // Without the Windows variables, the Unix rules apply.
        let unix_only = MapEnv::new().with("HOME", "/home/u");
        assert_eq!(
            default_config_path_on(&unix_only, Platform::Windows),
            default_config_path_on(&unix_only, Platform::Unix)
        );
    }

    // ---------- discovery ----------

    /// A sandbox with every lookup location as a separate directory.
    struct Sandbox {
        _dir: TempDir,
        xdg: PathBuf,
        home: PathBuf,
        etc: PathBuf,
        cwd: PathBuf,
    }

    impl Sandbox {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let [xdg, home, etc, cwd] = ["xdg", "home", "etc", "cwd"].map(|d| dir.path().join(d));
            for d in [&xdg, &home, &etc, &cwd] {
                fs::create_dir_all(d).unwrap();
            }
            Self {
                _dir: dir,
                xdg,
                home,
                etc,
                cwd,
            }
        }
        fn env(&self) -> MapEnv {
            MapEnv::new()
                .with("XDG_CONFIG_HOME", self.xdg.to_str().unwrap())
                .with("HOME", self.home.to_str().unwrap())
        }
        fn roots(&self) -> SearchRoots {
            SearchRoots {
                system_config_dir: self.etc.clone(),
                working_dir: self.cwd.clone(),
            }
        }
        // Joined part by part, so each path reads as locate prints it.
        fn xdg_file(&self) -> PathBuf {
            self.xdg.join("passalong").join("config.toml")
        }
        fn home_file(&self) -> PathBuf {
            self.home
                .join(".config")
                .join("passalong")
                .join("config.toml")
        }
        fn etc_file(&self) -> PathBuf {
            self.etc.join("passalong").join("config.toml")
        }
        fn cwd_file(&self) -> PathBuf {
            self.cwd.join("config.toml")
        }
        fn touch(&self, path: &Path) -> PathBuf {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
            path.to_path_buf()
        }
        fn touch_all_standard(&self) {
            for f in [
                self.xdg_file(),
                self.home_file(),
                self.etc_file(),
                self.cwd_file(),
            ] {
                self.touch(&f);
            }
        }
    }

    #[test]
    fn explicit_path_wins_over_every_other_location() {
        let sb = Sandbox::new();
        sb.touch_all_standard();
        let explicit = sb.touch(&sb.cwd.join("mine.toml"));
        let from_env = sb.touch(&sb.cwd.join("env.toml"));
        let env = sb.env().with(CONFIG_FILE_ENV, from_env.to_str().unwrap());
        let found = locate(Some(&explicit), &env, &sb.roots()).unwrap();
        assert_eq!(
            found,
            LocatedConfig {
                path: explicit,
                origin: ConfigOrigin::Explicit
            }
        );
    }

    #[test]
    fn env_var_wins_when_no_explicit_path() {
        let sb = Sandbox::new();
        sb.touch_all_standard();
        let from_env = sb.touch(&sb.cwd.join("env.toml"));
        let env = sb.env().with(CONFIG_FILE_ENV, from_env.to_str().unwrap());
        let found = locate(None, &env, &sb.roots()).unwrap();
        assert_eq!(
            found,
            LocatedConfig {
                path: from_env,
                origin: ConfigOrigin::EnvVar
            }
        );
    }

    #[test]
    fn standard_locations_are_searched_in_order() {
        let sb = Sandbox::new();
        let order = [
            (sb.xdg_file(), ConfigOrigin::XdgConfigHome),
            (sb.home_file(), ConfigOrigin::HomeConfig),
            (sb.etc_file(), ConfigOrigin::System),
            (sb.cwd_file(), ConfigOrigin::WorkingDir),
        ];
        for (path, _) in &order {
            sb.touch(path);
        }
        for (path, origin) in order {
            let found = locate(None, &sb.env(), &sb.roots()).unwrap();
            assert_eq!(
                found,
                LocatedConfig {
                    path: path.clone(),
                    origin
                }
            );
            fs::remove_file(&path).unwrap();
        }
    }

    #[test]
    fn not_found_names_every_probed_path() {
        let sb = Sandbox::new();
        let err = locate(None, &sb.env(), &sb.roots()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("no config file found"), "{msg}");
        for path in [sb.xdg_file(), sb.home_file(), sb.etc_file(), sb.cwd_file()] {
            assert!(
                msg.contains(path.to_str().unwrap()),
                "{msg} lacks {}",
                path.display()
            );
        }
        match err {
            ConfigError::NotFound { searched } => assert_eq!(searched.len(), 4),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn missing_explicit_or_env_path_is_an_error_not_a_fallback() {
        let sb = Sandbox::new();
        sb.touch_all_standard();
        let ghost = sb.cwd.join("ghost.toml");
        let err = locate(Some(&ghost), &sb.env(), &sb.roots()).unwrap_err();
        assert!(
            matches!(
                &err,
                ConfigError::Missing {
                    origin: ConfigOrigin::Explicit,
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("--config"), "{err}");

        let env = sb.env().with(CONFIG_FILE_ENV, ghost.to_str().unwrap());
        let err = locate(None, &env, &sb.roots()).unwrap_err();
        assert!(
            matches!(
                &err,
                ConfigError::Missing {
                    origin: ConfigOrigin::EnvVar,
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains(CONFIG_FILE_ENV), "{err}");
    }

    #[test]
    fn empty_and_relative_environment_values_are_ignored() {
        let sb = Sandbox::new();
        sb.touch(&sb.home_file());
        let env = sb
            .env()
            .with(CONFIG_FILE_ENV, "")
            .with("XDG_CONFIG_HOME", "relative/dir");
        let found = locate(None, &env, &sb.roots()).unwrap();
        assert_eq!(found.origin, ConfigOrigin::HomeConfig);
    }

    #[test]
    fn unset_home_and_xdg_skip_those_locations() {
        let sb = Sandbox::new();
        let err = locate(None, &MapEnv::new(), &sb.roots()).unwrap_err();
        match err {
            ConfigError::NotFound { searched } => {
                assert_eq!(searched, vec![sb.etc_file(), sb.cwd_file()]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn relative_explicit_path_is_resolved_against_the_working_directory() {
        let sb = Sandbox::new();
        let file = sb.touch(&sb.cwd.join("rel.toml"));
        let found = locate(Some(Path::new("rel.toml")), &sb.env(), &sb.roots()).unwrap();
        assert_eq!(found.path, file);
    }

    #[cfg(windows)]
    #[test]
    fn system_search_roots_use_programdata_on_windows() {
        let roots = SearchRoots::from_system().unwrap();
        assert!(
            roots.system_config_dir.ends_with("ProgramData"),
            "{roots:?}"
        );
        assert_eq!(roots.working_dir, std::env::current_dir().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn system_search_roots_use_etc_and_the_current_directory() {
        let roots = SearchRoots::from_system().unwrap();
        assert_eq!(roots.system_config_dir, PathBuf::from("/etc"));
        assert_eq!(roots.working_dir, std::env::current_dir().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_candidate_is_reported_instead_of_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let sb = Sandbox::new();
        let file = sb.touch(&sb.xdg_file());
        sb.touch(&sb.home_file());
        let locked = sb.xdg.join("passalong");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let enforced = fs::metadata(&file).is_err();
        let result = locate(None, &sb.env(), &sb.roots());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if !enforced {
            return; // running as root: permissions are not enforced
        }
        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "cannot read config file {}: permission denied",
                file.display()
            )
        );
        assert!(matches!(err, ConfigError::Unreadable { .. }), "{err:?}");
    }

    #[test]
    fn a_path_through_a_regular_file_counts_as_absent() {
        let sb = Sandbox::new();
        fs::write(sb.xdg.join("passalong"), b"a file, not a directory").unwrap();
        let home = sb.touch(&sb.home_file());
        assert_eq!(locate(None, &sb.env(), &sb.roots()).unwrap().path, home);
    }

    // ---------- parsing and validation ----------

    const MINIMAL_SSH: &str = r#"
[server]
kind = "ssh"

[server.ssh]
host = "10.0.0.2"
user = "pa"
host_key = "ssh-ed25519 AAAAkey"
identity_file = "/keys/id"
remote_path = "/srv/pa"
"#;

    fn env() -> MapEnv {
        MapEnv::new().with("HOME", "/home/u").with_hostname("box")
    }

    fn parse_ok(text: &str) -> Config {
        parse(text, Path::new("/cfg.toml"), &env()).unwrap()
    }

    fn parse_err(text: &str) -> ConfigError {
        parse(text, Path::new("/cfg.toml"), &env()).unwrap_err()
    }

    fn invalid_key(err: ConfigError) -> String {
        match err {
            ConfigError::InvalidValue { key, .. } => key,
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn minimal_ssh_config_gets_the_documented_defaults() {
        let cfg = parse_ok(MINIMAL_SSH);
        assert_eq!(cfg.client.device_name, "box");
        assert_eq!(cfg.client.log_level, LogLevel::Info);
        assert_eq!(cfg.server.kind, "ssh");
        assert!(cfg.server.local.is_none());
        let ssh = cfg.server.ssh.expect("ssh section");
        assert_eq!(ssh.host, "10.0.0.2");
        assert_eq!(ssh.port, 22);
        assert_eq!(ssh.user, "pa");
        assert_eq!(ssh.host_key, "ssh-ed25519 AAAAkey");
        assert_eq!(ssh.identity_file, PathBuf::from("/keys/id"));
        assert_eq!(ssh.remote_path, "/srv/pa");
        assert_eq!(ssh.connect_timeout_secs, 10);
        assert!(ssh.passphrase.is_none());
        assert_eq!(cfg.serve.drop_folder, PathBuf::from("/home/u/PassAlong"));
        assert_eq!(cfg.serve.clipboard_poll_interval_ms, 750);
        assert_eq!(cfg.serve.file_stable_wait_ms, 1000);
        assert_eq!(cfg.serve.after_send, AfterSend::Move);
    }

    #[test]
    fn the_committed_sample_config_is_valid() {
        let cfg = parse_ok(include_str!("../../../config.sample.toml"));
        assert_eq!(cfg.server.kind, "ssh");
        assert_eq!(
            cfg.server.ssh.unwrap().identity_file,
            PathBuf::from("/home/u/.ssh/id_ed25519")
        );
    }

    #[test]
    fn explicit_values_override_defaults() {
        let text = format!(
            "{MINIMAL_SSH}port = 2222\nconnect_timeout_secs = 3\n\n[client]\ndevice_name = \"laptop\"\nlog_level = \"Verbose\"\n\n[serve]\ndrop_folder = \"/drop\"\nclipboard_poll_interval_ms = 200\nfile_stable_wait_ms = 0\nafter_send = \"delete\"\n"
        );
        let cfg = parse_ok(&text);
        assert_eq!(cfg.client.device_name, "laptop");
        assert_eq!(cfg.client.log_level, LogLevel::Verbose);
        let ssh = cfg.server.ssh.unwrap();
        assert_eq!((ssh.port, ssh.connect_timeout_secs), (2222, 3));
        assert_eq!(cfg.serve.drop_folder, PathBuf::from("/drop"));
        assert_eq!(cfg.serve.clipboard_poll_interval_ms, 200);
        assert_eq!(cfg.serve.file_stable_wait_ms, 0);
        assert_eq!(cfg.serve.after_send, AfterSend::Delete);
    }

    #[test]
    fn each_required_ssh_key_is_named_when_missing() {
        for key in ["host", "user", "host_key", "identity_file", "remote_path"] {
            let text: String = MINIMAL_SSH
                .lines()
                .filter(|l| !l.starts_with(&format!("{key} =")))
                .map(|l| format!("{l}\n"))
                .collect();
            match parse_err(&text) {
                ConfigError::MissingKey(k) => assert_eq!(k, format!("server.ssh.{key}")),
                other => panic!("{key}: unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn missing_ssh_section_names_its_first_required_key() {
        match parse_err("[server]\nkind = \"ssh\"\n") {
            ConfigError::MissingKey(k) => assert_eq!(k, "server.ssh.host"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn server_kind_is_required() {
        for text in ["", "[server]\n", "[client]\ndevice_name = \"x\"\n"] {
            match parse_err(text) {
                ConfigError::MissingKey(k) => assert_eq!(k, "server.kind"),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn local_kind_requires_a_path_and_expands_tilde() {
        match parse_err("[server]\nkind = \"local\"\n") {
            ConfigError::MissingKey(k) => assert_eq!(k, "server.local.path"),
            other => panic!("unexpected {other:?}"),
        }
        let cfg = parse_ok("[server]\nkind = \"local\"\n[server.local]\npath = \"~/share\"\n");
        assert_eq!(
            cfg.server.local.unwrap().path,
            PathBuf::from("/home/u/share")
        );
        assert!(cfg.server.ssh.is_none());
    }

    #[test]
    fn unknown_backend_kinds_are_left_to_the_store_factory() {
        let cfg = parse_ok("[server]\nkind = \"s3\"\n");
        assert_eq!(cfg.server.kind, "s3");
        assert!(cfg.server.ssh.is_none() && cfg.server.local.is_none());
    }

    #[test]
    fn tilde_expansion_rules() {
        let env = env();
        let cases = [
            ("~", "/home/u"),
            ("~/.ssh/id", "/home/u/.ssh/id"),
            ("/abs/path", "/abs/path"),
            ("rel/path", "rel/path"),
            ("~other/x", "~other/x"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                expand_tilde(input, "k", &env).unwrap(),
                PathBuf::from(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn tilde_without_home_names_the_key() {
        let err = parse(
            MINIMAL_SSH,
            Path::new("/c.toml"),
            &MapEnv::new().with_hostname("h"),
        )
        .unwrap_err();
        // identity_file is absolute here, so the failing key is the default
        // download directory, the first `~` path validated.
        assert_eq!(invalid_key(err), "client.download_dir");
        let text = MINIMAL_SSH.replace("\"/keys/id\"", "\"~/id\"");
        let err = parse(
            &text,
            Path::new("/c.toml"),
            &MapEnv::new().with_hostname("h"),
        )
        .unwrap_err();
        assert_eq!(invalid_key(err), "server.ssh.identity_file");
    }

    #[test]
    fn out_of_range_and_unknown_values_name_their_key() {
        let cases = [
            (format!("{MINIMAL_SSH}port = 0\n"), "server.ssh.port"),
            (format!("{MINIMAL_SSH}port = 70000\n"), "server.ssh.port"),
            (
                format!("{MINIMAL_SSH}connect_timeout_secs = 0\n"),
                "server.ssh.connect_timeout_secs",
            ),
            (
                MINIMAL_SSH.replace("\"10.0.0.2\"", "\"\""),
                "server.ssh.host",
            ),
            (
                MINIMAL_SSH.replace("\"/srv/pa\"", "\" \""),
                "server.ssh.remote_path",
            ),
            (
                format!("{MINIMAL_SSH}[client]\nlog_level = \"loud\"\n"),
                "client.log_level",
            ),
            (
                format!("{MINIMAL_SSH}[client]\ndevice_name = \"\"\n"),
                "client.device_name",
            ),
            (
                format!("{MINIMAL_SSH}[serve]\nafter_send = \"copy\"\n"),
                "serve.after_send",
            ),
            (
                format!("{MINIMAL_SSH}[serve]\nclipboard_poll_interval_ms = 0\n"),
                "serve.clipboard_poll_interval_ms",
            ),
            (
                format!("{MINIMAL_SSH}[serve]\nfile_stable_wait_ms = -1\n"),
                "serve.file_stable_wait_ms",
            ),
            ("[server]\nkind = \"\"\n".to_owned(), "server.kind"),
        ];
        for (text, key) in cases {
            let err = parse_err(&text);
            assert!(err.to_string().contains(key), "{err}");
            assert_eq!(invalid_key(err), key);
        }
    }

    #[test]
    fn unknown_keys_and_bad_syntax_are_parse_errors_naming_the_file() {
        for (text, needle) in [
            (MINIMAL_SSH.replace("host =", "hots ="), "hots"),
            ("[server\nkind = 1".to_owned(), "/cfg.toml"),
            (format!("{MINIMAL_SSH}port = \"twenty\"\n"), "port"),
        ] {
            let err = parse_err(&text);
            assert!(matches!(err, ConfigError::Parse { .. }), "{err:?}");
            let msg = err.to_string();
            assert!(msg.contains("/cfg.toml") && msg.contains(needle), "{msg}");
        }
    }

    #[test]
    fn passphrase_comes_only_from_the_environment() {
        let with = env().with(PASSPHRASE_ENV, "s3cret-value");
        let cfg = parse(MINIMAL_SSH, Path::new("/c.toml"), &with).unwrap();
        let ssh = cfg.server.ssh.clone().unwrap();
        assert_eq!(
            ssh.passphrase.as_ref().map(Passphrase::expose),
            Some("s3cret-value")
        );
        assert!(
            !format!("{cfg:?}").contains("s3cret-value"),
            "Config Debug leaks the passphrase"
        );

        let empty = env().with(PASSPHRASE_ENV, "");
        let cfg = parse(MINIMAL_SSH, Path::new("/c.toml"), &empty).unwrap();
        assert!(cfg.server.ssh.unwrap().passphrase.is_none());

        let in_file = format!("{MINIMAL_SSH}passphrase = \"nope\"\n");
        assert!(matches!(parse_err(&in_file), ConfigError::Parse { .. }));
    }

    #[test]
    fn passphrase_debug_is_redacted() {
        assert_eq!(
            format!("{:?}", Passphrase::new("hunter2")),
            "Passphrase(<redacted>)"
        );
    }

    #[test]
    fn log_level_precedence_is_flag_then_env_then_file() {
        let cfg = parse_ok(&format!("{MINIMAL_SSH}[client]\nlog_level = \"warning\"\n"));
        let plain = env();
        let with_env = env().with(LOG_LEVEL_ENV, "debug");
        assert_eq!(
            effective_log_level(None, &plain, &cfg).unwrap(),
            LogLevel::Warning
        );
        assert_eq!(
            effective_log_level(None, &with_env, &cfg).unwrap(),
            LogLevel::Debug
        );
        assert_eq!(
            effective_log_level(Some(LogLevel::Error), &with_env, &cfg).unwrap(),
            LogLevel::Error
        );
        let empty_env = env().with(LOG_LEVEL_ENV, "");
        assert_eq!(
            effective_log_level(None, &empty_env, &cfg).unwrap(),
            LogLevel::Warning
        );
        let bad = env().with(LOG_LEVEL_ENV, "loud");
        assert_eq!(
            invalid_key(effective_log_level(None, &bad, &cfg).unwrap_err()),
            LOG_LEVEL_ENV
        );
    }

    #[test]
    fn load_reads_the_file_and_reports_unreadable_paths() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, MINIMAL_SSH).unwrap();
        assert_eq!(load(&path, &env()).unwrap().server.kind, "ssh");

        let err = load(&dir.path().join("absent.toml"), &env()).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }), "{err:?}");
        assert!(err.to_string().contains("absent.toml"), "{err}");
    }

    #[test]
    fn std_env_reads_the_process_environment() {
        assert!(StdEnv.var("PATH").is_some());
        assert!(!StdEnv.hostname().is_empty());
    }

    fn answers() -> InitAnswers {
        InitAnswers {
            device_name: "box".into(),
            host: "nas.local".into(),
            port: 2222,
            user: "pa".into(),
            host_key: "ssh-ed25519 AAAAkey".into(),
            identity_file: "~/.ssh/id_ed25519".into(),
            remote_path: "/data".into(),
        }
    }

    #[test]
    fn rendered_init_config_parses_back_to_the_answers() {
        let text = render(&answers());
        assert!(
            text.starts_with("# passalong configuration, written by `passalong init`."),
            "{text}"
        );
        let cfg = parse(&text, Path::new("/c.toml"), &env()).unwrap();
        assert_eq!(cfg.client.device_name, "box");
        assert_eq!(cfg.server.kind, "ssh");
        let ssh = cfg.server.ssh.unwrap();
        assert_eq!(
            (ssh.host.as_str(), ssh.port, ssh.user.as_str()),
            ("nas.local", 2222, "pa")
        );
        assert_eq!(ssh.host_key, "ssh-ed25519 AAAAkey");
        assert_eq!(ssh.identity_file, PathBuf::from("/home/u/.ssh/id_ed25519"));
        assert_eq!(ssh.remote_path, "/data");
        assert_eq!(cfg.serve.drop_folder, PathBuf::from("/home/u/PassAlong"));
    }

    #[test]
    fn rendered_values_are_escaped() {
        let mut a = answers();
        a.device_name = r#"my "box" \ 1"#.into();
        a.remote_path = "/srv/it's here".into();
        let cfg = parse(&render(&a), Path::new("/c.toml"), &env()).unwrap();
        assert_eq!(cfg.client.device_name, r#"my "box" \ 1"#);
        assert_eq!(cfg.server.ssh.unwrap().remote_path, "/srv/it's here");
    }

    #[test]
    fn init_writes_to_the_xdg_or_home_config_location() {
        let xdg = MapEnv::new()
            .with("XDG_CONFIG_HOME", "/x")
            .with("HOME", "/home/u");
        assert_eq!(
            default_config_path(&xdg),
            Some(PathBuf::from("/x/passalong/config.toml"))
        );
        let relative = MapEnv::new()
            .with("XDG_CONFIG_HOME", "rel")
            .with("HOME", "/home/u");
        assert_eq!(
            default_config_path(&relative),
            Some(PathBuf::from("/home/u/.config/passalong/config.toml"))
        );
        assert_eq!(default_config_path(&MapEnv::new()), None);
    }

    #[test]
    fn download_and_pull_settings_have_documented_defaults() {
        let cfg = parse_ok(MINIMAL_SSH);
        assert_eq!(cfg.client.download_dir, PathBuf::from("/home/u/Downloads"));
        assert!(cfg.serve.clipboard_images);
        assert!(!cfg.serve.pull);
        assert_eq!(cfg.serve.pull_interval_ms, 5000);
        assert!(cfg.serve.list_cache);
        assert_eq!(cfg.serve.list_cache_check_secs, 60);
    }

    #[test]
    fn the_key_file_defaults_beside_the_default_config() {
        let home = MapEnv::new().with("HOME", "/home/u");
        let cfg = parse(MINIMAL_SSH, Path::new("/c.toml"), &home).unwrap();
        assert_eq!(
            cfg.client.key_file.as_deref(),
            Some(Path::new("/home/u/.config/passalong/store.key"))
        );
        let xdg = MapEnv::new()
            .with("HOME", "/home/u")
            .with("XDG_CONFIG_HOME", "/x");
        let cfg = parse(MINIMAL_SSH, Path::new("/c.toml"), &xdg).unwrap();
        assert_eq!(
            cfg.client.key_file.as_deref(),
            Some(Path::new("/x/passalong/store.key"))
        );
        assert_eq!(default_key_path(&MapEnv::new()), None);
    }

    #[test]
    fn the_key_file_can_be_set_and_must_be_absolute() {
        let home = MapEnv::new().with("HOME", "/home/u");
        let with = |value: &str| {
            parse(
                &format!("{MINIMAL_SSH}\n[client]\nkey_file = \"{value}\"\n"),
                Path::new("/c.toml"),
                &home,
            )
        };
        assert_eq!(
            with("~/keys/work.key").unwrap().client.key_file.as_deref(),
            Some(Path::new("/home/u/keys/work.key"))
        );
        assert_eq!(
            with("/etc/k/store.key").unwrap().client.key_file.as_deref(),
            Some(Path::new("/etc/k/store.key"))
        );
        assert_eq!(
            invalid_key(with("keys/store.key").unwrap_err()),
            "client.key_file"
        );
    }

    #[test]
    fn download_and_pull_settings_can_be_set() {
        let text = format!(
            "{MINIMAL_SSH}\n[client]\ndownload_dir = \"~/dl\"\n\n[serve]\nclipboard_images = false\npull = true\npull_interval_ms = 1000\nlist_cache = false\nlist_cache_check_secs = 30\n"
        );
        let cfg = parse_ok(&text);
        assert_eq!(cfg.client.download_dir, PathBuf::from("/home/u/dl"));
        assert!(!cfg.serve.clipboard_images);
        assert!(cfg.serve.pull);
        assert_eq!(cfg.serve.pull_interval_ms, 1000);
        assert!(!cfg.serve.list_cache);
        assert_eq!(cfg.serve.list_cache_check_secs, 30);
    }

    #[test]
    fn invalid_download_and_pull_settings_name_their_key() {
        for (section, bad, key) in [
            (
                "[serve]\npull_interval_ms = 10",
                "",
                "serve.pull_interval_ms",
            ),
            (
                "[serve]\npull_interval_ms = 3600001",
                "",
                "serve.pull_interval_ms",
            ),
            (
                "[serve]\nlist_cache_check_secs = 9",
                "",
                "serve.list_cache_check_secs",
            ),
            (
                "[serve]\nlist_cache_check_secs = 86401",
                "",
                "serve.list_cache_check_secs",
            ),
            ("[client]\ndownload_dir = \"dl\"", "", "client.download_dir"),
            ("[client]\ndownload_dir = \"\"", "", "client.download_dir"),
        ] {
            let text = format!("{MINIMAL_SSH}\n{section}\n{bad}");
            assert_eq!(invalid_key(parse_err(&text)), key, "{section}");
        }
    }

    #[test]
    fn pulled_files_may_not_land_in_the_drop_folder() {
        for download_dir in ["/drop", "/drop/in"] {
            let text = format!(
                "{MINIMAL_SSH}\n[client]\ndownload_dir = \"{download_dir}\"\n\n[serve]\ndrop_folder = \"/drop\"\npull = true\n"
            );
            let err = parse_err(&text);
            assert!(err.to_string().contains("serve.drop_folder"), "{err}");
            assert_eq!(invalid_key(err), "client.download_dir", "{download_dir}");
        }
        // Without pull, nothing is written there automatically.
        let text = format!(
            "{MINIMAL_SSH}\n[client]\ndownload_dir = \"/drop\"\n\n[serve]\ndrop_folder = \"/drop\"\n"
        );
        assert_eq!(parse_ok(&text).client.download_dir, PathBuf::from("/drop"));
        // A sibling that merely shares a name prefix is fine.
        let text = format!(
            "{MINIMAL_SSH}\n[client]\ndownload_dir = \"/drop2\"\n\n[serve]\ndrop_folder = \"/drop\"\npull = true\n"
        );
        assert_eq!(parse_ok(&text).client.download_dir, PathBuf::from("/drop2"));
    }
}
