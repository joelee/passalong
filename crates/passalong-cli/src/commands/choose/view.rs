//! Drawing the picker: the item table, the status or filter line, and the
//! key hints.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use super::state::{Mode, Picker};
use crate::output::{human_size, kind_label};
use crate::resolve::age;

/// The key hints on the last line.
const HINTS: &str = "Enter load  c print  g details  d delete  / filter  r reload  q quit";

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
}

/// The filter being typed, the delete question, the last status, or a
/// count of the items.
fn bottom_line(picker: &Picker) -> String {
    match picker.mode() {
        Mode::Filter => format!("/{}", picker.filter()),
        Mode::ConfirmDelete(id) => format!("Delete {id}? y to delete, any other key to keep it"),
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
