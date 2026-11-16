//! The helpers a conversation sent off: each `explore` call the model
//! made, with what it was asked and what it found. Like the notebook it is
//! a reading of the transcript, not something saved on its own. Also how
//! delegated batches stand, and the cards the transcript draws for the
//! calls that hand work to other agents.
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
        if item.role != "tool" || item.tool_name.as_deref() != Some(EXPLORE) {
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

/// How far a batch has come, with how many of its tasks stand each other
/// way: running, waiting, blocked, failed or stopped. A count of none is
/// left out.
pub fn batch_counts(batch: &wisp_server::desktop::DelegatedBatch) -> String {
    use crate::i18n::{fill, tr};
    let count = |statuses: &[&str]| {
        batch
            .tasks
            .iter()
            .filter(|task| statuses.contains(&task.status.as_str()))
            .count()
    };
    let mut line = batch_progress(batch);
    for (statuses, part) in [
        (&["running", "waiting_run"][..], tr(" · {} running")),
        (&["queued"][..], tr(" · {} waiting")),
        (&["blocked"][..], tr(" · {} blocked")),
        (&["failed"][..], tr(" · {} failed")),
        (&["cancelled"][..], tr(" · {} stopped")),
    ] {
        let found = count(statuses);
        if found > 0 {
            line.push_str(&fill(part, &[&found]));
        }
    }
    line
}

// The calls in the transcript that hand work to other agents, or report on
// that work, and the cards they are drawn as.

const EXPLORE: &str = "explore";
const DISPATCH: &str = "dispatch_subagent";
const SUBAGENT_STATUS: &str = "subagent_status";
const STOP_SUBAGENT: &str = "stop_subagent";
const DELEGATE: &str = "delegate_tasks";
const DELEGATED_RESULT: &str = "get_delegated_result";
/// The row a background batch's result comes back to the conversation as.
const BATCH_COMPLETION: &str = "delegate_tasks_completion";

/// A subagent of the open conversation, as the conversation list gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subagent {
    pub title: String,
    /// The list's word for it: `running`, `needs_you` or `complete`.
    pub status: String,
}

/// The subagents `session` started, by conversation id, with the title and
/// status the conversation list gives each (none when it is not listed).
pub fn subagents_of(
    session: &str,
    dispatched: &std::collections::HashMap<String, String>,
    known: &std::collections::HashMap<String, wisp_dto::SessionSearchInfo>,
) -> std::collections::HashMap<String, Subagent> {
    dispatched
        .iter()
        .filter(|(_, parent)| parent.as_str() == session)
        .map(|(child, _)| {
            let row = known.get(child);
            let subagent = Subagent {
                title: row.map(|row| row.title.clone()).unwrap_or_default(),
                status: row.map(|row| row.status.clone()).unwrap_or_default(),
            };
            (child.clone(), subagent)
        })
        .collect()
}

/// How a subagent stands, as its card says: as the sidebar says it, or
/// idle when it is not working and not waiting on anyone.
pub fn subagent_status(status: &str) -> (&'static str, Tone) {
    present::session_status(status).unwrap_or((crate::i18n::tr("Idle"), Tone::Neutral))
}

/// What the cards are made from: the open conversation's batches (`None`
/// until they have been read for it) and its subagents.
pub struct CardData<'a> {
    pub batches: Option<&'a [wisp_server::desktop::DelegatedBatch]>,
    pub subagents: &'a std::collections::HashMap<String, Subagent>,
    pub turn_running: bool,
}

/// What a transcript row that hands work to other agents is drawn as.
#[derive(Clone, Debug, PartialEq)]
pub enum Card {
    /// An `explore` call: its question, how it stands and what it found.
    Explore(Helper),
    /// A `dispatch_subagent` call: the task and the conversation doing it.
    Subagent(SubagentCard),
    /// A `delegate_tasks` call: the batch and each of its tasks.
    Batch(BatchCard),
    /// A call that checks on, stops or reads back work that has a card of
    /// its own: one quiet line that names it.
    Mention(Mention),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentCard {
    /// The subagent's conversation.
    pub session: String,
    pub title: String,
    /// What it was told to do.
    pub task: String,
    /// The call sent a further instruction to a subagent already started.
    pub follow_up: bool,
    pub status: (&'static str, Tone),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchCard {
    pub batch: wisp_server::desktop::DelegatedBatch,
    /// The turn went on while it ran; its result comes back later.
    pub background: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    /// The shared icon for what it names: a subagent or a batch.
    pub icon: &'static str,
    pub text: String,
    pub status: Option<(String, Tone)>,
}

/// Whether the transcript has calls whose cards need the conversation's
/// delegated batches.
pub fn wants_batches(rows: &[Item]) -> bool {
    rows.iter().any(|item| {
        item.role == "tool"
            && matches!(
                item.tool_name.as_deref(),
                Some(DELEGATE | DELEGATED_RESULT | BATCH_COMPLETION)
            )
    })
}

/// The card each row that hands work to other agents is drawn as, by row.
/// A row whose work cannot be found in `data` (not read yet, or gone) has
/// none and stays an ordinary tool row.
pub fn cards(rows: &[Item], data: &CardData) -> std::collections::HashMap<usize, Card> {
    let mut found = std::collections::HashMap::new();
    for helper in helpers(rows, data.turn_running) {
        found.insert(helper.row, Card::Explore(helper));
    }
    let batches = data.batches.unwrap_or_default();
    // Each batch belongs to one call. A call that came back names its batch;
    // one still out (waiting on its plan, or running in the turn) is the
    // latest unclaimed batch with its goal.
    let mut claimed = std::collections::HashSet::new();
    let mut by_row = std::collections::HashMap::new();
    let delegations: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, item)| item.role == "tool" && item.tool_name.as_deref() == Some(DELEGATE))
        .map(|(row, _)| row)
        .collect();
    for &row in &delegations {
        if let Some(batch) = batch_named(&rows[row].text, batches) {
            if claimed.insert(batch.id.clone()) {
                by_row.insert(row, batch);
            }
        }
    }
    for &row in delegations.iter().rev() {
        let item = &rows[row];
        // A call that came back without naming a kept batch (refused,
        // denied, or failed before one was kept) has none.
        if by_row.contains_key(&row) || item.ok == Some(false) || !item.text.trim().is_empty() {
            continue;
        }
        let goal = argument(item, "goal");
        // Newest first, as the host lists them.
        let batch = batches.iter().find(|batch| {
            !batch.launched && !batch.nested && batch.goal == goal && !claimed.contains(&batch.id)
        });
        if let Some(batch) = batch {
            claimed.insert(batch.id.clone());
            by_row.insert(row, batch);
        }
    }
    for (row, batch) in by_row {
        let background = arguments(&rows[row])
            .get("background")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        found.insert(
            row,
            Card::Batch(BatchCard {
                batch: batch.clone(),
                background,
            }),
        );
    }
    for (row, item) in rows.iter().enumerate() {
        if item.role != "tool" {
            continue;
        }
        let card = match item.tool_name.as_deref() {
            Some(DISPATCH) => subagent_card(item, data.subagents).map(Card::Subagent),
            Some(SUBAGENT_STATUS | STOP_SUBAGENT | DELEGATED_RESULT | BATCH_COMPLETION) => {
                mention(item, batches, data).map(Card::Mention)
            }
            _ => None,
        };
        if let Some(card) = card {
            found.insert(row, card);
        }
    }
    found
}

/// A `dispatch_subagent` call's card, once the subagent it started (named
/// in what the call returned) or instructed again (named in what it was
/// given) is listed as this conversation's.
fn subagent_card(
    item: &Item,
    subagents: &std::collections::HashMap<String, Subagent>,
) -> Option<SubagentCard> {
    if item.ok == Some(false) {
        return None;
    }
    let again = argument(item, "session_id");
    let session = if again.is_empty() {
        named(&item.text, subagents.keys())?
    } else {
        again.clone()
    };
    let subagent = subagents.get(&session)?;
    let task = argument(item, "instruction");
    let title = [subagent.title.trim(), &argument(item, "title")]
        .into_iter()
        .find(|title| !title.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| short(&task));
    Some(SubagentCard {
        session,
        title,
        task,
        follow_up: !again.is_empty(),
        status: subagent_status(&subagent.status),
    })
}

/// The one line for a call that checks on, stops or reads back work that
/// has its own card, naming that work; none when it cannot be named.
fn mention(
    item: &Item,
    batches: &[wisp_server::desktop::DelegatedBatch],
    data: &CardData,
) -> Option<Mention> {
    use crate::i18n::{fill, tr};
    let status = present::tool_status(item, data.turn_running);
    let subagent = |id: &str| {
        data.subagents
            .get(id)
            .map(|subagent| subagent.title.clone())
    };
    let (icon, text, status) = match item.tool_name.as_deref()? {
        SUBAGENT_STATUS => match argument(item, "session_id").as_str() {
            "" => (
                "bot",
                tr("Checked on this conversation's subagents").to_owned(),
                status,
            ),
            id => (
                "bot",
                fill(tr("Checked on the subagent “{}”"), &[&subagent(id)?]),
                status,
            ),
        },
        STOP_SUBAGENT => {
            let title = subagent(&argument(item, "session_id"))?;
            (
                "bot",
                fill(tr("Asked the subagent “{}” to stop"), &[&title]),
                status,
            )
        }
        DELEGATED_RESULT => {
            let id = argument(item, "workflow_id");
            let batch = batches.iter().find(|batch| batch.id == id)?;
            let task = argument(item, "task_id");
            (
                "swarm",
                fill(
                    tr("Read the result of {} in “{}”"),
                    &[&task, &short(&batch.goal)],
                ),
                status,
            )
        }
        BATCH_COMPLETION => {
            let batch = batch_named(&item.text, batches)?;
            let (word, tone) = batch_status(&batch.status);
            (
                "swarm",
                fill(
                    tr("The background batch “{}” came back"),
                    &[&short(&batch.goal)],
                ),
                Some((word.to_owned(), tone)),
            )
        }
        _ => return None,
    };
    Some(Mention { icon, text, status })
}

/// The batch a call's result names: its `workflow_id`, or, when the result
/// was cut short, the id of a batch found in its text.
fn batch_named<'a>(
    text: &str,
    batches: &'a [wisp_server::desktop::DelegatedBatch],
) -> Option<&'a wisp_server::desktop::DelegatedBatch> {
    let id = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("workflow_id")?.as_str().map(str::to_owned));
    match id {
        Some(id) => batches.iter().find(|batch| batch.id == id),
        None => {
            let id = named(text, batches.iter().map(|batch| &batch.id))?;
            batches.iter().find(|batch| batch.id == id)
        }
    }
}

/// The first of `ids` that `text` mentions.
fn named<'a>(text: &str, ids: impl Iterator<Item = &'a String>) -> Option<String> {
    ids.filter(|id| !id.is_empty())
        .filter_map(|id| text.find(id.as_str()).map(|at| (at, id)))
        .min()
        .map(|(_, id)| id.clone())
}

fn arguments(item: &Item) -> serde_json::Value {
    item.input
        .as_deref()
        .and_then(|input| serde_json::from_str(input).ok())
        .unwrap_or_default()
}

/// A text argument of the call, trimmed; empty when it has none.
fn argument(item: &Item, key: &str) -> String {
    arguments(item)
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

/// The first line of `text`, cut to a line's length.
fn short(text: &str) -> String {
    const MOST: usize = 80;
    let line = text.trim().lines().next().unwrap_or_default();
    if line.chars().count() > MOST {
        format!("{}…", line.chars().take(MOST - 1).collect::<String>())
    } else {
        line.to_owned()
    }
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

    fn task_of(id: &str, status: &str) -> wisp_server::desktop::DelegatedTask {
        wisp_server::desktop::DelegatedTask {
            id: id.into(),
            instruction: format!("Do {id}"),
            depends_on: vec![],
            capabilities: vec![],
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
        }
    }

    fn batch_of(id: &str, goal: &str, status: &str) -> wisp_server::desktop::DelegatedBatch {
        wisp_server::desktop::DelegatedBatch {
            id: id.into(),
            goal: goal.into(),
            status: status.into(),
            created_at: 1,
            updated_at: 1,
            tasks: vec![task_of("a", "succeeded"), task_of("b", "running")],
            nested: false,
            launched: false,
        }
    }

    fn listed(id: &str, title: &str, status: &str) -> wisp_dto::SessionSearchInfo {
        wisp_dto::SessionSearchInfo {
            id: id.into(),
            project_id: "p".into(),
            project_name: "P".into(),
            title: title.into(),
            ts: 0,
            activity_at: 0,
            status: status.into(),
            shelved: false,
        }
    }

    #[test]
    fn a_batch_counts_its_tasks_by_how_each_stands() {
        let mut batch = batch_of("b", "Count", "running");
        batch.tasks = [
            "succeeded",
            "running",
            "queued",
            "queued",
            "blocked",
            "failed",
            "cancelled",
        ]
        .iter()
        .enumerate()
        .map(|(at, status)| task_of(&at.to_string(), status))
        .collect();
        assert_eq!(
            batch_counts(&batch),
            "1 of 7 tasks done · 1 running · 2 waiting · 1 blocked · 1 failed · 1 stopped"
        );
        // A count of none is left out.
        assert_eq!(
            batch_counts(&batch_of("b", "Count", "running")),
            "1 of 2 tasks done · 1 running"
        );
    }

    #[test]
    fn a_conversations_subagents_are_those_it_started_as_the_list_names_them() {
        let dispatched = std::collections::HashMap::from([
            ("c1".to_owned(), "s".to_owned()),
            ("c2".to_owned(), "other".to_owned()),
            ("c3".to_owned(), "s".to_owned()),
        ]);
        let known = std::collections::HashMap::from([
            ("c1".to_owned(), listed("c1", "Read counts", "running")),
            ("c2".to_owned(), listed("c2", "Not ours", "complete")),
        ]);
        let found = subagents_of("s", &dispatched, &known);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found["c1"],
            Subagent {
                title: "Read counts".into(),
                status: "running".into()
            }
        );
        // Started here but not listed (yet): known by id only.
        assert_eq!(found["c3"].title, "");
        assert_eq!(subagent_status("running"), ("Running", Tone::Active));
        assert_eq!(subagent_status("needs_you"), ("Needs you", Tone::Attention));
        assert_eq!(subagent_status("complete"), ("Idle", Tone::Neutral));
        assert_eq!(subagent_status(""), ("Idle", Tone::Neutral));
    }

    #[test]
    fn each_call_that_hands_work_off_is_matched_to_its_batch_or_subagent() {
        let started = "Subagent c1 has started and runs in the background.";
        let rows = [
            row(json!({"role": "user", "text": "count the reads"})),
            // 1: a helper is drawn from its own row.
            row(
                json!({"tool_name": "explore", "input": "{\"question\":\"Where?\"}", "text": "Here.", "ok": true}),
            ),
            // 2: the subagent it started, named in what the call returned.
            row(json!({"tool_name": "dispatch_subagent",
                "input": "{\"instruction\":\"Count reads per sample\",\"title\":\"Counts\"}",
                "text": started, "ok": true})),
            // 3: a further instruction to it, named in what it was given.
            row(json!({"tool_name": "dispatch_subagent",
                "input": "{\"instruction\":\"Now plot them\",\"session_id\":\"c1\"}",
                "text": started, "ok": true})),
            // 4: refused.
            row(json!({"tool_name": "dispatch_subagent",
                "input": "{\"instruction\":\"x\"}", "text": "'instruction' cannot be empty", "ok": false})),
            // 5: names a conversation that is not this one's subagent.
            row(json!({"tool_name": "dispatch_subagent",
                "input": "{\"instruction\":\"x\"}", "text": "Subagent stranger has started", "ok": true})),
            // 6, 7, 8: checks and stops are one line naming what they ask about.
            row(
                json!({"tool_name": "subagent_status", "input": "{\"session_id\":\"c1\"}", "text": "Status: running", "ok": true}),
            ),
            row(
                json!({"tool_name": "subagent_status", "input": "{}", "text": "c1 · Counts · running", "ok": true}),
            ),
            row(
                json!({"tool_name": "stop_subagent", "input": "{\"session_id\":\"stranger\"}", "ok": false}),
            ),
            // 9: a batch that came back names itself.
            row(
                json!({"tool_name": "delegate_tasks", "input": "{\"goal\":\"Count\",\"tasks\":[]}",
                "text": "{\"status\":\"succeeded\",\"workflow_id\":\"b-old\"}", "ok": true}),
            ),
            // 10: one still running in the turn is the latest batch with its goal.
            row(
                json!({"tool_name": "delegate_tasks", "input": "{\"goal\":\" Count \",\"tasks\":[]}",
                "status": "running"}),
            ),
            // 11: reading back a task names the batch.
            row(json!({"tool_name": "get_delegated_result",
                "input": "{\"workflow_id\":\"b-old\",\"task_id\":\"a\"}", "text": "{}", "ok": true})),
            // 12: a denied batch is gone and does not take another's card.
            row(
                json!({"tool_name": "delegate_tasks", "input": "{\"goal\":\"Count\",\"tasks\":[]}",
                "text": "{\"status\":\"denied\",\"workflow_id\":\"gone\"}", "ok": true}),
            ),
            // 13: a background batch, and 14: its result coming back, cut short.
            row(
                json!({"tool_name": "delegate_tasks", "input": "{\"goal\":\"Survey\",\"background\":true,\"tasks\":[]}",
                "text": "{\"auto_resume\":true,\"workflow_id\":\"b-bg\"}", "ok": true}),
            ),
            row(json!({"tool_name": "delegate_tasks_completion",
                "text": "{\"generation\":1,\"result\":{\"lookup\":{\"workflow_id\":\"b-bg\"},\"resul", "ok": true})),
        ];
        // Newest first, as the host lists them; a workflow the researcher
        // ran as agents has no call of its own.
        let launched = wisp_server::desktop::DelegatedBatch {
            launched: true,
            ..batch_of("b-launched", "Count", "draft")
        };
        let batches = [
            launched,
            batch_of("b-new", "Count", "running"),
            batch_of("b-bg", "Survey", "succeeded"),
            batch_of("b-old", "Count", "succeeded"),
        ];
        let subagents = std::collections::HashMap::from([(
            "c1".to_owned(),
            Subagent {
                title: "Read counts".into(),
                status: "running".into(),
            },
        )]);
        let data = CardData {
            batches: Some(&batches),
            subagents: &subagents,
            turn_running: true,
        };
        let found = cards(&rows, &data);
        let batch = |row: usize| match &found[&row] {
            Card::Batch(card) => (card.batch.id.as_str(), card.background),
            other => panic!("row {row}: {other:?}"),
        };
        let line = |row: usize| match &found[&row] {
            Card::Mention(mention) => mention.text.as_str(),
            other => panic!("row {row}: {other:?}"),
        };
        assert!(matches!(&found[&1], Card::Explore(helper) if helper.question == "Where?"));
        assert_eq!(
            found[&2],
            Card::Subagent(SubagentCard {
                session: "c1".into(),
                title: "Read counts".into(),
                task: "Count reads per sample".into(),
                follow_up: false,
                status: ("Running", Tone::Active),
            })
        );
        assert!(
            matches!(&found[&3], Card::Subagent(card) if card.follow_up && card.task == "Now plot them")
        );
        for plain in [0, 4, 5, 8, 12] {
            assert!(!found.contains_key(&plain), "row {plain}");
        }
        assert_eq!(line(6), "Checked on the subagent “Read counts”");
        assert_eq!(line(7), "Checked on this conversation's subagents");
        assert_eq!(batch(9), ("b-old", false));
        assert_eq!(batch(10), ("b-new", false));
        assert_eq!(line(11), "Read the result of a in “Count”");
        assert_eq!(batch(13), ("b-bg", true));
        assert_eq!(line(14), "The background batch “Survey” came back");
        assert!(matches!(&found[&14], Card::Mention(mention)
            if mention.status == Some(("Done".into(), Tone::Success))));
        assert!(wants_batches(&rows));
        assert!(!wants_batches(&rows[..9]));

        // Before the batches are read, their calls stay ordinary tool rows;
        // helpers and subagents do not wait for them.
        let unread = cards(
            &rows,
            &CardData {
                batches: None,
                ..data
            },
        );
        for plain in [9, 10, 11, 13, 14] {
            assert!(!unread.contains_key(&plain), "row {plain}");
        }
        assert!(unread.contains_key(&1) && unread.contains_key(&2) && unread.contains_key(&6));
        // A subagent the list does not have yet is not guessed at.
        let none = std::collections::HashMap::new();
        let unlisted = cards(
            &rows,
            &CardData {
                subagents: &none,
                ..data
            },
        );
        for plain in [2, 3, 6] {
            assert!(!unlisted.contains_key(&plain), "row {plain}");
        }
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
