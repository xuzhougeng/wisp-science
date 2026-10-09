//! Research mandates, host-independent: what the researcher's draft becomes,
//! what the agent is told at the start of every turn in the mandate's
//! conversation, and the ledger a round reports into with `end_round`.
//! Hosts own scheduling, windows and notifications.

use async_trait::async_trait;
use chrono::{Local, NaiveDateTime, TimeZone};
use serde_json::{json, Value};
use wisp_dto::{
    MandateConstraints, MandateDraft, MandateKpi, MandateKpiValue, MandateRecord, MandateRound,
};
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Tool, ToolEnv, ToolResult};

/// The scheduler polls every 30 s; anything shorter is a busy loop.
pub const MIN_ROUND_INTERVAL_SECS: i64 = 5 * 60;
pub const MAX_ROUND_INTERVAL_SECS: i64 = 30 * 86_400;
const MAX_NAME_CHARS: usize = 80;
const MAX_KPIS: usize = 12;
/// Rounds the brief carries: enough to continue, small enough to stay cheap.
pub const BRIEF_ROUNDS: usize = 3;
const MAX_ROUND_FIELD_CHARS: usize = 2_000;
pub const END_ROUND: &str = "end_round";

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

fn local_minute(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// `90 min`, `6 h`, `7 d`: the largest unit that divides evenly.
fn span(secs: i64) -> String {
    match secs {
        s if s % 86_400 == 0 => format!("{} d", s / 86_400),
        s if s % 3_600 == 0 => format!("{} h", s / 3_600),
        s => format!("{} min", (s / 60).max(1)),
    }
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
/// compaction can never lose the responsibility itself. `rounds` are the
/// latest ledger entries, newest first.
pub fn brief(mandate: &MandateRecord, rounds: &[MandateRound]) -> String {
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
    if rounds.is_empty() {
        lines.push("Round ledger: no round has reported yet. This is the first.".into());
    } else {
        lines.push(
            "Round ledger — your own latest reports, oldest first. Continue from them:".into(),
        );
        for round in rounds.iter().rev() {
            let mut line = format!(
                "#{} · {} — Done: {}",
                round.seq,
                local_minute(round.created_at),
                round.done
            );
            if !round.blockers.is_empty() {
                line.push_str(&format!(" | Blocked: {}", round.blockers));
            }
            if !round.next_step.is_empty() {
                line.push_str(&format!(" | Next: {}", round.next_step));
            }
            if round.source == "host" {
                line.push_str(" | (this round ended without calling end_round)");
            }
            lines.push(line);
        }
    }
    lines.push(format!(
        "Ending a round: when this round's work is finished, call `{END_ROUND}` once with what \
         was done, the current value of every KPI you measured, what is blocked, the next step, \
         and when the next round should happen. Choose that time from the work — soon when \
         something is about to finish, later when you are only waiting — between {} and {} from \
         now; leave it out for the usual {}. Then end your turn with a few lines for the \
         researcher. The ledger is what your next round is briefed from, so write it for \
         yourself: concrete, with names and paths.",
        span(constraints.min_interval_secs),
        span(constraints.max_interval_secs),
        span(mandate.interval_secs),
    ));
    lines.push("</research_mandate>".into());
    lines.join("\n")
}

/// What a round reports when it ends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoundReport {
    pub done: String,
    pub kpis: Vec<MandateKpiValue>,
    pub blockers: String,
    pub next_step: String,
    /// When the round wants the next one; `None` uses the mandate's cadence.
    pub next_run_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundOutcome {
    pub round: MandateRound,
    /// The time the round asked for, when it had to be moved into bounds.
    pub moved_from: Option<i64>,
    /// Reported KPI names the mandate does not define.
    pub unknown_kpis: Vec<String>,
}

/// The next round's time: the requested one, or the mandate's cadence, kept
/// within the mandate's bounds and never past its end. The round that comes
/// due at the end is the one that closes the mandate.
pub fn next_run_at(mandate: &MandateRecord, requested: Option<i64>, now: i64) -> i64 {
    let constraints = &mandate.constraints;
    let at = requested.unwrap_or(now + mandate.interval_secs).clamp(
        now + constraints.min_interval_secs,
        now + constraints.max_interval_secs,
    );
    mandate.ends_at.map_or(at, |end| at.min(end))
}

/// Record a round the agent reported: append it to the ledger, update the
/// KPIs it measured, and schedule the next round.
pub async fn record_round(
    store: &Store,
    mandate_id: &str,
    report: RoundReport,
    now: i64,
) -> Result<RoundOutcome, String> {
    let err = |error: anyhow::Error| error.to_string();
    let mut mandate = store
        .get_mandate(mandate_id)
        .await
        .map_err(err)?
        .ok_or("This mandate no longer exists.")?;
    if mandate.status == "done" {
        return Err("This mandate is closed; there is no round to report.".into());
    }
    if report.done.trim().is_empty() {
        return Err(
            "'done' cannot be empty. Say what this round did, even if that is \"nothing yet, because …\"."
                .into(),
        );
    }
    let mut unknown_kpis = Vec::new();
    let mut measured = Vec::new();
    for value in report.kpis {
        let (name, reading) = (value.name.trim(), clip(&value.value, 80));
        if name.is_empty() || reading.is_empty() {
            continue;
        }
        match mandate
            .kpis
            .iter_mut()
            .find(|kpi| kpi.name.to_lowercase() == name.to_lowercase())
        {
            Some(kpi) => {
                kpi.current = Some(reading.clone());
                measured.push(MandateKpiValue {
                    name: kpi.name.clone(),
                    value: reading,
                });
            }
            None => unknown_kpis.push(name.to_string()),
        }
    }
    let next = next_run_at(&mandate, report.next_run_at, now);
    let round = store
        .add_mandate_round(&MandateRound {
            id: uuid::Uuid::new_v4().to_string(),
            mandate_id: mandate.id.clone(),
            seq: 0,
            done: clip(&report.done, MAX_ROUND_FIELD_CHARS),
            kpis: measured,
            blockers: clip(&report.blockers, MAX_ROUND_FIELD_CHARS),
            next_step: clip(&report.next_step, MAX_ROUND_FIELD_CHARS),
            next_run_at: Some(next),
            source: "agent".into(),
            created_at: now,
        })
        .await
        .map_err(err)?;
    if !round.kpis.is_empty() {
        mandate.updated_at = now;
        store.update_mandate(&mandate).await.map_err(err)?;
    }
    store
        .schedule_mandate_round(&mandate.id, next, None, now)
        .await
        .map_err(err)?;
    Ok(RoundOutcome {
        round,
        moved_from: report.next_run_at.filter(|requested| *requested != next),
        unknown_kpis,
    })
}

/// A round that ended without reporting still leaves a ledger entry, so the
/// next one is not briefed as if nothing happened. The schedule is untouched.
pub async fn record_unreported_round(
    store: &Store,
    mandate_id: &str,
    answer: &str,
    now: i64,
) -> Result<MandateRound, String> {
    let done = match answer.trim() {
        "" => "(The round ended without an answer or a report.)".to_string(),
        answer => clip(answer, 600),
    };
    store
        .add_mandate_round(&MandateRound {
            id: uuid::Uuid::new_v4().to_string(),
            mandate_id: mandate_id.into(),
            seq: 0,
            done,
            source: "host".into(),
            created_at: now,
            ..Default::default()
        })
        .await
        .map_err(|error| error.to_string())
}

/// `YYYY-MM-DD HH:MM` (or with a `T`) in local time.
fn parse_local_minute(value: &str) -> Option<i64> {
    let value = value.trim().replace('T', " ");
    let naive = NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M").ok()?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.timestamp())
}

fn report_from_args(args: &Value, now: i64) -> Result<RoundReport, String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let kpis = args
        .get("kpis")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| MandateKpiValue {
            name: entry["name"].as_str().unwrap_or_default().into(),
            // A model often sends a bare number for a numeric KPI.
            value: match &entry["value"] {
                Value::String(text) => text.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            },
        })
        .collect();
    let next_run_at = match (
        args.get("next_round_in_minutes").and_then(Value::as_i64),
        args.get("next_round_at").and_then(Value::as_str),
    ) {
        (Some(minutes), _) => Some(now + minutes.clamp(0, 365 * 24 * 60) * 60),
        (None, Some(at)) if !at.trim().is_empty() => Some(parse_local_minute(at).ok_or(
            "'next_round_at' must be a local time like 2026-10-12 09:00, or use next_round_in_minutes.",
        )?),
        _ => None,
    };
    Ok(RoundReport {
        done: text("done"),
        kpis,
        blockers: text("blockers"),
        next_step: text("next_step"),
        next_run_at,
    })
}

/// `end_round`, registered only in a mandate's conversation.
pub struct EndRoundTool {
    store: Store,
    mandate_id: String,
}

impl EndRoundTool {
    pub fn new(store: Store, mandate_id: impl Into<String>) -> Self {
        Self {
            store,
            mandate_id: mandate_id.into(),
        }
    }
}

#[async_trait]
impl Tool for EndRoundTool {
    fn name(&self) -> &str {
        END_ROUND
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            END_ROUND,
            "Close this round of the research mandate: record what it did in the mandate's ledger, report KPI values, and schedule the next round. Call it once, when the round's work is finished. Your next round is briefed from this entry.",
            json!({
                "type": "object",
                "properties": {
                    "done": {"type": "string", "description": "What this round completed, with run names and output paths. Say so plainly if nothing could be done."},
                    "kpis": {
                        "type": "array",
                        "description": "The current value of each KPI you measured this round, by its exact name in the mandate. Omit KPIs you did not measure.",
                        "items": {
                            "type": "object",
                            "properties": {"name": {"type": "string"}, "value": {"type": "string"}},
                            "required": ["name", "value"]
                        }
                    },
                    "blockers": {"type": "string", "description": "What is blocked and on whom or what. Empty when nothing is."},
                    "next_step": {"type": "string", "description": "The first thing the next round should do."},
                    "next_round_in_minutes": {"type": "integer", "description": "Minutes until the next round. Omit to keep the mandate's usual cadence."},
                    "next_round_at": {"type": "string", "description": "Alternative to next_round_in_minutes: a local time, YYYY-MM-DD HH:MM."}
                },
                "required": ["done", "next_step"]
            }),
        )
    }

    /// It writes only the mandate's own ledger and schedule, never project
    /// state, so a mandate that reviews every change can still close a round
    /// unattended.
    fn read_only(&self) -> bool {
        true
    }

    fn preview(&self, args: &Value) -> String {
        clip(args["done"].as_str().unwrap_or_default(), 80)
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let now = chrono::Utc::now().timestamp();
        let report = match report_from_args(args, now) {
            Ok(report) => report,
            Err(error) => return ToolResult::fail(error),
        };
        match record_round(&self.store, &self.mandate_id, report, now).await {
            Ok(outcome) => ToolResult::ok(round_receipt(&outcome)),
            Err(error) => ToolResult::fail(error),
        }
    }
}

/// What the agent reads back after `end_round`.
pub fn round_receipt(outcome: &RoundOutcome) -> String {
    let round = &outcome.round;
    let next = round.next_run_at.unwrap_or_default();
    let mut text = format!(
        "Round {} recorded. Next round: {} (local time).",
        round.seq,
        local_minute(next)
    );
    if let Some(requested) = outcome.moved_from {
        text.push_str(&format!(
            " You asked for {}; it was moved to stay within the mandate's bounds.",
            local_minute(requested)
        ));
    }
    if !outcome.unknown_kpis.is_empty() {
        text.push_str(&format!(
            " Not recorded, because the mandate defines no such KPI: {}. Use the exact KPI names from the brief.",
            outcome.unknown_kpis.join(", ")
        ));
    }
    text.push_str(" Now end your turn with a few lines for the researcher.");
    text
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
        let text = brief(&mandate, &[]);
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
        assert!(text.contains("asks the researcher for approval first"));
        assert!(text.contains("  - Lead with methods"));
        assert!(text.contains("no round has reported yet"));
        assert!(text.contains("between 5 min and 1 h from now; leave it out for the usual 1 h"));

        mandate.kpis[0].current = Some("3".into());
        mandate.constraints.review_mutations = false;
        mandate.ends_at = None;
        let text = brief(&mandate, &[]);
        assert!(text.contains("current 3 (short by 2)"));
        assert!(text.contains("follow this project's approval settings"));
        assert!(text.contains("Period: open-ended"));
        assert!(
            round_prompt(&mandate).starts_with("[Mandate round: Track new single-cell papers.]")
        );
    }

    #[test]
    fn the_brief_replays_the_ledger_oldest_first() {
        let mandate = new_mandate(draft(), 1_000).unwrap();
        let round = |seq, done: &str, blockers: &str, source: &str| MandateRound {
            seq,
            done: done.into(),
            blockers: blockers.into(),
            next_step: format!("step after {seq}"),
            source: source.into(),
            created_at: 86_400 * seq,
            ..Default::default()
        };
        // Newest first, as the store returns them.
        let text = brief(
            &mandate,
            &[
                round(3, "Filed 2 papers", "", "agent"),
                round(2, "Run finished", "", "host"),
                round(1, "Searched PubMed", "Paywall on one paper", "agent"),
            ],
        );
        let (first, second, third) = (
            text.find("#1 · ").unwrap(),
            text.find("#2 · ").unwrap(),
            text.find("#3 · ").unwrap(),
        );
        assert!(first < second && second < third);
        assert!(text.contains(
            "Done: Searched PubMed | Blocked: Paywall on one paper | Next: step after 1"
        ));
        assert!(text.contains("Done: Run finished | Next: step after 2 | (this round ended without calling end_round)"));
        assert!(!text.contains("no round has reported yet"));
    }

    #[test]
    fn the_next_round_stays_within_bounds_and_the_period() {
        let mut mandate = new_mandate(draft(), 1_000).unwrap();
        // Bounds are 5 min – 1 h; the cadence is 1 h.
        assert_eq!(next_run_at(&mandate, None, 10_000), 10_000 + 3_600);
        assert_eq!(
            next_run_at(&mandate, Some(10_000 + 1_800), 10_000),
            10_000 + 1_800
        );
        assert_eq!(
            next_run_at(&mandate, Some(10_000 + 10), 10_000),
            10_000 + 300,
            "too soon"
        );
        assert_eq!(
            next_run_at(&mandate, Some(5_000), 10_000),
            10_000 + 300,
            "in the past"
        );
        assert_eq!(
            next_run_at(&mandate, Some(10_000 + 86_400), 10_000),
            10_000 + 3_600,
            "too late"
        );
        mandate.ends_at = Some(10_000 + 600);
        assert_eq!(
            next_run_at(&mandate, None, 10_000),
            10_000 + 600,
            "the last round comes due at the end, where it closes the mandate"
        );
    }

    #[test]
    fn round_arguments_accept_minutes_or_a_local_time() {
        let report = report_from_args(
            &json!({
                "done": "Filed 2", "next_step": "Read reviews", "blockers": "none",
                "kpis": [{"name": "Papers filed", "value": 2}, {"name": "Noted", "value": "3 of 5"}],
                "next_round_in_minutes": 90, "next_round_at": "ignored when minutes are given",
            }),
            1_000,
        )
        .unwrap();
        assert_eq!(report.next_run_at, Some(1_000 + 90 * 60));
        assert_eq!(report.kpis[0].value, "2", "a bare number is accepted");
        assert_eq!(report.kpis[1].value, "3 of 5");
        let at = report_from_args(
            &json!({"done": "x", "next_round_at": "2026-10-12T09:00"}),
            0,
        )
        .unwrap()
        .next_run_at
        .unwrap();
        assert_eq!(local_minute(at), "2026-10-12 09:00");
        assert_eq!(
            report_from_args(&json!({"done": "x"}), 0)
                .unwrap()
                .next_run_at,
            None
        );
        assert!(report_from_args(&json!({"done": "x", "next_round_at": "tomorrow"}), 0).is_err());
    }

    async fn store_with_mandate() -> (Store, tempfile::TempDir, MandateRecord) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("wisp.sqlite")).await.unwrap();
        store.create_project("p1", "RNA-seq", "").await.unwrap();
        let mut d = draft();
        d.project_id = "p1".into();
        d.constraints.max_interval_secs = 2 * 86_400;
        d.interval_secs = 86_400;
        d.ends_at = None;
        let mandate = new_mandate(d, 1_000).unwrap();
        store.create_mandate(&mandate).await.unwrap();
        (store, dir, mandate)
    }

    #[tokio::test]
    async fn a_closed_mandate_or_an_empty_report_records_nothing() {
        let (store, _dir, mandate) = store_with_mandate().await;
        let empty = RoundReport {
            done: "  ".into(),
            ..Default::default()
        };
        assert!(record_round(&store, &mandate.id, empty, 2_000)
            .await
            .is_err());
        store
            .set_mandate_status(&mandate.id, "done", 2_000)
            .await
            .unwrap();
        let report = RoundReport {
            done: "Filed 2".into(),
            ..Default::default()
        };
        assert!(record_round(&store, &mandate.id, report.clone(), 2_000)
            .await
            .is_err());
        assert!(record_round(&store, "missing", report, 2_000)
            .await
            .is_err());
        assert!(store
            .mandate_rounds(&mandate.id, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn an_unreported_round_keeps_the_ledger_continuous_and_the_schedule_untouched() {
        let (store, _dir, mandate) = store_with_mandate().await;
        let round = record_unreported_round(&store, &mandate.id, "  Looked at the queue.  ", 2_000)
            .await
            .unwrap();
        assert_eq!((round.seq, round.source.as_str()), (1, "host"));
        assert_eq!(round.done, "Looked at the queue.");
        assert_eq!(round.next_run_at, None);
        let silent = record_unreported_round(&store, &mandate.id, "", 3_000)
            .await
            .unwrap();
        assert_eq!(silent.seq, 2);
        assert!(silent.done.contains("without an answer"));
        assert_eq!(
            store
                .get_mandate(&mandate.id)
                .await
                .unwrap()
                .unwrap()
                .next_run_at,
            mandate.next_run_at
        );
    }

    /// Three rounds through the real agent loop with a scripted provider: the
    /// ledger stays gapless, the next round follows what the agent asked for,
    /// and a time outside the mandate's bounds is moved inside them.
    #[tokio::test]
    async fn three_scripted_rounds_keep_the_ledger_and_reschedule_within_bounds() {
        use wisp_llm::{ScriptedCompletion, ScriptedProvider, ScriptedToolCall};
        let (store, dir, mandate) = store_with_mandate().await;
        let call = |arguments: Value| ScriptedCompletion {
            tool_calls: vec![ScriptedToolCall {
                id: String::new(),
                name: END_ROUND.into(),
                arguments,
            }],
            ..Default::default()
        };
        let say = |content: &str| ScriptedCompletion {
            content: content.into(),
            ..Default::default()
        };
        let provider = ScriptedProvider::new(
            "scripted-mandate",
            vec![
                call(
                    json!({"done": "Searched PubMed, filed 2 papers", "next_step": "Read the flagged review",
                    "kpis": [{"name": "papers filed", "value": "2"}], "next_round_in_minutes": 120}),
                ),
                say("Round one done."),
                call(
                    json!({"done": "Read the review, filed 1 more", "next_step": "Wait for the preprint",
                    "blockers": "Preprint not yet posted", "kpis": [{"name": "Papers filed", "value": "3"}, {"name": "Citations", "value": "9"}],
                    "next_round_in_minutes": 60 * 24 * 30}),
                ),
                say("Round two done."),
                call(
                    json!({"done": "Nothing new was published", "next_step": "Search again", "next_round_in_minutes": 1}),
                ),
                say("Round three done."),
            ],
        );
        let mut tools = wisp_tools::Registry::builtins().filtered(&[]);
        tools.add(Box::new(EndRoundTool::new(store.clone(), &mandate.id)));
        let mut agent = wisp_core::Agent::with_provider(
            Box::new(provider.clone()),
            None,
            tools,
            dir.path().to_path_buf(),
            128_000,
            8,
        );
        for _ in 0..3 {
            // What the host does before every turn in the mandate's conversation.
            let current = store.get_mandate(&mandate.id).await.unwrap().unwrap();
            let rounds = store
                .mandate_rounds(&mandate.id, BRIEF_ROUNDS)
                .await
                .unwrap();
            agent.ctx.clear_runtime_injections();
            agent.ctx.inject_user(brief(&current, &rounds));
            agent.ctx.prefix_runtime_injections_to_user();
            agent
                .run_with_images(
                    &round_prompt(&current),
                    &[],
                    false,
                    &wisp_core::NullOutput,
                    None,
                    None,
                )
                .await
                .unwrap();
        }
        assert_eq!(provider.remaining(), 0);

        let rounds = store.mandate_rounds(&mandate.id, 10).await.unwrap();
        assert_eq!(rounds.iter().map(|r| r.seq).collect::<Vec<_>>(), [3, 2, 1]);
        assert!(rounds.iter().all(|r| r.source == "agent"));
        let asked = |round: &MandateRound| round.next_run_at.unwrap() - round.created_at;
        assert_eq!(
            asked(&rounds[2]),
            120 * 60,
            "round 1: the time it asked for"
        );
        assert_eq!(
            asked(&rounds[1]),
            2 * 86_400,
            "round 2: 30 days is moved to the 2-day maximum"
        );
        assert_eq!(
            asked(&rounds[0]),
            300,
            "round 3: 1 minute is moved to the 5-minute minimum"
        );
        assert_eq!(rounds[1].blockers, "Preprint not yet posted");
        assert_eq!(rounds[1].kpis.len(), 1, "an undefined KPI is not recorded");
        assert_eq!(
            rounds[2].kpis[0].name, "Papers filed",
            "matched to the mandate's own spelling"
        );

        let saved = store.get_mandate(&mandate.id).await.unwrap().unwrap();
        assert_eq!(saved.kpis[0].current.as_deref(), Some("3"));
        assert_eq!(saved.next_run_at, rounds[0].next_run_at.unwrap());

        // Each round was briefed from the ledger the previous rounds left,
        // and told what happened to its own request.
        let requests = provider.snapshot().requests;
        let text = |index: usize| {
            requests[index]
                .messages
                .iter()
                .map(|message| message.content.as_text())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(text(0).contains("no round has reported yet"));
        assert!(requests[0].tool_names.contains(&END_ROUND.to_string()));
        assert!(text(2)
            .contains("Done: Searched PubMed, filed 2 papers | Next: Read the flagged review"));
        assert!(text(2).contains("current 2 (short by 3)"));
        assert!(text(3).contains("Round 2 recorded."));
        assert!(text(3).contains("it was moved to stay within the mandate's bounds"));
        assert!(text(3).contains("defines no such KPI: Citations"));
        assert!(text(4).contains("#2 · ") && text(4).contains("Blocked: Preprint not yet posted"));
        assert!(text(5).contains("Round 3 recorded."));
    }
}
