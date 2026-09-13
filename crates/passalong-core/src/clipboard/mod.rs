//! Clipboard access behind a trait, so commands and `serve` can be tested
//! with `testing::MockClipboard` (available with the `testing` feature),
//! and GUI or Android front-ends can supply their own implementation.
//!
//! Text is UTF-8; images are 8-bit RGBA pixels, stored as PNG.

#[cfg(feature = "desktop")]
mod desktop;
mod image;

pub use image::{ImageError, MAX_IMAGE_PIXELS, RgbaImage, decode_png, encode_png};

#[cfg(feature = "desktop")]
pub use desktop::ArboardClipboard;

/// Read and write access to a clipboard's text and images.
pub trait Clipboard: Send {
    /// Returns the clipboard's text, or `None` when it holds no text.
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError>;

    /// Replaces the clipboard's content with `text`.
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError>;

    /// Returns the clipboard's image, or `None` when it holds none. The
    /// default, for clipboards without image support, is always `None`.
    fn read_image(&mut self) -> Result<Option<RgbaImage>, ClipboardError> {
        Ok(None)
    }

    /// Replaces the clipboard's content with `image`. The default, for
    /// clipboards without image support, fails with `Unavailable`.
    fn write_image(&mut self, image: &RgbaImage) -> Result<(), ClipboardError> {
        let _ = image;
        Err(ClipboardError::Unavailable(
            "this clipboard does not support images".to_owned(),
        ))
    }
}

/// Reads what passalong sends from a clipboard: its text, or its image
/// when `images` is on and the text is missing, blank, or only refers to
/// that image. Browsers copying an image put such a reference beside it: a
/// link, or an `<img>` tag. Text containing NUL characters is a non-text
/// format, such as UTF-16 `text/x-moz-url`, read as text by mistake, and
/// counts as missing. Returns the text alone when there is no image.
///
/// # Errors
///
/// The clipboard's error.
pub fn read_payload(
    clipboard: &mut dyn Clipboard,
    images: bool,
) -> Result<(Option<String>, Option<RgbaImage>), ClipboardError> {
    let text = clipboard.read_text()?.filter(|text| !text.contains('\0'));
    let is_content = text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty() && !refers_to_image(text));
    if is_content || !images {
        return Ok((text, None));
    }
    match clipboard.read_image()? {
        Some(image) => Ok((None, Some(image))),
        None => Ok((text, None)),
    }
}

/// Whether `text` only refers to an image instead of being content: a
/// single `http`, `https`, or `file` link, optionally followed by one title
/// line, or HTML that is nothing but an `<img>` tag after any `<meta>`
/// tags. Browsers put these beside an image they copy.
pub fn refers_to_image(text: &str) -> bool {
    let text = text.trim();
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let is_link = |line: &str| {
        let lower = line.to_ascii_lowercase();
        ["http://", "https://", "file://"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
            && !line.contains(char::is_whitespace)
    };
    if lines.len() <= 2 && lines.first().is_some_and(|line| is_link(line)) {
        return true;
    }
    let mut html = text.to_ascii_lowercase();
    while let Some(rest) = html.trim_start().strip_prefix("<meta") {
        match rest.find('>') {
            Some(end) => html = rest[end + 1..].to_owned(),
            None => return false,
        }
    }
    let html = html.trim();
    html.starts_with("<img")
        && html
            .find('>')
            .is_some_and(|end| html[end + 1..].trim().is_empty())
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

    /// A clipboard that only knows text, relying on the trait's defaults.
    struct TextOnly;

    impl Clipboard for TextOnly {
        fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
            Ok(None)
        }
        fn write_text(&mut self, _: &str) -> Result<(), ClipboardError> {
            Ok(())
        }
    }

    #[test]
    fn text_only_clipboards_have_no_images() {
        let mut clip = TextOnly;
        assert_eq!(clip.read_image().unwrap(), None);
        let image = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        assert!(matches!(
            clip.write_image(&image),
            Err(ClipboardError::Unavailable(_))
        ));
    }

    #[test]
    fn the_mock_keeps_either_text_or_an_image_like_a_real_clipboard() {
        let image = RgbaImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let other = RgbaImage::new(1, 1, vec![5, 6, 7, 8]).unwrap();
        let mock = MockClipboard::with_image(image.clone());
        let mut clip = mock.clone();
        assert_eq!(clip.read_text().unwrap(), None);
        assert_eq!(clip.read_image().unwrap(), Some(image.clone()));
        clip.write_text("words").unwrap();
        assert_eq!(clip.read_image().unwrap(), None, "text replaces the image");
        clip.write_image(&other).unwrap();
        assert_eq!(
            clip.read_text().unwrap(),
            None,
            "an image replaces the text"
        );
        assert_eq!(mock.image_writes(), std::slice::from_ref(&other));
        assert_eq!(mock.current_image(), Some(other.clone()));

        let scripted = MockClipboard::new().with_image_reads([Some(image.clone()), None]);
        let mut clip = scripted.clone();
        assert_eq!(clip.read_image().unwrap(), Some(image.clone()));
        assert_eq!(clip.read_image().unwrap(), None);
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

    /// Needs a desktop session; CI runs it under Xvfb.
    #[cfg(feature = "desktop")]
    #[test]
    #[ignore = "needs a desktop session with a clipboard"]
    fn desktop_image_round_trip() {
        let _clipboard = DESKTOP_CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
        let rgba: Vec<u8> = (0..4 * 3 * 4)
            .map(|i| {
                if i % 4 == 3 {
                    255
                } else {
                    (i * 19 % 256) as u8
                }
            })
            .collect();
        let image = RgbaImage::new(4, 3, rgba).unwrap();
        let mut clip = ArboardClipboard::new().unwrap();
        clip.write_image(&image).unwrap();
        assert_eq!(clip.read_image().unwrap(), Some(image));
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

    /// A UTF-16 clipboard format, such as `text/x-moz-url`, misread as UTF-8
    /// text: every ASCII character followed by a NUL.
    fn utf16_as_text(text: &str) -> String {
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        String::from_utf8(bytes).unwrap()
    }

    fn picture() -> RgbaImage {
        RgbaImage::new(1, 1, vec![1, 2, 3, 255]).unwrap()
    }

    #[test]
    fn links_and_image_tags_refer_to_an_image() {
        for text in [
            "https://cdn.example.com/a.webp?v=1",
            "https://cdn.example.com/a.webp\nPage title",
            "  http://x/y.png  ",
            "file:///home/u/a.png",
            "<meta charset='utf-8'><img src=\"https://x/a.png\">",
            "<IMG SRC=a.png>",
        ] {
            assert!(refers_to_image(text), "{text:?}");
        }
        for text in [
            "hello",
            "see https://x.com for details",
            "https://a\nline two\nline three",
            "<p>hi</p><img src=a.png>",
            "",
        ] {
            assert!(!refers_to_image(text), "{text:?}");
        }
    }

    #[test]
    fn the_image_wins_over_a_link_to_it() {
        let mut clip = MockClipboard::with_text("https://cdn.example.com/a.webp")
            .with_image_reads([Some(picture())]);
        assert_eq!(
            read_payload(&mut clip, true).unwrap(),
            (None, Some(picture()))
        );
    }

    #[test]
    fn utf16_link_text_is_never_text() {
        let text = utf16_as_text("https://cdn.example.com/a.webp\nPage title");
        let mut clip = MockClipboard::with_text(&text).with_image_reads([Some(picture())]);
        assert_eq!(
            read_payload(&mut clip, true).unwrap(),
            (None, Some(picture()))
        );
        let mut clip = MockClipboard::with_text(&text);
        assert_eq!(
            read_payload(&mut clip, true).unwrap(),
            (None, None),
            "garbled text is dropped"
        );
    }

    #[test]
    fn real_text_still_wins_and_links_without_an_image_stay_text() {
        let mut clip = MockClipboard::with_text("words").with_image_reads([Some(picture())]);
        assert_eq!(
            read_payload(&mut clip, true).unwrap(),
            (Some("words".to_owned()), None)
        );
        let mut clip = MockClipboard::with_text("https://example.com/page");
        assert_eq!(
            read_payload(&mut clip, true).unwrap(),
            (Some("https://example.com/page".to_owned()), None)
        );
        let mut clip = MockClipboard::with_text("https://cdn.example.com/a.webp")
            .with_image_reads([Some(picture())]);
        assert_eq!(
            read_payload(&mut clip, false).unwrap(),
            (Some("https://cdn.example.com/a.webp".to_owned()), None),
            "with images off, the link is the content"
        );
    }
}
