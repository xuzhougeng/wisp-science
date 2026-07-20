//! On-demand "what did we actually send to the model" export.
//!
//! Answers two transparency questions users can't otherwise see: how large the
//! built-in system prompt is, and whether an uploaded file's contents were
//! inlined into the request (vs. read by a tool). It serializes the exact
//! provider-agnostic request — system prompt + full message history + runtime
//! injections + tool schemas — with a per-section token/char breakdown.
//!
//! Preferred source is the live `Agent` cached in `SessionRuntime` (highest
//! fidelity: tools + provider/model + post-compaction context). When no agent
//! is resident (never run this launch) or a turn holds the lock, it falls back
//! to the persisted messages, which still carry the system prompt (message[0])
//! and any inlined file content.

use super::AppState;
use serde::Serialize;
use tauri::{AppHandle, State};
use wisp_core::ContextManager;
use wisp_llm::{Message, Role, ToolSchema};

#[derive(Serialize)]
struct DebugToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct DebugSection {
    index: usize,
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<DebugToolCall>,
    chars: usize,
    est_tokens: usize,
}

#[derive(Serialize)]
struct DebugToolSchema {
    name: String,
    description: String,
    est_tokens: usize,
}

#[derive(Serialize)]
struct DebugRequestSnapshot {
    session_id: String,
    captured_at: String,
    /// "live-agent" (full fidelity) or "stored-messages" (fallback; no tools).
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    system_prompt_chars: usize,
    system_prompt_est_tokens: usize,
    total_est_tokens: usize,
    message_count: usize,
    tool_count: usize,
    tools: Vec<DebugToolSchema>,
    messages: Vec<DebugSection>,
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// Pure builder: turn the request inputs into a serializable snapshot. Token
/// counts reuse the same estimator the context compactor uses, so the numbers
/// match what the app reports elsewhere.
fn build_snapshot(
    session_id: &str,
    captured_at: String,
    messages: &[Message],
    tools: &[ToolSchema],
    provider: Option<String>,
    model: Option<String>,
    source: &'static str,
) -> DebugRequestSnapshot {
    let mut sections = Vec::with_capacity(messages.len());
    let mut total = 0usize;
    let mut sys_chars = 0usize;
    let mut sys_tokens = 0usize;
    for (i, m) in messages.iter().enumerate() {
        let text = m.content.as_text();
        let chars = text.chars().count();
        let est = ContextManager::estimated_tokens(m);
        total += est;
        if m.role == Role::System {
            sys_chars += chars;
            sys_tokens += est;
        }
        sections.push(DebugSection {
            index: i,
            role: role_str(m.role),
            tool_name: m.tool_name.clone(),
            tool_call_id: m.tool_call_id.clone(),
            tool_calls: m
                .tool_calls
                .iter()
                .map(|tc| DebugToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                })
                .collect(),
            text,
            chars,
            est_tokens: est,
        });
    }
    let tool_schemas: Vec<DebugToolSchema> = tools
        .iter()
        .map(|t| {
            let params = t.function.parameters.to_string();
            // ponytail: 4-chars-per-token heuristic, same ballpark as the
            // message estimator; exact tool tokenization isn't worth it here.
            let est = (t.function.name.len() + t.function.description.len() + params.len()) / 4 + 2;
            total += est;
            DebugToolSchema {
                name: t.function.name.clone(),
                description: t.function.description.clone(),
                est_tokens: est,
            }
        })
        .collect();
    DebugRequestSnapshot {
        session_id: session_id.to_string(),
        captured_at,
        source,
        provider,
        model,
        system_prompt_chars: sys_chars,
        system_prompt_est_tokens: sys_tokens,
        total_est_tokens: total,
        message_count: messages.len(),
        tool_count: tool_schemas.len(),
        tools: tool_schemas,
        messages: sections,
    }
}

fn sanitize_component(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[tauri::command]
pub(super) async fn export_debug_request(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let captured_at = chrono::Utc::now().to_rfc3339();

    // Prefer the live agent (tools + provider + post-compaction context). Use a
    // non-blocking try_lock so an in-flight turn falls back to persisted
    // messages instead of stalling the export behind a long turn.
    let rt = { state.sessions.lock().await.get(&session_id).cloned() };
    let live = rt.as_ref().and_then(|rt| {
        rt.agent.try_lock().ok().and_then(|guard| {
            guard.as_ref().map(|agent| {
                let mut msgs: Vec<Message> = agent.ctx.messages.clone();
                msgs.extend(agent.ctx.runtime_injections.iter().cloned());
                build_snapshot(
                    &session_id,
                    captured_at.clone(),
                    &msgs,
                    &agent.tools.schemas(),
                    Some(agent.provider.name().to_string()),
                    Some(agent.provider.model().to_string()),
                    "live-agent",
                )
            })
        })
    });

    let snapshot = match live {
        Some(s) => s,
        None => {
            let msgs = state
                .store
                .load_messages(&session_id)
                .await
                .map_err(|e| format!("{e}"))?;
            build_snapshot(
                &session_id,
                captured_at,
                &msgs,
                &[],
                None,
                None,
                "stored-messages",
            )
        }
    };

    if snapshot.message_count == 0 {
        return Err("No request to export yet — send a message first.".into());
    }

    let json = serde_json::to_string_pretty(&snapshot).map_err(|e| format!("{e}"))?;
    let default_name = format!("wisp-debug-request-{}.json", sanitize_component(&session_id));
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(dest) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None);
    };
    let dest_path = std::path::PathBuf::from(dest.to_string());
    std::fs::write(&dest_path, json).map_err(|e| format!("{e}"))?;
    Ok(Some(dest_path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_breaks_out_system_prompt_and_sums_tokens() {
        let msgs = vec![
            Message::system("You are wisp-science. ".repeat(50)),
            Message::user("analyze the uploaded sheet"),
            Message::assistant("on it"),
        ];
        let snap = build_snapshot("s1", "t".into(), &msgs, &[], None, None, "stored-messages");

        assert_eq!(snap.message_count, 3);
        assert_eq!(snap.messages[0].role, "system");
        assert!(snap.system_prompt_est_tokens > 0, "system prompt sized");
        assert_eq!(
            snap.system_prompt_chars,
            msgs[0].content.as_text().chars().count()
        );
        // With no tools, the total is exactly the sum of per-section estimates.
        let sum: usize = snap.messages.iter().map(|m| m.est_tokens).sum();
        assert_eq!(snap.total_est_tokens, sum);
        assert_eq!(snap.tool_count, 0);
    }

    #[test]
    fn inlined_file_content_shows_up_in_a_section() {
        // The whole point: if an uploaded file was inlined into the request
        // (rather than read by a tool), it must be visible in the export.
        let msgs = vec![
            Message::system("sys"),
            Message::user(
                "Selected excerpt from workspace file data.xls:\ncol_a,col_b\n1,2\n3,4",
            ),
        ];
        let snap = build_snapshot("s1", "t".into(), &msgs, &[], None, None, "stored-messages");
        assert!(snap.messages[1].text.contains("data.xls"));
        assert!(snap.messages[1].text.contains("col_a,col_b"));
    }

    #[test]
    fn tool_schemas_count_toward_the_total() {
        let msgs = vec![Message::user("hi")];
        let tools = vec![ToolSchema::new(
            "read",
            "Read a file from disk",
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        )];
        let snap = build_snapshot("s1", "t".into(), &msgs, &tools, None, None, "live-agent");
        assert_eq!(snap.tool_count, 1);
        assert!(snap.tools[0].est_tokens > 0);
        let msg_sum: usize = snap.messages.iter().map(|m| m.est_tokens).sum();
        assert_eq!(snap.total_est_tokens, msg_sum + snap.tools[0].est_tokens);
    }
}
