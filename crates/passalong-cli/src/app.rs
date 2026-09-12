//! Start-up and dispatch: configuration, logging, the store, then the
//! command.

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Context as _;
use chrono::{FixedOffset, Local, Offset, Utc};
use passalong_core::clipboard::{ArboardClipboard, Clipboard, ClipboardError};
use passalong_core::config::{self, Config, EnvProvider, SearchRoots};
use passalong_core::random::StdRandom;
use passalong_core::store::BackendRegistry;
use passalong_core::telemetry;
use tracing::Instrument;

use crate::cli::{Cli, Command};
use crate::commands;
use crate::commands::clipboard::TextSource;
use crate::prompt::TerminalPrompt;

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
    let mut backends = BackendRegistry::with_builtin();
    passalong_ssh::register(&mut backends);
    let device = config.client.device_name.as_str();
    match command {
        // `serve` opens, and re-opens, its own store.
        Command::Serve => commands::serve::run(config, backends).await,
        Command::Clipboard { stdin } => {
            let store = backends.open(config).await?;
            if stdin {
                let mut input = io::stdin().lock();
                commands::clipboard::run(
                    store.as_ref(),
                    TextSource::Reader(&mut input),
                    device,
                    out,
                )
                .await
            } else {
                let mut clipboard = ArboardClipboard::new()?;
                commands::clipboard::run(
                    store.as_ref(),
                    TextSource::Clipboard(&mut clipboard),
                    device,
                    out,
                )
                .await
            }
        }
        Command::File { path } => {
            let store = backends.open(config).await?;
            commands::file::run(store.as_ref(), &path, device, out).await
        }
        Command::List { json } => {
            let store = backends.open(config).await?;
            commands::list::run(store.as_ref(), json, local_offset(), out).await
        }
        Command::Delete { ids } => {
            let store = backends.open(config).await?;
            commands::delete::run(store.as_ref(), &ids, out).await
        }
        Command::Prune {
            older_than,
            keep,
            dry_run,
            yes,
        } => {
            let store = backends.open(config).await?;
            let options = commands::prune::PruneOptions {
                older_than,
                keep,
                dry_run,
                yes,
            };
            let mut prompt = TerminalPrompt;
            commands::prune::run(
                store.as_ref(),
                &options,
                Utc::now(),
                &mut prompt,
                local_offset(),
                out,
            )
            .await
        }
        Command::Load { id, dest, force } => {
            let store = backends.open(config).await?;
            let mut open_clipboard = || -> Result<Box<dyn Clipboard>, ClipboardError> {
                Ok(Box::new(ArboardClipboard::new()?))
            };
            commands::load::run(
                store.as_ref(),
                &id,
                dest.as_deref(),
                force,
                &mut open_clipboard,
                out,
            )
            .await
        }
    }
}

/// The machine's current UTC offset, for showing times in local time.
fn local_offset() -> FixedOffset {
    Local::now().offset().fix()
}
