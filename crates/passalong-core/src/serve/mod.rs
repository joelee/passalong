//! `passalong serve`: send every new clipboard text and every file dropped
//! into the drop folder, until told to stop.
//!
//! Three tasks cooperate:
//!
//! - the clipboard watcher polls the clipboard and queues new text;
//! - the drop watcher scans the drop folder whenever the operating system
//!   reports a change, and at least every `rescan_interval` in case events
//!   are missed, queueing each file once it has stopped changing;
//! - the uploader, running in [`run`]'s own task, sends queued jobs one at a
//!   time, retrying each with a freshly opened store until it succeeds.

pub mod clipboard_watcher;
pub mod drop_watcher;
pub mod retry;
pub mod upload;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, MissedTickBehavior};

use crate::clipboard::Clipboard;
use crate::config::{AfterSend, Config};
use crate::store::{BackendFuture, StoreError};

use self::clipboard_watcher::ClipboardWatcher;
use self::drop_watcher::DropTracker;
use self::upload::{JobOutcome, Uploader};

/// Folder inside the drop folder that sent files move to.
pub const SENT_DIR: &str = "sent";
const DEFAULT_RESCAN_INTERVAL: Duration = Duration::from_secs(5);
const QUEUE_DEPTH: usize = 64;

/// Opens a fresh store. `serve` calls it at start-up and again before every
/// retry, so a dropped SSH connection is re-established.
pub type StoreOpener = Arc<dyn Fn() -> BackendFuture<'static> + Send + Sync>;

/// One thing to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// Clipboard text.
    Text(String),
    /// A file in the drop folder.
    File(PathBuf),
}

/// How `serve` behaves; normally built with [`ServeOptions::from_config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOptions {
    /// Folder watched for files to send.
    pub drop_folder: PathBuf,
    /// How often the clipboard is read.
    pub clipboard_poll_interval: Duration,
    /// How long a file must stay unchanged before it is sent.
    pub file_stable_wait: Duration,
    /// Longest time between two scans of the drop folder.
    pub rescan_interval: Duration,
    /// What happens to a file once it is sent.
    pub after_send: AfterSend,
    /// Device name recorded on sent items.
    pub device: String,
}

impl ServeOptions {
    /// Options from `[serve]` and `client.device_name`, rescanning the drop
    /// folder at least every 5 seconds.
    pub fn from_config(config: &Config) -> Self {
        Self {
            drop_folder: config.serve.drop_folder.clone(),
            clipboard_poll_interval: Duration::from_millis(config.serve.clipboard_poll_interval_ms),
            file_stable_wait: Duration::from_millis(config.serve.file_stable_wait_ms),
            rescan_interval: DEFAULT_RESCAN_INTERVAL,
            after_send: config.serve.after_send,
            device: config.client.device_name.clone(),
        }
    }
}

/// Reasons `serve` cannot start. Once running, it never stops because of a
/// single failed item.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The store could not be opened at start-up.
    #[error("cannot open the store: {0}")]
    Store(#[from] StoreError),
    /// The drop folder or its `sent` folder could not be created.
    #[error("cannot prepare the drop folder {path}: {message}")]
    DropFolder {
        /// The drop folder.
        path: String,
        /// Underlying error.
        message: String,
    },
    /// The operating system refused to watch the drop folder.
    #[error("cannot watch the drop folder {path}: {message}")]
    Watch {
        /// The drop folder.
        path: String,
        /// Underlying error.
        message: String,
    },
}

/// Runs until `shutdown` becomes `true` or its sender is dropped. The store
/// is opened first so configuration and connection problems fail fast.
///
/// # Errors
///
/// A [`ServeError`] when start-up fails; after that, failures are logged
/// and retried.
pub async fn run(
    options: ServeOptions,
    clipboard: Option<Box<dyn Clipboard>>,
    open_store: StoreOpener,
    shutdown: watch::Receiver<bool>,
) -> Result<(), ServeError> {
    let (ready, _) = oneshot::channel();
    run_with_ready(options, clipboard, open_store, shutdown, ready).await
}

/// Like [`run`], and sends on `ready` once start-up has succeeded: the store
/// is open, the drop folder exists and is watched, and the watchers run.
/// `serve --daemon` uses it to report a successful start.
///
/// # Errors
///
/// As [`run`].
pub async fn run_with_ready(
    options: ServeOptions,
    clipboard: Option<Box<dyn Clipboard>>,
    open_store: StoreOpener,
    mut shutdown: watch::Receiver<bool>,
    ready: oneshot::Sender<()>,
) -> Result<(), ServeError> {
    let store = open_store().await?;
    let folder = options.drop_folder.clone();
    let shown = folder.display().to_string();
    let sent_dir = folder.join(SENT_DIR);
    tokio::fs::create_dir_all(&sent_dir)
        .await
        .map_err(|err| ServeError::DropFolder {
            path: shown.clone(),
            message: err.to_string(),
        })?;
    let (changed_tx, changed_rx) = mpsc::channel(1);
    let _watcher =
        drop_watcher::watch_folder(&folder, changed_tx).map_err(|err| ServeError::Watch {
            path: shown.clone(),
            message: err.to_string(),
        })?;

    let (jobs_tx, mut jobs_rx) = mpsc::channel(QUEUE_DEPTH);
    let mut tasks = Vec::new();
    match clipboard {
        Some(clipboard) => tasks.push(tokio::spawn(clipboard_loop(
            clipboard,
            options.clipboard_poll_interval,
            jobs_tx.clone(),
            shutdown.clone(),
        ))),
        None => tracing::warn!("no clipboard available; only the drop folder is watched"),
    }
    let (skipped_tx, skipped_rx) = mpsc::unbounded_channel();
    tasks.push(tokio::spawn(drop_loop(
        folder,
        options.file_stable_wait,
        options.rescan_interval,
        jobs_tx,
        changed_rx,
        skipped_rx,
        shutdown.clone(),
    )));
    tracing::info!(
        path = shown.as_str(),
        "serving: watching the clipboard and the drop folder"
    );

    // Nobody may be waiting for readiness; that is fine.
    let _ = ready.send(());
    let mut uploader = Uploader::new(
        store,
        open_store,
        options.device,
        options.after_send,
        sent_dir,
    );
    loop {
        tokio::select! {
            () = stopped(&mut shutdown) => break,
            job = jobs_rx.recv() => match job {
                Some(job) => {
                    let file = match &job {
                        Job::File(path) => Some(path.clone()),
                        Job::Text(_) => None,
                    };
                    if uploader.handle(job, &mut shutdown).await == JobOutcome::Skipped
                        && let Some(path) = file
                    {
                        // The drop watcher offers it again once it changes.
                        let _ = skipped_tx.send(path);
                    }
                }
                None => break,
            },
        }
    }
    for task in tasks {
        task.abort();
    }
    tracing::info!("serve stopped");
    Ok(())
}

/// Resolves once a stop is requested: the flag turns `true`, or its sender
/// is dropped.
pub(crate) async fn stopped(shutdown: &mut watch::Receiver<bool>) {
    // `wait_for` fails only when the sender is gone, which also means stop.
    let _ = shutdown.wait_for(|stop| *stop).await;
}

async fn clipboard_loop(
    mut clipboard: Box<dyn Clipboard>,
    interval: Duration,
    jobs: mpsc::Sender<Job>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut watcher = ClipboardWatcher::new();
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // Log a failing clipboard once per distinct error, not on every poll.
    let mut last_error: Option<String> = None;
    loop {
        tokio::select! {
            () = stopped(&mut shutdown) => return,
            _ = ticker.tick() => {}
        }
        match clipboard.read_text() {
            Ok(text) => {
                last_error = None;
                if let Some(text) = watcher.observe(text) {
                    tracing::debug!(size = text.len() as u64, "new clipboard text");
                    if jobs.send(Job::Text(text)).await.is_err() {
                        return;
                    }
                }
            }
            Err(err) => {
                let message = err.to_string();
                if last_error.as_deref() != Some(message.as_str()) {
                    tracing::warn!(error = message.as_str(), "cannot read the clipboard");
                    last_error = Some(message);
                }
            }
        }
    }
}

async fn drop_loop(
    folder: PathBuf,
    stable_wait: Duration,
    rescan: Duration,
    jobs: mpsc::Sender<Job>,
    mut changed: mpsc::Receiver<()>,
    mut skipped: mpsc::UnboundedReceiver<PathBuf>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut tracker = DropTracker::new(stable_wait);
    // While files are settling, look again often enough to notice promptly.
    let settle_tick = (stable_wait / 2).clamp(Duration::from_millis(50), Duration::from_secs(1));
    let mut watching = true;
    loop {
        match drop_watcher::scan(&folder).await {
            Ok(files) => {
                for path in tracker.observe(files, Instant::now()) {
                    if jobs.send(Job::File(path)).await.is_err() {
                        return;
                    }
                }
            }
            Err(err) => {
                tracing::warn!(path = %folder.display(), error = %err, "cannot scan the drop folder")
            }
        }
        let wait = if tracker.has_pending() {
            settle_tick
        } else {
            rescan
        };
        tokio::select! {
            () = stopped(&mut shutdown) => return,
            event = changed.recv(), if watching => {
                // The watcher is gone; keep going on periodic scans alone.
                if event.is_none() {
                    watching = false;
                }
            }
            Some(path) = skipped.recv() => tracker.mark_skipped(&path),
            () = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MapEnv;

    #[test]
    fn options_come_from_the_config() {
        let text = "[client]\ndevice_name = \"lap\"\n[server]\nkind = \"local\"\n[server.local]\npath = \"/s\"\n[serve]\ndrop_folder = \"/drop\"\nclipboard_poll_interval_ms = 300\nfile_stable_wait_ms = 0\nafter_send = \"delete\"\n";
        let config =
            crate::config::parse(text, std::path::Path::new("/c.toml"), &MapEnv::new()).unwrap();
        assert_eq!(
            ServeOptions::from_config(&config),
            ServeOptions {
                drop_folder: PathBuf::from("/drop"),
                clipboard_poll_interval: Duration::from_millis(300),
                file_stable_wait: Duration::ZERO,
                rescan_interval: Duration::from_secs(5),
                after_send: AfterSend::Delete,
                device: "lap".into(),
            }
        );
    }
}
