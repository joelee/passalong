//! Command-line interface definition.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use passalong_core::telemetry::LogLevel;

fn parse_age(text: &str) -> Result<Duration, String> {
    passalong_core::retention::parse_age(text).map_err(|err| err.to_string())
}

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(
    name = "passalong",
    version,
    about = "Lightweight cross-platform clipboard and file sharing over SSH",
    propagate_version = true
)]
pub struct Cli {
    /// Use this config file instead of searching the standard locations.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Log verbosity: error, warning, info, verbose, or debug.
    #[arg(long, global = true, value_name = "LEVEL")]
    pub log_level: Option<LogLevel>,

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
    },
    /// Copy an item to DEST, or to the clipboard when DEST is omitted.
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
    },
    /// Write a config file for your SSH server, pinning its host key after
    /// you confirm its fingerprint.
    Init(InitArgs),
    /// Keeps text on the Linux clipboard after `load` exits (internal).
    #[command(name = "__hold-clipboard", hide = true)]
    HoldClipboard,
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
            Self::Serve(_) => "serve",
            Self::Delete { .. } => "delete",
            Self::Prune { .. } => "prune",
            Self::Init(_) => "init",
            Self::HoldClipboard => "hold-clipboard",
        }
    }
}

/// Options of `passalong serve`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Args)]
pub struct ServeArgs {
    /// Run in the background, logging to a file (Linux and macOS).
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

/// Options of `passalong init`. Anything not given is asked for, or takes
/// its default with `--yes`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Args)]
pub struct InitArgs {
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
    /// Take defaults instead of asking; needs --host-key or --fingerprint.
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
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn version_flag_prints_name_and_version() {
        let err = Cli::try_parse_from(["passalong", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert_eq!(err.to_string(), "passalong 0.1.1\n");
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
        assert_eq!(parse(&["list"]).command, Command::List { json: false });
        assert_eq!(
            parse(&["list", "--json"]).command,
            Command::List { json: true }
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
        assert_eq!(parse(&["__hold-clipboard"]).command, Command::HoldClipboard);
        assert_eq!(Command::HoldClipboard.name(), "hold-clipboard");
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
            })
        );
        assert_eq!(parse(&["init"]).command, Command::Init(InitArgs::default()));
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
                yes: true
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
            Command::List { json: false },
            Command::Load {
                id: "x".into(),
                dest: None,
                force: false,
            },
            Command::Cat {
                id: "x".into(),
                force: false,
            },
            Command::Serve(ServeArgs::default()),
            Command::Delete { ids: vec![] },
            Command::Init(InitArgs::default()),
            Command::Prune {
                older_than: None,
                keep: None,
                dry_run: false,
                yes: false,
            },
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
                "serve",
                "delete",
                "init",
                "prune"
            ]
        );
    }
}
