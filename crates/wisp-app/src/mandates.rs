//! Research mandates, host-independent: what the researcher's draft becomes,
//! and what the agent is told at the start of every turn in the mandate's
//! conversation. Hosts own scheduling, windows and notifications.

use chrono::{Local, TimeZone};
use wisp_dto::{MandateConstraints, MandateDraft, MandateKpi, MandateRecord};

/// The scheduler polls every 30 s; anything shorter is a busy loop.
pub const MIN_ROUND_INTERVAL_SECS: i64 = 5 * 60;
pub const MAX_ROUND_INTERVAL_SECS: i64 = 30 * 86_400;
const MAX_NAME_CHARS: usize = 80;
const MAX_KPIS: usize = 12;

fn clip(text: &str, chars: usize) -> String {
    text.trim().chars().take(chars).collect()
}

fn local_day(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Bring bounds into a usable shape instead of rejecting a form over them.
pub fn normalize_constraints(mut constraints: MandateConstraints) -> MandateConstraints {
    constraints.min_interval_secs = constraints
        .min_interval_secs
        .clamp(MIN_ROUND_INTERVAL_SECS, MAX_ROUND_INTERVAL_SECS);
    constraints.max_interval_secs = constraints
        .max_interval_secs
        .clamp(constraints.min_interval_secs, MAX_ROUND_INTERVAL_SECS);
    constraints.max_rounds_per_day = constraints.max_rounds_per_day.clamp(1, 48);
    constraints.notes = constraints
        .notes
        .iter()
        .map(|note| note.trim().to_string())
        .filter(|note| !note.is_empty())
        .collect();
    constraints
}

fn normalize_kpis(kpis: Vec<MandateKpi>) -> Vec<MandateKpi> {
    kpis.into_iter()
        .map(|kpi| MandateKpi {
            name: clip(&kpi.name, 80),
            definition: clip(&kpi.definition, 400),
            target: clip(&kpi.target, 80),
            period: clip(&kpi.period, 80),
            current: kpi
                .current
                .map(|value| clip(&value, 80))
                .filter(|value| !value.is_empty()),
        })
        .filter(|kpi| !kpi.name.is_empty())
        .take(MAX_KPIS)
        .collect()
}

/// Apply a draft to `mandate`: everything the researcher may edit. Timing of
/// the next round and the mandate's status are left to the caller.
pub fn apply_draft(mandate: &mut MandateRecord, draft: MandateDraft) -> Result<(), String> {
    let goal = draft.goal.trim();
    if goal.is_empty() {
        return Err("A mandate needs a goal.".into());
    }
    mandate.name = match draft.name.trim() {
        "" => clip(goal.lines().next().unwrap_or_default(), MAX_NAME_CHARS),
        name => clip(name, MAX_NAME_CHARS),
    };
    mandate.goal = goal.to_string();
    mandate.kpis = normalize_kpis(draft.kpis);
    mandate.constraints = normalize_constraints(draft.constraints);
    mandate.ends_at = draft.ends_at;
    mandate.interval_secs = draft.interval_secs.clamp(
        mandate.constraints.min_interval_secs,
        mandate.constraints.max_interval_secs,
    );
    mandate.report_interval_secs = match draft.report_interval_secs {
        secs if secs <= 0 => 7 * 86_400,
        secs => secs.clamp(86_400, 90 * 86_400),
    };
    Ok(())
}

/// A new active mandate from the researcher's draft.
pub fn new_mandate(draft: MandateDraft, now: i64) -> Result<MandateRecord, String> {
    if draft.ends_at.is_some_and(|end| end <= now) {
        return Err("The mandate's end date is already past.".into());
    }
    let mut mandate = MandateRecord {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: draft.project_id.trim().to_string(),
        status: "active".into(),
        created_at: now,
        updated_at: now,
        ..Default::default()
    };
    let start_at = draft.start_at;
    apply_draft(&mut mandate, draft)?;
    // A past start means "begin now"; absent means one interval from now.
    mandate.next_run_at = start_at.unwrap_or(now + mandate.interval_secs);
    mandate.next_report_at = Some(now + mandate.report_interval_secs);
    Ok(mandate)
}

/// Host context for every turn in the mandate's conversation. It is injected
/// fresh each turn, so an edit to the goal or a KPI takes effect at once and
/// compaction can never lose the responsibility itself.
pub fn brief(mandate: &MandateRecord) -> String {
    let mut lines = vec![
        "<research_mandate>".to_string(),
        "You carry a long-running research mandate for the researcher. This conversation is its \
         working record: each round continues the same responsibility, so build on what earlier \
         rounds did instead of starting over. Treat this block as the researcher's standing brief."
            .to_string(),
        format!("Name: {}", mandate.name),
        format!("Goal: {}", mandate.goal),
        match mandate.ends_at {
            Some(end) => format!("Period: until {}", local_day(end)),
            None => "Period: open-ended, reviewed stage by stage".to_string(),
        },
    ];
    if !mandate.kpis.is_empty() {
        lines.push("KPIs — measure each one the way its definition says:".into());
        for kpi in &mandate.kpis {
            let mut line = format!("- {}", kpi.name);
            if !kpi.definition.is_empty() {
                line.push_str(&format!(" — {}", kpi.definition));
            }
            if !kpi.target.is_empty() {
                line.push_str(&format!("; target {}", kpi.target));
                if !kpi.period.is_empty() {
                    line.push_str(&format!(" {}", kpi.period));
                }
            }
            match (&kpi.current, kpi.gap()) {
                (Some(current), Some(gap)) if gap > 0.0 => {
                    line.push_str(&format!("; current {current} (short by {gap})"))
                }
                (Some(current), _) => line.push_str(&format!("; current {current}")),
                (None, _) => line.push_str("; not measured yet"),
            }
            lines.push(line);
        }
    }
    lines.push("Constraints:".into());
    let constraints = &mandate.constraints;
    for (label, text) in [
        ("You may do on your own", &constraints.autonomous),
        (
            "Needs the researcher's review first",
            &constraints.review_first,
        ),
        ("Stop and ask for help when", &constraints.ask_for_help),
    ] {
        if !text.trim().is_empty() {
            lines.push(format!("- {label}: {}", text.trim()));
        }
    }
    lines.push(if constraints.review_mutations {
        "- Every action that changes files, runs commands or submits anything asks the researcher \
         for approval first. Prepare the change so it is easy to review."
            .into()
    } else {
        "- Actions follow this project's approval settings.".to_string()
    });
    if !constraints.notes.is_empty() {
        lines.push("- Standing instructions from the researcher's feedback:".into());
        lines.extend(constraints.notes.iter().map(|note| format!("  - {note}")));
    }
    lines.push("</research_mandate>".into());
    lines.join("\n")
}

/// What a scheduled round sends into the mandate's conversation.
pub fn round_prompt(mandate: &MandateRecord) -> String {
    format!(
        "[Mandate round: {}]\n\nContinue this mandate. Check where things stand, do the next \
         useful step toward the goal within your constraints, and report what you did, what is \
         blocked, and what should happen next.",
        mandate.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> MandateDraft {
        MandateDraft {
            project_id: " p1 ".into(),
            name: String::new(),
            goal: "Track new single-cell papers.\nReport weekly.".into(),
            kpis: vec![
                MandateKpi {
                    name: "Papers filed".into(),
                    definition: "Screened and added to the library".into(),
                    target: "5".into(),
                    period: "per week".into(),
                    current: None,
                },
                MandateKpi::default(),
            ],
            constraints: MandateConstraints {
                autonomous: "Search and summarize".into(),
                min_interval_secs: 1,
                max_interval_secs: 3_600,
                max_rounds_per_day: 0,
                notes: vec!["  ".into(), " Lead with methods ".into()],
                ..Default::default()
            },
            ends_at: Some(2_000_000),
            interval_secs: 86_400,
            report_interval_secs: 0,
            start_at: None,
        }
    }

    #[test]
    fn a_draft_becomes_an_active_mandate_inside_its_own_bounds() {
        let mandate = new_mandate(draft(), 1_000).unwrap();
        assert_eq!(mandate.project_id, "p1");
        assert_eq!(mandate.name, "Track new single-cell papers.");
        assert_eq!(mandate.status, "active");
        assert_eq!(mandate.kpis.len(), 1, "a nameless KPI row is dropped");
        assert_eq!(
            mandate.constraints.min_interval_secs,
            MIN_ROUND_INTERVAL_SECS
        );
        assert_eq!(mandate.constraints.max_rounds_per_day, 1);
        assert_eq!(mandate.constraints.notes, ["Lead with methods"]);
        assert_eq!(mandate.interval_secs, 3_600, "cadence stays within max");
        assert_eq!(mandate.next_run_at, 1_000 + 3_600);
        assert_eq!(mandate.report_interval_secs, 7 * 86_400);
        assert_eq!(mandate.next_report_at, Some(1_000 + 7 * 86_400));

        let mut started = draft();
        started.start_at = Some(500);
        assert_eq!(new_mandate(started, 1_000).unwrap().next_run_at, 500);
    }

    #[test]
    fn a_draft_without_a_goal_or_with_a_past_end_is_rejected() {
        let mut empty = draft();
        empty.goal = "  ".into();
        assert!(new_mandate(empty, 1_000).is_err());
        assert!(new_mandate(draft(), 2_000_000).is_err());
    }

    #[test]
    fn the_brief_states_goal_kpi_progress_and_constraints() {
        let mut mandate = new_mandate(draft(), 1_000).unwrap();
        let text = brief(&mandate);
        assert!(text.starts_with("<research_mandate>") && text.ends_with("</research_mandate>"));
        assert!(text.contains("Goal: Track new single-cell papers.\nReport weekly."));
        assert!(text.contains(
            "- Papers filed — Screened and added to the library; target 5 per week; not measured yet"
        ));
        assert!(text.contains("- You may do on your own: Search and summarize"));
        assert!(
            !text.contains("Needs the researcher's review first"),
            "empty lines are omitted"
        );
        assert!(text.contains(
            "asks the researcher \
         for approval first"
        ));
        assert!(text.contains("  - Lead with methods"));

        mandate.kpis[0].current = Some("3".into());
        mandate.constraints.review_mutations = false;
        mandate.ends_at = None;
        let text = brief(&mandate);
        assert!(text.contains("current 3 (short by 2)"));
        assert!(text.contains("follow this project's approval settings"));
        assert!(text.contains("Period: open-ended"));
        assert!(
            round_prompt(&mandate).starts_with("[Mandate round: Track new single-cell papers.]")
        );
    }
}
