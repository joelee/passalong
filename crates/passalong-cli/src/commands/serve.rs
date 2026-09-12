//! `passalong serve`: run until stopped, sending new clipboard text and
//! files dropped into the drop folder.

use std::sync::Arc;

use passalong_core::clipboard::{ArboardClipboard, Clipboard};
use passalong_core::config::Config;
use passalong_core::serve::{self, ServeOptions, StoreOpener};
use passalong_core::store::{BackendFuture, BackendRegistry};
use tokio::sync::watch;

/// Runs `serve` until Ctrl-C, or SIGTERM on Unix. Without a desktop
/// clipboard it keeps watching the drop folder.
pub async fn run(config: &Config, backends: BackendRegistry) -> anyhow::Result<()> {
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
    serve::run(
        ServeOptions::from_config(config),
        clipboard,
        open_store,
        stopped,
    )
    .await?;
    Ok(())
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
