//! `passalong serve`: run until stopped, sending new clipboard text and
//! files dropped into the drop folder; with `--daemon`, in the background.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use anyhow::Context as _;
use passalong_core::cache::{self, ListCache};
use passalong_core::clipboard::{ArboardClipboard, Clipboard};
use passalong_core::clock::SystemClock;
use passalong_core::config::{Config, EnvProvider};
use passalong_core::serve::{self, ServeOptions, StoreOpener};
use passalong_core::store::{BackendFuture, BackendRegistry};
use passalong_core::telemetry::LogLevel;
use tokio::sync::{oneshot, watch};

use crate::cli::ServeArgs;
use crate::commands::QuietExit;
use crate::daemon::{self, Os, PidLock, StatePaths, Status};

// `--daemon` and `--stop` do not run on Windows yet, so what only they use
// is unused there.
/// How long `--daemon` waits for the background process to start.
#[cfg_attr(not(unix), allow(dead_code))]
const START_TIMEOUT: Duration = Duration::from_secs(5);
/// How long `--stop` waits for `serve` to exit.
#[cfg_attr(not(unix), allow(dead_code))]
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg_attr(not(unix), allow(dead_code))]
const POLL: Duration = Duration::from_millis(100);

/// What `serve` needs from start-up.
pub struct ServeContext<'a> {
    /// The loaded configuration.
    pub config: &'a Config,
    /// Where it was loaded from, passed on to the background process.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub config_path: &'a Path,
    /// The effective log level, passed on to the background process.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub log_level: LogLevel,
    /// Environment, for the pid and log file locations.
    pub env: &'a dyn EnvProvider,
}

/// Runs `serve` in the mode `args` selects.
pub async fn run(
    context: ServeContext<'_>,
    args: &ServeArgs,
    backends: BackendRegistry,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let paths = StatePaths::resolve(context.env, Os::current())
        .context("cannot choose a place for serve's pid and log files: set HOME")?;
    if args.status {
        return report_status(&paths, out);
    }
    if args.stop {
        return stop(&paths, out).await;
    }
    if args.daemon {
        return start_daemon(&paths, &context, out).await;
    }
    run_foreground(context.config, backends, &paths, args.daemon_child).await
}

async fn run_foreground(
    config: &Config,
    backends: BackendRegistry,
    paths: &StatePaths,
    daemon_child: bool,
) -> anyhow::Result<()> {
    let lock = PidLock::acquire(&paths.pid)?;
    #[cfg(unix)]
    let _hangup = if daemon_child {
        // Registering a handler replaces the default action, so a hang-up
        // no longer stops the background process.
        Some(tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::hangup(),
        )?)
    } else {
        None
    };
    #[cfg(not(unix))]
    let _ = daemon_child;
    let clipboard: Option<Box<dyn Clipboard>> = match ArboardClipboard::new() {
        Ok(clipboard) => Some(Box::new(clipboard)),
        Err(err) => {
            tracing::warn!(error = %err, "clipboard unavailable");
            None
        }
    };
    let owned = config.clone();
    let open_store: StoreOpener = Arc::new(move || -> BackendFuture<'static> {
        let config = owned.clone();
        let backends = backends.clone();
        Box::pin(async move { backends.open(&config).await })
    });
    let (stop, stopped) = watch::channel(false);
    tokio::spawn(async move {
        wait_for_stop_signal().await;
        tracing::info!("stop requested");
        let _ = stop.send(true);
    });
    let (ready_tx, ready_rx) = oneshot::channel();
    let lock = Arc::new(lock);
    let marker = Arc::clone(&lock);
    let refresh = list_cache_identity(config).map(|identity| {
        let interval = Duration::from_secs(config.serve.list_cache_check_secs);
        tracing::info!(
            path = %paths.cache.display(),
            every_secs = interval.as_secs(),
            "keeping the list cache current"
        );
        cache::refresh_loop(
            open_store.clone(),
            paths.cache.clone(),
            identity,
            interval,
            Arc::new(SystemClock),
            stopped.clone(),
        )
    });
    tokio::spawn(async move {
        if ready_rx.await.is_err() {
            return;
        }
        if let Err(err) = marker.mark_ready() {
            tracing::warn!(error = %err, "cannot mark serve as ready in the pid file");
        }
        drop(marker);
        // Started once serve is, so a store that cannot be opened fails
        // start-up first.
        if let Some(refresh) = refresh {
            refresh.await;
        }
    });
    serve::run_with_ready(
        ServeOptions::from_config(config),
        clipboard,
        open_store,
        stopped,
        ready_tx,
    )
    .await?;
    drop(lock);
    Ok(())
}

/// The store whose list `serve` keeps, when it keeps one: the ssh backend,
/// with `serve.list_cache` on.
fn list_cache_identity(config: &Config) -> Option<String> {
    ListCache::store_identity(config).filter(|_| config.serve.list_cache)
}

fn report_status(paths: &StatePaths, out: &mut dyn Write) -> anyhow::Result<()> {
    match daemon::status(&paths.pid)? {
        Status::Running { pid, .. } => {
            let pid = pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string());
            writeln!(out, "running (pid {pid}, log {})", paths.log.display())?;
            Ok(())
        }
        Status::NotRunning => {
            writeln!(out, "not running")?;
            Err(QuietExit(3).into())
        }
    }
}

#[cfg(unix)]
async fn stop(paths: &StatePaths, out: &mut dyn Write) -> anyhow::Result<()> {
    let pid = match daemon::status(&paths.pid)? {
        Status::NotRunning => {
            writeln!(out, "not running")?;
            return Ok(());
        }
        Status::Running { pid, .. } => {
            pid.context("serve is running but its pid file is unreadable")?
        }
    };
    let signalled = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .context("cannot run `kill`")?;
    anyhow::ensure!(signalled.success(), "`kill -TERM {pid}` failed");
    let deadline = Instant::now() + STOP_TIMEOUT;
    while daemon::status(&paths.pid)? != Status::NotRunning {
        anyhow::ensure!(
            Instant::now() < deadline,
            "serve (pid {pid}) is still running after {} s",
            STOP_TIMEOUT.as_secs()
        );
        tokio::time::sleep(POLL).await;
    }
    writeln!(out, "stopped")?;
    Ok(())
}

#[cfg(not(unix))]
async fn stop(_paths: &StatePaths, _out: &mut dyn Write) -> anyhow::Result<()> {
    anyhow::bail!("serve --stop is supported on Linux and macOS only")
}

#[cfg(unix)]
async fn start_daemon(
    paths: &StatePaths,
    context: &ServeContext<'_>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if let Status::Running { pid, .. } = daemon::status(&paths.pid)? {
        return Err(daemon::LockError::AlreadyRunning(pid).into());
    }
    if let Some(dir) = paths.log.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
        .with_context(|| format!("cannot open {}", paths.log.display()))?;
    let offset = log.metadata().map(|meta| meta.len()).unwrap_or(0);
    let exe = std::env::current_exe().context("cannot find the passalong executable")?;
    let config_path = std::path::absolute(context.config_path)?;
    let mut child = daemon::detached_command(&exe)
        .arg("--config")
        .arg(&config_path)
        .args([
            "--log-level",
            context.log_level.as_str(),
            "serve",
            "--daemon-child",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(log))
        .spawn()
        .context("cannot start the background process")?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            let tail = daemon::log_tail(&paths.log, offset, 8);
            anyhow::bail!("serve stopped during start-up ({status}); last log lines:\n{tail}");
        }
        if let Status::Running {
            pid: Some(pid),
            ready: true,
        } = daemon::status(&paths.pid)?
        {
            writeln!(
                out,
                "serve started (pid {pid}, log {})",
                paths.log.display()
            )?;
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "serve did not finish starting within {} s; see {}",
            START_TIMEOUT.as_secs(),
            paths.log.display()
        );
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(not(unix))]
async fn start_daemon(
    _paths: &StatePaths,
    _context: &ServeContext<'_>,
    _out: &mut dyn Write,
) -> anyhow::Result<()> {
    anyhow::bail!("serve --daemon is supported on Linux and macOS only")
}

/// Resolves on Ctrl-C, or on SIGTERM, which systemd and launchd send.
async fn wait_for_stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use passalong_core::testing::MapEnv;

    fn parse(text: &str) -> Config {
        let text = format!("[client]\ndevice_name = \"t\"\n\n{text}");
        let env = MapEnv::new().with("HOME", "/home/u");
        passalong_core::config::parse(&text, Path::new("/c.toml"), &env).unwrap()
    }

    const SSH: &str = "[server]\nkind = \"ssh\"\n\n[server.ssh]\nhost = \"nas\"\nport = 22\nuser = \"pa\"\nhost_key = \"ssh-ed25519 AAAAkey\"\nidentity_file = \"/keys/id\"\nremote_path = \"/srv/passalong\"\n";

    #[test]
    fn only_ssh_stores_with_the_cache_on_get_a_list_cache() {
        assert_eq!(
            list_cache_identity(&parse(SSH)).as_deref(),
            Some("ssh pa@nas:22 /srv/passalong plain")
        );
        let off = parse(&format!("{SSH}\n[serve]\nlist_cache = false\n"));
        assert_eq!(list_cache_identity(&off), None);
        let local = parse("[server]\nkind = \"local\"\n\n[server.local]\npath = \"/srv/share\"\n");
        assert_eq!(list_cache_identity(&local), None);
    }
}
