//! Import OpenAI Codex CLI conversations (~/.codex/sessions rollout jsonl)
//! into Wisp sessions, so work started in Codex continues here without
//! copy/paste. Parsing mirrors the rollout envelope: a `session_meta` line
//! plus `response_item` lines whose payload is a user/assistant `message`.
//! Re-imports are idempotent via the `codex_imports` table keyed by the
//! Codex thread id.

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;
use wisp_llm::{Content, Message, Role};
use wisp_store::Store;

use super::AppState;

fn codex_sessions_root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex").join("sessions"))
}

#[derive(Debug, Default)]
struct ParsedCodexSession {
    session_id: String,
    cwd: String,
    created_at_ms: i64,
    last_active_at_ms: i64,
    messages: Vec<ParsedCodexMessage>,
}

#[derive(Debug)]
struct ParsedCodexMessage {
    role: Role,
    text: String,
    ts_ms: i64,
}

/// Codex prepends AGENTS.md and environment wrappers as synthetic user turns;
/// they are context plumbing, not conversation.
fn is_noise_user_text(text: &str) -> bool {
    text.contains("<environment_context>")
        || text.contains("AGENTS.md instructions")
        || text.contains("<user_instructions>")
}

fn timestamp_ms(value: Option<&serde_json::Value>) -> i64 {
    value
        .and_then(serde_json::Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

/// All `text` fields of a message content value, joined. Codex writes either a
/// plain string or a list of `{type: input_text|output_text, text}` parts.
fn content_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_codex_jsonl(jsonl: &str) -> ParsedCodexSession {
    let mut session = ParsedCodexSession::default();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let ts_ms = timestamp_ms(value.get("timestamp"));
        let kind = value.get("type").and_then(serde_json::Value::as_str);
        // Older Codex versions wrote fields at the top level; current ones
        // wrap them in `payload`. Fall back to the envelope itself.
        let payload = value
            .get("payload")
            .filter(|p| p.is_object())
            .unwrap_or(&value);
        match kind {
            Some("session_meta") => {
                if let Some(id) = payload.get("id").and_then(serde_json::Value::as_str) {
                    session.session_id = id.to_string();
                }
                if let Some(cwd) = payload.get("cwd").and_then(serde_json::Value::as_str) {
                    session.cwd = cwd.to_string();
                }
            }
            Some("response_item") => {
                if value.get("payload").is_some()
                    && payload.get("type").and_then(serde_json::Value::as_str) != Some("message")
                {
                    continue;
                }
                let role = match payload.get("role").and_then(serde_json::Value::as_str) {
                    Some("user") => Role::User,
                    Some("assistant") => Role::Assistant,
                    _ => continue,
                };
                let Some(content) = payload.get("content") else {
                    continue;
                };
                let text = content_text(content);
                if text.trim().is_empty() || (role == Role::User && is_noise_user_text(&text)) {
                    continue;
                }
                session
                    .messages
                    .push(ParsedCodexMessage { role, text, ts_ms });
            }
            _ => continue,
        }
        if ts_ms > 0 {
            if session.created_at_ms == 0 {
                session.created_at_ms = ts_ms;
            }
            session.last_active_at_ms = session.last_active_at_ms.max(ts_ms);
        }
    }
    session
}

fn scan_rollout_files(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
        })
        .map(|entry| entry.into_path())
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(super) struct CodexSessionInfo {
    pub path: String,
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub message_count: usize,
    /// Unix seconds of the last Codex activity.
    pub last_active_at: i64,
    /// "new" (never imported), "imported" (up to date), or "updatable"
    /// (the Codex side has messages the imported frame does not).
    pub state: String,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub(super) struct CodexImportSummary {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
    pub failed: usize,
}

async fn list_codex_sessions_in(store: &Store, root: &Path) -> Vec<CodexSessionInfo> {
    let mut out = vec![];
    for path in scan_rollout_files(root) {
        let Ok(bytes) = std::fs::read_to_string(&path) else {
            continue;
        };
        let parsed = parse_codex_jsonl(&bytes);
        if parsed.session_id.is_empty() || parsed.messages.is_empty() {
            continue;
        }
        let state = match store
            .find_codex_import(&parsed.session_id)
            .await
            .ok()
            .flatten()
        {
            Some(frame_id) => {
                let stored = store.message_count(&frame_id).await.unwrap_or(0);
                if (parsed.messages.len() as i64) > stored {
                    "updatable"
                } else {
                    "imported"
                }
            }
            None => "new",
        };
        let title = parsed
            .messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| m.text.trim().chars().take(120).collect::<String>())
            .unwrap_or_default();
        out.push(CodexSessionInfo {
            path: path.display().to_string(),
            session_id: parsed.session_id,
            title,
            cwd: parsed.cwd,
            message_count: parsed.messages.len(),
            last_active_at: parsed.last_active_at_ms / 1000,
            state: state.to_string(),
        });
    }
    out.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    out
}

fn to_wisp_messages(parsed: &ParsedCodexSession) -> Vec<Message> {
    parsed
        .messages
        .iter()
        .map(|m| Message {
            role: m.role,
            content: Content::text(m.text.clone()),
            tool_calls: vec![],
            tool_call_id: None,
            tool_name: None,
            reasoning: None,
            ts: m.ts_ms / 1000,
            model_name: (m.role == Role::Assistant).then(|| "Codex CLI".to_string()),
        })
        .collect()
}

/// Import one rollout file into `project_id`. Returns what happened so batch
/// callers can tally a summary.
async fn import_codex_file(
    store: &Store,
    project_id: &str,
    model_id: &str,
    path: &Path,
) -> Result<&'static str, String> {
    let bytes = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let parsed = parse_codex_jsonl(&bytes);
    if parsed.session_id.is_empty() || parsed.messages.is_empty() {
        return Err(format!("{}: no importable messages", path.display()));
    }
    let now = chrono::Utc::now().timestamp();
    let created_at = if parsed.created_at_ms > 0 {
        parsed.created_at_ms / 1000
    } else {
        now
    };
    let updated_at = if parsed.last_active_at_ms > 0 {
        parsed.last_active_at_ms / 1000
    } else {
        now
    };

    if let Some(frame_id) = store
        .find_codex_import(&parsed.session_id)
        .await
        .map_err(|e| e.to_string())?
    {
        let stored = store
            .message_count(&frame_id)
            .await
            .map_err(|e| e.to_string())?;
        // ponytail: only fast-forward. If the frame was continued inside Wisp
        // it can hold more turns than the rollout; merging diverged histories
        // is out of scope, so leave it untouched.
        if (parsed.messages.len() as i64) <= stored {
            return Ok("skipped");
        }
        store
            .replace_messages(&frame_id, &to_wisp_messages(&parsed))
            .await
            .map_err(|e| e.to_string())?;
        store
            .set_frame_timestamps(&frame_id, created_at, updated_at)
            .await
            .map_err(|e| e.to_string())?;
        return Ok("updated");
    }

    let frame_id = uuid::Uuid::new_v4().to_string();
    store
        .create_frame(&frame_id, project_id, "Codex", model_id)
        .await
        .map_err(|e| e.to_string())?;
    for (i, msg) in to_wisp_messages(&parsed).iter().enumerate() {
        store
            .append_message(&frame_id, (i + 1) as i64, msg)
            .await
            .map_err(|e| e.to_string())?;
    }
    store
        .set_frame_timestamps(&frame_id, created_at, updated_at)
        .await
        .map_err(|e| e.to_string())?;
    store
        .record_codex_import(&parsed.session_id, &frame_id, &path.display().to_string())
        .await
        .map_err(|e| e.to_string())?;
    Ok("imported")
}

#[tauri::command]
pub(super) async fn list_codex_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<CodexSessionInfo>, String> {
    let Some(root) = codex_sessions_root() else {
        return Ok(vec![]);
    };
    if !root.is_dir() {
        return Ok(vec![]);
    }
    Ok(list_codex_sessions_in(&state.store, &root).await)
}

#[tauri::command]
pub(super) async fn import_codex_sessions(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    paths: Vec<String>,
) -> Result<CodexImportSummary, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let model_id = super::models::active_profile_id(&state.store).await;
    let mut summary = CodexImportSummary::default();
    for path in &paths {
        match import_codex_file(&state.store, &ap.id, &model_id, Path::new(path)).await {
            Ok("imported") => summary.imported += 1,
            Ok("updated") => summary.updated += 1,
            Ok(_) => summary.skipped += 1,
            Err(error) => {
                tracing::warn!("codex import failed: {error}");
                summary.failed += 1;
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX_JSONL: &str = concat!(
        r#"{"type":"session_meta","timestamp":"2026-05-31T10:00:00Z","payload":{"id":"codex-abc","cwd":"/home/me/project"}}"#,
        "\n",
        r#"{"type":"response_item","timestamp":"2026-05-31T10:00:30Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}"#,
        "\n",
        r##"{"type":"response_item","timestamp":"2026-05-31T10:00:31Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /p"}]}}"##,
        "\n",
        r#"{"type":"response_item","timestamp":"2026-05-31T10:01:00Z","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"system prompt"}]}}"#,
        "\n",
        r#"{"type":"response_item","timestamp":"2026-05-31T10:01:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the renderer crash"}]}}"#,
        "\n",
        r#"{"type":"response_item","timestamp":"2026-05-31T10:02:00.500Z","payload":{"type":"reasoning","summary":[]}}"#,
        "\n",
        r#"not json"#,
        "\n",
        r#"{"type":"response_item","timestamp":"2026-05-31T18:03:00+08:00","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"I found "},{"type":"output_text","text":"the issue."}]}}"#,
        "\n",
    );

    #[test]
    fn parses_envelope_metadata_messages_and_noise() {
        let parsed = parse_codex_jsonl(CODEX_JSONL);
        assert_eq!(parsed.session_id, "codex-abc");
        assert_eq!(parsed.cwd, "/home/me/project");
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[0].text, "Fix the renderer crash");
        assert_eq!(parsed.messages[1].role, Role::Assistant);
        assert_eq!(parsed.messages[1].text, "I found \nthe issue.");
        assert_eq!(parsed.created_at_ms, 1780221600000); // 2026-05-31T10:00:00Z
                                                         // +08:00 offset normalizes to 10:03:00Z.
        assert_eq!(parsed.last_active_at_ms, 1780221780000);
    }

    #[test]
    fn parses_legacy_top_level_lines() {
        let jsonl = concat!(
            r#"{"type":"session_meta","id":"legacy-1","cwd":"/w","timestamp":"2026-05-31T10:00:00Z"}"#,
            "\n",
            r#"{"type":"response_item","role":"user","content":[{"type":"input_text","text":"hello"}],"timestamp":"2026-05-31T10:01:00Z"}"#,
            "\n",
        );
        let parsed = parse_codex_jsonl(jsonl);
        assert_eq!(parsed.session_id, "legacy-1");
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].text, "hello");
    }

    async fn temp_store() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_codex_import_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "/w").await.unwrap();
        (store, path)
    }

    #[tokio::test]
    async fn import_creates_updates_and_skips() {
        let (store, db_path) = temp_store().await;
        let dir = std::env::temp_dir().join(format!("codex_sessions_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("2026/05/31")).unwrap();
        let rollout = dir.join("2026/05/31/rollout-2026-05-31T10-00-00-codex-abc.jsonl");
        std::fs::write(&rollout, CODEX_JSONL).unwrap();

        assert_eq!(
            import_codex_file(&store, "p", "m", &rollout).await.unwrap(),
            "imported"
        );
        let frame_id = store.find_codex_import("codex-abc").await.unwrap().unwrap();
        assert_eq!(store.message_count(&frame_id).await.unwrap(), 2);
        let sessions = store.list_sessions("p").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].1, "Fix the renderer crash");
        assert_eq!(sessions[0].2, 1780221600); // keeps Codex chronology

        // Unchanged rollout → idempotent skip.
        assert_eq!(
            import_codex_file(&store, "p", "m", &rollout).await.unwrap(),
            "skipped"
        );

        // Codex side grew → fast-forward the frame.
        let grown = format!(
            "{CODEX_JSONL}{}\n",
            r#"{"type":"response_item","timestamp":"2026-05-31T10:10:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"thanks"}]}}"#
        );
        std::fs::write(&rollout, &grown).unwrap();
        assert_eq!(
            import_codex_file(&store, "p", "m", &rollout).await.unwrap(),
            "updated"
        );
        assert_eq!(store.message_count(&frame_id).await.unwrap(), 3);

        // Frame continued inside Wisp beyond the rollout → left untouched.
        for seq in 4..=6 {
            store
                .append_message(&frame_id, seq, &wisp_llm::Message::user("wisp-side"))
                .await
                .unwrap();
        }
        assert_eq!(
            import_codex_file(&store, "p", "m", &rollout).await.unwrap(),
            "skipped"
        );
        assert_eq!(store.message_count(&frame_id).await.unwrap(), 6);

        let listed = list_codex_sessions_in(&store, &dir).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "codex-abc");
        assert_eq!(listed[0].state, "imported");
        assert_eq!(listed[0].title, "Fix the renderer crash");

        drop(store);
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_dir_all(dir);
    }
}
