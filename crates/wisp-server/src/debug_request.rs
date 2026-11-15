//! What a conversation sends the model, saved as JSON: a port of the
//! WebView's `src-tauri/src/debug_request.rs` export. It answers how large
//! the built-in instructions are and whether a file's contents were put
//! into the request rather than read by a tool, with a per-message size
//! breakdown. This host keeps no agent between turns, so the export is the
//! WebView's stored-messages form: the saved working set as the last turn
//! left it, without the tool definitions.

use anyhow::{Context as _, Result};
use serde::Serialize;
use wisp_core::ContextManager;
use wisp_llm::{Message, Role, ToolSchema};

use crate::Server;

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

fn sanitize_component(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

impl Server {
    /// The conversation's request as JSON and the file name to save it as.
    pub async fn debug_request(&self, project: &str, session: &str) -> Result<(String, String)> {
        self.own_conversation(project, session).await?;
        let store = &self.0.store;
        let messages = store.load_messages(session).await?;
        anyhow::ensure!(
            !messages.is_empty(),
            "No request to export yet — send a message first."
        );
        let live = self.session(session);
        let (running, stop_reason) = {
            let live = live.live.lock().unwrap();
            (live.running, live.stop_reason.clone())
        };
        // How the last turn ended, unless one is running now.
        let termination_reason = match running {
            true => None,
            false => store.latest_server_turn(session).await?.and_then(|turn| {
                match turn.status.as_str() {
                    "completed" if stop_reason.as_deref() == Some("max_iterations") => {
                        Some("max_iterations".to_owned())
                    }
                    "completed" => Some("completed".to_owned()),
                    "failed" | "interrupted" => Some("error".to_owned()),
                    _ => None,
                }
            }),
        };
        let options = self.stored_options(session).await?;
        let model = self.0.model.read().unwrap().pick(&options.model).ok();
        let configured_max_iter = crate::runtime::turn_settings(&self.0.config, store)
            .await
            .max_iterations;
        let snapshot = build_snapshot(
            session,
            chrono::Utc::now().to_rfc3339(),
            &messages,
            &[],
            model
                .as_ref()
                .map(|model| format!("{:?}", model.provider.kind).to_lowercase()),
            model.as_ref().map(|model| model.provider.model.clone()),
            "stored-messages",
            Vec::new(),
            configured_max_iter,
            None,
            termination_reason,
            (model.as_ref().map(|model| model.max_context), None, None),
        );
        let json =
            serde_json::to_string_pretty(&snapshot).context("Could not write the request")?;
        Ok((
            format!("wisp-debug-request-{}.json", sanitize_component(session)),
            json,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_export_filename_is_one_component_for_both_frontends() {
        assert_eq!(sanitize_component("source-1"), "source_1");
        assert_eq!(
            sanitize_component("../folder\\session:研究"),
            "___folder_session___"
        );
    }

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
}
