//! Command-line interface definition.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use passalong_core::telemetry::LogLevel;

fn parse_age(text: &str) -> Result<Duration, String> {
    passalong_core::retention::parse_age(text).map_err(|err| err.to_string())
}

/// What passalong is, for `--help` and the `choose` help.
pub const ABOUT: &str = "Lightweight cross-platform clipboard and file sharing over SSH";

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(
    name = "passalong",
    version,
    about = ABOUT,
    propagate_version = true
)]
pub struct Cli {
    /// Use this config file instead of searching the standard locations.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Log verbosity: error, warning, info, verbose, or debug.
    #[arg(long, global = true, value_name = "LEVEL")]
    pub log_level: Option<LogLevel>,

    /// Print nothing but errors and prompts; `cat` still prints the item.
    /// Scripts can rely on the exit code.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The subcommands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Send the clipboard's text.
    Clipboard {
        /// Read the text from standard input instead of the clipboard.
        #[arg(long)]
        stdin: bool,
    },
    /// Send a file.
    File {
        /// The file to send.
        path: PathBuf,
    },
    /// List stored items, newest first.
    List {
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
        /// Read the server even when the list cache is fresh, and rewrite
        /// the cache.
        #[arg(long)]
        nocache: bool,
    },
    /// Copy an item to DEST. Without DEST, text goes to the clipboard and
    /// files to the download directory.
    Load {
        /// The item's id, or at least 4 characters of it.
        id: String,
        /// File or directory to write the item to.
        dest: Option<PathBuf>,
        /// Overwrite the target file if it exists.
        #[arg(long)]
        force: bool,
    },
    /// Print an item's content to standard output.
    Cat {
        /// The item's id, or at least 4 characters of it.
        id: String,
        /// Print a binary item even when standard output is a terminal.
        #[arg(long)]
        force: bool,
    },
    /// Print an item's metadata.
    Get {
        /// The item's id, or at least 4 characters of it.
        id: String,
        /// Print JSON instead of one field per line.
        #[arg(long)]
        json: bool,
    },
    /// Keep running: send every new clipboard text and every file dropped
    /// into the drop folder.
    Serve(ServeArgs),
    /// Delete items by age or count; asks for confirmation.
    Prune {
        /// Delete items created at least this long ago, such as 30d
        /// (units: m, h, d, w).
        #[arg(long, value_name = "AGE", value_parser = parse_age)]
        older_than: Option<Duration>,
        /// Always keep this many of the newest items.
        #[arg(long, value_name = "N")]
        keep: Option<usize>,
        /// Show what would be deleted without deleting anything.
        #[arg(long)]
        dry_run: bool,
        /// Delete without asking; required when not running in a terminal.
        #[arg(long)]
        yes: bool,
        /// Prune the unencrypted items a fresh start left in plain/, not
        /// the store's items.
        #[arg(long)]
        plain: bool,
    },
    /// Write a config file for your SSH server, pinning its host key after
    /// you confirm its fingerprint.
    Init(InitArgs),
    /// Check the configuration, and that the server can be reached, read,
    /// and written.
    Check,
    /// Encrypt the store, or change its words; with --join, give this
    /// device the key of an encrypted store.
    Encrypt(EncryptArgs),
    /// Install `passalong serve` as a service that starts at login: a
    /// systemd user unit on Linux, a launchd agent on macOS.
    ServiceInstall(ServiceInstallArgs),
    /// Stop, disable, and remove the service `service-install` installed.
    ServiceRemove,
    /// Pick an item from a full-screen list, then load, print, show, or
    /// delete it.
    Choose,
    /// Keeps text on the Linux clipboard after `load` exits (internal).
    #[command(name = "__hold-clipboard", hide = true)]
    HoldClipboard {
        /// Hold an image, read as PNG from standard input, instead of text.
        #[arg(long)]
        image: bool,
    },
    /// Delete items from the store.
    Delete {
        /// The items: full ids, or at least 4 characters of each.
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

impl Command {
    /// Stable name used in log records.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Clipboard { .. } => "clipboard",
            Self::File { .. } => "file",
            Self::List { .. } => "list",
            Self::Load { .. } => "load",
            Self::Cat { .. } => "cat",
            Self::Get { .. } => "get",
            Self::Serve(_) => "serve",
            Self::Delete { .. } => "delete",
            Self::Prune { .. } => "prune",
            Self::Init(_) => "init",
            Self::Check => "check",
            Self::Encrypt(_) => "encrypt",
            Self::ServiceInstall(_) => "service-install",
            Self::ServiceRemove => "service-remove",
            Self::Choose => "choose",
            Self::HoldClipboard { .. } => "hold-clipboard",
        }
    }
}

/// Options of `passalong serve`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Args)]
pub struct ServeArgs {
    /// Run in the background, logging to a file.
    #[arg(long, conflicts_with_all = ["status", "stop"])]
    pub daemon: bool,
    /// Report whether serve is running; exit code 3 when it is not.
    #[arg(long, conflicts_with = "stop")]
    pub status: bool,
    /// Stop the running serve.
    #[arg(long)]
    pub stop: bool,
    /// Set by `--daemon` on the background process it starts.
    #[arg(long, hide = true)]
    pub daemon_child: bool,
}

/// Options of `passalong service-install`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Args)]
pub struct ServiceInstallArgs {
    /// Write the unit, but do not enable or start it.
    #[arg(long)]
    pub no_start: bool,
    /// Replace an installed unit that differs.
    #[arg(long)]
    pub force: bool,
    /// Windows: register a Task Scheduler task, which restarts serve if it
    /// fails, instead of the Run key; needs an administrator prompt.
    #[arg(long)]
    pub scheduler: bool,
}

/// Options of `passalong encrypt`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Args)]
pub struct EncryptArgs {
    /// Give this device the key of an encrypted store, by typing its six
    /// words.
    #[arg(long, conflicts_with_all = ["rotate", "recover"])]
    pub join: bool,
    /// Replace the store's key and words, re-encrypting every item; every
    /// other device must join again. Use it after losing a device.
    #[arg(long, conflicts_with = "recover")]
    pub rotate: bool,
    /// Finish or undo a re-encryption that was interrupted.
    #[arg(long)]
    pub recover: bool,
}

/// The kinds of store `passalong init` sets up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Backend {
    /// An SSH server.
    Ssh,
    /// A passalong-server.
    Https,
}

/// Options of `passalong init`. Anything not given is asked for, or takes
/// its default with `--yes`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Args)]
pub struct InitArgs {
    /// The kind of server [default: ssh, or https with --url].
    #[arg(long, value_enum)]
    pub backend: Option<Backend>,
    /// A passalong-server's URL, such as https://box.example:8443.
    #[arg(long, conflicts_with_all = ["host", "port", "user", "identity_file", "remote_path", "host_key", "fingerprint"])]
    pub url: Option<String>,
    /// Accept the passalong-server's certificate only if its public key has
    /// this pin, as `passalong-server tls fingerprint` prints it.
    #[arg(long, value_name = "sha256/...")]
    pub tls_pin: Option<String>,
    /// Where this device keeps its passalong-server API key [default:
    /// api.key beside the config file]. A key already there is used.
    #[arg(long, value_name = "PATH")]
    pub api_key_file: Option<String>,
    /// Server host name or address.
    #[arg(long)]
    pub host: Option<String>,
    /// SSH port [default: 22].
    #[arg(long)]
    pub port: Option<u16>,
    /// Login user on the server [default: passalong].
    #[arg(long)]
    pub user: Option<String>,
    /// Private key for logging in [default: ~/.ssh/id_ed25519].
    #[arg(long, value_name = "PATH")]
    pub identity_file: Option<String>,
    /// Storage directory on the server [default: /srv/passalong].
    #[arg(long, value_name = "PATH")]
    pub remote_path: Option<String>,
    /// Name recorded on items this device sends [default: host name].
    #[arg(long, value_name = "NAME")]
    pub device_name: Option<String>,
    /// Pin this host key instead of fetching it from the server.
    #[arg(long, value_name = "KEY")]
    pub host_key: Option<String>,
    /// Accept the fetched host key only if it has this SHA-256 fingerprint.
    #[arg(long, value_name = "SHA256:...")]
    pub fingerprint: Option<String>,
    /// Take defaults instead of asking; needs --host-key or --fingerprint,
    /// or for a passalong-server, --tls-pin or a certificate the system
    /// trusts.
    #[arg(long)]
    pub yes: bool,
    /// Replace an existing config file.
    #[arg(long)]
    pub force: bool,
    /// Do not test the connection after writing the config.
    #[arg(long)]
    pub no_test: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser, error::ErrorKind};
    use passalong_core::telemetry::LogLevel;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("passalong").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn parses_encrypt_and_prune_plain() {
        assert_eq!(
            parse(&["encrypt"]).command,
            Command::Encrypt(EncryptArgs::default())
        );
        assert_eq!(
            parse(&["encrypt", "--join"]).command,
            Command::Encrypt(EncryptArgs {
                join: true,
                ..EncryptArgs::default()
            })
        );
        assert!(matches!(
            parse(&["encrypt", "--rotate"]).command,
            Command::Encrypt(EncryptArgs { rotate: true, .. })
        ));
        assert!(matches!(
            parse(&["encrypt", "--recover"]).command,
            Command::Encrypt(EncryptArgs { recover: true, .. })
        ));
        for pair in [
            ["--join", "--rotate"],
            ["--join", "--recover"],
            ["--rotate", "--recover"],
        ] {
            assert!(Cli::try_parse_from(["passalong", "encrypt", pair[0], pair[1]]).is_err());
        }
        assert_eq!(parse(&["encrypt"]).command.name(), "encrypt");
        assert!(matches!(
            parse(&["prune", "--plain", "--keep", "0"]).command,
            Command::Prune { plain: true, .. }
        ));
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn version_flag_prints_name_and_version() {
        let err = Cli::try_parse_from(["passalong", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert_eq!(err.to_string(), "passalong 0.2.1\n");
    }

    #[test]
    fn parses_every_subcommand() {
        assert_eq!(
            parse(&["clipboard"]).command,
            Command::Clipboard { stdin: false }
        );
        assert_eq!(
            parse(&["clipboard", "--stdin"]).command,
            Command::Clipboard { stdin: true }
        );
        assert_eq!(
            parse(&["file", "a.txt"]).command,
            Command::File {
                path: "a.txt".into()
            }
        );
        assert_eq!(
            parse(&["list"]).command,
            Command::List {
                json: false,
                nocache: false
            }
        );
        assert_eq!(
            parse(&["list", "--json", "--nocache"]).command,
            Command::List {
                json: true,
                nocache: true
            }
        );
        assert_eq!(
            parse(&["load", "2cf2"]).command,
            Command::Load {
                id: "2cf2".into(),
                dest: None,
                force: false
            }
        );
        assert_eq!(
            parse(&["load", "2cf2", "out", "--force"]).command,
            Command::Load {
                id: "2cf2".into(),
                dest: Some("out".into()),
                force: true
            }
        );
        assert_eq!(
            parse(&["serve"]).command,
            Command::Serve(ServeArgs::default())
        );
        assert_eq!(
            parse(&["serve", "--daemon"]).command,
            Command::Serve(ServeArgs {
                daemon: true,
                ..ServeArgs::default()
            })
        );
        assert_eq!(
            parse(&["serve", "--status"]).command,
            Command::Serve(ServeArgs {
                status: true,
                ..ServeArgs::default()
            })
        );
        assert_eq!(
            parse(&["serve", "--stop"]).command,
            Command::Serve(ServeArgs {
                stop: true,
                ..ServeArgs::default()
            })
        );
        assert!(Cli::try_parse_from(["passalong", "serve", "--daemon", "--stop"]).is_err());
        assert!(Cli::try_parse_from(["passalong", "serve", "--status", "--stop"]).is_err());
        let help = Cli::command()
            .find_subcommand_mut("serve")
            .unwrap()
            .render_help()
            .to_string();
        assert!(!help.contains("daemon-child"), "{help}");
        assert_eq!(
            parse(&["__hold-clipboard"]).command,
            Command::HoldClipboard { image: false }
        );
        assert_eq!(
            parse(&["__hold-clipboard", "--image"]).command,
            Command::HoldClipboard { image: true }
        );
        assert_eq!(
            Command::HoldClipboard { image: false }.name(),
            "hold-clipboard"
        );
        let top = Cli::command().render_help().to_string();
        assert!(!top.contains("hold-clipboard"), "{top}");
        let init = parse(&[
            "init",
            "--host",
            "nas",
            "--port",
            "2222",
            "--user",
            "pa",
            "--identity-file",
            "~/.ssh/k",
            "--remote-path",
            "/data",
            "--device-name",
            "lap",
            "--fingerprint",
            "SHA256:abc",
            "--yes",
            "--force",
            "--no-test",
        ]);
        assert_eq!(
            init.command,
            Command::Init(InitArgs {
                host: Some("nas".into()),
                port: Some(2222),
                user: Some("pa".into()),
                identity_file: Some("~/.ssh/k".into()),
                remote_path: Some("/data".into()),
                device_name: Some("lap".into()),
                host_key: None,
                fingerprint: Some("SHA256:abc".into()),
                yes: true,
                force: true,
                no_test: true,
                ..InitArgs::default()
            })
        );
        assert_eq!(parse(&["init"]).command, Command::Init(InitArgs::default()));
        assert_eq!(
            parse(&[
                "init",
                "--url",
                "https://box.example",
                "--tls-pin",
                "sha256/abc",
                "--api-key-file",
                "/k/api.key",
            ])
            .command,
            Command::Init(InitArgs {
                url: Some("https://box.example".into()),
                tls_pin: Some("sha256/abc".into()),
                api_key_file: Some("/k/api.key".into()),
                ..InitArgs::default()
            })
        );
        assert_eq!(
            parse(&["init", "--backend", "https"]).command,
            Command::Init(InitArgs {
                backend: Some(Backend::Https),
                ..InitArgs::default()
            })
        );
        assert!(
            Cli::try_parse_from(["passalong", "init", "--url", "https://b", "--host", "nas"])
                .is_err(),
            "a URL is not for SSH"
        );
        assert_eq!(
            parse(&[
                "prune",
                "--older-than",
                "30d",
                "--keep",
                "5",
                "--dry-run",
                "--yes"
            ])
            .command,
            Command::Prune {
                older_than: Some(std::time::Duration::from_secs(30 * 86_400)),
                keep: Some(5),
                dry_run: true,
                yes: true,
                plain: false,
            }
        );
        let err = Cli::try_parse_from(["passalong", "prune", "--older-than", "soon"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
        assert_eq!(
            parse(&["delete", "2cf2", "6aa52107-2c"]).command,
            Command::Delete {
                ids: vec!["2cf2".into(), "6aa52107-2c".into()]
            }
        );
        assert!(
            Cli::try_parse_from(["passalong", "delete"]).is_err(),
            "delete needs at least one id"
        );
        assert_eq!(
            parse(&["cat", "2cf2", "--force"]).command,
            Command::Cat {
                id: "2cf2".into(),
                force: true
            }
        );
        assert!(Cli::try_parse_from(["passalong", "cat"]).is_err());
        assert_eq!(
            parse(&["get", "2cf2"]).command,
            Command::Get {
                id: "2cf2".into(),
                json: false
            }
        );
        assert_eq!(
            parse(&["get", "2cf2", "--json"]).command,
            Command::Get {
                id: "2cf2".into(),
                json: true
            }
        );
        assert!(Cli::try_parse_from(["passalong", "get"]).is_err());
        assert_eq!(parse(&["check"]).command, Command::Check);
        assert_eq!(
            parse(&["service-install"]).command,
            Command::ServiceInstall(ServiceInstallArgs::default())
        );
        assert_eq!(
            parse(&["service-install", "--no-start", "--force"]).command,
            Command::ServiceInstall(ServiceInstallArgs {
                no_start: true,
                force: true,
                scheduler: false,
            })
        );
        assert_eq!(
            parse(&["service-install", "--scheduler"]).command,
            Command::ServiceInstall(ServiceInstallArgs {
                scheduler: true,
                ..ServiceInstallArgs::default()
            })
        );
        assert_eq!(parse(&["service-remove"]).command, Command::ServiceRemove);
        assert_eq!(parse(&["choose"]).command, Command::Choose);
        assert!(Cli::try_parse_from(["passalong", "install-service"]).is_err());
        assert!(Cli::try_parse_from(["passalong", "service-install", "--uninstall"]).is_err());
    }

    #[test]
    fn global_options_go_before_or_after_the_subcommand() {
        let cli = parse(&["list", "--config", "c.toml", "--log-level", "Verbose"]);
        assert_eq!(cli.config.as_deref(), Some(std::path::Path::new("c.toml")));
        assert_eq!(cli.log_level, Some(LogLevel::Verbose));
        let cli = parse(&["--log-level", "debug", "serve"]);
        assert_eq!(cli.log_level, Some(LogLevel::Debug));
        assert_eq!(cli.config, None);
    }

    #[test]
    fn quiet_is_a_global_flag() {
        assert!(!parse(&["list"]).quiet);
        assert!(parse(&["-q", "list"]).quiet);
        assert!(parse(&["list", "--quiet"]).quiet);
        assert!(parse(&["cat", "2cf2", "-q"]).quiet);
    }

    #[test]
    fn rejects_unknown_levels_and_a_missing_subcommand() {
        let err = Cli::try_parse_from(["passalong", "--log-level", "loud", "list"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
        assert!(
            err.to_string()
                .contains("error, warning, info, verbose, debug"),
            "{err}"
        );
        assert!(Cli::try_parse_from(["passalong"]).is_err());
        assert!(Cli::try_parse_from(["passalong", "file"]).is_err());
    }

    #[test]
    fn commands_have_stable_names_for_logs() {
        let names: Vec<_> = [
            Command::Clipboard { stdin: false },
            Command::File { path: "f".into() },
            Command::List {
                json: false,
                nocache: false,
            },
            Command::Load {
                id: "x".into(),
                dest: None,
                force: false,
            },
            Command::Cat {
                id: "x".into(),
                force: false,
            },
            Command::Get {
                id: "x".into(),
                json: false,
            },
            Command::Serve(ServeArgs::default()),
            Command::Delete { ids: vec![] },
            Command::Init(InitArgs::default()),
            Command::Prune {
                older_than: None,
                keep: None,
                dry_run: false,
                yes: false,
                plain: false,
            },
            Command::Check,
            Command::ServiceInstall(ServiceInstallArgs::default()),
            Command::ServiceRemove,
            Command::Choose,
        ]
        .iter()
        .map(Command::name)
        .collect();
        assert_eq!(
            names,
            [
                "clipboard",
                "file",
                "list",
                "load",
                "cat",
                "get",
                "serve",
                "delete",
                "init",
                "prune",
                "check",
                "service-install",
                "service-remove",
                "choose"
            ]
        );
    }
}
