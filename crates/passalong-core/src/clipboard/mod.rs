//! Clipboard access behind a trait, so commands and `serve` can be tested
//! with [`MockClipboard`](crate::testing::MockClipboard), and GUI or Android
//! front-ends can supply their own implementation.
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

    /// Needs a real desktop session, so it never runs in CI. Run it with
    /// `cargo test -p passalong-core -- --ignored desktop_`.
    #[cfg(feature = "desktop")]
    #[test]
    #[ignore = "needs a desktop session with a clipboard"]
    fn desktop_clipboard_round_trip() {
        let mut clip = ArboardClipboard::new().unwrap();
        clip.write_text("passalong desktop test").unwrap();
        assert_eq!(
            clip.read_text().unwrap().as_deref(),
            Some("passalong desktop test")
        );
    }
}
