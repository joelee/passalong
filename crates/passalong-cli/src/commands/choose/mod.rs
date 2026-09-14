//! `passalong choose`: pick an item from a full-screen list, then load or
//! print it, see its metadata, or delete it.
//!
//! The list itself is a [`state::Picker`] driven by key events and drawn by
//! [`view::render`]. Showing metadata and deleting happen inside the list.
//! Loading and printing run after the list is closed and the terminal
//! restored, through the same code as `load` and `cat`, so their output
//! stays in the terminal.

pub mod state;
pub mod view;

use std::io::{self, Write};
use std::path::Path;

use chrono::{FixedOffset, Utc};
use passalong_core::model::ItemId;
use passalong_core::store::Store;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::commands::load::OpenClipboard;
use crate::output::render_meta;
use crate::resolve::Lookup;
pub use state::Action;
use state::{Outcome, Picker};

/// The item the user chose and what to do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    /// What to do.
    pub action: Action,
    /// Which item.
    pub id: ItemId,
}

/// Where the picker is shown and where its keys come from.
pub trait Screen {
    /// Draws the picker.
    fn draw(&mut self, picker: &Picker) -> io::Result<()>;

    /// Waits for the next key press.
    fn next_key(&mut self) -> io::Result<KeyEvent>;

    /// Forgets what is on the screen, so the next frame is drawn in full.
    /// Store operations may log to standard error, which is the same
    /// terminal, so the list is redrawn after them.
    fn redraw_all(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// What the chosen action needs besides the store.
pub struct ActionTargets<'a, 'b> {
    /// Where loaded files go.
    pub download_dir: &'a Path,
    /// Opens the clipboard loaded text and images go to.
    pub open_clipboard: &'a mut OpenClipboard<'b>,
    /// Whether standard output is a terminal, which decides whether a binary
    /// item may be printed.
    pub stdout_is_terminal: bool,
    /// The UTC offset shown times use.
    pub offset: FixedOffset,
}

/// Shows the full-screen list on the terminal, then carries out the chosen
/// action, if any, once the terminal is restored.
///
/// # Errors
///
/// When the terminal cannot be set up, or the store or the action fails.
pub async fn run(
    store: &dyn Store,
    targets: ActionTargets<'_, '_>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let chosen = {
        let mut screen = TerminalScreen::open()?;
        run_picker(store, &mut screen, targets.offset).await?
        // The screen is dropped here, which restores the terminal.
    };
    match chosen {
        Some(chosen) => perform(store, &chosen, targets, out).await,
        None => Ok(()),
    }
}

/// Lists the store and handles keys until an item is chosen or the user
/// quits. Showing metadata (with times at `offset`), deleting, and
/// reloading happen here, and the list stays open. While the store is being
/// read or changed, the status line says so, since keys wait until it is
/// done.
///
/// # Errors
///
/// When listing the store fails at the start, or the screen fails.
pub async fn run_picker(
    store: &dyn Store,
    screen: &mut dyn Screen,
    offset: FixedOffset,
) -> anyhow::Result<Option<Chosen>> {
    let mut picker = Picker::new(Vec::new());
    picker.set_status("Loading...");
    screen.draw(&picker)?;
    picker.replace(store.list().await?);
    picker.clear_status();
    loop {
        screen.draw(&picker)?;
        match picker.handle(screen.next_key()?) {
            Outcome::Continue => {}
            Outcome::Quit => return Ok(None),
            Outcome::Act(action, id) => return Ok(Some(Chosen { action, id })),
            Outcome::Details(id) => {
                match store.get_meta(&id).await {
                    Ok(meta) => picker.show_details(
                        meta.id.to_string(),
                        render_meta(&meta, offset)
                            .lines()
                            .map(str::to_owned)
                            .collect(),
                    ),
                    Err(err) => picker.set_status(format!("cannot read {id}: {err}")),
                }
                screen.redraw_all()?;
            }
            Outcome::Delete(id) => {
                picker.set_status(format!("Deleting {id}..."));
                screen.draw(&picker)?;
                match store.delete(&id).await {
                    Ok(_) => {
                        if reload(store, screen, &mut picker).await?.is_some() {
                            picker.set_status(format!("deleted {id}"));
                        }
                    }
                    Err(err) => picker.set_status(format!("cannot delete {id}: {err}")),
                }
                screen.redraw_all()?;
            }
            Outcome::Reload => {
                if let Some(n) = reload(store, screen, &mut picker).await? {
                    picker.set_status(format!("reloaded: {n} items"));
                }
                screen.redraw_all()?;
            }
        }
    }
}

/// Lists the store again, showing `Reloading...` meanwhile. Returns how
/// many items there are, or `None` when listing failed, which the status
/// line then reports.
async fn reload(
    store: &dyn Store,
    screen: &mut dyn Screen,
    picker: &mut Picker,
) -> io::Result<Option<usize>> {
    picker.set_status("Reloading...");
    screen.draw(picker)?;
    Ok(match store.list().await {
        Ok(items) => {
            let n = items.len();
            picker.replace(items);
            Some(n)
        }
        Err(err) => {
            picker.set_status(format!("cannot reload: {err}"));
            None
        }
    })
}

/// Carries out `chosen` through the code of `load` or `cat`.
///
/// # Errors
///
/// Whatever that command returns.
pub async fn perform(
    store: &dyn Store,
    chosen: &Chosen,
    targets: ActionTargets<'_, '_>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let id = chosen.id.to_string();
    let lookup = Lookup {
        input: &id,
        chooser: None,
    };
    match chosen.action {
        Action::Load => {
            crate::commands::load::run(
                store,
                lookup,
                None,
                false,
                targets.download_dir,
                targets.open_clipboard,
                out,
            )
            .await
        }
        Action::Cat => {
            crate::commands::cat::run(store, lookup, false, targets.stdout_is_terminal, out).await
        }
    }
}

/// The real terminal: raw mode on the alternate screen while it exists.
/// Dropping it, or a panic, restores the terminal.
struct TerminalScreen {
    terminal: ratatui::DefaultTerminal,
}

impl TerminalScreen {
    fn open() -> io::Result<Self> {
        // `try_init` also installs a panic hook that restores the terminal.
        Ok(Self {
            terminal: ratatui::try_init()?,
        })
    }
}

impl Drop for TerminalScreen {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

impl Screen for TerminalScreen {
    fn draw(&mut self, picker: &Picker) -> io::Result<()> {
        let now = Utc::now();
        self.terminal
            .draw(|frame| view::render(frame, picker, now))
            .map(|_| ())
    }

    fn next_key(&mut self) -> io::Result<KeyEvent> {
        loop {
            match event::read()? {
                // Only presses: some terminals also report releases.
                Event::Key(key) if key.kind == KeyEventKind::Press => return Ok(key),
                // A key that does nothing, so the list is drawn at the new size.
                Event::Resize(..) => return Ok(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)),
                _ => {}
            }
        }
    }

    fn redraw_all(&mut self) -> io::Result<()> {
        self.terminal.clear()
    }
}

/// Stored items for the picker tests, newest first, from device `box`.
#[cfg(test)]
pub(crate) async fn sample(
    texts: &[&str],
) -> (
    crate::commands::support::TestStore,
    Vec<passalong_core::model::ItemMeta>,
) {
    use passalong_core::model::NewItem;
    let ts = crate::commands::support::TestStore::new();
    for text in texts {
        ts.store
            .put(
                NewItem::text("box"),
                crate::commands::support::bytes(text.as_bytes()),
            )
            .await
            .unwrap();
        ts.clock.advance(1);
    }
    let items = ts.store.list().await.unwrap();
    (ts, items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::{TestStore, bytes};
    use passalong_core::clipboard::{Clipboard, ClipboardError};
    use passalong_core::model::NewItem;
    use passalong_core::testing::MockClipboard;
    use state::Mode;
    use std::collections::VecDeque;
    use std::path::PathBuf;

    /// What one frame showed.
    #[derive(Debug, Clone)]
    struct Shown {
        status: Option<String>,
        mode: Mode,
        items: usize,
    }

    /// A screen that replays keys, records each frame, and can remove an
    /// item's folder before a given key, behind the picker's back.
    struct FakeScreen {
        keys: VecDeque<KeyEvent>,
        frames: Vec<Shown>,
        keys_read: usize,
        remove_before_key: Option<(usize, PathBuf)>,
    }

    impl FakeScreen {
        fn new(keys: impl IntoIterator<Item = KeyCode>) -> Self {
            Self {
                keys: keys
                    .into_iter()
                    .map(|code| KeyEvent::new(code, KeyModifiers::NONE))
                    .collect(),
                frames: Vec::new(),
                keys_read: 0,
                remove_before_key: None,
            }
        }

        fn statuses(&self) -> Vec<Option<&str>> {
            self.frames
                .iter()
                .map(|frame| frame.status.as_deref())
                .collect()
        }
    }

    impl Screen for FakeScreen {
        fn draw(&mut self, picker: &Picker) -> io::Result<()> {
            self.frames.push(Shown {
                status: picker.status().map(str::to_owned),
                mode: picker.mode().clone(),
                items: picker.total(),
            });
            Ok(())
        }
        fn next_key(&mut self) -> io::Result<KeyEvent> {
            if let Some((at, dir)) = &self.remove_before_key
                && *at == self.keys_read
            {
                std::fs::remove_dir_all(dir)?;
            }
            self.keys_read += 1;
            self.keys
                .pop_front()
                .ok_or_else(|| io::Error::other("no keys left"))
        }
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn item_dir(ts: &TestStore, id: &ItemId) -> PathBuf {
        ts.dir.path().join("items").join(id.as_str())
    }

    #[tokio::test]
    async fn loading_shows_first_then_enter_chooses_loading_the_item() {
        let (ts, items) = sample(&["newer", "older"]).await;
        let mut screen = FakeScreen::new([KeyCode::Down, KeyCode::Enter]);
        let chosen = run_picker(&ts.store, &mut screen, utc()).await.unwrap();
        assert_eq!(
            chosen,
            Some(Chosen {
                action: Action::Load,
                id: items[1].id.clone()
            })
        );
        assert_eq!(screen.statuses(), [Some("Loading..."), None, None]);
        assert_eq!(screen.frames[0].items, 0);
        assert_eq!(screen.frames[1].items, 2);
    }

    #[tokio::test]
    async fn quitting_chooses_nothing() {
        let (ts, _) = sample(&["a"]).await;
        let mut screen = FakeScreen::new([KeyCode::Char('q')]);
        assert_eq!(
            run_picker(&ts.store, &mut screen, utc()).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn g_shows_the_metadata_in_the_list_and_esc_returns_to_it() {
        let (ts, items) = sample(&["hello"]).await;
        let mut screen = FakeScreen::new([KeyCode::Char('g'), KeyCode::Esc, KeyCode::Char('q')]);
        assert_eq!(
            run_picker(&ts.store, &mut screen, utc()).await.unwrap(),
            None
        );
        let Mode::Details {
            title,
            lines,
            scroll,
        } = &screen.frames[2].mode
        else {
            panic!("no details: {:?}", screen.frames[2]);
        };
        assert_eq!(title, items[0].id.as_str());
        assert_eq!(*scroll, 0);
        assert_eq!(lines[0], format!("id:      {}", items[0].id));
        assert!(
            lines.iter().any(|line| line == "preview: hello"),
            "{lines:?}"
        );
        assert_eq!(screen.frames[3].mode, Mode::Browse);
    }

    #[tokio::test]
    async fn g_on_an_item_that_is_gone_says_so() {
        let (ts, items) = sample(&["a"]).await;
        let mut screen = FakeScreen::new([KeyCode::Char('g'), KeyCode::Char('q')]);
        screen.remove_before_key = Some((0, item_dir(&ts, &items[0].id)));
        assert_eq!(
            run_picker(&ts.store, &mut screen, utc()).await.unwrap(),
            None
        );
        let last = screen.frames.last().unwrap();
        assert_eq!(last.mode, Mode::Browse);
        assert!(
            last.status
                .as_deref()
                .unwrap()
                .starts_with(&format!("cannot read {}", items[0].id)),
            "{last:?}"
        );
    }

    #[tokio::test]
    async fn d_then_y_deletes_and_shows_the_list_read_again() {
        let (ts, items) = sample(&["other", "chosen"]).await;
        let mut screen =
            FakeScreen::new([KeyCode::Char('d'), KeyCode::Char('y'), KeyCode::Char('q')]);
        // Another device removes the other item meanwhile; reading the list
        // again shows it gone too.
        screen.remove_before_key = Some((1, item_dir(&ts, &items[1].id)));
        assert_eq!(
            run_picker(&ts.store, &mut screen, utc()).await.unwrap(),
            None
        );
        assert!(ts.store.list().await.unwrap().is_empty());
        let deleted = format!("deleted {}", items[0].id);
        let deleting = format!("Deleting {}...", items[0].id);
        assert_eq!(
            screen.statuses()[3..],
            [
                Some(deleting.as_str()),
                Some("Reloading..."),
                Some(deleted.as_str())
            ]
        );
        assert_eq!(screen.frames.last().unwrap().items, 0);
    }

    #[tokio::test]
    async fn r_shows_reloading_until_the_list_is_read() {
        let (ts, _) = sample(&["a"]).await;
        let mut screen = FakeScreen::new([KeyCode::Char('r'), KeyCode::Char('q')]);
        assert_eq!(
            run_picker(&ts.store, &mut screen, utc()).await.unwrap(),
            None
        );
        assert_eq!(
            screen.statuses(),
            [
                Some("Loading..."),
                None,
                Some("Reloading..."),
                Some("reloaded: 1 items")
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_delete_is_shown_and_the_list_stays_open() {
        let (ts, items) = sample(&["a"]).await;
        let mut screen =
            FakeScreen::new([KeyCode::Char('d'), KeyCode::Char('y'), KeyCode::Char('q')]);
        // Gone from the store just before `y` is pressed.
        screen.remove_before_key = Some((1, item_dir(&ts, &items[0].id)));
        assert_eq!(
            run_picker(&ts.store, &mut screen, utc()).await.unwrap(),
            None
        );
        let last = screen.frames.last().unwrap().status.clone().unwrap();
        assert!(
            last.starts_with(&format!("cannot delete {}", items[0].id)),
            "{last}"
        );
    }

    fn targets<'a, 'b>(
        downloads: &'a Path,
        open: &'a mut OpenClipboard<'b>,
    ) -> ActionTargets<'a, 'b> {
        ActionTargets {
            download_dir: downloads,
            open_clipboard: open,
            stdout_is_terminal: false,
            offset: utc(),
        }
    }

    async fn perform_on(
        ts: &TestStore,
        action: Action,
        id: &ItemId,
        clip: &MockClipboard,
    ) -> String {
        let downloads = ts.dir.path().join("downloads");
        let clip = clip.clone();
        let mut open =
            move || -> Result<Box<dyn Clipboard>, ClipboardError> { Ok(Box::new(clip.clone())) };
        let mut out = Vec::new();
        let chosen = Chosen {
            action,
            id: id.clone(),
        };
        perform(&ts.store, &chosen, targets(&downloads, &mut open), &mut out)
            .await
            .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[tokio::test]
    async fn the_actions_do_what_load_and_cat_do() {
        let (ts, items) = sample(&["hello"]).await;
        let clip = MockClipboard::new();
        let id = &items[0].id;
        assert_eq!(perform_on(&ts, Action::Cat, id, &clip).await, "hello");
        perform_on(&ts, Action::Load, id, &clip).await;
        assert_eq!(clip.writes(), ["hello"]);
        let file = ts
            .store
            .put(NewItem::file("notes.txt", "box"), bytes(b"file body"))
            .await
            .unwrap()
            .meta;
        let loaded = perform_on(&ts, Action::Load, &file.id, &clip).await;
        let target = ts.dir.path().join("downloads/notes.txt");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "file body");
        assert!(loaded.contains("notes.txt"), "{loaded}");
    }
}
