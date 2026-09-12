//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use passalong_core::telemetry::LogLevel;

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
    /// Keep running: send every new clipboard text and every file dropped
    /// into the drop folder.
    Serve,
}

impl Command {
    /// Stable name used in log records.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Clipboard { .. } => "clipboard",
            Self::File { .. } => "file",
            Self::List { .. } => "list",
            Self::Load { .. } => "load",
            Self::Serve => "serve",
        }
    }
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
        assert_eq!(err.to_string(), "passalong 0.1.0\n");
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
        assert_eq!(parse(&["serve"]).command, Command::Serve);
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
            Command::Serve,
        ]
        .iter()
        .map(Command::name)
        .collect();
        assert_eq!(names, ["clipboard", "file", "list", "load", "serve"]);
    }
}
