//! Keeping text on the Linux clipboard after `load` exits.
//!
//! On Linux the clipboard's content lives in the process that set it. `load`
//! therefore hands the text to a detached `passalong __hold-clipboard`
//! process, which serves it until another program replaces it, as `wl-copy`
//! and `xclip` do.

use std::io::{self, Write};

use passalong_core::clipboard::{Clipboard, ClipboardError};

/// Starts a background holder for some text.
pub trait HolderLauncher {
    /// Hands `text` to a new holder and returns without waiting for it.
    fn launch(&self, text: &str) -> io::Result<()>;
}

/// Starts `passalong __hold-clipboard` detached and pipes the text to it.
pub struct ProcessLauncher;

#[cfg(unix)]
impl HolderLauncher for ProcessLauncher {
    fn launch(&self, text: &str) -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let mut child = crate::daemon::detached_command(&exe)
            .arg("__hold-clipboard")
            .stdin(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("the holder has no standard input"))?;
        stdin.write_all(text.as_bytes())?;
        // Closing standard input tells the holder the text is complete. The
        // holder outlives this process and is reaped by init.
        drop(stdin);
        Ok(())
    }
}

#[cfg(not(unix))]
impl HolderLauncher for ProcessLauncher {
    fn launch(&self, _text: &str) -> io::Result<()> {
        Err(io::Error::other(
            "the clipboard holder is only used on Linux",
        ))
    }
}

/// [`Clipboard`] whose writes go to a holder process. It cannot read.
pub struct HolderClipboard<L> {
    launcher: L,
}

impl<L> HolderClipboard<L> {
    /// A clipboard that starts holders with `launcher`.
    pub fn new(launcher: L) -> Self {
        Self { launcher }
    }
}

impl<L: HolderLauncher + Send> Clipboard for HolderClipboard<L> {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        Err(ClipboardError::Other(
            "the clipboard holder can only write".to_owned(),
        ))
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.launcher.launch(text).map_err(|err| {
            ClipboardError::Other(format!("cannot start the clipboard holder: {err}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{TestStore, bytes};
    use passalong_core::model::NewItem;
    use passalong_core::store::Store as _;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Recorder {
        texts: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    impl HolderLauncher for Recorder {
        fn launch(&self, text: &str) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::other("no such executable"));
            }
            self.texts.lock().unwrap().push(text.to_owned());
            Ok(())
        }
    }

    #[test]
    fn writes_hand_the_text_to_a_holder_and_reads_are_refused() {
        let recorder = Recorder::default();
        let mut clipboard = HolderClipboard::new(recorder.clone());
        clipboard.write_text("keep me").unwrap();
        assert_eq!(*recorder.texts.lock().unwrap(), ["keep me"]);
        assert!(clipboard.read_text().is_err());
        let mut failing = HolderClipboard::new(Recorder {
            fail: true,
            ..Recorder::default()
        });
        let err = failing.write_text("x").unwrap_err();
        assert!(
            err.to_string().contains("clipboard holder")
                && err.to_string().contains("no such executable"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn load_to_the_clipboard_goes_through_the_holder() {
        let ts = TestStore::new();
        let meta = ts
            .store
            .put(NewItem::text("box"), bytes(b"loaded text"))
            .await
            .unwrap()
            .meta;
        let recorder = Recorder::default();
        let launcher = recorder.clone();
        let mut open = move || -> Result<Box<dyn Clipboard>, ClipboardError> {
            Ok(Box::new(HolderClipboard::new(launcher.clone())))
        };
        crate::commands::load::run(
            &ts.store,
            meta.id.as_str(),
            None,
            false,
            &mut open,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(*recorder.texts.lock().unwrap(), ["loaded text"]);
    }
}
