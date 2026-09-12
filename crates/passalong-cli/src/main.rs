//! `passalong` command-line client.

mod app;
mod cli;
mod clipboard_holder;
mod commands;
mod daemon;
mod output;
mod prompt;

use std::process::ExitCode;

use clap::Parser;
use passalong_core::config::StdEnv;

fn main() -> ExitCode {
    // Secrets such as the SSH key passphrase live in `./.env`. The file is
    // optional and never overrides variables already set in the environment.
    if let Err(err) = dotenvy::from_path(".env")
        && !err.not_found()
    {
        eprintln!("warning: ignoring .env: {err}");
    }
    let cli = cli::Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: cannot start the async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(app::run(cli, &StdEnv))
}
