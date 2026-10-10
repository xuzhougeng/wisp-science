//! The Agents pane's subagent feed: every subagent the current conversation
//! started, whichever mechanism ran it. Watch-only child conversations
//! (`dispatch_subagent`, #1061) come from the frames table; `explore` runs
//! are in-process, so their only trace here is the anchor in the parent's
//! transcript. `delegate_tasks` workflows keep their own snapshot command.

use crate::AppState;
use serde_json::Value;
use tauri::State;
use wisp_dto::SubagentActivity;
use wisp_llm::{Message, Role};

const SUMMARY_CHARS: usize = 160;
const TITLE_CHARS: usize = 80;
const PARENT_BRIEF_PREFIX: &str = "[From the parent conversation's agent]";

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
    for (child, parent) in dispatched {
        if parent != root && parent != session_id {
            continue;
        }
        let reference = store.get_session_reference(&child).await.ok().flatten();
        let messages = store.load_messages(&child).await.unwrap_or_default();
        items.push(conversation_activity(
            &child,
            reference.as_ref().map(|r| r.title.as_str()).unwrap_or(""),
            reference.as_ref().map(|r| r.created_at).unwrap_or(0),
            running.contains(&child),
            &messages,
        ));
    }
    let messages = store
        .load_messages(&session_id)
        .await
        .map_err(|error| error.to_string())?;
    items.extend(explore_activities(&messages));
    items.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(items)
}

/// The turn's real answer: an `attempt_completion` result counts, the last
/// assistant bubble alone does not (it is often empty when a tool ended the turn).
pub(crate) fn final_answer(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|m| {
            m.role == Role::Assistant
                || (m.role == Role::Tool && m.tool_name.as_deref() == Some("attempt_completion"))
        })
        .map(|m| m.content.as_text())
        .find(|t| !t.trim().is_empty())
}

fn conversation_activity(
    session: &str,
    title: &str,
    created_at: i64,
    running: bool,
    messages: &[Message],
) -> SubagentActivity {
    let instruction = messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| strip_parent_brief(&m.content.as_text()))
        .unwrap_or_default();
    let answer = final_answer(messages).unwrap_or_default();
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
            .then(|| messages.last().map(|m| m.ts).filter(|ts| *ts > 0))
            .flatten(),
        tool_calls: messages
            .iter()
            .filter(|m| {
                m.role == Role::Tool && m.tool_name.as_deref() != Some("attempt_completion")
            })
            .count() as u32,
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
    fn child_conversation_promotes_attempt_completion_and_strips_the_brief() {
        let messages = vec![
            Message::user(format!(
                "{PARENT_BRIEF_PREFIX}\nYou are its subagent, working in a conversation of your own.\n\nAlign the reads to GRCh38."
            )),
            Message::assistant(""),
            Message::tool("c1", "shell", "ok"),
            Message::tool("c2", "attempt_completion", "Aligned 12 samples.\nBAMs under results/."),
        ];
        let item = conversation_activity("sub", "Align the reads", 5, false, &messages);
        assert_eq!(item.status, "completed");
        assert_eq!(item.instruction, "Align the reads to GRCh38.");
        assert_eq!(item.answer, "Aligned 12 samples.\nBAMs under results/.");
        assert_eq!(item.summary, "Aligned 12 samples.");
        assert_eq!(item.tool_calls, 1);
        assert_eq!(item.session_id.as_deref(), Some("sub"));

        let running = conversation_activity("sub", "", 5, true, &messages[..1]);
        assert_eq!(running.status, "running");
        assert_eq!(running.title, "Align the reads to GRCh38.");
        assert!(running.answer.is_empty());
        assert_eq!(
            conversation_activity("sub", "t", 5, false, &messages[..1]).status,
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
    fn failed_explore_keeps_the_partial_trace() {
        let (status, answer, _, trace) =
            parse_explore_anchor("explore subagent failed: provider timeout (partial trace archived at /p/explore-2.txt)");
        assert_eq!(status, "failed");
        assert_eq!(answer, "Error: provider timeout");
        assert_eq!(trace.as_deref(), Some("/p/explore-2.txt"));
    }
}
