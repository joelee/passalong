//! Start-up and dispatch: configuration, logging, the store, then the
//! command.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use chrono::{FixedOffset, Local, Offset, Utc};
use passalong_core::clipboard::{ArboardClipboard, Clipboard, ClipboardError};
use passalong_core::config::{self, Config, EnvProvider, SearchRoots};
use passalong_core::random::StdRandom;
use passalong_core::store::BackendRegistry;
use passalong_core::telemetry::{self, LogLevel};
use tracing::Instrument;

use crate::cli::{Cli, Command, InitArgs};
use crate::commands;
use crate::commands::clipboard::TextSource;
use crate::prompt::TerminalPrompt;
use crate::resolve::{Chooser, Lookup};

/// Runs one invocation and maps the outcome to the process exit code:
/// 0 on success, 1 on any runtime error. Usage errors never get here; clap
/// exits with 2 for them.
pub async fn run(cli: Cli, env: &dyn EnvProvider) -> ExitCode {
    let mut stdout = io::stdout();
    match execute(cli, env, &mut stdout).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if let Some(commands::QuietExit(code)) = err.downcast_ref::<commands::QuietExit>() {
                return ExitCode::from(*code);
            }
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
    // `init` creates the config, so it runs before any lookup.
    if let Command::Init(args) = &cli.command {
        return init(args, cli.config.as_deref(), cli.log_level, env, out).await;
    }
    // The clipboard holder needs no configuration either.
    if cli.command == Command::HoldClipboard {
        return hold_clipboard();
    }
    let roots = SearchRoots::from_system().context("cannot determine the working directory")?;
    let located = config::locate(cli.config.as_deref(), env, &roots)?;
    let config = config::load(&located.path, env)?;
    let level = config::effective_log_level(cli.log_level, env, &config)?;
    // `init` fails only if logging is already set up, which cannot happen
    // in a fresh process.
    let _ = telemetry::init(level, io::stderr);
    tracing::debug!(path = %located.path.display(), "using config from {}", located.origin);
    let span = telemetry::op_span(cli.command.name(), &mut StdRandom::new());
    let context = commands::serve::ServeContext {
        config: &config,
        config_path: &located.path,
        log_level: level,
        env,
    };
    dispatch(cli.command, context, out).instrument(span).await
}

async fn dispatch(
    command: Command,
    context: commands::serve::ServeContext<'_>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let config: &Config = context.config;
    let mut backends = BackendRegistry::with_builtin();
    passalong_ssh::register(&mut backends);
    let device = config.client.device_name.as_str();
    match command {
        Command::Init(_) => anyhow::bail!("init runs before configuration is loaded"),
        Command::HoldClipboard => {
            anyhow::bail!("the clipboard holder runs before configuration is loaded")
        }
        // `serve` opens, and re-opens, its own store.
        Command::Serve(args) => commands::serve::run(context, &args, backends, out).await,
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
        Command::Cat { id, force } => {
            let store = backends.open(config).await?;
            let terminal = io::stdout().is_terminal();
            let (mut prompt, mut stderr) = (TerminalPrompt, io::stderr());
            let mut chooser = Chooser {
                prompt: &mut prompt,
                err: &mut stderr,
                now: Utc::now(),
            };
            let lookup = Lookup {
                input: &id,
                chooser: Some(&mut chooser),
            };
            commands::cat::run(store.as_ref(), lookup, force, terminal, out).await
        }
        Command::Delete { ids } => {
            let store = backends.open(config).await?;
            let (mut prompt, mut stderr) = (TerminalPrompt, io::stderr());
            let mut chooser = Chooser {
                prompt: &mut prompt,
                err: &mut stderr,
                now: Utc::now(),
            };
            commands::delete::run(store.as_ref(), &ids, Some(&mut chooser), out).await
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
            let mut open_clipboard =
                || -> Result<Box<dyn Clipboard>, ClipboardError> { clipboard_for_load() };
            let (mut prompt, mut stderr) = (TerminalPrompt, io::stderr());
            let mut chooser = Chooser {
                prompt: &mut prompt,
                err: &mut stderr,
                now: Utc::now(),
            };
            let lookup = Lookup {
                input: &id,
                chooser: Some(&mut chooser),
            };
            commands::load::run(
                store.as_ref(),
                lookup,
                dest.as_deref(),
                force,
                &config.client.download_dir,
                &mut open_clipboard,
                out,
            )
            .await
        }
    }
}

/// Runs `init`: logging from the flag or the environment, the target from
/// `--config` or the standard location, then the command.
async fn init(
    args: &InitArgs,
    config_flag: Option<&Path>,
    level_flag: Option<LogLevel>,
    env: &dyn EnvProvider,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let level = match level_flag {
        Some(level) => level,
        None => match env
            .var(config::LOG_LEVEL_ENV)
            .filter(|value| !value.is_empty())
        {
            Some(value) => value
                .parse()
                .with_context(|| format!("invalid {}", config::LOG_LEVEL_ENV))?,
            None => LogLevel::default(),
        },
    };
    let _ = telemetry::init(level, io::stderr);
    let target = match config_flag {
        Some(path) => path.to_path_buf(),
        None => config::default_config_path(env)
            .context("cannot choose where to write the config: set HOME or pass --config")?,
    };
    let mut backends = BackendRegistry::with_builtin();
    passalong_ssh::register(&mut backends);
    let mut prompt = TerminalPrompt;
    let deps = commands::init::InitDeps {
        env,
        prompt: &mut prompt,
        keys: &commands::init::NetworkHostKeys,
        check: &commands::init::StoreCheck(backends),
    };
    let span = telemetry::op_span("init", &mut StdRandom::new());
    commands::init::run(args, &target, deps, out)
        .instrument(span)
        .await
}

/// The clipboard `load` writes to. On Linux, clipboard content lives in the
/// process that set it, so a detached holder process keeps it after `load`
/// exits; the probe first makes sure a clipboard is reachable at all.
/// Elsewhere the system keeps the content.
fn clipboard_for_load() -> Result<Box<dyn Clipboard>, ClipboardError> {
    #[cfg(target_os = "linux")]
    {
        drop(ArboardClipboard::new()?);
        Ok(Box::new(crate::clipboard_holder::HolderClipboard::new(
            crate::clipboard_holder::ProcessLauncher,
        )))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(Box::new(ArboardClipboard::new()?))
    }
}

/// The hidden `__hold-clipboard` command: reads text from standard input and
/// holds it on the clipboard until something else replaces it.
fn hold_clipboard() -> anyhow::Result<()> {
    use std::io::Read as _;
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .context("reading the text to hold")?;
    ArboardClipboard::new()?.hold_text(&text)?;
    Ok(())
}

/// The machine's current UTC offset, for showing times in local time.
fn local_offset() -> FixedOffset {
    Local::now().offset().fix()
}
