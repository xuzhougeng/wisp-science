//! The research calendar across projects: the weeks of a month, how many
//! records each day holds, and what happened on one day. It reads the same
//! history as the journey, for every project at once.
use std::collections::HashSet;

use chrono::{Datelike, NaiveDate, TimeZone, Timelike};
use wisp_dto::{ResearchCalendarProject, ResearchJourneyEntry};

use crate::journey::Month;

/// The weeks of `month`, Monday first: each place holds the day of the
/// month, or nothing where the week belongs to a neighbouring month.
pub fn weeks(month: Month) -> Vec<[Option<u32>; 7]> {
    let Some(first) = NaiveDate::from_ymd_opt(month.year, month.month, 1) else {
        return Vec::new();
    };
    let next = month.next();
    let length = NaiveDate::from_ymd_opt(next.year, next.month, 1)
        .map_or(30, |after| (after - first).num_days() as u32);
    let mut weeks = Vec::new();
    let mut week = [None; 7];
    let mut place = first.weekday().num_days_from_monday() as usize;
    for day in 1..=length {
        week[place] = Some(day);
        place += 1;
        if place == 7 {
            weeks.push(week);
            week = [None; 7];
            place = 0;
        }
    }
    if place > 0 {
        weeks.push(week);
    }
    weeks
}

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// The day of `month` that `at` falls on in `zone`, if it is in the month.
fn day_in<Tz: TimeZone>(at: i64, month: Month, zone: &Tz) -> Option<(u32, String)> {
    let local = zone.timestamp_opt(at, 0).earliest()?;
    (local.year() == month.year && local.month() == month.month).then(|| {
        (
            local.day(),
            format!("{:02}:{:02}", local.hour(), local.minute()),
        )
    })
}

/// The entries of the projects that are not hidden.
fn shown<'a>(
    projects: &'a [ResearchCalendarProject],
    hidden: &'a HashSet<String>,
) -> impl Iterator<Item = (&'a str, &'a ResearchJourneyEntry)> {
    projects
        .iter()
        .filter(|project| !hidden.contains(&project.project_id))
        .flat_map(|project| {
            project
                .history
                .entries
                .iter()
                .map(|entry| (project.project_id.as_str(), entry))
        })
}

/// How many records each day of `month` holds; index 0 is unused, so a day
/// of the month is its own index.
pub fn counts<Tz: TimeZone>(
    projects: &[ResearchCalendarProject],
    hidden: &HashSet<String>,
    month: Month,
    zone: &Tz,
) -> [usize; 32] {
    let mut counts = [0; 32];
    for (_, entry) in shown(projects, hidden) {
        if let Some((day, _)) = day_in(entry.occurred_at, month, zone) {
            counts[day as usize] += 1;
        }
    }
    counts
}

/// One record on the chosen day.
#[derive(Debug, PartialEq, Eq)]
pub struct Item<'a> {
    pub project: &'a str,
    /// Local time of day, "14:05".
    pub time: String,
    pub entry: &'a ResearchJourneyEntry,
}

/// What happened on `day` of `month`, in every project that is not hidden,
/// newest first.
pub fn on_day<'a, Tz: TimeZone>(
    projects: &'a [ResearchCalendarProject],
    hidden: &'a HashSet<String>,
    month: Month,
    day: u32,
    zone: &Tz,
) -> Vec<Item<'a>> {
    let mut items: Vec<Item> = shown(projects, hidden)
        .filter_map(|(project, entry)| {
            let (on, time) = day_in(entry.occurred_at, month, zone)?;
            (on == day).then_some(Item {
                project,
                time,
                entry,
            })
        })
        .collect();
    items.sort_by(|a, b| {
        b.entry
            .occurred_at
            .cmp(&a.entry.occurred_at)
            .then_with(|| a.entry.id.cmp(&b.entry.id))
    });
    items
}

/// The day to show when a month opens: today when the month is the
/// present one, otherwise its last day with something on it, if any.
pub fn opening_day(counts: &[usize; 32], today: Option<u32>) -> Option<u32> {
    today.or_else(|| (1..32u32).rev().find(|day| counts[*day as usize] > 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use wisp_dto::ResearchJourney;

    fn zone(hours: i32) -> FixedOffset {
        FixedOffset::east_opt(hours * 3_600).unwrap()
    }

    const NOVEMBER: Month = Month {
        year: 2026,
        month: 11,
    };
    /// 2026-11-14 12:00 UTC, a Saturday.
    const NOON: i64 = 1_794_657_600;

    fn project(id: &str, at: &[(i64, &str)]) -> ResearchCalendarProject {
        ResearchCalendarProject {
            project_id: id.into(),
            history: ResearchJourney {
                entries: at
                    .iter()
                    .map(|(occurred_at, title)| ResearchJourneyEntry {
                        id: (*title).into(),
                        title: (*title).into(),
                        kind: "session".into(),
                        occurred_at: *occurred_at,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
            error: None,
        }
    }

    #[test]
    fn a_month_is_laid_out_in_weeks_from_monday() {
        // November 2026 starts on a Sunday and has thirty days.
        let november = weeks(NOVEMBER);
        assert_eq!(november.len(), 6);
        assert_eq!(november[0], [None, None, None, None, None, None, Some(1)]);
        assert_eq!(november[1][0], Some(2));
        assert_eq!(november[2][5], Some(14));
        assert_eq!(november[5], [Some(30), None, None, None, None, None, None]);
        // February 2027 starts on a Monday and fills four weeks exactly.
        let february = weeks(Month {
            year: 2027,
            month: 2,
        });
        assert_eq!(february.len(), 4);
        assert_eq!(february[0][0], Some(1));
        assert_eq!(february[3][6], Some(28));
        // Every day of a month appears once.
        let days: Vec<u32> = weeks(NOVEMBER).into_iter().flatten().flatten().collect();
        assert_eq!(days, (1..=30).collect::<Vec<_>>());
        assert_eq!(WEEKDAYS.len(), 7);
    }

    #[test]
    fn days_count_what_every_shown_project_did() {
        let projects = [
            project("lab", &[(NOON, "align"), (NOON + 3_600, "count")]),
            // 23:30 UTC on the 13th: the 14th already in UTC+8. And one in
            // October, which is not this month at all.
            project(
                "field",
                &[
                    (NOON - 12 * 3_600 - 1_800, "sample"),
                    (NOON - 20 * 86_400, "old"),
                ],
            ),
        ];
        let none = HashSet::new();
        let utc = counts(&projects, &none, NOVEMBER, &zone(0));
        assert_eq!((utc[13], utc[14]), (1, 2));
        assert_eq!(utc.iter().sum::<usize>(), 3);
        let east = counts(&projects, &none, NOVEMBER, &zone(8));
        assert_eq!((east[13], east[14]), (0, 3));

        // The day's records, newest first, each with its project and time.
        let day = on_day(&projects, &none, NOVEMBER, 14, &zone(8));
        assert_eq!(
            day.iter()
                .map(|item| (item.project, item.time.as_str(), item.entry.title.as_str()))
                .collect::<Vec<_>>(),
            [
                ("lab", "21:00", "count"),
                ("lab", "20:00", "align"),
                ("field", "07:30", "sample")
            ]
        );
        assert!(on_day(&projects, &none, NOVEMBER, 2, &zone(0)).is_empty());

        // A hidden project counts for nothing and lists nothing.
        let hidden = HashSet::from(["lab".to_string()]);
        let without = counts(&projects, &hidden, NOVEMBER, &zone(0));
        assert_eq!((without[13], without[14]), (1, 0));
        assert!(on_day(&projects, &hidden, NOVEMBER, 14, &zone(0)).is_empty());

        // The present month opens on today; another on its last busy day.
        assert_eq!(opening_day(&utc, Some(20)), Some(20));
        assert_eq!(opening_day(&utc, None), Some(14));
        assert_eq!(opening_day(&[0; 32], None), None);
    }
}
