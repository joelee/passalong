//! Where log records go: standard error, except while `choose` shows its
//! full-screen list, when they are held back and written once it closes.
//! A record written to the terminal under the list would scroll it and mix
//! with what is drawn there.

use std::io::{self, Write};
use std::sync::{Mutex, PoisonError};

/// Records held back while a [`Hold`] exists; `None` when they go straight
/// to standard error.
static HELD: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// The file records go to instead of standard error, once [`to_file`] set it.
#[cfg(windows)]
static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Sends log records to the end of `path` from now on, instead of standard
/// error: a background `serve` on Windows has no standard error to
/// redirect (see `daemon::launch_detached`).
///
/// # Errors
///
/// When `path` cannot be opened.
#[cfg(windows)]
pub fn to_file(path: &std::path::Path) -> io::Result<()> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    *FILE.lock().unwrap_or_else(PoisonError::into_inner) = Some(file);
    Ok(())
}

/// The writer given to telemetry: standard error, or the hold buffer.
pub fn writer() -> LogWriter {
    LogWriter
}

/// Writes log records where they currently belong.
pub struct LogWriter;

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(buffer) = held.as_mut() {
            buffer.extend_from_slice(buf);
            return Ok(buf.len());
        }
        drop(held);
        #[cfg(windows)]
        if let Some(file) = FILE.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            return file.write(buf);
        }
        io::stderr().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        if let Some(file) = FILE.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            return file.flush();
        }
        io::stderr().flush()
    }
}

/// Holds log records back until the returned guard is dropped, which writes
/// them to standard error in order.
#[must_use = "records are released when the guard is dropped"]
pub fn hold() -> Hold {
    HELD.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(Vec::new);
    Hold
}

/// Releases the held log records when dropped.
pub struct Hold;

impl Drop for Hold {
    fn drop(&mut self) {
        let held = HELD.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(buffer) = held {
            let _ = io::stderr().write_all(&buffer);
        }
    }
}

/// A copy of what is held, or `None` when nothing is being held.
#[cfg(test)]
fn held() -> Option<Vec<u8>> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use passalong_core::telemetry::{self, LogLevel};
    use passalong_core::testing::FixedClock;
    use std::sync::Arc;

    #[test]
    fn records_are_held_while_the_list_is_open_and_released_after() {
        let subscriber = telemetry::subscriber(
            LogLevel::Info,
            Arc::new(FixedClock::at("2026-09-12T09:53:11Z")),
            writer,
        );
        tracing::subscriber::with_default(subscriber, || {
            assert_eq!(held(), None, "nothing is held by default");
            let hold = hold();
            tracing::info!(target: "passalong::commands::choose", "item deleted");
            let buffered = String::from_utf8(held().unwrap()).unwrap();
            assert!(
                buffered.contains("info passalong::commands::choose"),
                "{buffered}"
            );
            assert!(buffered.contains("item deleted"), "{buffered}");
            drop(hold);
            assert_eq!(held(), None, "released when the hold ends");
        });
    }
}
