//! The picker's state: the items, the filter, the selection, and what each
//! key does. It knows nothing about the terminal, so it is tested with
//! plain key events.

use passalong_core::model::{ItemId, ItemMeta};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::output::kind_label;

/// How far Page Up and Page Down move.
const PAGE: usize = 10;

/// What to do with the chosen item once the list is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// As `passalong load <ID>`.
    Load,
    /// As `passalong cat <ID>`.
    Cat,
    /// As `passalong get <ID>`.
    Get,
}

/// What a key press led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Keep going.
    Continue,
    /// Close the list and do nothing.
    Quit,
    /// Close the list and act on the item.
    Act(Action, ItemId),
    /// Delete the item and stay in the list.
    Delete(ItemId),
    /// List the store again.
    Reload,
}

/// What keys mean at the moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Keys move and act.
    Browse,
    /// Keys edit the filter.
    Filter,
    /// The next key confirms deleting this item, if it is `y`.
    ConfirmDelete(ItemId),
}

/// The items, the filter, the selection, and a status message.
#[derive(Debug)]
pub struct Picker {
    items: Vec<ItemMeta>,
    filter: String,
    mode: Mode,
    selected: usize,
    status: Option<String>,
}

impl Picker {
    /// A picker over `items`, shown in the given order, with the first
    /// selected.
    pub fn new(items: Vec<ItemMeta>) -> Self {
        Self {
            items,
            filter: String::new(),
            mode: Mode::Browse,
            selected: 0,
            status: None,
        }
    }

    /// The items the filter lets through, in order.
    pub fn visible(&self) -> Vec<&ItemMeta> {
        let needle = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|meta| needle.is_empty() || matches(meta, &needle))
            .collect()
    }

    /// The selected item, if any is visible.
    pub fn selected(&self) -> Option<&ItemMeta> {
        self.visible().get(self.selected).copied()
    }

    /// The selected row among the visible items.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The filter text.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// What keys mean at the moment.
    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// The last status message.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// How many items there are, filtered or not.
    pub fn total(&self) -> usize {
        self.items.len()
    }

    /// Shows `text` in the status line.
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some(text.into());
    }

    /// Replaces the items, keeping the selection in range.
    pub fn replace(&mut self, items: Vec<ItemMeta>) {
        self.items = items;
        self.clamp();
    }

    /// Drops a deleted item, keeping the selection in range.
    pub fn remove(&mut self, id: &ItemId) {
        self.items.retain(|meta| &meta.id != id);
        self.clamp();
    }

    /// Handles one key.
    pub fn handle(&mut self, key: KeyEvent) -> Outcome {
        if key.kind != KeyEventKind::Press {
            return Outcome::Continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Outcome::Quit;
        }
        match self.mode.clone() {
            Mode::Filter => {
                self.filter_key(key.code);
                Outcome::Continue
            }
            Mode::ConfirmDelete(id) => {
                self.mode = Mode::Browse;
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    Outcome::Delete(id)
                } else {
                    self.set_status("not deleted");
                    Outcome::Continue
                }
            }
            Mode::Browse => self.browse_key(key.code),
        }
    }

    fn filter_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            KeyCode::Enter => self.mode = Mode::Browse,
            KeyCode::Esc => {
                self.filter.clear();
                self.selected = 0;
                self.mode = Mode::Browse;
            }
            _ => {}
        }
    }

    fn browse_key(&mut self, code: KeyCode) -> Outcome {
        let last = self.visible().len().saturating_sub(1);
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(PAGE),
            KeyCode::PageDown => self.selected = (self.selected + PAGE).min(last),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = last,
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                self.status = None;
            }
            KeyCode::Char('r') => return Outcome::Reload,
            KeyCode::Char('q') | KeyCode::Esc => return Outcome::Quit,
            KeyCode::Enter => return self.act(Action::Load),
            KeyCode::Char('c') => return self.act(Action::Cat),
            KeyCode::Char('g') => return self.act(Action::Get),
            KeyCode::Char('d') => match self.selected().map(|meta| meta.id.clone()) {
                Some(id) => self.mode = Mode::ConfirmDelete(id),
                None => self.set_status("no item selected"),
            },
            _ => {}
        }
        Outcome::Continue
    }

    fn act(&mut self, action: Action) -> Outcome {
        match self.selected().map(|meta| meta.id.clone()) {
            Some(id) => Outcome::Act(action, id),
            None => {
                self.set_status("no item selected");
                Outcome::Continue
            }
        }
    }

    fn clamp(&mut self) {
        let last = self.visible().len().saturating_sub(1);
        self.selected = self.selected.min(last);
    }
}

/// Whether the lower-case `needle` is in the item's id, full name or
/// preview, device, or kind, ignoring case.
fn matches(meta: &ItemMeta, needle: &str) -> bool {
    let name = meta
        .name
        .as_deref()
        .or(meta.preview.as_deref())
        .unwrap_or_default();
    [
        meta.id.as_str(),
        name,
        meta.device.as_str(),
        kind_label(meta),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::choose::sample;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ch(c: char) -> KeyEvent {
        key(KeyCode::Char(c))
    }

    fn typed(picker: &mut Picker, text: &str) {
        for c in text.chars() {
            assert_eq!(picker.handle(ch(c)), Outcome::Continue);
        }
    }

    #[tokio::test]
    async fn moving_stays_within_the_list() {
        let (_ts, items) = sample(&["a", "b", "c"]).await;
        let mut picker = Picker::new(items.clone());
        assert_eq!(picker.selected().unwrap().id, items[0].id, "newest first");
        picker.handle(key(KeyCode::Up));
        assert_eq!(picker.selected_index(), 0);
        for _ in 0..5 {
            picker.handle(key(KeyCode::Down));
        }
        assert_eq!(picker.selected_index(), 2);
        picker.handle(ch('k'));
        assert_eq!(picker.selected_index(), 1);
        picker.handle(ch('j'));
        assert_eq!(picker.selected_index(), 2);
        picker.handle(key(KeyCode::Home));
        assert_eq!(picker.selected_index(), 0);
        picker.handle(key(KeyCode::End));
        assert_eq!(picker.selected_index(), 2);
        picker.handle(key(KeyCode::PageUp));
        assert_eq!(picker.selected_index(), 0);
        picker.handle(key(KeyCode::PageDown));
        assert_eq!(picker.selected_index(), 2);
    }

    #[tokio::test]
    async fn slash_filters_case_insensitively_until_enter_or_esc() {
        let (_ts, items) = sample(&["Alpha one", "beta two", "gamma"]).await;
        let mut picker = Picker::new(items);
        picker.handle(key(KeyCode::Down));
        assert_eq!(picker.handle(ch('/')), Outcome::Continue);
        assert_eq!(picker.mode(), &Mode::Filter);
        typed(&mut picker, "ALPx");
        assert!(picker.visible().is_empty());
        assert_eq!(picker.selected(), None);
        picker.handle(key(KeyCode::Backspace));
        assert_eq!(picker.filter(), "ALP");
        assert_eq!(picker.visible().len(), 1);
        assert_eq!(picker.selected_index(), 0, "a new filter starts at the top");
        // Letters are text while filtering, not commands.
        assert_eq!(picker.handle(key(KeyCode::Enter)), Outcome::Continue);
        assert_eq!(picker.mode(), &Mode::Browse);
        assert_eq!(picker.filter(), "ALP", "Enter keeps the filter");
        assert_eq!(picker.visible().len(), 1);
        picker.handle(ch('/'));
        picker.handle(key(KeyCode::Esc));
        assert_eq!(picker.mode(), &Mode::Browse);
        assert_eq!(picker.filter(), "", "Esc clears the filter");
        assert_eq!(picker.visible().len(), 3);
        // The id, kind, and device match too.
        for text in ["box", "TEXT"] {
            picker.handle(ch('/'));
            typed(&mut picker, text);
            assert_eq!(picker.visible().len(), 3, "{text}");
            picker.handle(key(KeyCode::Esc));
        }
        let id = picker.visible()[2].id.to_string();
        picker.handle(ch('/'));
        typed(&mut picker, &id);
        assert_eq!(picker.visible().len(), 1);
    }

    #[tokio::test]
    async fn enter_c_and_g_choose_an_action_for_the_selected_item() {
        let (_ts, items) = sample(&["a", "b"]).await;
        let mut picker = Picker::new(items.clone());
        picker.handle(key(KeyCode::Down));
        let id = items[1].id.clone();
        assert_eq!(
            picker.handle(key(KeyCode::Enter)),
            Outcome::Act(Action::Load, id.clone())
        );
        assert_eq!(
            picker.handle(ch('c')),
            Outcome::Act(Action::Cat, id.clone())
        );
        assert_eq!(picker.handle(ch('g')), Outcome::Act(Action::Get, id));
        picker.handle(ch('/'));
        typed(&mut picker, "nothing matches");
        picker.handle(key(KeyCode::Enter));
        assert_eq!(picker.handle(key(KeyCode::Enter)), Outcome::Continue);
        assert_eq!(picker.status(), Some("no item selected"));
    }

    #[tokio::test]
    async fn d_asks_and_only_y_deletes() {
        let (_ts, items) = sample(&["a"]).await;
        let id = items[0].id.clone();
        let mut picker = Picker::new(items);
        assert_eq!(picker.handle(ch('d')), Outcome::Continue);
        assert_eq!(picker.mode(), &Mode::ConfirmDelete(id.clone()));
        assert_eq!(picker.handle(ch('n')), Outcome::Continue);
        assert_eq!(picker.mode(), &Mode::Browse);
        assert_eq!(picker.status(), Some("not deleted"));
        picker.handle(ch('d'));
        assert_eq!(picker.handle(ch('y')), Outcome::Delete(id));
        assert_eq!(picker.mode(), &Mode::Browse);
    }

    #[tokio::test]
    async fn r_reloads_and_q_esc_or_ctrl_c_quit() {
        let (_ts, items) = sample(&["a"]).await;
        let mut picker = Picker::new(items);
        assert_eq!(picker.handle(ch('r')), Outcome::Reload);
        assert_eq!(picker.handle(ch('q')), Outcome::Quit);
        assert_eq!(picker.handle(key(KeyCode::Esc)), Outcome::Quit);
        assert_eq!(
            picker.handle(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Outcome::Quit
        );
    }

    #[tokio::test]
    async fn the_selection_stays_in_range_as_items_go() {
        let (_ts, items) = sample(&["a", "b", "c"]).await;
        let mut picker = Picker::new(items.clone());
        picker.handle(key(KeyCode::End));
        picker.remove(&items[2].id);
        assert_eq!(picker.selected().unwrap().id, items[1].id);
        assert_eq!(picker.visible().len(), 2);
        picker.replace(items[..1].to_vec());
        assert_eq!(picker.selected().unwrap().id, items[0].id);
        picker.replace(Vec::new());
        assert_eq!(picker.selected(), None);
        picker.set_status("reloaded");
        assert_eq!(picker.status(), Some("reloaded"));
    }
}
