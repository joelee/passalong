//! The desktop clipboard through `arboard`: macOS, and Linux under X11 or a
//! Wayland compositor that supports the `wlr-data-control` protocol.

use std::borrow::Cow;

use crate::clipboard::{Clipboard, ClipboardError, RgbaImage};

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

    /// Puts `text` on the clipboard and, on Linux, keeps serving it until
    /// another program replaces it, returning only then. This is what lets
    /// text outlive the short-lived command that loaded it. Elsewhere the
    /// system keeps clipboard content itself, so this writes and returns.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Other`] when the clipboard refuses the text.
    pub fn hold_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        hold(&mut self.inner, text).map_err(|err| ClipboardError::Other(err.to_string()))
    }

    /// Puts `image` on the clipboard and, like [`Self::hold_text`], keeps
    /// serving it on Linux until another program replaces it.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Other`] when the clipboard refuses the image.
    pub fn hold_image(&mut self, image: &RgbaImage) -> Result<(), ClipboardError> {
        hold_image(&mut self.inner, image_data(image))
            .map_err(|err| ClipboardError::Other(err.to_string()))
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

    fn read_image(&mut self) -> Result<Option<RgbaImage>, ClipboardError> {
        match self.inner.get_image() {
            Ok(data) => {
                let width = u32::try_from(data.width)
                    .map_err(|_| ClipboardError::Other("image too wide".to_owned()))?;
                let height = u32::try_from(data.height)
                    .map_err(|_| ClipboardError::Other("image too tall".to_owned()))?;
                RgbaImage::new(width, height, data.bytes.into_owned())
                    .map(Some)
                    .map_err(|err| ClipboardError::Other(err.to_string()))
            }
            Err(::arboard::Error::ContentNotAvailable) => Ok(None),
            Err(err) => Err(ClipboardError::Other(err.to_string())),
        }
    }

    fn write_image(&mut self, image: &RgbaImage) -> Result<(), ClipboardError> {
        set_image(&mut self.inner, image_data(image))
            .map_err(|err| ClipboardError::Other(err.to_string()))
    }
}

fn image_data(image: &RgbaImage) -> ::arboard::ImageData<'_> {
    ::arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: Cow::Borrowed(image.rgba()),
    }
}

#[cfg(target_os = "linux")]
fn set_image(
    inner: &mut ::arboard::Clipboard,
    image: ::arboard::ImageData<'_>,
) -> Result<(), ::arboard::Error> {
    use ::arboard::SetExtLinux;
    inner
        .set()
        .wait_until(std::time::Instant::now() + LINUX_HANDOVER)
        .image(image)
}

#[cfg(not(target_os = "linux"))]
fn set_image(
    inner: &mut ::arboard::Clipboard,
    image: ::arboard::ImageData<'_>,
) -> Result<(), ::arboard::Error> {
    inner.set_image(image)
}

#[cfg(target_os = "linux")]
fn hold_image(
    inner: &mut ::arboard::Clipboard,
    image: ::arboard::ImageData<'_>,
) -> Result<(), ::arboard::Error> {
    use ::arboard::SetExtLinux;
    inner.set().wait().image(image)
}

#[cfg(not(target_os = "linux"))]
fn hold_image(
    inner: &mut ::arboard::Clipboard,
    image: ::arboard::ImageData<'_>,
) -> Result<(), ::arboard::Error> {
    inner.set_image(image)
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

#[cfg(target_os = "linux")]
fn hold(inner: &mut ::arboard::Clipboard, text: &str) -> Result<(), ::arboard::Error> {
    use ::arboard::SetExtLinux;
    inner.set().wait().text(text.to_owned())
}

#[cfg(not(target_os = "linux"))]
fn hold(inner: &mut ::arboard::Clipboard, text: &str) -> Result<(), ::arboard::Error> {
    inner.set_text(text)
}
