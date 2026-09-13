//! Detecting new clipboard text.

use crate::clipboard::RgbaImage;
use crate::model::ContentHasher;

/// Remembers the last clipboard text by its SHA-256, so repeated polls of
/// unchanged text send nothing.
#[derive(Debug, Default)]
pub struct ClipboardWatcher {
    last: Option<[u8; 32]>,
    last_image: Option<[u8; 32]>,
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

    /// Returns `image` when its size or pixels differ from the last image
    /// seen. `None` (no image on the clipboard) changes nothing.
    pub fn observe_image(&mut self, image: Option<RgbaImage>) -> Option<RgbaImage> {
        let image = image?;
        let mut hasher = ContentHasher::new();
        hasher.update(&image.width().to_le_bytes());
        hasher.update(&image.height().to_le_bytes());
        hasher.update(image.rgba());
        let digest = *hasher.finalize().sha256_bytes();
        if self.last_image == Some(digest) {
            return None;
        }
        self.last_image = Some(digest);
        Some(image)
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

    #[test]
    fn reports_each_new_image_once() {
        use crate::clipboard::RgbaImage;
        let a = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let b = RgbaImage::new(1, 1, vec![5, 6, 7, 8]).unwrap();
        let mut watcher = ClipboardWatcher::new();
        assert_eq!(watcher.observe_image(Some(a.clone())), Some(a.clone()));
        assert_eq!(watcher.observe_image(Some(a.clone())), None);
        assert_eq!(watcher.observe_image(None), None);
        assert_eq!(
            watcher.observe_image(Some(a.clone())),
            None,
            "a read without an image does not reset"
        );
        assert_eq!(watcher.observe_image(Some(b.clone())), Some(b));
        // The same bytes in another shape are another image.
        let tall = RgbaImage::new(1, 2, vec![1, 2, 3, 4, 1, 2, 3, 4]).unwrap();
        let wide = RgbaImage::new(2, 1, vec![1, 2, 3, 4, 1, 2, 3, 4]).unwrap();
        assert_eq!(watcher.observe_image(Some(tall.clone())), Some(tall));
        assert_eq!(watcher.observe_image(Some(wide.clone())), Some(wide));
    }
}
