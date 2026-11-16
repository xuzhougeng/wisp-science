//! The helpers a conversation sent off: each `explore` call the model
//! made, with what it was asked and what it found. Like the notebook it is
//! a reading of the transcript, not something saved on its own.
use wisp_dto::native_conversations::Item;

use crate::present::{self, Tone};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Helper {
    /// From 1, in the order they were sent.
    pub number: usize,
    /// What it was asked to find out.
    pub question: String,
    /// What the caller cared about most, if it said.
    pub focus: String,
    /// What it reported back; empty while it is still out.
    pub findings: String,
    /// How it went, as the transcript's tool row says.
    pub status: Option<(String, Tone)>,
    /// The transcript row it comes from.
    pub row: usize,
}

/// The helpers of a transcript, in order.
pub fn helpers(rows: &[Item], turn_running: bool) -> Vec<Helper> {
    let mut found = Vec::new();
    for (row, item) in rows.iter().enumerate() {
        if item.role != "tool" || item.tool_name.as_deref() != Some("explore") {
            continue;
        }
        let arguments: serde_json::Value = item
            .input
            .as_deref()
            .and_then(|input| serde_json::from_str(input).ok())
            .unwrap_or_default();
        let text = |key: &str| {
            arguments
                .get(key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .unwrap_or_default()
                .to_owned()
        };
        found.push(Helper {
            number: found.len() + 1,
            question: text("question"),
            focus: text("focus"),
            findings: item.text.trim().to_owned(),
            status: present::tool_status(item, turn_running),
            row,
        });
    }
    found
}

/// The line under the tab's title.
pub fn summary(helpers: &[Helper], has_conversation: bool) -> String {
    use crate::i18n::{Locale, current, tr};
    match (has_conversation, helpers.len()) {
        (false, _) => tr("Open a conversation to see the helpers it sent off.").to_owned(),
        (true, 0) => tr(
            "No helper has been sent off in this conversation. The model sends one when a \
             question needs a search of its own.",
        )
        .to_owned(),
        (true, count) => match current() {
            Locale::Zh => format!("本会话派出了 {count} 个帮手"),
            Locale::En if count == 1 => "1 helper was sent off in this conversation".to_owned(),
            Locale::En => format!("{count} helpers were sent off in this conversation"),
        },
    }
}

/// How a delegated batch stands, as its card's chip says.
pub fn batch_status(status: &str) -> (&'static str, Tone) {
    use crate::i18n::tr;
    match status {
        "draft" => (tr("Waiting for approval"), Tone::Attention),
        "approved" | "running" => (tr("Running"), Tone::Active),
        "succeeded" => (tr("Done"), Tone::Success),
        "failed" => (tr("Failed"), Tone::Danger),
        "cancelled" => (tr("Stopped"), Tone::Neutral),
        _ => (tr("Unknown"), Tone::Neutral),
    }
}

/// How one task of a batch stands.
pub fn task_status(status: &str) -> (&'static str, Tone) {
    use crate::i18n::tr;
    match status {
        "queued" => (tr("Waiting"), Tone::Neutral),
        "running" | "waiting_run" => (tr("Running"), Tone::Active),
        "succeeded" => (tr("Done"), Tone::Success),
        "failed" => (tr("Failed"), Tone::Danger),
        "blocked" => (tr("Blocked"), Tone::Attention),
        "cancelled" => (tr("Stopped"), Tone::Neutral),
        _ => (tr("Unknown"), Tone::Neutral),
    }
}

/// How work over many items stands, as its chip says.
pub fn swarm_phase(phase: wisp_dto::SwarmPhase) -> (&'static str, Tone) {
    use crate::i18n::tr;
    use wisp_dto::SwarmPhase::*;
    match phase {
        Starting | Running => (tr("Running"), Tone::Active),
        Summarizing => (tr("Summing up"), Tone::Active),
        Completed => (tr("Done"), Tone::Success),
        Partial => (tr("Partly done"), Tone::Attention),
        Failed => (tr("Failed"), Tone::Danger),
        Paused => (tr("Paused"), Tone::Neutral),
        Cancelled => (tr("Stopped"), Tone::Neutral),
    }
}

/// How one of its tasks stands.
pub fn swarm_task(status: wisp_dto::SwarmTaskStatus) -> (&'static str, Tone) {
    use crate::i18n::tr;
    use wisp_dto::SwarmTaskStatus::*;
    match status {
        Queued => (tr("Waiting"), Tone::Neutral),
        Running => (tr("Running"), Tone::Active),
        Completed => (tr("Done"), Tone::Success),
        Failed => (tr("Failed"), Tone::Danger),
        Paused => (tr("Paused"), Tone::Neutral),
        Cancelled => (tr("Stopped"), Tone::Neutral),
    }
}

/// How far it has come: items done of all of them, and those that failed.
pub fn swarm_line(progress: &wisp_dto::SwarmProgress) -> String {
    use crate::i18n::{fill, tr};
    let done: usize = progress.tasks.iter().map(|task| task.completed_units).sum();
    let failed: usize = progress.tasks.iter().map(|task| task.failed_units).sum();
    let total = progress
        .total
        .max(progress.tasks.iter().map(|task| task.total_units).sum());
    let mut line = fill(tr("{} of {} items done"), &[&done, &total]);
    if failed > 0 {
        line.push_str(&fill(tr(" · {} failed"), &[&failed]));
    }
    line
}

/// Whether the researcher approves or discards a batch from its card: a
/// workflow they ran as agents that waits in Draft. An agent's own batch
/// waits on its plan card instead.
pub fn awaits_approval(batch: &wisp_server::desktop::DelegatedBatch) -> bool {
    batch.launched && batch.status == "draft"
}

/// Whether a batch can be run again: one that failed or was stopped.
pub fn can_retry(batch: &wisp_server::desktop::DelegatedBatch) -> bool {
    // A task's own batch runs again with that task, not on its own.
    !batch.nested && matches!(batch.status.as_str(), "failed" | "cancelled")
}

/// Whether a task's token budget can be changed: before its batch runs
/// again, and only when the task did not finish.
pub fn budget_editable(
    batch: &wisp_server::desktop::DelegatedBatch,
    task: &wisp_server::desktop::DelegatedTask,
) -> bool {
    can_retry(batch) && task.status != "succeeded"
}

/// A task's token budget, or the one the rerun will give it when it was
/// changed.
pub fn budget_line(task: &wisp_server::desktop::DelegatedTask, changed: Option<u32>) -> String {
    let tokens = changed.or(task.max_tokens).filter(|tokens| *tokens > 0);
    match (tokens, changed.is_some()) {
        (Some(tokens), true) => crate::i18n::fill(
            crate::i18n::tr("Runs again with up to {} tokens"),
            &[&tokens],
        ),
        (Some(tokens), false) => crate::i18n::fill(crate::i18n::tr("Up to {} tokens"), &[&tokens]),
        (None, true) => crate::i18n::tr("Runs again with no token limit").to_owned(),
        (None, false) => crate::i18n::tr("No token limit").to_owned(),
    }
}

/// A typed token budget: a whole number, zero for no limit.
pub fn parse_budget(text: &str) -> Result<u32, &'static str> {
    text.trim()
        .replace([',', '_', ' '], "")
        .parse::<u32>()
        .map_err(|_| crate::i18n::tr("Enter a whole number of tokens; 0 is no limit"))
}

/// The budgets a rerun of `batch` sends: its tasks that did not finish and
/// were given another one, by task id.
pub fn rerun_budgets(
    batch: &wisp_server::desktop::DelegatedBatch,
    changed: &std::collections::HashMap<(String, String), u32>,
) -> std::collections::HashMap<String, u32> {
    batch
        .tasks
        .iter()
        .filter(|task| budget_editable(batch, task))
        .filter_map(|task| {
            let tokens = *changed.get(&(batch.id.clone(), task.id.clone()))?;
            (task.max_tokens.unwrap_or(0) != tokens).then(|| (task.id.clone(), tokens))
        })
        .collect()
}

/// How far a batch has come: its tasks that finished, of all of them.
pub fn batch_progress(batch: &wisp_server::desktop::DelegatedBatch) -> String {
    let done = batch
        .tasks
        .iter()
        .filter(|task| task.status == "succeeded")
        .count();
    crate::i18n::fill(
        crate::i18n::tr("{} of {} tasks done"),
        &[&done, &batch.tasks.len()],
    )
}

/// What a task used, when it used anything.
pub fn task_usage(task: &wisp_server::desktop::DelegatedTask) -> Option<String> {
    let tokens = task.input_tokens + task.output_tokens;
    (tokens > 0 || task.tool_calls > 0).then(|| {
        crate::i18n::fill(
            crate::i18n::tr("{} tokens · {} tool calls"),
            &[&tokens, &task.tool_calls],
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(value: serde_json::Value) -> Item {
        let mut whole = json!({"role": "tool", "text": ""});
        for (key, changed) in value.as_object().unwrap() {
            whole[key] = changed.clone();
        }
        serde_json::from_value(whole).unwrap()
    }

    #[test]
    fn work_over_many_items_reads_as_how_far_it_came() {
        let progress: wisp_dto::SwarmProgress = serde_json::from_value(json!({
            "id": "s", "source": "map_items", "title": "Read", "model": "m",
            "started_at_ms": 1, "revision": 2, "phase": "running", "total": 3,
            "summary": "",
            "tasks": [
                {"id": "a", "title": "a", "instruction": "", "status": "completed", "revision": 1,
                 "completed_units": 1, "total_units": 1, "failed_units": 0, "log": [], "result": ""},
                {"id": "b", "title": "b", "instruction": "", "status": "failed", "revision": 1,
                 "completed_units": 0, "total_units": 1, "failed_units": 1, "log": [], "result": ""},
            ],
        }))
        .unwrap();
        assert_eq!(swarm_line(&progress), "1 of 3 items done · 1 failed");
        assert_eq!(
            swarm_phase(wisp_dto::SwarmPhase::Partial),
            ("Partly done", Tone::Attention)
        );
        assert_eq!(
            swarm_task(wisp_dto::SwarmTaskStatus::Queued),
            ("Waiting", Tone::Neutral)
        );
    }

    #[test]
    fn a_delegated_batch_reads_as_its_status_and_how_far_it_came() {
        use wisp_server::desktop::{DelegatedBatch, DelegatedTask};
        let task = |id: &str, status: &str| DelegatedTask {
            id: id.into(),
            instruction: format!("Do {id}"),
            depends_on: vec![],
            capabilities: vec!["project_read".into()],
            expert: None,
            status: status.into(),
            summary: None,
            error: None,
            conversation: None,
            input_tokens: 0,
            output_tokens: 0,
            tool_calls: 0,
            max_tokens: None,
            isolated: false,
        };
        let batch = DelegatedBatch {
            id: "b".into(),
            goal: "Count".into(),
            status: "running".into(),
            created_at: 1,
            updated_at: 2,
            tasks: vec![
                task("a", "succeeded"),
                task("b", "running"),
                task("c", "queued"),
            ],
            nested: false,
            launched: false,
        };
        assert!(batch.running());
        assert!(!can_retry(&batch));
        // A workflow the researcher ran as agents is approved on its card;
        // an agent's own draft is approved on its plan card.
        let drafted = DelegatedBatch {
            status: "draft".into(),
            launched: true,
            ..batch.clone()
        };
        assert!(awaits_approval(&drafted));
        assert!(!awaits_approval(&DelegatedBatch {
            launched: false,
            ..drafted.clone()
        }));
        assert!(!awaits_approval(&DelegatedBatch {
            status: "running".into(),
            ..drafted
        }));
        for (status, again) in [("failed", true), ("cancelled", true), ("succeeded", false)] {
            let ended = DelegatedBatch {
                status: status.into(),
                ..batch.clone()
            };
            assert_eq!(can_retry(&ended), again, "{status}");
            let nested = DelegatedBatch {
                nested: true,
                ..ended
            };
            assert!(!can_retry(&nested), "{status}");
        }
        assert_eq!(batch_progress(&batch), "1 of 3 tasks done");
        assert_eq!(batch_status("running"), ("Running", Tone::Active));
        assert_eq!(batch_status("cancelled"), ("Stopped", Tone::Neutral));
        assert_eq!(task_status("blocked"), ("Blocked", Tone::Attention));
        assert_eq!(task_status("failed"), ("Failed", Tone::Danger));
        assert_eq!(task_usage(&batch.tasks[0]), None);
        let used = DelegatedTask {
            input_tokens: 1200,
            output_tokens: 300,
            tool_calls: 4,
            ..task("d", "succeeded")
        };
        assert_eq!(
            task_usage(&used).as_deref(),
            Some("1500 tokens · 4 tool calls")
        );

        // Before a failed batch runs again, the tasks that did not finish
        // can be given another token budget.
        let failed = DelegatedBatch {
            status: "failed".into(),
            tasks: vec![
                task("a", "succeeded"),
                DelegatedTask {
                    max_tokens: Some(4000),
                    ..task("b", "failed")
                },
                task("c", "cancelled"),
            ],
            ..batch.clone()
        };
        assert!(!budget_editable(&batch, &batch.tasks[1]));
        assert!(!budget_editable(&failed, &failed.tasks[0]));
        assert!(budget_editable(&failed, &failed.tasks[1]));
        assert_eq!(budget_line(&failed.tasks[1], None), "Up to 4000 tokens");
        assert_eq!(budget_line(&failed.tasks[2], None), "No token limit");
        assert_eq!(
            budget_line(&failed.tasks[1], Some(9000)),
            "Runs again with up to 9000 tokens"
        );
        assert_eq!(
            budget_line(&failed.tasks[1], Some(0)),
            "Runs again with no token limit"
        );
        assert_eq!(parse_budget(" 12,000 "), Ok(12_000));
        assert_eq!(parse_budget("0"), Ok(0));
        assert!(parse_budget("lots").is_err());
        assert!(parse_budget("-5").is_err());
        let key = |task: &str| ("b".to_owned(), task.to_owned());
        let changed = std::collections::HashMap::from([
            // A finished task's is not sent, nor one set to what it was.
            (key("a"), 100),
            (key("b"), 4000),
            (key("c"), 9000),
            (("other".to_owned(), "c".to_owned()), 1),
        ]);
        assert_eq!(
            rerun_budgets(&failed, &changed),
            std::collections::HashMap::from([("c".to_owned(), 9000)])
        );
    }

    #[test]
    fn each_explore_call_is_a_helper_with_its_question_and_findings() {
        let rows = [
            row(json!({"role": "user", "text": "where is the aligner configured?"})),
            row(json!({"tool_name": "shell", "input": "{\"cmd\":\"ls\"}", "text": "a"})),
            row(json!({
                "tool_name": "explore",
                "input": "{\"question\":\" Where is the aligner set up? \",\"focus\":\"config files\"}",
                "text": " In config/align.yml. ",
                "ok": true,
                "duration_ms": 4200
            })),
            row(json!({
                "tool_name": "explore",
                "input": "{\"question\":\"Which samples failed QC?\"}",
                "status": "running"
            })),
            // Arguments that cannot be read still make a helper.
            row(json!({"tool_name": "explore", "input": "not json", "ok": false})),
        ];
        let found = helpers(&rows, true);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].number, 1);
        assert_eq!(found[0].question, "Where is the aligner set up?");
        assert_eq!(found[0].focus, "config files");
        assert_eq!(found[0].findings, "In config/align.yml.");
        assert_eq!(found[0].row, 2);
        assert_eq!(
            found[0].status,
            Some(("Done · 4.2 s".into(), Tone::Success))
        );
        // Still out: nothing found yet.
        assert_eq!(found[1].findings, "");
        assert_eq!(found[1].status, Some(("Running".into(), Tone::Active)));
        assert_eq!(found[2].question, "");
        assert_eq!(found[2].status, Some(("Failed".into(), Tone::Danger)));
        // The turn over, one that never came back was interrupted.
        assert_eq!(
            helpers(&rows, false)[1].status,
            Some(("Interrupted".into(), Tone::Neutral))
        );
    }

    #[test]
    fn the_summary_counts_the_helpers_or_says_why_there_are_none() {
        let one = helpers(
            &[row(
                json!({"tool_name": "explore", "input": "{\"question\":\"q\"}"}),
            )],
            false,
        );
        assert_eq!(
            summary(&one, true),
            "1 helper was sent off in this conversation"
        );
        assert!(summary(&[], true).starts_with("No helper has been sent off"));
        assert!(summary(&[], false).starts_with("Open a conversation"));
    }
}
