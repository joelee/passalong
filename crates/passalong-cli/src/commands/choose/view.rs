//! Drawing the picker: the item table, the status or filter line, and the
//! key hints.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Cell, Clear, Padding, Paragraph, Row, Table, TableState};

use super::state::{Mode, Picker};
use crate::output::{human_size, kind_label};
use crate::resolve::age;

/// The key hints on the last line.
const HINTS: &str = "? help  Enter load  c print  g details  d delete  / filter  r reload  q quit";

/// Every key the picker understands, and what it does, for the help.
pub const KEYS: [(&str, &str); 11] = [
    ("Up, Down, k, j", "Move"),
    ("Page Up, Down", "Move ten rows"),
    ("Home, End", "First or last item"),
    ("/", "Filter; Enter keeps it, Esc clears it"),
    ("Enter", "Load: text to the clipboard, files to downloads"),
    ("c", "Print the item"),
    ("g", "Show its metadata in a scrollable dialog"),
    ("d", "Delete after y, then reload the list"),
    ("r", "Reload the list"),
    ("?", "Show this help"),
    ("q, Esc, Ctrl-C", "Quit"),
];

/// Width of the key column in the help.
const KEY_WIDTH: usize = 16;

/// One row of the help's key table.
pub fn key_line(keys: &str, action: &str) -> String {
    format!("{keys:<KEY_WIDTH$}{action}")
}

/// Draws the picker: the table of visible items with the selection, then
/// the filter, question, or status line, then the key hints. Ages are
/// relative to `now`.
pub fn render(frame: &mut Frame, picker: &Picker, now: DateTime<Utc>) {
    let [table_area, status_area, hints_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let rows = picker.visible().into_iter().map(|meta| {
        let name = meta
            .name
            .as_deref()
            .or(meta.preview.as_deref())
            .unwrap_or("-")
            .to_owned();
        Row::new([
            Cell::from(meta.id.to_string()),
            Cell::from(kind_label(meta)),
            Cell::from(name),
            Cell::from(human_size(meta.size)),
            Cell::from(meta.device.clone()),
            Cell::from(age(meta.created_at, now)),
        ])
    });
    let widths = [
        Constraint::Length(21),
        Constraint::Length(5),
        Constraint::Fill(1),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(10),
    ];
    let header = Row::new(["ID", "KIND", "NAME", "SIZE", "DEVICE", "AGE"])
        .style(Style::new().add_modifier(Modifier::BOLD));
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    let mut state =
        TableState::new().with_selected(picker.selected().map(|_| picker.selected_index()));
    frame.render_stateful_widget(table, table_area, &mut state);
    frame.render_widget(Paragraph::new(bottom_line(picker)), status_area);
    frame.render_widget(
        Paragraph::new(HINTS).style(Style::new().add_modifier(Modifier::DIM)),
        hints_area,
    );
    match picker.mode() {
        Mode::Help => render_help(frame),
        Mode::Details {
            title,
            lines,
            scroll,
        } => render_details(frame, title, lines, *scroll),
        _ => {}
    }
}

/// A `width` by `height` rectangle in the middle of `area`, cut down to
/// fit it.
fn centered(area: Rect, width: usize, height: usize) -> Rect {
    let width = u16::try_from(width).unwrap_or(u16::MAX).min(area.width);
    let height = u16::try_from(height).unwrap_or(u16::MAX).min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// A dialog over the list with `lines` from line `scroll` on, as far as
/// the dialog fits.
fn render_details(frame: &mut Frame, title: &str, lines: &[String], scroll: usize) {
    let widest = lines
        .iter()
        .map(|line| line.chars().count())
        .chain([title.chars().count()])
        .max()
        .unwrap_or(0);
    // Borders and one column of padding on each side.
    let popup = centered(frame.area(), widest + 4, lines.len() + 2);
    let shown = usize::from(popup.height.saturating_sub(2));
    let scroll = scroll.min(lines.len().saturating_sub(shown));
    let block = Block::bordered()
        .title(format!(" {title} "))
        .title_bottom(" Up/Down scroll, Esc closes ")
        .padding(Padding::horizontal(1));
    let text: Vec<Line> = lines.iter().map(|line| Line::raw(line.as_str())).collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text)
            .block(block)
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        popup,
    );
}

/// A dialog over the list: what passalong is, then every key. It is cut
/// down to fit a small terminal.
fn render_help(frame: &mut Frame) {
    let mut lines = vec![
        Line::styled(
            format!("passalong {}", env!("CARGO_PKG_VERSION")),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::raw(crate::cli::ABOUT),
        Line::raw(format!(
            "{}, {}",
            env!("CARGO_PKG_REPOSITORY"),
            env!("CARGO_PKG_LICENSE")
        )),
        Line::raw(""),
    ];
    lines.extend(
        KEYS.iter()
            .map(|(keys, action)| Line::raw(key_line(keys, action))),
    );
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Any key closes this help",
        Style::new().add_modifier(Modifier::DIM),
    ));
    // Borders and one column of padding on each side.
    let width = lines.iter().map(Line::width).max().unwrap_or(0) + 4;
    let popup = centered(frame.area(), width, lines.len() + 2);
    let block = Block::bordered()
        .title(" Help ")
        .padding(Padding::horizontal(1));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The filter being typed, the delete question, the last status, or a
/// count of the items.
fn bottom_line(picker: &Picker) -> String {
    match picker.mode() {
        Mode::Filter => format!("/{}", picker.filter()),
        Mode::ConfirmDelete(id) => format!("Delete {id}? y to delete, any other key to keep it"),
        Mode::Help => "any key closes the help".to_owned(),
        Mode::Details { .. } => "Up and Down scroll, Esc closes the details".to_owned(),
        Mode::Browse => match picker.status() {
            Some(status) => status.to_owned(),
            None if picker.filter().is_empty() => format!("{} items", picker.total()),
            None => format!(
                "{} of {} items, filter: {}",
                picker.visible().len(),
                picker.total(),
                picker.filter()
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::choose::sample;
    use crate::commands::choose::state::Picker;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Draws `picker` on a 100×8 test screen and returns its text.
    fn draw(picker: &Picker, now: DateTime<Utc>) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 8)).unwrap();
        terminal.draw(|frame| render(frame, picker, now)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[tokio::test]
    async fn the_table_shows_the_columns_the_selection_and_the_key_hints() {
        let (_ts, items) = sample(&["older", "hello world"]).await;
        let now = items[0].created_at + chrono::TimeDelta::minutes(5);
        let picker = Picker::new(items.clone());
        let screen = draw(&picker, now);
        for header in ["ID", "KIND", "NAME", "SIZE", "DEVICE", "AGE"] {
            assert!(screen.contains(header), "{header}:\n{screen}");
        }
        let selected = screen
            .lines()
            .find(|line| line.contains(items[0].id.as_str()))
            .unwrap();
        assert!(selected.starts_with("> "), "{screen}");
        for cell in ["text", "hello world", "11 B", "box", "5 min ago"] {
            assert!(selected.contains(cell), "{cell}: {selected}");
        }
        assert!(screen.contains("2 items"), "{screen}");
        for hint in [
            "? help",
            "Enter load",
            "c print",
            "g details",
            "d delete",
            "/ filter",
            "r reload",
            "q quit",
        ] {
            assert!(screen.contains(hint), "{hint}:\n{screen}");
        }
    }

    #[tokio::test]
    async fn question_mark_shows_the_about_details_and_every_key() {
        let (_ts, items) = sample(&["hello"]).await;
        let mut picker = Picker::new(items.clone());
        picker.handle(ch('?'));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| render(frame, &picker, items[0].created_at))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        let version = format!("passalong {}", env!("CARGO_PKG_VERSION"));
        for text in [
            version.as_str(),
            "clipboard and file sharing over SSH",
            "https://github.com/joelee/passalong, Apache-2.0",
            "Any key closes this help",
        ] {
            assert!(screen.contains(text), "{text}:\n{screen}");
        }
        for (keys, action) in KEYS {
            let row = key_line(keys, action);
            assert!(screen.contains(&row), "{row}:\n{screen}");
        }
    }

    #[tokio::test]
    async fn the_details_dialog_shows_a_scrolled_window_of_the_metadata() {
        let (_ts, items) = sample(&["hello"]).await;
        let mut picker = Picker::new(items.clone());
        let lines: Vec<String> = (0..40).map(|n| format!("row {n:02}")).collect();
        picker.show_details(items[0].id.to_string(), lines);
        for _ in 0..5 {
            picker.handle(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal
            .draw(|frame| render(frame, &picker, items[0].created_at))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(screen.contains(items[0].id.as_str()), "title:\n{screen}");
        assert!(screen.contains("row 05"), "{screen}");
        assert!(!screen.contains("row 04"), "scrolled past: {screen}");
        assert!(screen.contains("Esc closes"), "{screen}");
    }

    #[tokio::test]
    async fn the_bottom_line_shows_the_filter_the_question_or_the_status() {
        let (_ts, items) = sample(&["hello"]).await;
        let now = items[0].created_at;
        let mut picker = Picker::new(items.clone());
        picker.handle(ch('/'));
        picker.handle(ch('h'));
        assert!(draw(&picker, now).contains("/h"));
        picker.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(draw(&picker, now).contains("1 of 1 items, filter: h"));
        picker.handle(ch('d'));
        let screen = draw(&picker, now);
        assert!(
            screen.contains(&format!(
                "Delete {}? y to delete, any other key to keep it",
                items[0].id
            )),
            "{screen}"
        );
        picker.handle(ch('n'));
        assert!(draw(&picker, now).contains("not deleted"));
    }
}
