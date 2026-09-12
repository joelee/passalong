//! Choosing which items `prune` deletes.
//!
//! Pure functions over item metadata, so the rules are easy to test:
//! [`parse_age`] reads ages such as `30d`, and [`select`] applies
//! `--older-than` and `--keep`.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::model::{ItemId, ItemMeta};

/// An age that could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid age `{0}`: use a positive number followed by m, h, d, or w, such as 30d")]
pub struct InvalidAge(String);

/// Parses an age: a positive whole number followed by `m` (minutes), `h`
/// (hours), `d` (days), or `w` (weeks), such as `90m` or `30d`.
///
/// # Errors
///
/// [`InvalidAge`] for anything else, including zero and ages too large to
/// represent.
pub fn parse_age(text: &str) -> Result<Duration, InvalidAge> {
    let invalid = || InvalidAge(text.to_owned());
    let (number, unit) = text.split_at(text.len().saturating_sub(1));
    let unit_secs: u64 = match unit {
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return Err(invalid()),
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let count: u64 = number.parse().map_err(|_| invalid())?;
    let secs = count
        .checked_mul(unit_secs)
        .filter(|secs| *secs > 0)
        .ok_or_else(invalid)?;
    // Keep ages within what date arithmetic can subtract from "now".
    TimeDelta::try_seconds(i64::try_from(secs).map_err(|_| invalid())?).ok_or_else(invalid)?;
    Ok(Duration::from_secs(secs))
}

/// The items to delete, oldest last, given the two rules:
///
/// - `keep`: the newest `keep` items survive;
/// - `older_than`: items created more than that long before `now` are
///   eligible.
///
/// With both, an item survives if either rule protects it. With neither,
/// nothing is selected.
pub fn select(
    items: &[ItemMeta],
    now: DateTime<Utc>,
    older_than: Option<Duration>,
    keep: Option<usize>,
) -> Vec<ItemId> {
    if older_than.is_none() && keep.is_none() {
        return Vec::new();
    }
    let cutoff = older_than.map(|age| {
        TimeDelta::from_std(age)
            .ok()
            .and_then(|age| now.checked_sub_signed(age))
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
    });
    let mut newest_first: Vec<&ItemMeta> = items.iter().collect();
    newest_first.sort_unstable_by(|a, b| b.id.cmp(&a.id));
    newest_first
        .into_iter()
        .enumerate()
        .filter(|(index, meta)| {
            let beyond_keep = keep.is_none_or(|keep| *index >= keep);
            let old_enough = cutoff.is_none_or(|cutoff| meta.created_at < cutoff);
            beyond_keep && old_enough
        })
        .map(|(_, meta)| meta.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContentHasher, ItemMeta, NewItem};
    use chrono::{DateTime, TimeDelta, Utc};
    use std::time::Duration;

    const NOW: &str = "2026-09-12T12:00:00Z";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(NOW)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn item(secs_ago: i64, text: &str) -> ItemMeta {
        let mut h = ContentHasher::new();
        h.update(text.as_bytes());
        NewItem::text("box")
            .finish(now() - TimeDelta::seconds(secs_ago), &h.finalize(), None)
            .unwrap()
    }

    const DAY: i64 = 86_400;

    fn ids(items: &[&ItemMeta]) -> Vec<crate::model::ItemId> {
        items.iter().map(|m| m.id.clone()).collect()
    }

    #[test]
    fn ages_parse_with_minute_hour_day_and_week_units() {
        for (text, secs) in [
            ("1m", 60),
            ("12h", 43_200),
            ("30d", 2_592_000),
            ("2w", 1_209_600),
            ("90m", 5_400),
        ] {
            assert_eq!(
                parse_age(text).unwrap(),
                Duration::from_secs(secs),
                "{text}"
            );
        }
    }

    #[test]
    fn malformed_zero_and_huge_ages_are_rejected() {
        for bad in [
            "",
            "30",
            "d",
            "30x",
            "-1d",
            "1.5h",
            " 30d",
            "30d ",
            "30D",
            "0d",
            "0m",
            "99999999999999999999w",
            "30000000000000w",
        ] {
            let err = parse_age(bad).unwrap_err();
            assert!(err.to_string().contains("such as 30d"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn keep_alone_deletes_everything_beyond_the_newest_n() {
        let items = [
            item(10, "a"),
            item(20, "b"),
            item(30, "c"),
            item(40, "d"),
            item(50, "e"),
        ];
        assert_eq!(
            select(&items, now(), None, Some(2)),
            ids(&[&items[2], &items[3], &items[4]])
        );
        assert!(select(&items, now(), None, Some(5)).is_empty());
        assert!(select(&items, now(), None, Some(9)).is_empty());
        assert_eq!(select(&items, now(), None, Some(0)).len(), 5);
    }

    #[test]
    fn age_alone_deletes_items_strictly_older_than_it() {
        let items = [
            item(DAY, "a"),
            item(10 * DAY, "b"),
            item(30 * DAY, "exactly"),
            item(40 * DAY, "c"),
        ];
        let thirty_days = Duration::from_secs(30 * DAY as u64);
        assert_eq!(
            select(&items, now(), Some(thirty_days), None),
            ids(&[&items[3]])
        );
    }

    #[test]
    fn with_both_an_item_survives_if_it_is_recent_or_among_the_newest() {
        let thirty_days = Some(Duration::from_secs(30 * DAY as u64));
        let recent = [item(DAY, "a"), item(40 * DAY, "b"), item(50 * DAY, "c")];
        assert_eq!(
            select(&recent, now(), thirty_days, Some(1)),
            ids(&[&recent[1], &recent[2]])
        );
        let all_old = [item(40 * DAY, "d"), item(50 * DAY, "e")];
        assert_eq!(
            select(&all_old, now(), thirty_days, Some(1)),
            ids(&[&all_old[1]]),
            "newest old item kept by --keep"
        );
        let all_recent = [item(DAY, "f"), item(2 * DAY, "g"), item(3 * DAY, "h")];
        assert!(
            select(&all_recent, now(), thirty_days, Some(1)).is_empty(),
            "recent items kept by age"
        );
    }

    #[test]
    fn selection_does_not_depend_on_input_order_and_ignores_empty_rules() {
        let items = [item(50, "old"), item(10, "new"), item(30, "mid")];
        assert_eq!(
            select(&items, now(), None, Some(1)),
            ids(&[&items[2], &items[0]])
        );
        assert!(select(&items, now(), None, None).is_empty());
        assert!(select(&[], now(), Some(Duration::from_secs(60)), Some(1)).is_empty());
    }
}
