//! Scheduled tasks: how a cadence is chosen in the form, what it comes
//! to in seconds and a first time, and how a task reads in the list.
use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone};
use wisp_server::desktop::{ScheduleRow, ScheduleRun};

use crate::i18n::{fill, tr};

const HOUR: i64 = 3_600;
const DAY: i64 = 24 * HOUR;

/// How often a task repeats, as the form offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    /// Every so many hours, counted from when the task is made.
    Hours,
    /// Every day at a time.
    Daily,
    /// Every week on a day at a time.
    Weekly,
}

impl Repeat {
    pub const ALL: [Repeat; 3] = [Repeat::Hours, Repeat::Daily, Repeat::Weekly];

    pub fn label(self) -> &'static str {
        match self {
            Repeat::Hours => tr("Every few hours"),
            Repeat::Daily => tr("Daily"),
            Repeat::Weekly => tr("Weekly"),
        }
    }
}

/// The days of the week as the form names them, Monday first.
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A time of day typed as `HH:MM`, on the 24-hour clock.
pub fn parse_time(typed: &str) -> Option<NaiveTime> {
    let (hours, minutes) = typed.trim().split_once(':')?;
    if minutes.len() != 2 || hours.is_empty() || hours.len() > 2 {
        return None;
    }
    NaiveTime::from_hms_opt(hours.parse().ok()?, minutes.parse().ok()?, 0)
}

/// What the form comes to: the cadence in seconds and, for a time of
/// day, the first time it comes round after `now`. `Err` says what to
/// correct.
pub fn cadence<Tz: TimeZone>(
    repeat: Repeat,
    hours: &str,
    time: &str,
    weekday: u32,
    now: &DateTime<Tz>,
) -> Result<(i64, Option<i64>), &'static str> {
    if repeat == Repeat::Hours {
        return match hours.trim().parse::<i64>() {
            Ok(count) if (1..=168).contains(&count) => Ok((count * HOUR, None)),
            _ => Err(tr("Give a number of hours from 1 to 168.")),
        };
    }
    let at = parse_time(time).ok_or(tr("Use a 24-hour HH:MM time."))?;
    let mut day = now.date_naive();
    if repeat == Repeat::Weekly {
        let ahead = (weekday + 7 - day.weekday().num_days_from_monday()) % 7;
        day += Duration::days(i64::from(ahead));
    }
    let step = if repeat == Repeat::Weekly { 7 } else { 1 };
    // A time that does not exist on that day (a clock change), or that
    // has passed, moves on one cadence.
    for _ in 0..3 {
        if let Some(first) = now
            .timezone()
            .from_local_datetime(&day.and_time(at))
            .earliest()
            .filter(|first| first > now)
        {
            return Ok((step * DAY, Some(first.timestamp())));
        }
        day += Duration::days(step);
    }
    Err(tr("That time cannot be scheduled."))
}

/// A cadence in words.
pub fn cadence_label(interval_secs: i64) -> String {
    match interval_secs {
        secs if secs == DAY => tr("Daily").to_owned(),
        secs if secs == 7 * DAY => tr("Weekly").to_owned(),
        secs if secs % HOUR == 0 => fill(tr("Every {} h"), &[&(secs / HOUR)]),
        secs => fill(tr("Every {} min"), &[&(secs / 60).max(1)]),
    }
}

fn stamp<Tz: TimeZone>(at: i64, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    zone.timestamp_opt(at, 0)
        .earliest()
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// When a task fires next, or that it is off.
pub fn next_label<Tz: TimeZone>(row: &ScheduleRow, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if row.enabled {
        fill(tr("Next: {}"), &[&stamp(row.next_run_at, zone)])
    } else {
        tr("Off").to_owned()
    }
}

/// How the latest firing went.
pub fn last_label<Tz: TimeZone>(run: Option<&ScheduleRun>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match run {
        None => tr("Not run yet").to_owned(),
        Some(run) => match &run.error {
            Some(error) => fill(
                tr("Failed {}: {}"),
                &[
                    &stamp(run.fired_at, zone),
                    &error.lines().next().unwrap_or_default(),
                ],
            ),
            None => fill(tr("Last run: {}"), &[&stamp(run.fired_at, zone)]),
        },
    }
}

/// How the daily recap run last went.
pub fn recap_line<Tz: TimeZone>(daily: &wisp_dto::DailyRecapAutomation, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if daily.running {
        return tr("Drafting recaps now…").to_owned();
    }
    match (daily.last_run_at, &daily.error) {
        (None, _) => tr("Not run yet").to_owned(),
        (Some(at), Some(error)) => fill(
            tr("Failed {}: {}"),
            &[&stamp(at, zone), &error.lines().next().unwrap_or_default()],
        ),
        (Some(at), None) => fill(
            tr("Last run: {} · {} drafted"),
            &[&stamp(at, zone), &daily.drafted],
        ),
    }
}

/// The state line of a conversation's timer, in the strip above its
/// composer: how often it repeats and when it fires next, or that it is
/// paused, and the error of its latest firing when that failed. A time
/// that has passed is due: it fires at the next look once the
/// conversation is idle.
pub fn timer_line<Tz: TimeZone>(row: &ScheduleRow, now: i64, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let mut parts = Vec::new();
    if !row.enabled {
        parts.push(tr("Timer paused").to_owned());
    }
    parts.push(cadence_label(row.interval_secs));
    if row.enabled {
        parts.push(if row.next_run_at <= now {
            tr("Due now").to_owned()
        } else {
            fill(tr("Next: {}"), &[&clock(row.next_run_at, now, zone)])
        });
    }
    if let Some(error) = row.runs.first().and_then(|run| run.error.as_deref()) {
        parts.push(fill(
            tr("Last run failed: {}"),
            &[&error.lines().next().unwrap_or_default()],
        ));
    }
    parts.join(" · ")
}

/// A time as the hour on the day of `now`, with the date on another day.
fn clock<Tz: TimeZone>(at: i64, now: i64, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let (Some(at), Some(now)) = (
        zone.timestamp_opt(at, 0).earliest(),
        zone.timestamp_opt(now, 0).earliest(),
    ) else {
        return String::new();
    };
    if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%Y-%m-%d %H:%M").to_string()
    }
}

/// What the timer strip's buttons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerAct {
    /// Pause a timer that runs, resume one that is paused.
    Toggle,
    /// Remove it, as `/timer off` does.
    Cancel,
}

/// What a button of the timer strip asks the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimerCall {
    /// Pausing keeps the prompt and the interval. Resuming keeps the
    /// cadence's place on the clock: a time that passed while it was
    /// paused fires once at the next look, as the WebView's does.
    SetEnabled { id: String, enabled: bool },
    /// Removes the conversation's timers.
    Cancel { project: String, session: String },
}

pub fn timer_call(row: &ScheduleRow, act: TimerAct) -> Option<TimerCall> {
    Some(match act {
        TimerAct::Toggle => TimerCall::SetEnabled {
            id: row.id.clone(),
            enabled: !row.enabled,
        },
        TimerAct::Cancel => TimerCall::Cancel {
            project: row.project.clone(),
            session: row.session.clone()?,
        },
    })
}

/// Prompts to start a task from: a name and what is sent.
pub const TEMPLATES: [(&str, &str); 3] = [
    (
        "Weekly report",
        "Summarize this week's research journey for this project into a short weekly report: \
         completed work, key findings and decisions, problems met, and next week's plan. Cite \
         runs and outputs by name. Read-only.",
    ),
    (
        "Check the runs",
        "Look at this project's runs since the last check. For each one that finished, say \
         in a line what it produced; for each one that failed, read its log and say why. \
         Change nothing.",
    ),
    (
        "New literature",
        "Search the literature for papers from the last week on this project's question. \
         List what is new with one line each on why it matters here, with sources. If \
         nothing is new, say so.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate};

    fn zone() -> FixedOffset {
        FixedOffset::east_opt(8 * 3_600).unwrap()
    }

    #[test]
    fn the_daily_recap_says_how_it_last_went() {
        let mut daily = wisp_dto::DailyRecapAutomation::default();
        assert_eq!(recap_line(&daily, &zone()), "Not run yet");
        let at = wednesday(9, 5).timestamp();
        daily.last_run_at = Some(at);
        daily.drafted = 2;
        assert_eq!(
            recap_line(&daily, &zone()),
            "Last run: 2026-11-11 09:05 · 2 drafted"
        );
        daily.error = Some("Root: model unreachable\nmore".into());
        assert_eq!(
            recap_line(&daily, &zone()),
            "Failed 2026-11-11 09:05: Root: model unreachable"
        );
        daily.running = true;
        assert_eq!(recap_line(&daily, &zone()), "Drafting recaps now…");
    }

    /// 2026-11-11 is a Wednesday.
    fn wednesday(hour: u32, minute: u32) -> DateTime<FixedOffset> {
        zone()
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 11, 11)
                    .unwrap()
                    .and_hms_opt(hour, minute, 0)
                    .unwrap(),
            )
            .unwrap()
    }

    fn first(repeat: Repeat, time: &str, weekday: u32, now: &DateTime<FixedOffset>) -> String {
        let (_, start) = cadence(repeat, "", time, weekday, now).unwrap();
        stamp(start.unwrap(), &zone())
    }

    #[test]
    fn a_time_is_read_on_the_24_hour_clock() {
        assert_eq!(parse_time(" 9:05 "), NaiveTime::from_hms_opt(9, 5, 0));
        assert_eq!(parse_time("23:59"), NaiveTime::from_hms_opt(23, 59, 0));
        for wrong in ["", "9", "24:00", "9:5", "09:60", "9am", "009:00", ":30"] {
            assert_eq!(parse_time(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn hours_count_from_now_and_a_time_of_day_from_its_next_coming() {
        let now = wednesday(10, 0);
        assert_eq!(
            cadence(Repeat::Hours, " 6 ", "", 0, &now),
            Ok((6 * HOUR, None))
        );
        assert!(cadence(Repeat::Hours, "0", "", 0, &now).is_err());
        assert!(cadence(Repeat::Hours, "soon", "", 0, &now).is_err());
        assert!(cadence(Repeat::Daily, "", "25:00", 0, &now).is_err());

        // Later today, or tomorrow when the time has passed.
        assert_eq!(first(Repeat::Daily, "18:30", 0, &now), "2026-11-11 18:30");
        assert_eq!(first(Repeat::Daily, "09:00", 0, &now), "2026-11-12 09:00");
        assert_eq!(first(Repeat::Daily, "10:00", 0, &now), "2026-11-12 10:00");
        assert_eq!(cadence(Repeat::Daily, "", "18:30", 0, &now).unwrap().0, DAY);

        // The coming Friday; today when it is still ahead; a week on when not.
        assert_eq!(first(Repeat::Weekly, "09:00", 4, &now), "2026-11-13 09:00");
        assert_eq!(first(Repeat::Weekly, "18:00", 2, &now), "2026-11-11 18:00");
        assert_eq!(first(Repeat::Weekly, "09:00", 2, &now), "2026-11-18 09:00");
        assert_eq!(first(Repeat::Weekly, "09:00", 0, &now), "2026-11-16 09:00");
        assert_eq!(
            cadence(Repeat::Weekly, "", "09:00", 0, &now).unwrap().0,
            7 * DAY
        );
    }

    #[test]
    fn a_task_says_how_often_when_next_and_how_it_last_went() {
        assert_eq!(cadence_label(DAY), "Daily");
        assert_eq!(cadence_label(7 * DAY), "Weekly");
        assert_eq!(cadence_label(6 * HOUR), "Every 6 h");
        assert_eq!(cadence_label(48 * HOUR), "Every 48 h");
        assert_eq!(cadence_label(90 * 60), "Every 90 min");

        let at = wednesday(10, 0).timestamp();
        let mut row = ScheduleRow {
            id: "s".into(),
            project: "p".into(),
            project_name: "Lab".into(),
            name: "Report".into(),
            prompt: "report".into(),
            interval_secs: DAY,
            enabled: true,
            next_run_at: at,
            session: None,
            runs: vec![],
        };
        assert_eq!(next_label(&row, &zone()), "Next: 2026-11-11 10:00");
        row.enabled = false;
        assert_eq!(next_label(&row, &zone()), "Off");
        assert_eq!(last_label(None, &zone()), "Not run yet");
        let mut run = ScheduleRun {
            fired_at: at,
            session: Some("c".into()),
            error: None,
        };
        assert_eq!(
            last_label(Some(&run), &zone()),
            "Last run: 2026-11-11 10:00"
        );
        run.error = Some("No model is set up\nsecond line".into());
        assert_eq!(
            last_label(Some(&run), &zone()),
            "Failed 2026-11-11 10:00: No model is set up"
        );
    }

    fn timer(next: i64) -> ScheduleRow {
        ScheduleRow {
            id: "t".into(),
            project: "p".into(),
            project_name: "Lab".into(),
            name: "check the queue".into(),
            prompt: "check the queue".into(),
            interval_secs: 30 * 60,
            enabled: true,
            next_run_at: next,
            session: Some("c".into()),
            runs: vec![],
        }
    }

    #[test]
    fn a_timer_says_how_often_and_when_next_or_that_it_is_paused() {
        let now = wednesday(10, 0).timestamp();
        let mut row = timer(wednesday(10, 30).timestamp());
        assert_eq!(timer_line(&row, now, &zone()), "Every 30 min · Next: 10:30");
        // On another day the date is given.
        row.next_run_at = wednesday(23, 50).timestamp() + 3_600;
        assert_eq!(
            timer_line(&row, now, &zone()),
            "Every 30 min · Next: 2026-11-12 00:50"
        );
        // A time that has passed waits for the next look.
        row.next_run_at = now;
        assert_eq!(timer_line(&row, now, &zone()), "Every 30 min · Due now");
        row.interval_secs = 2 * HOUR;
        row.enabled = false;
        assert_eq!(timer_line(&row, now, &zone()), "Timer paused · Every 2 h");
        row.runs = vec![ScheduleRun {
            fired_at: now - 60,
            session: Some("c".into()),
            error: Some("A turn is already running\nmore".into()),
        }];
        assert_eq!(
            timer_line(&row, now, &zone()),
            "Timer paused · Every 2 h · Last run failed: A turn is already running"
        );
    }

    #[test]
    fn pause_resume_and_cancel_ask_the_host_for_one_change() {
        let mut row = timer(0);
        assert_eq!(
            timer_call(&row, TimerAct::Toggle),
            Some(TimerCall::SetEnabled {
                id: "t".into(),
                enabled: false
            })
        );
        row.enabled = false;
        assert_eq!(
            timer_call(&row, TimerAct::Toggle),
            Some(TimerCall::SetEnabled {
                id: "t".into(),
                enabled: true
            })
        );
        assert_eq!(
            timer_call(&row, TimerAct::Cancel),
            Some(TimerCall::Cancel {
                project: "p".into(),
                session: "c".into()
            })
        );
        // A task of the Scheduled tasks page is not a conversation's.
        row.session = None;
        assert_eq!(timer_call(&row, TimerAct::Cancel), None);
    }
}
