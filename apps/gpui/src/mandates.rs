//! How research mandates read in the window: their status, cadences, KPIs
//! and the form that drafts one.
use crate::i18n::{fill, tr};
use crate::present::Tone;
use chrono::TimeZone;
use wisp_dto::{MandateConstraints, MandateDraft, MandateKpi, MandateRecord};

/// How often a round may come, as the form offers it.
pub const CADENCES: [(i64, &str); 4] = [
    (3_600, "Hourly"),
    (6 * 3_600, "Every 6 hours"),
    (86_400, "Daily"),
    (7 * 86_400, "Weekly"),
];

/// How often a report is written, as the form offers it.
pub const REPORT_CADENCES: [(i64, &str); 2] = [(86_400, "Daily"), (7 * 86_400, "Weekly")];

/// How a mandate stands, as its chip says.
pub fn status(status: &str) -> (&'static str, Tone) {
    match status {
        "paused" => (tr("Paused"), Tone::Neutral),
        "waiting" => (tr("Needs you"), Tone::Attention),
        "done" => (tr("Closed"), Tone::Neutral),
        _ => (tr("Active"), Tone::Active),
    }
}

/// `90 min`, `6 h`, `7 d`.
pub fn cadence(secs: i64) -> String {
    match secs {
        s if s > 0 && s % 86_400 == 0 => format!("{} d", s / 86_400),
        s if s > 0 && s % 3_600 == 0 => format!("{} h", s / 3_600),
        s => format!("{} min", (s / 60).max(1)),
    }
}

/// One KPI from "name, target, period"; only the name is needed.
pub fn parse_kpi(text: &str) -> Option<MandateKpi> {
    let mut parts = text.split([',', '，']).map(str::trim);
    let name = parts.next().filter(|name| !name.is_empty())?;
    Some(MandateKpi {
        name: name.into(),
        target: parts.next().unwrap_or_default().into(),
        period: parts.next().unwrap_or_default().into(),
        ..Default::default()
    })
}

/// A KPI as the form shows it again.
pub fn kpi_text(kpi: &MandateKpi) -> String {
    [kpi.name.as_str(), kpi.target.as_str(), kpi.period.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Where a KPI stands: "Papers filed: 3 / 5 per week".
pub fn kpi_line(kpi: &MandateKpi) -> String {
    let mut line = format!("{}: {}", kpi.name, kpi.current.as_deref().unwrap_or("–"));
    if !kpi.target.is_empty() {
        line.push_str(&format!(" / {} {}", kpi.target, kpi.period));
    }
    line.trim_end().to_owned()
}

/// What the form makes of what was typed. `None` without a project.
pub fn draft(
    project: Option<&str>,
    name: &str,
    goal: &str,
    kpi: &str,
    interval_secs: i64,
    report_interval_secs: i64,
    review_mutations: bool,
) -> Option<MandateDraft> {
    Some(MandateDraft {
        project_id: project?.to_owned(),
        name: name.trim().into(),
        goal: goal.trim().into(),
        kpis: parse_kpi(kpi).into_iter().collect(),
        constraints: MandateConstraints {
            review_mutations,
            ..Default::default()
        },
        interval_secs,
        report_interval_secs,
        ..Default::default()
    })
}

/// When the next round comes, or why none does.
pub fn next_line<Tz: TimeZone>(mandate: &MandateRecord, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match mandate.status.as_str() {
        "paused" => tr("Paused: no round runs until it is resumed").to_owned(),
        "waiting" => tr("Waiting for your answer before the next round").to_owned(),
        "done" => tr("Closed").to_owned(),
        _ => {
            let at = zone
                .timestamp_opt(mandate.next_run_at, 0)
                .earliest()
                .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default();
            fill(
                tr("Next round {} · every {}"),
                &[&at, &cadence(mandate.interval_secs)],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn a_mandate_reads_as_its_status_cadence_and_kpis() {
        assert_eq!(status("waiting"), ("Needs you", Tone::Attention));
        assert_eq!(status("active"), ("Active", Tone::Active));
        assert_eq!(cadence(6 * 3_600), "6 h");
        assert_eq!(cadence(7 * 86_400), "7 d");
        assert_eq!(cadence(90 * 60), "90 min");
        let kpi = parse_kpi(" Papers filed , 5, per week ").unwrap();
        assert_eq!(
            (kpi.name.as_str(), kpi.target.as_str(), kpi.period.as_str()),
            ("Papers filed", "5", "per week")
        );
        assert_eq!(kpi_text(&kpi), "Papers filed, 5, per week");
        assert_eq!(kpi_line(&kpi), "Papers filed: – / 5 per week");
        assert!(parse_kpi("  ").is_none());
        assert_eq!(parse_kpi("Coverage").unwrap().target, "");

        assert!(draft(None, "", "Watch", "", 86_400, 86_400, true).is_none());
        let made = draft(
            Some("p1"),
            " Watch ",
            " Track papers ",
            "Papers, 5",
            86_400,
            7 * 86_400,
            false,
        )
        .unwrap();
        assert_eq!(made.name, "Watch");
        assert_eq!(made.goal, "Track papers");
        assert_eq!(made.kpis.len(), 1);
        assert!(!made.constraints.review_mutations);

        let zone = FixedOffset::east_opt(8 * 3_600).unwrap();
        let mut mandate = MandateRecord {
            status: "active".into(),
            interval_secs: 86_400,
            next_run_at: zone
                .with_ymd_and_hms(2026, 11, 16, 9, 0, 0)
                .unwrap()
                .timestamp(),
            ..Default::default()
        };
        assert_eq!(
            next_line(&mandate, &zone),
            "Next round 2026-11-16 09:00 · every 1 d"
        );
        mandate.status = "waiting".into();
        assert_eq!(
            next_line(&mandate, &zone),
            "Waiting for your answer before the next round"
        );
    }
}
