//! Clipboard access behind a trait, so commands and `serve` can be tested
//! with `testing::MockClipboard` (available with the `testing` feature),
//! and GUI or Android front-ends can supply their own implementation.
//!
//! Only UTF-8 text is supported in this release.

#[cfg(feature = "desktop")]
mod desktop;

#[cfg(feature = "desktop")]
pub use desktop::ArboardClipboard;

/// Read and write access to a clipboard's text.
pub trait Clipboard: Send {
    /// Returns the clipboard's text, or `None` when it holds no text.
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError>;

    /// Replaces the clipboard's content with `text`.
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError>;
}

/// Clipboard failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClipboardError {
    /// No clipboard can be reached, for example without a desktop session.
    #[error("clipboard unavailable: {0}")]
    Unavailable(String),
    /// Any other failure.
    #[error("clipboard error: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockClipboard;

    #[test]
    fn mock_serves_scripted_reads_then_the_current_text() {
        let mock = MockClipboard::new().with_reads([None, Some("a"), Some("b")]);
        let mut clip: Box<dyn Clipboard> = Box::new(mock.clone());
        assert_eq!(clip.read_text().unwrap(), None);
        assert_eq!(clip.read_text().unwrap().as_deref(), Some("a"));
        assert_eq!(clip.read_text().unwrap().as_deref(), Some("b"));
        assert_eq!(
            clip.read_text().unwrap().as_deref(),
            Some("b"),
            "script exhausted: current text"
        );
    }

    #[test]
    fn mock_records_writes_and_shares_state_between_clones() {
        let mock = MockClipboard::with_text("start");
        let mut clip = mock.clone();
        assert_eq!(clip.read_text().unwrap().as_deref(), Some("start"));
        clip.write_text("one").unwrap();
        clip.write_text("two").unwrap();
        assert_eq!(mock.writes(), ["one", "two"]);
        assert_eq!(mock.current().as_deref(), Some("two"));
        assert_eq!(clip.read_text().unwrap().as_deref(), Some("two"));
    }

    #[test]
    fn mock_injects_errors() {
        let mock = MockClipboard::new();
        let mut clip = mock.clone();
        mock.push_read_error(ClipboardError::Unavailable("no display".into()));
        assert_eq!(
            clip.read_text().unwrap_err(),
            ClipboardError::Unavailable("no display".into())
        );
        assert_eq!(clip.read_text().unwrap(), None);
        mock.fail_writes(true);
        assert!(clip.write_text("x").is_err());
        assert!(mock.writes().is_empty());
    }

    #[test]
    fn errors_describe_themselves() {
        assert_eq!(
            ClipboardError::Unavailable("no display".into()).to_string(),
            "clipboard unavailable: no display"
        );
        assert_eq!(
            ClipboardError::Other("busy".into()).to_string(),
            "clipboard error: busy"
        );
    }

    /// The desktop tests share the one system clipboard, so they take this
    /// lock instead of running in parallel.
    #[cfg(feature = "desktop")]
    static DESKTOP_CLIPBOARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Needs a desktop session; CI runs it under Xvfb. Run it locally with
    /// `cargo test -p passalong-core -- --ignored desktop_`.
    #[cfg(feature = "desktop")]
    #[test]
    #[ignore = "needs a desktop session with a clipboard"]
    fn desktop_clipboard_round_trip() {
        let _clipboard = DESKTOP_CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
        let mut clip = ArboardClipboard::new().unwrap();
        clip.write_text("passalong desktop test").unwrap();
        assert_eq!(
            clip.read_text().unwrap().as_deref(),
            Some("passalong desktop test")
        );
    }

    /// Holds text the way `load` does on Linux, then releases it. Needs a
    /// desktop session; CI runs it under Xvfb.
    #[cfg(feature = "desktop")]
    #[test]
    #[ignore = "needs a desktop session with a clipboard"]
    fn desktop_held_text_outlives_the_writer_until_replaced() {
        let _clipboard = DESKTOP_CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = ArboardClipboard::new()
                .unwrap()
                .hold_text("held by passalong");
            done_tx.send(result.is_ok()).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut reader = ArboardClipboard::new().unwrap();
        assert_eq!(
            reader.read_text().unwrap().as_deref(),
            Some("held by passalong")
        );
        reader.write_text("replacement").unwrap();
        let released = done_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("holder returns once replaced");
        assert!(released);
    }
}
