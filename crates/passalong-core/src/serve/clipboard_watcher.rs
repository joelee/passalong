//! Detecting new clipboard text.

use crate::model::ContentHasher;

/// Remembers the last clipboard text by its SHA-256, so repeated polls of
/// unchanged text send nothing.
#[derive(Debug, Default)]
pub struct ClipboardWatcher {
    last: Option<[u8; 32]>,
}

impl ClipboardWatcher {
    /// A watcher that has seen nothing yet, so the first text is sent.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `text` when it differs from the last text seen and is not
    /// blank. `None` (no text on the clipboard) changes nothing.
    pub fn observe(&mut self, text: Option<String>) -> Option<String> {
        let text = text?;
        let mut hasher = ContentHasher::new();
        hasher.update(text.as_bytes());
        let digest = *hasher.finalize().sha256_bytes();
        if self.last == Some(digest) {
            return None;
        }
        self.last = Some(digest);
        (!text.trim().is_empty()).then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some(text: &str) -> Option<String> {
        Some(text.to_owned())
    }

    #[test]
    fn reports_each_new_text_once() {
        let mut watcher = ClipboardWatcher::new();
        assert_eq!(watcher.observe(some("a")).as_deref(), Some("a"));
        assert_eq!(watcher.observe(some("a")), None);
        assert_eq!(watcher.observe(None), None);
        assert_eq!(
            watcher.observe(some("a")),
            None,
            "a read without text does not reset"
        );
        assert_eq!(watcher.observe(some("b")).as_deref(), Some("b"));
        assert_eq!(
            watcher.observe(some("a")).as_deref(),
            Some("a"),
            "changing back is new"
        );
    }

    #[test]
    fn never_reports_blank_text() {
        let mut watcher = ClipboardWatcher::default();
        assert_eq!(watcher.observe(some("  \n\t")), None);
        assert_eq!(watcher.observe(some("")), None);
        assert_eq!(watcher.observe(some("x")).as_deref(), Some("x"));
        assert_eq!(watcher.observe(some(" ")), None);
    }
}
