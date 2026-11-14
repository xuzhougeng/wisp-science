//! The research history of a project, a month at a time: which month is
//! shown, how its records are grouped into the researcher's own days, and
//! what each record is called. Days are local days, so the time zone is a
//! parameter here and the window's is used when drawing.
use crate::i18n::{Locale, tr};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone};
use wisp_dto::{ResearchJourney, ResearchJourneyEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Month {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
}

impl Month {
    /// The month `at` (seconds since 1970) falls in, in `zone`.
    pub fn containing<Tz: TimeZone>(at: i64, zone: &Tz) -> Self {
        let date = local(at, zone).date_naive();
        Self {
            year: date.year(),
            month: date.month(),
        }
    }

    pub fn previous(self) -> Self {
        match self.month {
            1 => Self {
                year: self.year - 1,
                month: 12,
            },
            month => Self {
                year: self.year,
                month: month - 1,
            },
        }
    }

    pub fn next(self) -> Self {
        match self.month {
            12 => Self {
                year: self.year + 1,
                month: 1,
            },
            month => Self {
                year: self.year,
                month: month + 1,
            },
        }
    }

    pub fn label(self) -> String {
        self.label_in(crate::i18n::current())
    }

    fn label_in(self, locale: Locale) -> String {
        if locale == Locale::Zh {
            return format!("{}年{}月", self.year, self.month);
        }
        const NAMES: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        format!("{} {}", NAMES[(self.month as usize - 1) % 12], self.year)
    }

    /// The month as seconds since 1970: from its first local midnight up to
    /// the next month's. A month is never more than 31 days, which is what
    /// the host accepts in one read.
    pub fn range<Tz: TimeZone>(self, zone: &Tz) -> (i64, i64) {
        (midnight(self, zone), midnight(self.next(), zone))
    }
}

fn midnight<Tz: TimeZone>(month: Month, zone: &Tz) -> i64 {
    let first = NaiveDate::from_ymd_opt(month.year, month.month, 1)
        .unwrap_or_default()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default();
    // A midnight that does not exist or happens twice (a clock change) is
    // taken at its earliest reading.
    zone.from_local_datetime(&first)
        .earliest()
        .map_or_else(|| first.and_utc().timestamp(), |local| local.timestamp())
}

fn local<Tz: TimeZone>(at: i64, zone: &Tz) -> DateTime<Tz> {
    zone.timestamp_opt(at, 0)
        .earliest()
        .unwrap_or_else(|| zone.timestamp_opt(0, 0).unwrap())
}

/// What kind of record an entry is, in words.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "run" => tr("Run"),
        "artifact" => tr("File"),
        "session" => tr("Asked"),
        "decision" => tr("Decision"),
        "paper" => tr("Paper"),
        "data_asset" => tr("Data"),
        "progress" => tr("Progress"),
        "finding" => tr("Finding"),
        "next" => tr("Next"),
        "archive" => tr("Archive"),
        _ => tr("Record"),
    }
}

/// The kinds a researcher can record themselves, with their labels.
pub const CATEGORIES: [(&str, &str); 4] = [
    ("progress", "Progress"),
    ("finding", "Finding"),
    ("decision", "Decision"),
    ("next", "Next"),
];

/// What an entry's status adds to its kind, if anything worth saying.
pub fn status_label(entry: &ResearchJourneyEntry) -> Option<&'static str> {
    match (entry.kind.as_str(), entry.status.as_str()) {
        ("run", "submitted") => Some(tr("submitted")),
        ("run", "started") => Some(tr("started")),
        ("run", "succeeded") => Some(tr("finished")),
        ("run", "failed") => Some(tr("failed")),
        ("run", "cancelled") => Some(tr("cancelled")),
        ("artifact", _) if entry.source_discarded => Some(tr("source discarded")),
        _ => None,
    }
}

/// One local day of the history.
#[derive(Debug, PartialEq, Eq)]
pub struct Day<'a> {
    /// "Saturday 14 November".
    pub label: String,
    /// Newest first, each with its local time of day, "14:05".
    pub entries: Vec<(String, &'a ResearchJourneyEntry)>,
}

/// The entries that match `filter`, grouped into local days, newest day
/// first. The filter is words found in a record's title, its summary or
/// the name of its kind, ignoring case; no words keep everything.
pub fn days<'a, Tz: TimeZone>(
    journey: &'a ResearchJourney,
    filter: &str,
    zone: &Tz,
) -> Vec<Day<'a>> {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    let mut days: Vec<(NaiveDate, Day)> = Vec::new();
    for entry in &journey.entries {
        let text = format!(
            "{} {} {}",
            entry.title,
            entry.summary,
            kind_label(&entry.kind)
        )
        .to_lowercase();
        if !words.iter().all(|word| text.contains(word)) {
            continue;
        }
        let at = local(entry.occurred_at, zone);
        let date = at.date_naive();
        let time = format!(
            "{:02}:{:02}",
            chrono::Timelike::hour(&at),
            chrono::Timelike::minute(&at)
        );
        match days.last_mut() {
            Some((last, day)) if *last == date => day.entries.push((time, entry)),
            _ => days.push((
                date,
                Day {
                    label: day_label(date),
                    entries: vec![(time, entry)],
                },
            )),
        }
    }
    days.into_iter().map(|(_, day)| day).collect()
}

fn day_label(date: NaiveDate) -> String {
    day_label_in(crate::i18n::current(), date)
}

fn day_label_in(locale: Locale, date: NaiveDate) -> String {
    let weekday = date.weekday().num_days_from_monday() as usize;
    if locale == Locale::Zh {
        const DAYS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
        return format!("{}月{}日 星期{}", date.month(), date.day(), DAYS[weekday]);
    }
    const DAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    let month = Month {
        year: date.year(),
        month: date.month(),
    }
    .label_in(Locale::En);
    let month = month.split(' ').next().unwrap_or_default();
    format!("{} {} {month}", DAYS[weekday], date.day())
}

/// One line saying how much the month holds.
pub fn summary(journey: &ResearchJourney, shown: usize) -> String {
    let total = journey.entries.len();
    if crate::i18n::current() == Locale::Zh {
        let mut line = match (total, shown == total) {
            (0, _) => return tr("Nothing is recorded for this month.").into(),
            (n, true) => format!("{n} 条记录"),
            (n, false) => format!("{n} 条记录中的 {shown} 条"),
        };
        if journey.truncated {
            line.push_str(" · 只显示最新的");
        }
        return line;
    }
    let mut line = match (total, shown == total) {
        (0, _) => return "Nothing is recorded for this month.".into(),
        (1, true) => "1 record".to_owned(),
        (n, true) => format!("{n} records"),
        (n, false) => format!("{shown} of {n} records"),
    };
    if journey.truncated {
        line.push_str(" · only the newest are shown");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn zone(hours: i32) -> FixedOffset {
        FixedOffset::east_opt(hours * 3_600).unwrap()
    }

    fn entry(kind: &str, title: &str, occurred_at: i64) -> ResearchJourneyEntry {
        ResearchJourneyEntry {
            kind: kind.into(),
            title: title.into(),
            occurred_at,
            ..Default::default()
        }
    }

    #[test]
    fn a_month_is_the_researchers_own_month() {
        // 2026-11-14 12:00 UTC.
        let noon = 1_794_657_600;
        let november = Month {
            year: 2026,
            month: 11,
        };
        assert_eq!(Month::containing(noon, &zone(0)), november);
        assert_eq!(november.label(), "November 2026");
        // Months wrap around the year in both directions.
        let january = Month {
            year: 2027,
            month: 1,
        };
        assert_eq!(november.next().next(), january);
        assert_eq!(january.previous().previous(), november);
        assert_eq!(january.previous().label(), "December 2026");

        // The range runs from local midnight to local midnight: thirty days
        // of November, and eight hours earlier in UTC+8 than in UTC.
        let (from, until) = november.range(&zone(0));
        assert_eq!(until - from, 30 * 86_400);
        assert_eq!(november.range(&zone(8)).0, from - 8 * 3_600);
        assert!(from <= noon && noon < until);
        // The last evening of October in UTC is already November in UTC+8.
        let evening = from - 3_600;
        assert_eq!(Month::containing(evening, &zone(0)).month, 10);
        assert_eq!(Month::containing(evening, &zone(8)), november);
        // No month is longer than the host reads at once.
        let mut month = november;
        for _ in 0..24 {
            let (from, until) = month.range(&zone(-5));
            assert!((28 * 86_400..=31 * 86_400).contains(&(until - from)));
            month = month.next();
        }
    }

    #[test]
    fn records_are_grouped_into_local_days_and_filtered_by_words() {
        // 2026-11-14 12:00 UTC, a Saturday.
        let noon = 1_794_657_600;
        let journey = ResearchJourney {
            entries: vec![
                entry("run", "Align reads", noon + 3_600),
                entry("finding", "Reads are clean", noon),
                // 23:30 UTC the day before: the same day in UTC, and
                // already the 14th in UTC+8.
                entry("session", "count reads per gene", noon - 12 * 3_600 - 1_800),
            ],
            ..Default::default()
        };
        let utc = days(&journey, "", &zone(0));
        assert_eq!(
            utc.iter().map(|day| day.label.as_str()).collect::<Vec<_>>(),
            ["Saturday 14 November", "Friday 13 November"]
        );
        assert_eq!(utc[0].entries.len(), 2);
        assert_eq!(utc[0].entries[0].0, "13:00");
        assert_eq!(utc[1].entries[0].0, "23:30");
        let east = days(&journey, "", &zone(8));
        assert_eq!(east.len(), 1);
        assert_eq!(east[0].entries[2].0, "07:30");

        // Words are looked for in the title, the summary and the kind.
        let found = days(&journey, "READS finding", &zone(0));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].entries[0].1.title, "Reads are clean");
        assert!(days(&journey, "nothing like this", &zone(0)).is_empty());

        assert_eq!(summary(&journey, 3), "3 records");
        assert_eq!(summary(&journey, 1), "1 of 3 records");
        assert_eq!(
            summary(&ResearchJourney::default(), 0),
            "Nothing is recorded for this month."
        );
        let cut = ResearchJourney {
            truncated: true,
            ..journey
        };
        assert!(summary(&cut, 3).ends_with("only the newest are shown"));
    }

    #[test]
    fn months_and_days_are_said_the_chinese_way_in_chinese() {
        let month = Month {
            year: 2026,
            month: 11,
        };
        assert_eq!(month.label_in(Locale::Zh), "2026年11月");
        assert_eq!(month.label_in(Locale::En), "November 2026");
        let saturday = NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
        assert_eq!(day_label_in(Locale::Zh, saturday), "11月14日 星期六");
        assert_eq!(day_label_in(Locale::En, saturday), "Saturday 14 November");
        let sunday = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        assert_eq!(day_label_in(Locale::Zh, sunday), "11月15日 星期日");
    }

    #[test]
    fn kinds_and_statuses_are_named_for_the_reader() {
        assert_eq!(kind_label("artifact"), "File");
        assert_eq!(kind_label("session"), "Asked");
        assert_eq!(kind_label("something new"), "Record");
        for (category, label) in CATEGORIES {
            assert_eq!(kind_label(category), label);
        }
        let mut run = entry("run", "Align", 0);
        run.status = "failed".into();
        assert_eq!(status_label(&run), Some("failed"));
        let mut file = entry("artifact", "plot.png", 0);
        assert_eq!(status_label(&file), None);
        file.source_discarded = true;
        assert_eq!(status_label(&file), Some("source discarded"));
        assert_eq!(status_label(&entry("finding", "x", 0)), None);
    }
}
