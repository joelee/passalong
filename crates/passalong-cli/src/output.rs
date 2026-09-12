//! Command output: aligned tables for people, JSON for scripts.

use chrono::FixedOffset;
use passalong_core::model::{ItemKind, ItemMeta};

/// Widest a NAME cell may be, in characters.
const NAME_WIDTH: usize = 40;

/// Formats a byte count with binary units: `5 B`, `1.5 KiB`, `3.0 GiB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Renders items as an aligned table in the given order, with times shown
/// at `offset`.
pub fn render_table(items: &[ItemMeta], offset: FixedOffset) -> String {
    if items.is_empty() {
        return "no items\n".to_owned();
    }
    let header = ["ID", "KIND", "NAME", "SIZE", "DEVICE", "CREATED"].map(str::to_owned);
    let rows: Vec<[String; 6]> = items
        .iter()
        .map(|meta| {
            [
                meta.id.to_string(),
                kind_name(meta.kind).to_owned(),
                display_name(meta),
                human_size(meta.size),
                meta.device.clone(),
                meta.created_at
                    .with_timezone(&offset)
                    .format("%Y-%m-%d %H:%M")
                    .to_string(),
            ]
        })
        .collect();
    let widths: [usize; 6] = std::array::from_fn(|column| {
        std::iter::once(&header)
            .chain(&rows)
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    });
    let mut out = String::new();
    for row in std::iter::once(&header).chain(&rows) {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// Renders items as a pretty-printed JSON array.
pub fn render_json(items: &[ItemMeta]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(items)? + "\n")
}

/// Lower-case kind name shown in tables.
pub fn kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Text => "text",
        ItemKind::File => "file",
    }
}

/// The file name for files, the preview for text, or `-`.
fn display_name(meta: &ItemMeta) -> String {
    match (&meta.name, &meta.preview) {
        (Some(text), _) | (None, Some(text)) => truncate(text, NAME_WIDTH),
        (None, None) => "-".to_owned(),
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(width - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use passalong_core::clock::Clock;
    use passalong_core::model::{ContentHasher, ItemMeta, NewItem, preview_of};
    use passalong_core::testing::FixedClock;

    fn meta(item: NewItem, bytes: &[u8], at: &str) -> ItemMeta {
        let mut h = ContentHasher::new();
        h.update(bytes);
        let preview = (item.name.is_none()).then(|| preview_of(&String::from_utf8_lossy(bytes)));
        item.finish(FixedClock::at(at).now(), &h.finalize(), preview)
            .unwrap()
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    #[test]
    fn sizes_are_human_readable() {
        let cases = [
            (0, "0 B"),
            (1023, "1023 B"),
            (1024, "1.0 KiB"),
            (1536, "1.5 KiB"),
            (5 * 1024 * 1024, "5.0 MiB"),
            (3 * 1024 * 1024 * 1024, "3.0 GiB"),
            (u64::MAX, "16777216.0 TiB"),
        ];
        for (bytes, text) in cases {
            assert_eq!(human_size(bytes), text, "{bytes}");
        }
    }

    #[test]
    fn table_lists_items_in_the_given_order_with_aligned_columns() {
        let file = meta(
            NewItem::file("report.pdf", "laptop"),
            &[7_u8; 1536],
            "2026-09-12T09:54:11Z",
        );
        let text = meta(NewItem::text("box"), b"hello", "2026-09-12T09:53:11Z");
        let row = |c: [&str; 6]| {
            format!(
                "{:<21}  {:<4}  {:<10}  {:<7}  {:<6}  {}\n",
                c[0], c[1], c[2], c[3], c[4], c[5]
            )
        };
        let expected = [
            row(["ID", "KIND", "NAME", "SIZE", "DEVICE", "CREATED"]),
            row([
                file.id.as_str(),
                "file",
                "report.pdf",
                "1.5 KiB",
                "laptop",
                "2026-09-12 09:54",
            ]),
            row([
                "6aa52107-2cf24dba5fb0",
                "text",
                "hello",
                "5 B",
                "box",
                "2026-09-12 09:53",
            ]),
        ]
        .concat();
        assert_eq!(render_table(&[file, text], utc()), expected);
    }

    #[test]
    fn table_uses_the_given_utc_offset() {
        let text = meta(NewItem::text("box"), b"hello", "2026-09-12T09:53:11Z");
        let table = render_table(&[text], FixedOffset::east_opt(2 * 3600).unwrap());
        assert!(table.contains("2026-09-12 11:53"), "{table}");
    }

    #[test]
    fn long_names_are_cut_to_forty_characters() {
        let long = "word ".repeat(30);
        let text = meta(
            NewItem::text("box"),
            long.as_bytes(),
            "2026-09-12T09:53:11Z",
        );
        let table = render_table(&[text], utc());
        let name = table
            .lines()
            .nth(1)
            .unwrap()
            .split("  ")
            .nth(2)
            .unwrap()
            .trim_end();
        assert_eq!(name.chars().count(), 40);
        assert!(name.ends_with('…'));
    }

    #[test]
    fn items_without_name_or_preview_show_a_dash() {
        let mut text = meta(NewItem::text("box"), b"hello", "2026-09-12T09:53:11Z");
        text.preview = None;
        let table = render_table(&[text], utc());
        assert!(table.lines().nth(1).unwrap().contains("  -  "), "{table}");
    }

    #[test]
    fn empty_lists_say_so() {
        assert_eq!(render_table(&[], utc()), "no items\n");
        assert_eq!(render_json(&[]).unwrap(), "[]\n");
    }

    #[test]
    fn json_round_trips_the_metadata() {
        let items = vec![meta(NewItem::text("box"), b"hello", "2026-09-12T09:53:11Z")];
        let json = render_json(&items).unwrap();
        assert!(json.ends_with("]\n"));
        let back: Vec<ItemMeta> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, items);
    }
}
