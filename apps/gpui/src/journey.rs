//! The research history of a project, a month at a time: which month is
//! shown, how its records are grouped into the researcher's own days, and
//! what each record is called. Days are local days, so the time zone is a
//! parameter here and the window's is used when drawing.
use crate::i18n::{Locale, tr};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone};
use wisp_dto::{
    ResearchGraph, ResearchJourney, ResearchJourneyEntry, ResearchJourneySource, ResearchRecap,
    ResearchRecapEdit,
};

use crate::present::Tone;

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

/// The icon of a record's kind, from the shared vocabulary, in place of a
/// word: a question asked `chat`, a run `flask`, a file `doc`, a decision
/// `check-square`, a next step `arrow-right`, a note of one's own
/// `note-plus`, and the rest each its own.
pub fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "session" => "chat",
        "run" => "flask",
        "artifact" => "doc",
        "decision" => "check-square",
        "next" => "arrow-right",
        "progress" | "finding" => "note-plus",
        "paper" => "book",
        "data_asset" => "database",
        "archive" => "archive",
        _ => "research-trail",
    }
}

/// How a record's state reads as a stamp: a run's progress, a source that
/// is gone.
pub fn status_tone(entry: &ResearchJourneyEntry) -> Tone {
    match (entry.kind.as_str(), entry.status.as_str()) {
        ("run", "succeeded") => Tone::Success,
        ("run", "failed") => Tone::Danger,
        ("run", "cancelled") => Tone::Neutral,
        ("run", _) => Tone::Active,
        ("artifact", _) => Tone::Attention,
        _ => Tone::Neutral,
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
    /// The local date.
    pub date: NaiveDate,
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
                    date,
                    label: day_label(date),
                    entries: vec![(time, entry)],
                },
            )),
        }
    }
    days.into_iter().map(|(_, day)| day).collect()
}

/// The local day as seconds since 1970: from its midnight up to the next.
pub fn day_range<Tz: TimeZone>(date: NaiveDate, zone: &Tz) -> (i64, i64) {
    let midnight = |date: NaiveDate| {
        date.and_hms_opt(0, 0, 0)
            .and_then(|at| zone.from_local_datetime(&at).earliest())
            .map_or(0, |at| at.timestamp())
    };
    let next = date.succ_opt().unwrap_or(date);
    (midnight(date), midnight(next))
}

/// The recap of a local day, unless the researcher dismissed it.
pub fn recap_for<'a, Tz: TimeZone>(
    journey: &'a ResearchJourney,
    date: NaiveDate,
    zone: &Tz,
) -> Option<&'a ResearchRecap> {
    journey.recaps.iter().find(|recap| {
        recap.status != "dismissed" && local(recap.day_start, zone).date_naive() == date
    })
}

/// How a recap stands, as its chip says.
pub fn recap_status(status: &str) -> (&'static str, Tone) {
    match status {
        "confirmed" => (tr("Confirmed"), Tone::Success),
        _ => (tr("Draft"), Tone::Attention),
    }
}

/// A recap's sections that say something, each with its items.
pub fn recap_sections(recap: &ResearchRecap) -> Vec<(&'static str, Vec<String>)> {
    [
        (tr("Done"), &recap.done),
        (tr("Findings"), &recap.findings),
        (tr("Issues"), &recap.issues),
        (tr("Next"), &recap.next),
    ]
    .into_iter()
    .filter(|(_, items)| !items.is_empty())
    .map(|(label, items)| (label, items.iter().map(|item| item.text.clone()).collect()))
    .collect()
}

/// The researcher's review of a recap as it stands, with a new status.
pub fn reviewed(recap: &ResearchRecap, status: &str) -> ResearchRecapEdit {
    ResearchRecapEdit {
        id: recap.id.clone(),
        status: status.into(),
        headline: recap.headline.clone(),
        done: recap.done.clone(),
        findings: recap.findings.clone(),
        issues: recap.issues.clone(),
        next: recap.next.clone(),
    }
}

/// A recap's list as the editor shows it: one item a line.
pub fn as_lines(items: &[wisp_dto::ResearchRecapItem]) -> String {
    items
        .iter()
        .map(|item| item.text.trim())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A list as edited: one item a non-empty line, a leading "- " or "• "
/// dropped. A line left as it was keeps the sources it had.
pub fn from_lines(
    before: &[wisp_dto::ResearchRecapItem],
    lines: &str,
) -> Vec<wisp_dto::ResearchRecapItem> {
    lines
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches("- ")
                .trim_start_matches("• ")
                .trim()
        })
        .filter(|line| !line.is_empty())
        .map(|line| wisp_dto::ResearchRecapItem {
            text: line.to_owned(),
            refs: before
                .iter()
                .find(|item| item.text.trim() == line)
                .map(|item| item.refs.clone())
                .unwrap_or_default(),
        })
        .collect()
}

/// A research-graph kind's name, as a column or section heading.
pub fn graph_kind_label(kind: &str) -> &'static str {
    match kind {
        "decision" => tr("Decisions"),
        "paper" => tr("Papers"),
        "data_asset" => tr("Data assets"),
        "run" => tr("Runs"),
        "artifact" => tr("Artifacts"),
        _ => tr("Records"),
    }
}

/// How much the graph shown holds, "3 nodes · 2 relationships".
pub fn graph_summary(graph: &ResearchGraph) -> String {
    crate::i18n::fill(
        tr("{} nodes · {} relationships"),
        &[&graph.nodes.len(), &graph.edges.len()],
    )
}

/// The part of the graph the page's filter asks for: the nodes whose
/// title, reference, kind or metadata hold every word, the nodes directly
/// related to one of them, and the relations that touch one of them. No
/// words keep the whole graph. Order stays as stored, so the layout of
/// what is kept does not jump about.
pub fn filter_graph(graph: &ResearchGraph, filter: &str) -> ResearchGraph {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return graph.clone();
    }
    let matched: std::collections::HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|node| {
            let text = format!(
                "{} {} {} {} {}",
                node.title,
                node.ref_id.as_deref().unwrap_or_default(),
                node.kind,
                graph_kind_label(&node.kind),
                wisp_dto::research_graph::metadata_summary(&node.metadata_json)
            )
            .to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .map(|node| node.id.as_str())
        .collect();
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| {
            matched.contains(edge.source_id.as_str()) || matched.contains(edge.target_id.as_str())
        })
        .cloned()
        .collect();
    let kept: std::collections::HashSet<&str> = edges
        .iter()
        .flat_map(|edge| [edge.source_id.as_str(), edge.target_id.as_str()])
        .chain(matched.iter().copied())
        .collect();
    ResearchGraph {
        nodes: graph
            .nodes
            .iter()
            .filter(|node| kept.contains(node.id.as_str()))
            .cloned()
            .collect(),
        edges,
    }
}

/// `text` cut to `limit` characters, the last of them an ellipsis.
pub fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars()
        .take(limit.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

/// A run's status in words, for any status the host records.
pub fn run_status(status: &str) -> &'static str {
    match status {
        "submitted" => tr("submitted"),
        "started" | "running" => tr("started"),
        "succeeded" => tr("finished"),
        "failed" => tr("failed"),
        "cancelled" => tr("cancelled"),
        "lost" => tr("lost"),
        _ => tr("unknown"),
    }
}

/// A moment as local date and time, "2026-11-14 13:05".
pub fn local_moment<Tz: TimeZone>(at: i64, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    local(at, zone).format("%Y-%m-%d %H:%M").to_string()
}

/// What the source details say about the run that made an output: its
/// title, or that none is recorded; then where it ran and how it ended,
/// and when it ended, when those are known.
pub fn source_run_lines<Tz: TimeZone>(
    source: &ResearchJourneySource,
    zone: &Tz,
) -> (String, Vec<String>)
where
    Tz::Offset: std::fmt::Display,
{
    if source.run_id.is_none() {
        return (tr("No producing run recorded").to_owned(), Vec::new());
    }
    let mut lines = vec![format!(
        "{} · {}",
        source.context_id,
        run_status(&source.run_status)
    )];
    if let Some(at) = source.generated_at {
        lines.push(crate::i18n::fill(
            tr("Completed {}"),
            &[&local_moment(at, zone)],
        ));
    }
    let title = if source.run_title.trim().is_empty() {
        tr("Untitled run").to_owned()
    } else {
        source.run_title.clone()
    };
    (title, lines)
}

/// What an input of the run is to it and how sure the record is of it,
/// "input · exact".
pub fn input_detail(input: &wisp_dto::ResearchJourneyInput) -> String {
    [input.role.trim(), input.confidence.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The two ends of an arrow's head at the last point of `points`, set back
/// along the last piece of the line.
pub fn arrow_head(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let [.., (fx, fy), (tx, ty)] = points else {
        return Vec::new();
    };
    let (dx, dy) = (tx - fx, ty - fy);
    let length = (dx * dx + dy * dy).sqrt();
    if length == 0. {
        return Vec::new();
    }
    let (ux, uy) = (dx / length, dy / length);
    let (back, side) = (7., 3.5);
    vec![
        (tx - ux * back - uy * side, ty - uy * back + ux * side),
        (tx - ux * back + uy * side, ty - uy * back - ux * side),
    ]
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

    #[test]
    fn a_records_kind_is_an_icon_of_its_own() {
        let kinds = [
            "session",
            "run",
            "artifact",
            "decision",
            "next",
            "progress",
            "paper",
            "data_asset",
            "archive",
            "other",
        ];
        for kind in kinds {
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("assets/icons/{}.svg", kind_icon(kind)))
                    .exists(),
                "{kind}"
            );
        }
        assert_eq!(kind_icon("session"), "chat");
        assert_eq!(kind_icon("run"), "flask");
        assert_eq!(kind_icon("decision"), "check-square");
        assert_eq!(kind_icon("next"), "arrow-right");
    }

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
    fn a_days_recap_is_found_by_its_local_date_unless_dismissed() {
        use wisp_dto::ResearchRecapItem;
        let tokyo = zone(9);
        let date = NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
        let (start, end) = day_range(date, &tokyo);
        assert_eq!(end - start, 86_400);
        assert_eq!(local(start, &tokyo).date_naive(), date);
        let item = |text: &str| ResearchRecapItem {
            text: text.into(),
            refs: vec![0],
        };
        let recap = ResearchRecap {
            id: "r".into(),
            day_start: start,
            status: "draft".into(),
            headline: "Filter settled".into(),
            findings: vec![item("Cells under 200 genes go")],
            next: vec![item("Re-cluster")],
            ..Default::default()
        };
        let mut journey = ResearchJourney {
            recaps: vec![recap.clone()],
            ..Default::default()
        };
        assert_eq!(
            recap_for(&journey, date, &tokyo).map(|r| r.id.as_str()),
            Some("r")
        );
        assert!(recap_for(&journey, date.succ_opt().unwrap(), &tokyo).is_none());
        assert_eq!(recap_status("draft"), ("Draft", Tone::Attention));
        assert_eq!(recap_status("confirmed"), ("Confirmed", Tone::Success));
        let sections = recap_sections(&recap);
        assert_eq!(
            sections,
            vec![
                ("Findings", vec!["Cells under 200 genes go".to_owned()]),
                ("Next", vec!["Re-cluster".to_owned()]),
            ]
        );
        let confirmed = reviewed(&recap, "confirmed");
        assert_eq!(confirmed.status, "confirmed");
        assert_eq!(confirmed.findings, recap.findings);
        journey.recaps[0].status = "dismissed".into();
        assert!(recap_for(&journey, date, &tokyo).is_none());
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

    fn graph() -> ResearchGraph {
        use wisp_dto::{ResearchEdge, ResearchNode};
        let node = |id: &str, kind: &str, title: &str| ResearchNode {
            id: id.into(),
            kind: kind.into(),
            title: title.into(),
            ref_id: None,
            metadata_json: "{}".into(),
        };
        let edge = |source: &str, target: &str, relation: &str| ResearchEdge {
            source_id: source.into(),
            target_id: target.into(),
            relation: relation.into(),
            metadata_json: "{}".into(),
        };
        let mut paper = node("p1", "paper", "Love et al. 2014");
        paper.metadata_json = r#"{"doi":"10.1186/s13059"}"#.into();
        ResearchGraph {
            nodes: vec![
                node("d1", "decision", "Use DESeq2"),
                paper,
                node("a1", "data_asset", "counts.tsv"),
                node("r1", "run", "Align reads"),
            ],
            edges: vec![
                edge("d1", "p1", "cites"),
                edge("d1", "a1", "applies to"),
                edge("r1", "a1", "produced"),
            ],
        }
    }

    #[test]
    fn the_filter_keeps_what_matches_and_what_it_is_directly_related_to() {
        let whole = graph();
        assert_eq!(filter_graph(&whole, "  "), whole);
        assert_eq!(graph_summary(&whole), "4 nodes · 3 relationships");
        // A paper found by its metadata brings the decision citing it.
        let cited = filter_graph(&whole, "s13059");
        assert_eq!(
            cited
                .nodes
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            ["d1", "p1"]
        );
        assert_eq!(cited.edges.len(), 1);
        // The decision brings both things it points at, in stored order,
        // and not the run that only shares a neighbour with it.
        let decided = filter_graph(&whole, "deseq2 DECISIONS");
        assert_eq!(
            decided
                .nodes
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            ["d1", "p1", "a1"]
        );
        assert_eq!(decided.edges.len(), 2);
        // The filtered graph lays out with every kept relation joined.
        let laid = wisp_dto::research_graph::layout(&decided);
        assert_eq!(laid.edges.len(), 2);
        assert!(filter_graph(&whole, "nothing like this").nodes.is_empty());
        assert_eq!(graph_kind_label("data_asset"), "Data assets");
        assert_eq!(shorten("applies to the counts", 10), "applies t…");
        assert_eq!(shorten("cites", 10), "cites");
    }

    #[test]
    fn an_outputs_source_names_its_run_and_its_inputs() {
        let utc = zone(0);
        assert_eq!(
            source_run_lines(&ResearchJourneySource::default(), &utc),
            ("No producing run recorded".to_owned(), vec![])
        );
        let source = ResearchJourneySource {
            run_id: Some("r1".into()),
            run_title: "Count reads".into(),
            run_status: "succeeded".into(),
            context_id: "local".into(),
            // 2026-11-14 12:00 UTC.
            generated_at: Some(1_794_657_600),
            inputs: vec![],
        };
        assert_eq!(
            source_run_lines(&source, &utc),
            (
                "Count reads".to_owned(),
                vec![
                    "local · finished".to_owned(),
                    "Completed 2026-11-14 12:00".to_owned()
                ]
            )
        );
        assert_eq!(
            source_run_lines(&source, &zone(8)).1[1],
            "Completed 2026-11-14 20:00"
        );
        let input = wisp_dto::ResearchJourneyInput {
            title: "data/reads.csv".into(),
            role: "input".into(),
            version_id: None,
            confidence: "exact".into(),
        };
        assert_eq!(input_detail(&input), "input · exact");
        assert_eq!(run_status("lost"), "lost");
        assert_eq!(run_status("weird"), "unknown");
    }

    #[test]
    fn an_arrow_head_points_back_along_the_last_piece() {
        // Ending rightwards at (10, 0): both ends behind it, one each side.
        let wings = arrow_head(&[(0., 0.), (10., 0.)]);
        assert_eq!(wings, [(3., 3.5), (3., -3.5)]);
        // Ending downwards.
        let wings = arrow_head(&[(0., 0.), (0., 10.)]);
        assert_eq!(wings, [(-3.5, 3.), (3.5, 3.)]);
        assert!(arrow_head(&[(1., 1.)]).is_empty());
        assert!(arrow_head(&[(1., 1.), (1., 1.)]).is_empty());
    }

    #[test]
    fn an_edited_recap_list_keeps_the_sources_of_lines_left_alone() {
        use wisp_dto::ResearchRecapItem;
        let before = [
            ResearchRecapItem {
                text: "Aligned 12 samples".into(),
                refs: vec![0, 2],
            },
            ResearchRecapItem {
                text: "Counted reads".into(),
                refs: vec![1],
            },
        ];
        assert_eq!(as_lines(&before), "Aligned 12 samples\nCounted reads");
        let after = from_lines(
            &before,
            "- Aligned 12 samples\n\n• Counted reads twice\n  Plotted PCA ",
        );
        assert_eq!(
            after
                .iter()
                .map(|item| (item.text.as_str(), item.refs.clone()))
                .collect::<Vec<_>>(),
            [
                ("Aligned 12 samples", vec![0, 2]),
                ("Counted reads twice", vec![]),
                ("Plotted PCA", vec![]),
            ]
        );
        assert!(from_lines(&before, " \n").is_empty());
    }
}
