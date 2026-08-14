//! On-demand "what did we actually send to the model" export.
//!
//! Answers two transparency questions users can't otherwise see: how large the
//! built-in system prompt is, and whether an uploaded file's contents were
//! inlined into the request (vs. read by a tool). It serializes the exact
//! provider-agnostic request — the persisted prefix and runtime injections used
//! by the latest model call, plus tool schemas — with a per-section token/char
//! breakdown. The subsequent assistant response is intentionally excluded.
//!
//! Preferred source is the live `Agent` cached in `SessionRuntime` (highest
//! fidelity: tools + provider/model + latest prepared request). When no agent
//! is resident (never run this launch) or a turn holds the lock, it falls back
//! to the persisted messages, which still carry the system prompt (message[0])
//! and any inlined file content.

use super::{terminal_ui_events, AppState};
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
    /// Number of tool definitions included with this exact request. This is
    /// zero for the stored-message fallback because schemas are runtime-only.
    tool_schema_count: usize,
    /// Historical invocations visible in the exported message prefix.
    tool_call_count: usize,
    configured_max_iter: usize,
    /// Exact value applied to the in-flight or most recent turn. `null` after
    /// restart when no resident runtime can prove the historical value.
    effective_max_iter: Option<usize>,
    termination_reason: Option<String>,
    /// Context window the compactor works against. Known for live agents;
    /// for the stored-message fallback it is re-resolved from the active
    /// model profile, which may have changed since the captured turn.
    max_context: Option<usize>,
    /// Session token-estimate calibration factor and compaction count. Only a
    /// live agent can report them; `null` in the stored-message fallback.
    token_estimate_factor: Option<f64>,
    compaction_revision: Option<u64>,
    terminal_event_count: usize,
    /// Persisted Done/Error boundaries. Older sessions may legitimately have
    /// none because builds before this field did not store terminal events.
    terminal_events: Vec<serde_json::Value>,
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
    terminal_events: Vec<serde_json::Value>,
    configured_max_iter: usize,
    effective_max_iter: Option<usize>,
    termination_reason: Option<String>,
    // (max_context, token_estimate_factor, compaction_revision) as the live
    // compactor reports them; the stored fallback can only recover the
    // context window, and only from the currently active profile.
    compactor: (Option<usize>, Option<f64>, Option<u64>),
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
            let est = ContextManager::estimated_tool_schema_tokens(t);
            total += est;
            DebugToolSchema {
                name: t.function.name.clone(),
                description: t.function.description.clone(),
                est_tokens: est,
            }
        })
        .collect();
    let tool_call_count = messages
        .iter()
        .map(|message| message.tool_calls.len())
        .sum();
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
        tool_schema_count: tool_schemas.len(),
        tool_call_count,
        configured_max_iter,
        effective_max_iter,
        termination_reason,
        max_context: compactor.0,
        token_estimate_factor: compactor.1,
        compaction_revision: compactor.2,
        terminal_event_count: terminal_events.len(),
        terminal_events,
        tools: tool_schemas,
        messages: sections,
    }
}

fn latest_termination(events: &[serde_json::Value]) -> (Option<String>, Option<usize>) {
    events
        .iter()
        .rev()
        .find_map(|event| {
            let effective_max_iter = event
                .get("effective_max_iter")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok());
            let reason = match event.get("kind").and_then(serde_json::Value::as_str) {
                Some("Done") => Some(
                    if event.get("stop_reason").and_then(serde_json::Value::as_str)
                        == Some("max_iterations")
                    {
                        "max_iterations"
                    } else {
                        "completed"
                    }
                    .to_string(),
                ),
                Some("Error") => Some("error".to_string()),
                _ => None,
            }?;
            Some((reason, effective_max_iter))
        })
        .map_or((None, None), |(reason, effective)| {
            (Some(reason), effective)
        })
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
    let terminal_events = terminal_ui_events(
        &state
            .store
            .load_session_ui_events(&session_id)
            .await
            .map_err(|e| format!("{e}"))?,
    );
    let turn_running = state.running_turns.lock().await.contains(&session_id);
    let (last_termination_reason, last_effective_max_iter) = latest_termination(&terminal_events);
    let termination_reason = (!turn_running).then_some(last_termination_reason).flatten();
    let configured_max_iter = state
        .store
        .get_setting("max_iter")
        .await
        .ok()
        .flatten()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(super::DEFAULT_MAX_ITER);

    // Prefer the live agent (tools + provider + latest prepared request). Use a
    // non-blocking try_lock so an in-flight turn falls back to persisted
    // messages instead of stalling the export behind a long turn.
    let rt = { state.sessions.lock().await.get(&session_id).cloned() };
    let effective_max_iter = if turn_running {
        rt.as_ref()
            .filter(|runtime| {
                runtime
                    .effective_max_iter_known
                    .load(std::sync::atomic::Ordering::SeqCst)
            })
            .map(|runtime| {
                runtime
                    .effective_max_iter
                    .load(std::sync::atomic::Ordering::SeqCst)
            })
    } else {
        last_effective_max_iter
    };
    let live = rt.as_ref().and_then(|rt| {
        rt.agent.try_lock().ok().and_then(|guard| {
            guard.as_ref().map(|agent| {
                let msgs = agent.ctx.last_request().unwrap_or_else(|| {
                    let mut messages: Vec<Message> = agent.ctx.messages.clone();
                    messages.extend(agent.ctx.runtime_injections.iter().cloned());
                    messages
                });
                let request_tools = if agent.ctx.last_request_tool_schema_count() == Some(0) {
                    Vec::new()
                } else {
                    agent.tools.schemas()
                };
                build_snapshot(
                    &session_id,
                    captured_at.clone(),
                    &msgs,
                    &request_tools,
                    Some(agent.provider.name().to_string()),
                    Some(agent.provider.model().to_string()),
                    "live-agent",
                    terminal_events.clone(),
                    configured_max_iter,
                    effective_max_iter,
                    termination_reason.clone(),
                    (
                        Some(agent.ctx.max_context),
                        Some(agent.ctx.token_estimate_factor()),
                        Some(agent.ctx.compaction_revision()),
                    ),
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
            // The compactor state is gone with the agent, but the context
            // window can still be recovered from the active model profile so
            // exports remain interpretable against the 80% trigger.
            let max_context =
                usize::try_from(crate::models::active_context_window(&state.store).await).ok();
            build_snapshot(
                &session_id,
                captured_at,
                &msgs,
                &[],
                None,
                None,
                "stored-messages",
                terminal_events,
                configured_max_iter,
                effective_max_iter,
                termination_reason,
                (max_context, None, None),
            )
        }
    };

    if snapshot.message_count == 0 {
        return Err("No request to export yet — send a message first.".into());
    }

    let json = serde_json::to_string_pretty(&snapshot).map_err(|e| format!("{e}"))?;
    let default_name = format!(
        "wisp-debug-request-{}.json",
        sanitize_component(&session_id)
    );
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

#[tauri::command]
pub(super) async fn get_context_usage_details(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<wisp_core::ContextUsageDetails, String> {
    let rt = { state.sessions.lock().await.get(&session_id).cloned() };
    if let Some(details) = rt.as_ref().and_then(|rt| {
        rt.agent.try_lock().ok().and_then(|guard| {
            guard.as_ref().map(|agent| {
                let (schemas, origins) = agent.tools.schemas_with_origins();
                agent.ctx.context_usage_details(&schemas, &origins)
            })
        })
    }) {
        return Ok(details);
    }
    let messages = state
        .store
        .load_messages(&session_id)
        .await
        .map_err(|e| format!("{e}"))?;
    let mut context = ContextManager::new(0);
    context.messages = messages;
    Ok(context.context_usage_details(&[], &[]))
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
        let snap = build_snapshot(
            "s1",
            "t".into(),
            &msgs,
            &[],
            None,
            None,
            "stored-messages",
            vec![],
            100,
            Some(100),
            None,
            (None, None, None),
        );

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
        assert_eq!(snap.tool_schema_count, 0);
        assert_eq!(snap.tool_call_count, 0);
        assert_eq!(snap.configured_max_iter, 100);
        assert_eq!(snap.effective_max_iter, Some(100));
        assert_eq!(snap.termination_reason, None);
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["tool_schema_count"], 0);
        assert_eq!(json["tool_call_count"], 0);
        assert_eq!(json["configured_max_iter"], 100);
        assert_eq!(json["effective_max_iter"], 100);
        assert!(json["termination_reason"].is_null());
        assert!(json.get("tool_count").is_none());
        assert!(json["max_context"].is_null());
        assert!(json["token_estimate_factor"].is_null());
        assert!(json["compaction_revision"].is_null());
    }

    #[test]
    fn inlined_file_content_shows_up_in_a_section() {
        // The whole point: if an uploaded file was inlined into the request
        // (rather than read by a tool), it must be visible in the export.
        let msgs = vec![
            Message::system("sys"),
            Message::user("Selected excerpt from workspace file data.xls:\ncol_a,col_b\n1,2\n3,4"),
        ];
        let snap = build_snapshot(
            "s1",
            "t".into(),
            &msgs,
            &[],
            None,
            None,
            "stored-messages",
            vec![],
            100,
            None,
            None,
            (None, None, None),
        );
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
        let snap = build_snapshot(
            "s1",
            "t".into(),
            &msgs,
            &tools,
            None,
            None,
            "live-agent",
            vec![],
            100,
            Some(100),
            None,
            (Some(128_000), Some(1.2), Some(3)),
        );
        assert_eq!(snap.tool_schema_count, 1);
        assert!(snap.tools[0].est_tokens > 0);
        let msg_sum: usize = snap.messages.iter().map(|m| m.est_tokens).sum();
        assert_eq!(snap.total_est_tokens, msg_sum + snap.tools[0].est_tokens);
        assert_eq!(snap.max_context, Some(128_000));
        assert_eq!(snap.token_estimate_factor, Some(1.2));
        assert_eq!(snap.compaction_revision, Some(3));
    }

    #[test]
    fn terminal_errors_are_included_in_debug_snapshot() {
        let terminal = serde_json::json!({
            "kind": "Error",
            "frame_id": "s1",
            "message": "api: 524 gateway timeout"
        });
        let snap = build_snapshot(
            "s1",
            "t".into(),
            &[Message::user("hi")],
            &[],
            None,
            None,
            "stored-messages",
            vec![terminal.clone()],
            100,
            Some(20),
            Some("error".into()),
            (None, None, None),
        );
        assert_eq!(snap.terminal_event_count, 1);
        assert_eq!(snap.terminal_events, vec![terminal]);
        assert_eq!(snap.termination_reason.as_deref(), Some("error"));
        assert_eq!(snap.configured_max_iter, 100);
        assert_eq!(snap.effective_max_iter, Some(20));
    }

    #[test]
    fn snapshot_separates_tool_calls_from_runtime_schemas() {
        let mut assistant = Message::assistant("");
        assistant.tool_calls.push(wisp_llm::ToolCall {
            id: "call-1".into(),
            kind: "function".into(),
            function: wisp_llm::FunctionCall {
                name: "read".into(),
                arguments: "{}".into(),
            },
        });
        let terminal = serde_json::json!({
            "kind": "Done",
            "frame_id": "s1",
            "stop_reason": "max_iterations"
        });
        let snap = build_snapshot(
            "s1",
            "t".into(),
            &[Message::user("hi"), assistant],
            &[],
            None,
            None,
            "stored-messages",
            vec![terminal],
            100,
            Some(20),
            Some("max_iterations".into()),
            (None, None, None),
        );

        assert_eq!(snap.tool_schema_count, 0);
        assert_eq!(snap.tool_call_count, 1);
        assert_eq!(snap.termination_reason.as_deref(), Some("max_iterations"));
    }

    #[test]
    fn latest_termination_keeps_effective_limit_with_new_events() {
        let events = vec![
            serde_json::json!({
                "kind": "Done",
                "frame_id": "s1",
                "effective_max_iter": 20
            }),
            serde_json::json!({
                "kind": "Error",
                "frame_id": "s1",
                "message": "summary failed",
                "effective_max_iter": 7
            }),
        ];

        assert_eq!(latest_termination(&events), (Some("error".into()), Some(7)));
    }

    #[test]
    fn latest_termination_leaves_legacy_effective_limit_unknown() {
        let events = vec![serde_json::json!({
            "kind": "Done",
            "frame_id": "s1",
            "stop_reason": "max_iterations"
        })];

        assert_eq!(
            latest_termination(&events),
            (Some("max_iterations".into()), None)
        );
    }

    #[test]
    fn latest_termination_normalizes_non_limit_done_reasons() {
        let events = vec![serde_json::json!({
            "kind": "Done",
            "frame_id": "s1",
            "stop_reason": "end_turn"
        })];

        assert_eq!(
            latest_termination(&events),
            (Some("completed".into()), None)
        );
    }
}
