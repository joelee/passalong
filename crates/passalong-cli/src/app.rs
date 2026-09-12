//! Start-up and dispatch: configuration, logging, the store, then the
//! command.

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Context as _;
use chrono::{FixedOffset, Local, Offset};
use passalong_core::config::{self, Config, EnvProvider, SearchRoots};
use passalong_core::random::StdRandom;
use passalong_core::{store, telemetry};
use tracing::Instrument;

use crate::cli::{Cli, Command};
use crate::commands;

/// Runs one invocation and maps the outcome to the process exit code:
/// 0 on success, 1 on any runtime error. Usage errors never get here; clap
/// exits with 2 for them.
pub async fn run(cli: Cli, env: &dyn EnvProvider) -> ExitCode {
    let mut stdout = io::stdout();
    match execute(cli, env, &mut stdout).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // The `error:` line below is the user-facing report; the log
            // record is kept at verbose level so it is not printed twice.
            let message = format!("{err:#}");
            tracing::debug!(error = message.as_str(), "command failed");
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn execute(cli: Cli, env: &dyn EnvProvider, out: &mut dyn Write) -> anyhow::Result<()> {
    let roots = SearchRoots::from_system().context("cannot determine the working directory")?;
    let located = config::locate(cli.config.as_deref(), env, &roots)?;
    let config = config::load(&located.path, env)?;
    let level = config::effective_log_level(cli.log_level, env, &config)?;
    // `init` fails only if logging is already set up, which cannot happen
    // in a fresh process.
    let _ = telemetry::init(level, io::stderr);
    tracing::debug!(path = %located.path.display(), "using config from {}", located.origin);
    let span = telemetry::op_span(cli.command.name(), &mut StdRandom::new());
    dispatch(cli.command, &config, out).instrument(span).await
}

async fn dispatch(command: Command, config: &Config, out: &mut dyn Write) -> anyhow::Result<()> {
    let store = store::open_store(config).await?;
    match command {
        Command::List { json } => {
            commands::list::run(store.as_ref(), json, local_offset(), out).await
        }
        other => anyhow::bail!("`{}` is not implemented yet", other.name()),
    }
}

/// The machine's current UTC offset, for showing times in local time.
fn local_offset() -> FixedOffset {
    Local::now().offset().fix()
}
