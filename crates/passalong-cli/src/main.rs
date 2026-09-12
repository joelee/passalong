//! `passalong` command-line client.

use clap::Parser;

/// Command-line arguments for `passalong`.
#[derive(Debug, Parser)]
#[command(
    name = "passalong",
    version,
    about = "Lightweight cross-platform clipboard and file sharing over SSH"
)]
struct Cli {}

fn main() {
    // Secrets such as the SSH key passphrase live in `./.env`. The file is
    // optional and never overrides variables already set in the environment.
    if let Err(err) = dotenvy::from_path(".env")
        && !err.not_found()
    {
        eprintln!("warning: ignoring .env: {err}");
    }
    let _cli = Cli::parse();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, error::ErrorKind};

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
}
