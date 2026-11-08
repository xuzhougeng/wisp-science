//! The Agents pane's subagent feed: every subagent the current conversation
//! started, whichever mechanism ran it. Watch-only child conversations
//! (`dispatch_subagent`, #1061) come from the frames table; `explore` runs
//! are in-process, so their only trace here is the anchor in the parent's
//! transcript. `delegate_tasks` workflows keep their own snapshot command.

use crate::AppState;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use tauri::State;
use wisp_dto::SubagentActivity;
use wisp_llm::{Message, Role};
use wisp_store::{MessageLogCursor, Store};

const SUMMARY_CHARS: usize = 160;
const TITLE_CHARS: usize = 80;
const PARENT_BRIEF_PREFIX: &str = "[From the parent conversation's agent]";
/// In-process subagent tools whose calls and anchors the feed reads.
const SUBAGENT_TOOLS: [&str; 2] = ["explore", "consult_expert"];
/// Conversations kept folded across polls; the least recently read goes first.
const CACHED_FRAMES: usize = 64;

#[tauri::command]
pub(crate) async fn list_subagent_activity(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: Option<String>,
) -> Result<Vec<SubagentActivity>, String> {
    let project = state.require_active(window.label())?;
    let Some(session_id) = session_id else {
        return Ok(vec![]);
    };
    let store = &state.store;
    // A taken-over delegation child keeps showing its root's subagents (#442).
    let root = store
        .root_frame_id(&session_id)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| session_id.clone());
    let running = state.running_turns.lock().await.clone();
    let mut items = Vec::new();
    let dispatched = store
        .list_dispatched_sessions(&project.id)
        .await
        .map_err(|error| error.to_string())?;
    let profiles = crate::models::ensure(store).await;
    for (child, parent) in dispatched {
        if parent != root && parent != session_id {
            continue;
        }
        let reference = store.get_session_reference(&child).await.ok().flatten();
        let conversation = with_frame_fold(store, &child, |fold| fold.conversation.clone())
            .await
            .unwrap_or_default();
        let mut activity = conversation_activity(
            &child,
            reference.as_ref().map(|r| r.title.as_str()).unwrap_or(""),
            reference.as_ref().map(|r| r.created_at).unwrap_or(0),
            running.contains(&child),
            &conversation,
        );
        // The child's bound model (#1638), so the feed shows which worker ran.
        activity.model = store
            .frame_model(&child)
            .await
            .ok()
            .flatten()
            .and_then(|id| {
                profiles
                    .iter()
                    .find(|profile| profile.id == id)
                    .map(|profile| profile.label.clone())
            })
            .unwrap_or_default();
        items.push(activity);
    }
    let (explores, experts) = with_frame_fold(store, &session_id, |fold| {
        (
            explore_activities(&fold.calls.messages),
            expert_activities(&fold.calls.messages),
        )
    })
    .await
    .map_err(|error| error.to_string())?;
    items.extend(explores);
    items.extend(experts);
    items.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(items)
}

/// What the feed needs from one conversation, advanced row by row so a poll
/// reads only what was appended since the previous one. The Agents pane polls
/// every second; reloading whole transcripts made that cost grow with the
/// conversation, which is exactly when a long autonomous turn is watched.
#[derive(Default)]
struct FrameFold {
    cursor: Option<MessageLogCursor>,
    conversation: ConversationFold,
    calls: SubagentCallLog,
    last_read: u64,
}

impl FrameFold {
    fn push(&mut self, message: &Message) {
        self.conversation.push(message);
        self.calls.push(message);
    }
}

#[derive(Default)]
struct FrameFolds {
    frames: HashMap<String, FrameFold>,
    clock: u64,
}

static FRAME_FOLDS: LazyLock<Mutex<FrameFolds>> = LazyLock::new(Mutex::default);

/// Bring `frame_id`'s fold up to date and read from it. A rewritten history
/// (compaction, undo, rewind) comes back as a reset and is refolded whole.
async fn with_frame_fold<T>(
    store: &Store,
    frame_id: &str,
    read: impl FnOnce(&FrameFold) -> T,
) -> anyhow::Result<T> {
    let cached = FRAME_FOLDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .frames
        .remove(frame_id);
    let mut fold = cached.unwrap_or_default();
    let delta = store
        .load_messages_since(frame_id, fold.cursor.as_ref())
        .await?;
    if delta.reset {
        fold = FrameFold::default();
    }
    for message in &delta.messages {
        fold.push(message);
    }
    fold.cursor = delta.cursor;
    let value = read(&fold);
    let mut folds = FRAME_FOLDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    folds.clock += 1;
    fold.last_read = folds.clock;
    folds.frames.insert(frame_id.to_string(), fold);
    if folds.frames.len() > CACHED_FRAMES {
        let oldest = folds
            .frames
            .iter()
            .min_by_key(|(_, fold)| fold.last_read)
            .map(|(id, _)| id.clone());
        if let Some(id) = oldest {
            folds.frames.remove(&id);
        }
    }
    Ok(value)
}

/// A child conversation's feed row inputs: its latest instruction, its real
/// answer, its last activity time and how many tools it ran.
#[derive(Clone, Default)]
struct ConversationFold {
    instruction: String,
    answer: Option<String>,
    last_ts: Option<i64>,
    tool_calls: u32,
}

impl ConversationFold {
    fn push(&mut self, message: &Message) {
        if message.role == Role::User {
            self.instruction = strip_parent_brief(&message.content.as_text());
        }
        if message.role == Role::Tool && message.tool_name.as_deref() != Some("attempt_completion")
        {
            self.tool_calls += 1;
        }
        if is_answer(message) {
            let text = message.content.as_text();
            if !text.trim().is_empty() {
                self.answer = Some(text);
            }
        }
        self.last_ts = Some(message.ts);
    }
}

#[cfg(test)]
impl From<&[Message]> for ConversationFold {
    fn from(messages: &[Message]) -> Self {
        let mut fold = Self::default();
        messages.iter().for_each(|message| fold.push(message));
        fold
    }
}

/// The parent rows `explore_activities` and `expert_activities` read: every
/// assistant message calling a subagent tool, and every tool row answering
/// one of those call ids. A result always follows its call, so the first
/// match after a call is the same here as in the whole transcript.
#[derive(Default)]
struct SubagentCallLog {
    messages: Vec<Message>,
    call_ids: HashSet<String>,
}

impl SubagentCallLog {
    fn push(&mut self, message: &Message) {
        match message.role {
            Role::Assistant => {
                let mut calls = false;
                for call in &message.tool_calls {
                    if SUBAGENT_TOOLS.contains(&call.function.name.as_str()) {
                        self.call_ids.insert(call.id.clone());
                        calls = true;
                    }
                }
                if calls {
                    self.messages.push(message.clone());
                }
            }
            Role::Tool
                if message
                    .tool_call_id
                    .as_ref()
                    .is_some_and(|id| self.call_ids.contains(id)) =>
            {
                self.messages.push(message.clone());
            }
            _ => {}
        }
    }
}

/// The turn's real answer: an `attempt_completion` result counts, the last
/// assistant bubble alone does not (it is often empty when a tool ended the turn).
pub(crate) fn final_answer(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|m| is_answer(m))
        .map(|m| m.content.as_text())
        .find(|t| !t.trim().is_empty())
}

fn is_answer(message: &Message) -> bool {
    message.role == Role::Assistant
        || (message.role == Role::Tool
            && message.tool_name.as_deref() == Some("attempt_completion"))
}

fn conversation_activity(
    session: &str,
    title: &str,
    created_at: i64,
    running: bool,
    fold: &ConversationFold,
) -> SubagentActivity {
    let instruction = fold.instruction.clone();
    let answer = fold.answer.clone().unwrap_or_default();
    let status = if running {
        "running"
    } else if answer.starts_with("Error: ") {
        "failed"
    } else if answer.is_empty() {
        "idle"
    } else {
        "completed"
    };
    let title = if title.trim().is_empty() {
        truncate(instruction.lines().next().unwrap_or_default(), TITLE_CHARS)
    } else {
        title.to_string()
    };
    SubagentActivity {
        id: session.to_string(),
        kind: "conversation".into(),
        title,
        status: status.into(),
        summary: summary_of(&answer),
        instruction,
        answer,
        started_at: created_at,
        ended_at: (!running)
            .then_some(fold.last_ts)
            .flatten()
            .filter(|ts| *ts > 0),
        tool_calls: fold.tool_calls,
        model: String::new(),
        session_id: Some(session.to_string()),
        trace_path: None,
    }
}

/// The dispatcher prefixes every instruction with a fixed brief; the
/// researcher wants the task, not the framing.
fn strip_parent_brief(text: &str) -> String {
    match text.strip_prefix(PARENT_BRIEF_PREFIX) {
        Some(rest) => rest
            .split_once("\n\n")
            .map(|(_, task)| task)
            .unwrap_or(rest)
            .trim()
            .to_string(),
        None => text.trim().to_string(),
    }
}

/// Explore runs: each `explore` tool call in the transcript, matched with its
/// result when one has arrived. Compacted (tombstoned) results are skipped.
fn explore_activities(messages: &[Message]) -> Vec<SubagentActivity> {
    let mut items = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if message.role != Role::Assistant {
            continue;
        }
        for call in message
            .tool_calls
            .iter()
            .filter(|call| call.function.name == "explore")
        {
            let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or_default();
            let question = args
                .get("question")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let instruction = match args.get("focus").and_then(Value::as_str) {
                Some(focus) if !focus.trim().is_empty() => format!("{question}\n\nFocus: {focus}"),
                _ => question.clone(),
            };
            let result = messages[index + 1..].iter().find(|m| {
                m.role == Role::Tool && m.tool_call_id.as_deref() == Some(call.id.as_str())
            });
            let text = result.map(|m| m.content.as_text()).unwrap_or_default();
            if text.starts_with(wisp_core::context::TOMBSTONE_PREFIX) {
                continue;
            }
            let (status, answer, tool_calls, trace_path) = match result {
                None => ("running", String::new(), 0, None),
                Some(_) => parse_explore_anchor(&text),
            };
            items.push(SubagentActivity {
                id: format!("explore:{}", call.id),
                kind: "explore".into(),
                title: truncate(question.lines().next().unwrap_or_default(), TITLE_CHARS),
                status: status.into(),
                summary: summary_of(&answer),
                instruction,
                answer,
                started_at: message.ts,
                ended_at: result.map(|m| m.ts).filter(|ts| *ts > 0),
                tool_calls,
                model: String::new(),
                session_id: None,
                trace_path,
            });
        }
    }
    items
}

/// Anchor shape (wisp-core `subagent.rs`):
/// `[explore subagent: N tool call(s) — read×3]\n<conclusion>\n[full trace archived at PATH — …]`
/// or `explore subagent failed: … (partial trace archived at PATH)`.
fn parse_explore_anchor(text: &str) -> (&'static str, String, u32, Option<String>) {
    if let Some(rest) = text.strip_prefix("explore subagent failed: ") {
        let trace = rest
            .rsplit_once("(partial trace archived at ")
            .map(|(_, tail)| tail.trim_end_matches(')').trim().to_string());
        let message = rest
            .rsplit_once("(partial trace archived at ")
            .map(|(head, _)| head)
            .unwrap_or(rest)
            .trim();
        return ("failed", format!("Error: {message}"), 0, trace);
    }
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    let tool_calls = header
        .strip_prefix("[explore subagent: ")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let mut trace = None;
    let mut body = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("[full trace archived at ") {
            trace = rest.split(" — ").next().map(|path| path.trim().to_string());
        } else if !line.starts_with("[... anchor truncated") {
            body.push(line);
        }
    }
    let answer = if header.starts_with("[explore subagent: ") {
        body.join("\n").trim().to_string()
    } else {
        text.trim().to_string()
    };
    ("completed", answer, tool_calls, trace)
}

fn expert_activities(messages: &[Message]) -> Vec<SubagentActivity> {
    let mut items = Vec::new();
    for (index, message) in messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::Assistant)
    {
        for call in message
            .tool_calls
            .iter()
            .filter(|call| call.function.name == "consult_expert")
        {
            let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or_default();
            let field = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
            let result = messages[index + 1..].iter().find(|m| {
                m.role == Role::Tool && m.tool_call_id.as_deref() == Some(call.id.as_str())
            });
            let text = result.map(|m| m.content.as_text()).unwrap_or_default();
            if text.starts_with(wisp_core::context::TOMBSTONE_PREFIX) {
                continue;
            }
            let (status, answer) = match result {
                None => ("running", String::new()),
                Some(_) if text.starts_with("[expert subagent: ") => ("completed", text),
                Some(_) => ("failed", text),
            };
            items.push(SubagentActivity {
                id: format!("expert:{}", call.id),
                kind: "expert".into(),
                title: truncate(field("question"), TITLE_CHARS),
                status: status.into(),
                summary: summary_of(&answer),
                instruction: format!(
                    "{}\n\n{}\n\n{}",
                    field("reason"),
                    field("question"),
                    field("context")
                ),
                answer,
                started_at: message.ts,
                ended_at: result.map(|m| m.ts).filter(|ts| *ts > 0),
                tool_calls: 0,
                model: String::new(),
                session_id: None,
                trace_path: None,
            });
        }
    }
    items
}

fn summary_of(answer: &str) -> String {
    truncate(
        answer
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && *line != "findings:")
            .unwrap_or_default()
            .trim_start_matches("- "),
        SUMMARY_CHARS,
    )
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_llm::{FunctionCall, ToolCall};

    fn explore_call(id: &str, question: &str) -> Message {
        let mut message = Message::assistant("");
        message.ts = 100;
        message.tool_calls.push(ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "explore".into(),
                arguments: serde_json::json!({ "question": question }).to_string(),
            },
        });
        message
    }

    #[test]
    fn expert_activity_exposes_reason_model_answer_and_failed_or_pending_calls() {
        let call = |id: &str| {
            let mut message = explore_call(id, "Why do the labels conflict?");
            message.tool_calls[0].function.name = "consult_expert".into();
            message.tool_calls[0].function.arguments = serde_json::json!({
                "question":"Why do the labels conflict?", "reason":"Two attempts failed",
                "context":"The user needs reproducible sample matching."
            })
            .to_string();
            message
        };
        let report = crate::expert_tool::ExpertReport {
            profile_id: "expert".into(),
            model: "expensive-model".into(),
            advice: "Check sample IDs before rerunning.".into(),
            input_tokens: 100,
            output_tokens: 20,
            reasoning_tokens: 0,
        };
        let messages = vec![
            call("done"),
            Message::tool("done", "consult_expert", report.anchor()),
            call("failed"),
            Message::tool(
                "failed",
                "consult_expert",
                "Expert consultation failed: Timed out.",
            ),
            call("pending"),
            call("archived"),
            Message::tool(
                "archived",
                "consult_expert",
                format!("{}archived]", wisp_core::context::TOMBSTONE_PREFIX),
            ),
        ];
        let items = expert_activities(&messages);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, "expert:done");
        assert_eq!(items[0].kind, "expert");
        assert_eq!(items[0].status, "completed");
        assert!(items[0].instruction.contains("Two attempts failed"));
        assert!(items[0].answer.contains("expensive-model"));
        assert!(items[0].answer.contains("Check sample IDs"));
        assert!(items[0].session_id.is_none());
        assert_eq!(items[1].status, "failed");
        assert_eq!(items[2].status, "running");
    }

    #[test]
    fn child_conversation_promotes_attempt_completion_and_strips_the_brief() {
        let messages = vec![
            Message::user(format!(
                "{PARENT_BRIEF_PREFIX}\nYou are its subagent, working in a conversation of your own.\n\nAlign the reads to GRCh38."
            )),
            Message::assistant(""),
            Message::tool("c1", "shell", "ok"),
            Message::tool("c2", "attempt_completion", "Aligned 12 samples.\nBAMs under results/."),
        ];
        let item = conversation_activity(
            "sub",
            "Align the reads",
            5,
            false,
            &ConversationFold::from(&messages[..]),
        );
        assert_eq!(item.status, "completed");
        assert_eq!(item.instruction, "Align the reads to GRCh38.");
        assert_eq!(item.answer, "Aligned 12 samples.\nBAMs under results/.");
        assert_eq!(item.summary, "Aligned 12 samples.");
        assert_eq!(item.tool_calls, 1);
        assert_eq!(item.session_id.as_deref(), Some("sub"));

        let running =
            conversation_activity("sub", "", 5, true, &ConversationFold::from(&messages[..1]));
        assert_eq!(running.status, "running");
        assert_eq!(running.title, "Align the reads to GRCh38.");
        assert!(running.answer.is_empty());
        assert_eq!(
            conversation_activity(
                "sub",
                "t",
                5,
                false,
                &ConversationFold::from(&messages[..1])
            )
            .status,
            "idle"
        );
    }

    #[test]
    fn explore_runs_pair_calls_with_anchors_and_skip_tombstones() {
        let anchor = "[explore subagent: 4 tool call(s) — read×3, grep×1]\nfindings:\n- claim: X | path: a.rs | lines: 1-3\nsummary: the loop lives in a.rs.\n[full trace archived at /p/.wisp/subagents/explore-1.txt — read/grep that file for details; traces are retained for 7 days under .wisp/subagents/]";
        let mut done = Message::tool("e1", "explore", anchor);
        done.ts = 130;
        let messages = vec![
            explore_call("e1", "How does the agent loop end?"),
            done,
            explore_call("e2", "Where is compaction?"),
            Message::tool(
                "e2",
                "explore",
                format!("{}archived]", wisp_core::context::TOMBSTONE_PREFIX),
            ),
            explore_call("e3", "Pending question"),
        ];
        let items = explore_activities(&messages);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "explore:e1");
        assert_eq!(items[0].status, "completed");
        assert_eq!(items[0].tool_calls, 4);
        assert_eq!(items[0].title, "How does the agent loop end?");
        assert_eq!(items[0].summary, "claim: X | path: a.rs | lines: 1-3");
        assert!(items[0].answer.starts_with("findings:"));
        assert!(!items[0].answer.contains("full trace"));
        assert_eq!(
            items[0].trace_path.as_deref(),
            Some("/p/.wisp/subagents/explore-1.txt")
        );
        assert_eq!(items[0].ended_at, Some(130));
        assert_eq!(items[1].id, "explore:e3");
        assert_eq!(items[1].status, "running");
    }

    #[test]
    fn call_log_reads_subagents_like_the_whole_transcript() {
        let mut expert = explore_call("x1", "Which normalisation?");
        expert.tool_calls[0].function.name = "consult_expert".into();
        let mut shell = Message::assistant("");
        shell.tool_calls = explore_call("s1", "unused").tool_calls;
        shell.tool_calls[0].function.name = "shell".into();
        let transcript = vec![
            Message::user("Map the pipeline"),
            // A reused id answered before the explore call must not pair with it.
            Message::tool("e1", "shell", "early"),
            shell,
            Message::tool("s1", "shell", "ok"),
            explore_call("e1", "Where are reads trimmed?"),
            Message::tool(
                "e1",
                "explore",
                "[explore subagent: 2 tool call(s) — read×2]\ntrimmed in qc.py",
            ),
            expert,
            Message::assistant("Waiting on the expert."),
            explore_call("e2", "Still running?"),
        ];
        let mut log = SubagentCallLog::default();
        // Folded in two polls, as the pane would see it.
        transcript[..5].iter().for_each(|message| log.push(message));
        transcript[5..].iter().for_each(|message| log.push(message));
        assert_eq!(log.messages.len(), 4);
        assert_eq!(
            explore_activities(&log.messages),
            explore_activities(&transcript)
        );
        assert_eq!(
            expert_activities(&log.messages),
            expert_activities(&transcript)
        );
        assert_eq!(
            explore_activities(&log.messages)[0].answer,
            "trimmed in qc.py"
        );
    }

    #[tokio::test]
    async fn frame_fold_reads_appended_rows_and_refolds_after_undo() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("wisp.sqlite")).await.unwrap();
        // The fold cache is process-wide; a fresh id keeps tests apart.
        let frame = uuid::Uuid::new_v4().to_string();
        store.create_project("p", "Project", "").await.unwrap();
        store.create_frame(&frame, "p", "Child", "m").await.unwrap();
        let read = |fold: &FrameFold| {
            (
                fold.conversation.instruction.clone(),
                fold.conversation.answer.clone(),
                fold.conversation.tool_calls,
            )
        };
        store
            .append_message(&frame, 0, &Message::user("Count the reads"))
            .await
            .unwrap();
        store
            .append_message(&frame, 1, &Message::tool("c1", "shell", "42"))
            .await
            .unwrap();
        assert_eq!(
            with_frame_fold(&store, &frame, read).await.unwrap(),
            ("Count the reads".into(), None, 1)
        );
        store
            .append_message(&frame, 2, &Message::assistant("There are 42 reads."))
            .await
            .unwrap();
        assert_eq!(
            with_frame_fold(&store, &frame, read).await.unwrap(),
            (
                "Count the reads".into(),
                Some("There are 42 reads.".into()),
                1
            )
        );
        // Undoing the turn's tail must not leave the old answer or count behind.
        store.truncate_model_context(&frame, 0).await.unwrap();
        assert_eq!(
            with_frame_fold(&store, &frame, read).await.unwrap(),
            ("Count the reads".into(), None, 0)
        );
    }

    #[test]
    fn failed_explore_keeps_the_partial_trace() {
        let (status, answer, _, trace) =
            parse_explore_anchor("explore subagent failed: provider timeout (partial trace archived at /p/explore-2.txt)");
        assert_eq!(status, "failed");
        assert_eq!(answer, "Error: provider timeout");
        assert_eq!(trace.as_deref(), Some("/p/explore-2.txt"));
    }
}
