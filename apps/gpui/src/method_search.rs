//! A method search's card in the Runs tab, after the WebView's
//! `MethodSearchRunPanel`: what its frozen contract promises, how far the
//! search has got, what it tried and kept, and what can be done with it
//! now. Data only; the view is in `workspace/panel_tabs.rs`.
use wisp_dto::{
    MethodCandidateView, MethodSearchProgressView, MethodSearchRunDetails, MethodSearchRunOutput,
    MethodStrategyView, RunRecord,
};

use crate::i18n::{fill, tr};

/// What can be done with a search from its card. Cancel is the run's own
/// Cancel, beside its status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Start,
    Pause,
    Resume,
}

impl Control {
    pub fn label(self) -> &'static str {
        match self {
            Control::Start => tr("Start the search"),
            Control::Pause => tr("Pause"),
            Control::Resume => tr("Resume"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Control::Start | Control::Resume => "play",
            Control::Pause => "pause",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Control::Start => "start",
            Control::Pause => "pause",
            Control::Resume => "resume",
        }
    }
}

/// As in the WebView: a draft starts once its contract was read, a running
/// search pauses at its next candidate, and a paused one resumes from its
/// checkpoint.
pub fn control(status: &str) -> Option<Control> {
    match status {
        "draft" => Some(Control::Start),
        "submitted" | "running" => Some(Control::Pause),
        "paused" => Some(Control::Resume),
        _ => None,
    }
}

/// Whether the card shows an older search than the conversation lists.
pub fn stale(details: &MethodSearchRunDetails, run: &RunRecord) -> bool {
    details.run.status != run.status || details.run.progress_json != run.progress_json
}

/// A score as the WebView shows it.
fn score(value: f64) -> String {
    format!("{value:.6}")
}

fn maybe_score(value: Option<f64>) -> String {
    value.map(score).unwrap_or_else(|| "—".into())
}

/// The start of a long id.
pub fn short(id: &str, length: usize) -> String {
    id.chars().take(length).collect()
}

/// The frozen contract, one labelled line each.
pub fn contract(details: &MethodSearchRunDetails) -> Vec<(&'static str, String)> {
    let spec = &details.spec;
    let baseline = &details.audit.baseline;
    let budget = &spec.budget;
    let direction = match spec.metrics.direction.as_str() {
        "maximize" => tr("higher is better").to_owned(),
        "minimize" => tr("lower is better").to_owned(),
        other => other.to_owned(),
    };
    vec![
        (
            tr("Target"),
            format!("{}::{}", spec.target.source_path, spec.target.symbol),
        ),
        (
            tr("Evaluator"),
            fill(
                tr("{} · {} runs, {} s each"),
                &[
                    &spec.evaluator.entry_path,
                    &spec.evaluator.repetitions,
                    &spec.evaluator.timeout_seconds,
                ],
            ),
        ),
        (
            tr("Metric"),
            format!("{} · {direction}", spec.metrics.primary),
        ),
        (
            tr("Baseline"),
            fill(
                tr("{} ± {} ({} of {} runs)"),
                &[
                    &score(baseline.median_primary),
                    &score(baseline.spread),
                    &baseline.successful_repetitions,
                    &baseline.repetitions,
                ],
            ),
        ),
        (tr("Noise floor"), score(baseline.noise_floor)),
        (
            tr("Reachable"),
            if details.audit.sentinel_reachable {
                tr("yes")
            } else {
                tr("no")
            }
            .to_owned(),
        ),
        (
            tr("Budget"),
            fill(
                tr("{} candidates · {} s · evaluator {} s · cost {}"),
                &[
                    &budget.max_candidates,
                    &budget.max_wall_seconds,
                    &budget.max_evaluator_seconds,
                    &budget.max_cost_microunits,
                ],
            ),
        ),
        (
            tr("Protected files"),
            spec.protected_paths.len().to_string(),
        ),
        (
            tr("Final check"),
            if spec.final_verification.is_some() {
                tr("held-out verification")
            } else {
                tr("validation only")
            }
            .to_owned(),
        ),
    ]
}

/// The limits a candidate must also keep, as `metric op value`.
pub fn guardrails(details: &MethodSearchRunDetails) -> Vec<String> {
    details
        .spec
        .metrics
        .guardrails
        .iter()
        .map(|guardrail| format!("{} {} {}", guardrail.metric, guardrail.op, guardrail.value))
        .collect()
}

/// The run's last progress report; a run that wrote none reads as empty.
pub fn progress(run: &RunRecord) -> MethodSearchProgressView {
    serde_json::from_str(&run.progress_json).unwrap_or_default()
}

fn phase(phase: &str) -> String {
    match phase {
        "" => "—".into(),
        "awaiting_approval" => tr("waiting to be started").into(),
        "baseline" => tr("measuring the baseline").into(),
        "search" => tr("searching").into(),
        "final_verification" => tr("final verification").into(),
        other => other.replace('_', " "),
    }
}

/// How far the search has got, one labelled line each. `now` is in seconds.
pub fn progress_rows(progress: &MethodSearchProgressView, now: i64) -> Vec<(&'static str, String)> {
    vec![
        (tr("Phase"), phase(&progress.phase)),
        (tr("Best"), maybe_score(progress.best_primary)),
        (
            tr("Candidates"),
            fill(
                tr("{} · {} scored, {} failed"),
                &[
                    &progress.candidate_count,
                    &progress.successful_count,
                    &progress.failed_count,
                ],
            ),
        ),
        (tr("Cost"), progress.cost_microunits.to_string()),
        (
            tr("Strategy"),
            progress
                .current_strategy
                .clone()
                .unwrap_or_else(|| "—".into()),
        ),
        (
            tr("Checkpoint"),
            progress
                .last_checkpoint_at
                .map(|at| fill(tr("{} s ago"), &[&now.saturating_sub(at).max(0)]))
                .unwrap_or_else(|| "—".into()),
        ),
    ]
}

/// One candidate the search tried: its place, family, strategy, and score
/// or why it has none.
pub fn candidate(candidate: &MethodCandidateView) -> String {
    let mut parts = vec![
        format!("#{}", candidate.sequence),
        candidate.family.clone(),
        candidate.strategy_key.clone(),
        candidate
            .primary_score
            .map(score)
            .unwrap_or_else(|| candidate.status.replace('_', " ")),
    ];
    if let Some(lines) = candidate.changed_lines {
        parts.push(format!("Δ {lines}"));
    }
    if let Some(ms) = candidate.runtime_ms {
        parts.push(format!("{ms} ms"));
    }
    if let Some(parent) = &candidate.parent_candidate_id {
        parts.push(format!("← {}", short(parent, 8)));
    }
    parts.join(" · ")
}

/// One strategy: how much the search now leans on it, and how often it
/// helped.
pub fn strategy(strategy: &MethodStrategyView) -> String {
    fill(
        tr("{} · {} · weight {} · {} of {} improved"),
        &[
            &strategy.strategy_key,
            &strategy.category,
            &format!("{:.3}", strategy.weight),
            &strategy.improvements,
            &strategy.attempts,
        ],
    )
}

/// What a promoted file is.
pub fn output_name(output: &MethodSearchRunOutput) -> String {
    let key = if output.logical_output_key.trim().is_empty() {
        output.role.as_str()
    } else {
        output.logical_output_key.as_str()
    };
    match key {
        "selected_method" => tr("Selected method").into(),
        "candidate_history" => tr("Candidate history").into(),
        "verification_report" => tr("Verification report").into(),
        other => match other.strip_prefix("top_k_") {
            Some(rank) => fill(tr("Top {}"), &[&rank]),
            None => other.replace('_', " "),
        },
    }
}

/// A promoted file's text, fenced so it reads as code.
pub fn output_markdown(path: &str, text: &str) -> String {
    let language = match path.rsplit_once('.').map(|(_, kind)| kind) {
        Some("py") => "python",
        Some("json") => "json",
        _ => "",
    };
    // Longer than any run of backticks inside, so nothing closes it early.
    let longest = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{language}\n{}\n{fence}", text.trim_end())
}

/// What the durable record says, for checking the card against the files.
pub fn integrity(details: &MethodSearchRunDetails) -> String {
    let result = details
        .state
        .result_status
        .as_deref()
        .map(|status| status.replace('_', " "))
        .unwrap_or_else(|| tr("pending").into());
    fill(
        tr("{} · result {} · baseline failures {}% · {} findings · spec {} · audit {}"),
        &[
            &details.state.control_state,
            &result,
            &format!("{:.1}", details.audit.baseline.failure_rate * 100.0),
            &details.audit.findings.len(),
            &short(&details.state.spec_artifact_version_id, 12),
            &short(&details.audit_artifact_version_id, 12),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(status: &str, progress: serde_json::Value) -> serde_json::Value {
        json!({
            "id": "run-1",
            "frame_id": "frame",
            "context_id": "local",
            "title": "Method search",
            "kind": "method_search",
            "status": status,
            "progress_json": progress.to_string(),
            "env_snapshot_json": "{}",
            "created_at": 1,
        })
    }

    fn details(status: &str) -> MethodSearchRunDetails {
        serde_json::from_value(json!({
            "run": run(status, json!({"phase": "search"})),
            "state": {
                "spec_artifact_version_id": "spec-version-0123456789",
                "spec_sha256": "abc",
                "control_state": "active",
                "result_status": null,
            },
            "spec": {
                "objective": "Fit better",
                "target": {
                    "source_artifact_version_id": "source",
                    "source_path": "analysis/model.py",
                    "symbol": "fit_model",
                },
                "evaluator": {
                    "artifact_version_id": "evaluator",
                    "entry_path": "analysis/evaluate.py",
                    "repetitions": 3,
                    "timeout_seconds": 30,
                },
                "metrics": {
                    "primary": "score",
                    "direction": "maximize",
                    "guardrails": [{"metric": "runtime", "op": "<=", "value": 2.5}],
                },
                "protected_paths": ["analysis/evaluate.py", "data/rows.csv"],
                "constraints": [],
                "budget": {
                    "max_candidates": 20,
                    "max_wall_seconds": 3600,
                    "max_evaluator_seconds": 30,
                    "max_cost_microunits": 1000000,
                },
                "final_verification": null,
            },
            "audit": {
                "baseline": {
                    "repetitions": 3,
                    "successful_repetitions": 3,
                    "failure_rate": 0.0,
                    "median_primary": 1.0,
                    "spread": 0.0,
                    "noise_floor": 0.01,
                },
                "sentinelReachable": true,
                "protectedFiles": [],
                "targetSourceSha256": "def",
                "findings": ["one"],
            },
            "auditArtifactVersionId": "audit-version-0123456789",
            "candidates": [],
            "strategies": [],
            "outputs": [],
        }))
        .unwrap()
    }

    #[test]
    fn a_search_offers_what_its_status_allows() {
        assert_eq!(control("draft"), Some(Control::Start));
        assert_eq!(control("running"), Some(Control::Pause));
        assert_eq!(control("submitted"), Some(Control::Pause));
        assert_eq!(control("paused"), Some(Control::Resume));
        for ended in ["succeeded", "failed", "cancelled", "cancelling"] {
            assert_eq!(control(ended), None, "{ended}");
        }
    }

    #[test]
    fn the_card_reads_the_frozen_contract() {
        let details = details("draft");
        let rows = contract(&details);
        let value = |label: &str| {
            rows.iter()
                .find(|(name, _)| *name == label)
                .map(|(_, value)| value.clone())
                .unwrap()
        };
        assert_eq!(value("Target"), "analysis/model.py::fit_model");
        assert_eq!(value("Metric"), "score · higher is better");
        assert_eq!(value("Baseline"), "1.000000 ± 0.000000 (3 of 3 runs)");
        assert_eq!(value("Reachable"), "yes");
        assert_eq!(value("Protected files"), "2");
        assert_eq!(value("Final check"), "validation only");
        assert!(value("Budget").starts_with("20 candidates"));
        assert_eq!(guardrails(&details), vec!["runtime <= 2.5".to_string()]);
        let line = integrity(&details);
        assert!(line.contains("pending"));
        assert!(line.contains("spec spec-version"));
        assert!(line.contains("1 findings"));
    }

    #[test]
    fn the_card_follows_the_run_the_conversation_lists() {
        let shown = details("running");
        let same: RunRecord =
            serde_json::from_value(run("running", json!({"phase": "search"}))).unwrap();
        assert!(!stale(&shown, &same));
        let further: RunRecord = serde_json::from_value(run(
            "running",
            json!({"phase": "search", "candidate_count": 1}),
        ))
        .unwrap();
        assert!(stale(&shown, &further));
        let ended: RunRecord =
            serde_json::from_value(run("succeeded", json!({"phase": "search"}))).unwrap();
        assert!(stale(&shown, &ended));
    }

    #[test]
    fn progress_and_candidates_read_as_lines() {
        let listed: RunRecord = serde_json::from_value(run(
            "running",
            json!({
                "phase": "final_verification",
                "best_primary": 1.5,
                "candidate_count": 4,
                "successful_count": 3,
                "failed_count": 1,
                "cost_microunits": 12,
                "last_checkpoint_at": 90,
            }),
        ))
        .unwrap();
        let rows = progress_rows(&progress(&listed), 100);
        assert_eq!(rows[0].1, "final verification");
        assert_eq!(rows[1].1, "1.500000");
        assert_eq!(rows[2].1, "4 · 3 scored, 1 failed");
        assert_eq!(rows[4].1, "—");
        assert_eq!(rows[5].1, "10 s ago");
        // A run that reported nothing yet still has a card.
        let quiet: RunRecord = serde_json::from_value(run("draft", json!(null))).unwrap();
        assert_eq!(progress_rows(&progress(&quiet), 100)[0].1, "—");

        let tried: MethodCandidateView = serde_json::from_value(json!({
            "id": "c2",
            "parent_candidate_id": "c1-0123456789",
            "sequence": 2,
            "strategy_key": "local_refine",
            "family": "linear",
            "status": "scored",
            "primary_score": 2.0,
            "runtime_ms": 15,
            "changed_lines": 3,
            "rationale": null,
            "error": null,
        }))
        .unwrap();
        assert_eq!(
            candidate(&tried),
            "#2 · linear · local_refine · 2.000000 · Δ 3 · 15 ms · ← c1-01234"
        );
        let broken: MethodCandidateView = serde_json::from_value(json!({
            "id": "c3",
            "parent_candidate_id": null,
            "sequence": 3,
            "strategy_key": "rewrite",
            "family": "tree",
            "status": "evaluation_failed",
            "primary_score": null,
            "runtime_ms": null,
            "changed_lines": null,
            "rationale": null,
            "error": "boom",
        }))
        .unwrap();
        assert_eq!(
            candidate(&broken),
            "#3 · tree · rewrite · evaluation failed"
        );
        let leaned: MethodStrategyView = serde_json::from_value(json!({
            "strategy_key": "rewrite",
            "category": "explore",
            "weight": 0.25,
            "attempts": 4,
            "improvements": 1,
        }))
        .unwrap();
        assert_eq!(
            strategy(&leaned),
            "rewrite · explore · weight 0.250 · 1 of 4 improved"
        );
    }

    #[test]
    fn promoted_files_are_named_and_shown_as_code() {
        let output = |key: &str| MethodSearchRunOutput {
            artifact_version_id: "v".into(),
            role: key.into(),
            logical_output_key: key.into(),
            source_path: "selected-method.py".into(),
        };
        assert_eq!(output_name(&output("selected_method")), "Selected method");
        assert_eq!(output_name(&output("top_k_2")), "Top 2");
        assert_eq!(
            output_name(&output("verification_report")),
            "Verification report"
        );
        assert_eq!(
            output_markdown("selected-method.py", "def f():\n    return 1\n"),
            "```python\ndef f():\n    return 1\n```"
        );
        // A file that holds a fence of its own is not cut short by it.
        let shown = output_markdown("notes.json", "```\n");
        assert!(shown.starts_with("````json\n"));
        assert!(shown.ends_with("\n````"));
    }
}
