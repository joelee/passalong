//! The desktop clipboard through `arboard`: macOS, and Linux under X11 or a
//! Wayland compositor that supports the `wlr-data-control` protocol.

use crate::clipboard::{Clipboard, ClipboardError};

/// How long a Linux write waits for a clipboard manager to take over the
/// content. On Linux the content lives in the writing process, so without a
/// clipboard manager it disappears when a short-lived command exits.
#[cfg(target_os = "linux")]
const LINUX_HANDOVER: std::time::Duration = std::time::Duration::from_secs(2);

/// [`Clipboard`] backed by the operating system's clipboard.
pub struct ArboardClipboard {
    inner: ::arboard::Clipboard,
}

impl ArboardClipboard {
    /// Connects to the system clipboard.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Unavailable`] when there is no clipboard to connect
    /// to, such as in a headless session or a container.
    pub fn new() -> Result<Self, ClipboardError> {
        ::arboard::Clipboard::new()
            .map(|inner| Self { inner })
            .map_err(|err| ClipboardError::Unavailable(err.to_string()))
    }
}

impl std::fmt::Debug for ArboardClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArboardClipboard")
    }
}

impl Clipboard for ArboardClipboard {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        match self.inner.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(::arboard::Error::ContentNotAvailable) => Ok(None),
            Err(err) => Err(ClipboardError::Other(err.to_string())),
        }
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        set_text(&mut self.inner, text).map_err(|err| ClipboardError::Other(err.to_string()))
    }
}

#[cfg(target_os = "linux")]
fn set_text(inner: &mut ::arboard::Clipboard, text: &str) -> Result<(), ::arboard::Error> {
    use ::arboard::SetExtLinux;
    inner
        .set()
        .wait_until(std::time::Instant::now() + LINUX_HANDOVER)
        .text(text.to_owned())
}

#[cfg(not(target_os = "linux"))]
fn set_text(inner: &mut ::arboard::Clipboard, text: &str) -> Result<(), ::arboard::Error> {
    inner.set_text(text)
}
