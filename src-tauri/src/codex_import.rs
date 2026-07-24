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
use wisp_store::{ExecutionContext, ExecutionContextKind, Store};

use super::AppState;

const CONTEXT_SCAN_PROTOCOL: &str = "WISP_CODEX_SCAN_V1\0";
const CONTEXT_FILE_PROTOCOL: &str = "WISP_CODEX_FILE_V1\0";
const CONTEXT_ROLLOUT_MAX_BYTES: u64 = 32 * 1024 * 1024;
const CONTEXT_SCAN_MAX_BYTES: u64 = 128 * 1024 * 1024;

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

#[derive(Debug)]
struct CodexSessionCandidate {
    path: String,
    parsed: ParsedCodexSession,
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

fn local_codex_candidates(root: &Path) -> Vec<CodexSessionCandidate> {
    scan_rollout_files(root)
        .into_iter()
        .filter_map(|path| {
            let jsonl = std::fs::read_to_string(&path).ok()?;
            let parsed = parse_codex_jsonl(&jsonl);
            (!parsed.session_id.is_empty() && !parsed.messages.is_empty()).then(|| {
                CodexSessionCandidate {
                    path: path.display().to_string(),
                    parsed,
                }
            })
        })
        .collect()
}

fn context_scan_script() -> String {
    format!(
        r#"LC_ALL=C
root=$HOME/.codex/sessions
printf 'WISP_CODEX_SCAN_V1\000'
if [ ! -d "$root" ]; then
  printf '\000'
  exit 0
fi
total=0
find "$root" -type f -name 'rollout-*.jsonl' -print 2>/dev/null |
while IFS= read -r file; do
  size=$(wc -c < "$file" 2>/dev/null) || continue
  case "$size" in ''|*[!0-9]*) continue ;; esac
  if [ "$size" -gt {CONTEXT_ROLLOUT_MAX_BYTES} ]; then continue; fi
  total=$((total + size))
  if [ "$total" -gt {CONTEXT_SCAN_MAX_BYTES} ]; then break; fi
  printf '%s\000%s\000' "$file" "$size"
  head -c "$size" "$file" || exit 68
done
printf '\000'"#
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn validate_context_rollout_path(path: &str) -> Result<(), String> {
    if path.len() > 4096 || path.contains(['\0', '\n', '\r']) {
        return Err("Invalid Codex rollout path".into());
    }
    if path.split('/').any(|part| part == "..") || !path.contains("/.codex/sessions/") {
        return Err("Codex rollout is outside ~/.codex/sessions".into());
    }
    let name = path.rsplit('/').next().unwrap_or_default();
    if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
        return Err("Invalid Codex rollout filename".into());
    }
    Ok(())
}

fn context_file_script(path: &str) -> Result<String, String> {
    validate_context_rollout_path(path)?;
    let path = shell_single_quote(path);
    Ok(format!(
        r#"LC_ALL=C
root=$HOME/.codex/sessions
file={path}
case "$file" in "$root"/*) ;; *)
  printf 'Codex rollout is outside ~/.codex/sessions\n' >&2
  exit 66
esac
if [ ! -f "$file" ] || [ -L "$file" ]; then
  printf 'Cannot read Codex rollout: %s\n' "$file" >&2
  exit 66
fi
size=$(wc -c < "$file" 2>/dev/null) || exit 66
if [ "$size" -gt {CONTEXT_ROLLOUT_MAX_BYTES} ]; then
  printf 'Codex rollout exceeds {CONTEXT_ROLLOUT_MAX_BYTES} byte limit\n' >&2
  exit 67
fi
printf 'WISP_CODEX_FILE_V1\000'
cat "$file""#
    ))
}

fn run_context_script(
    context: &ExecutionContext,
    script: &str,
    runner: &mut dyn crate::context_probe::ProbeRunner,
) -> Result<String, String> {
    crate::ssh_hosts::require_managed_ssh_ready(context)?;
    let command = crate::context_probe::build_probe_command(context, script)?;
    let output = runner.run(&command)?;
    if output.status != 0 {
        let detail = if output.stderr.trim().is_empty() {
            "no error details returned"
        } else {
            output.stderr.trim()
        };
        return Err(format!(
            "Codex scan failed in {} (exit {}): {detail}",
            context.label, output.status
        ));
    }
    Ok(output.stdout)
}

fn protocol_payload<'a>(stdout: &'a str, marker: &str) -> Result<&'a [u8], String> {
    let stdout = stdout.as_bytes();
    let marker = marker.as_bytes();
    stdout
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|start| &stdout[start + marker.len()..])
        .ok_or_else(|| "Codex source returned an invalid response".into())
}

fn take_nul_field<'a>(payload: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], String> {
    let rest = payload
        .get(*cursor..)
        .ok_or_else(|| "Codex source returned an incomplete response".to_string())?;
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| "Codex source returned an incomplete response".to_string())?;
    *cursor += end + 1;
    Ok(&rest[..end])
}

fn parse_context_scan(stdout: &str) -> Result<Vec<CodexSessionCandidate>, String> {
    let payload = protocol_payload(stdout, CONTEXT_SCAN_PROTOCOL)?;
    let mut cursor = 0;
    let mut candidates = Vec::new();
    loop {
        let path = take_nul_field(payload, &mut cursor)?;
        if path.is_empty() {
            break;
        }
        let size = std::str::from_utf8(take_nul_field(payload, &mut cursor)?)
            .map_err(|_| "Codex source returned an invalid file size")?
            .parse::<usize>()
            .map_err(|_| "Codex source returned an invalid file size")?;
        if size as u64 > CONTEXT_ROLLOUT_MAX_BYTES {
            return Err("Codex source returned an oversized rollout".into());
        }
        let end = cursor
            .checked_add(size)
            .filter(|end| *end <= payload.len())
            .ok_or_else(|| "Codex source returned an incomplete rollout".to_string())?;
        let path = std::str::from_utf8(path)
            .map_err(|_| "Codex source returned a non-UTF-8 path")?
            .to_string();
        validate_context_rollout_path(&path)?;
        let jsonl = std::str::from_utf8(&payload[cursor..end])
            .map_err(|_| "Codex source returned a non-UTF-8 rollout")?;
        cursor = end;
        let parsed = parse_codex_jsonl(jsonl);
        if !parsed.session_id.is_empty() && !parsed.messages.is_empty() {
            candidates.push(CodexSessionCandidate { path, parsed });
        }
    }
    Ok(candidates)
}

fn context_codex_candidates_with_runner(
    context: &ExecutionContext,
    runner: &mut dyn crate::context_probe::ProbeRunner,
) -> Result<Vec<CodexSessionCandidate>, String> {
    let stdout = run_context_script(context, &context_scan_script(), runner)?;
    parse_context_scan(&stdout)
}

fn read_context_rollout_with_runner(
    context: &ExecutionContext,
    path: &str,
    runner: &mut dyn crate::context_probe::ProbeRunner,
) -> Result<String, String> {
    let stdout = run_context_script(context, &context_file_script(path)?, runner)?;
    let payload = protocol_payload(&stdout, CONTEXT_FILE_PROTOCOL)?;
    if payload.len() as u64 > CONTEXT_ROLLOUT_MAX_BYTES {
        return Err("Codex source returned an oversized rollout".into());
    }
    std::str::from_utf8(payload)
        .map(str::to_string)
        .map_err(|_| "Codex source returned a non-UTF-8 rollout".into())
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
    /// Paths whose final state is known without rescanning the source.
    pub synced_paths: Vec<String>,
}

async fn list_codex_candidates(
    store: &Store,
    candidates: Vec<CodexSessionCandidate>,
) -> Vec<CodexSessionInfo> {
    let mut out = vec![];
    for CodexSessionCandidate { path, parsed } in candidates {
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
            path,
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

async fn list_codex_sessions_in(store: &Store, root: &Path) -> Vec<CodexSessionInfo> {
    list_codex_candidates(store, local_codex_candidates(root)).await
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

async fn ensure_codex_folder(store: &Store, project_id: &str) -> Result<String, String> {
    if let Some((id, _, _)) = store
        .list_folders(project_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|(_, name, _)| name.eq_ignore_ascii_case("codex"))
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    store
        .create_folder(&id, project_id, "codex")
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Import one rollout into `project_id`. Returns what happened so batch callers
/// can tally a summary.
async fn import_codex_jsonl(
    store: &Store,
    project_id: &str,
    model_id: &str,
    source_path: &str,
    jsonl: &str,
) -> Result<&'static str, String> {
    let parsed = parse_codex_jsonl(jsonl);
    if parsed.session_id.is_empty() || parsed.messages.is_empty() {
        return Err(format!("{source_path}: no importable messages"));
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
    let folder_id = ensure_codex_folder(store, project_id).await?;
    store
        .create_frame(&frame_id, project_id, "Codex", model_id)
        .await
        .map_err(|e| e.to_string())?;
    store
        .move_session_to_folder(&frame_id, project_id, Some(&folder_id))
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
        .record_codex_import(&parsed.session_id, &frame_id, source_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok("imported")
}

#[cfg(test)]
async fn import_codex_file(
    store: &Store,
    project_id: &str,
    model_id: &str,
    path: &Path,
) -> Result<&'static str, String> {
    let jsonl = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    import_codex_jsonl(
        store,
        project_id,
        model_id,
        &path.display().to_string(),
        &jsonl,
    )
    .await
}

#[tauri::command]
pub(super) async fn list_codex_sessions(
    state: State<'_, AppState>,
    context_id: Option<String>,
) -> Result<Vec<CodexSessionInfo>, String> {
    let context_id = context_id.as_deref().unwrap_or("local");
    if context_id == "local" {
        let Some(root) = codex_sessions_root().filter(|root| root.is_dir()) else {
            return Ok(vec![]);
        };
        return Ok(list_codex_sessions_in(&state.store, &root).await);
    }
    let context = state
        .store
        .get_execution_context(context_id)
        .await
        .map_err(|e| e.to_string())?
        .filter(|context| context.kind != ExecutionContextKind::Local)
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    let candidates = tokio::task::spawn_blocking(move || {
        let mut runner = crate::context_probe::ProcessProbeRunner;
        context_codex_candidates_with_runner(&context, &mut runner)
    })
    .await
    .map_err(|e| format!("Codex scan task failed: {e}"))??;
    Ok(list_codex_candidates(&state.store, candidates).await)
}

#[tauri::command]
pub(super) async fn import_codex_sessions(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    paths: Vec<String>,
    context_id: Option<String>,
) -> Result<CodexImportSummary, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let model_id = super::models::active_profile_id(&state.store).await;
    let context_id = context_id.unwrap_or_else(|| "local".into());
    let context = if context_id == "local" {
        None
    } else {
        Some(
            state
                .store
                .get_execution_context(&context_id)
                .await
                .map_err(|e| e.to_string())?
                .filter(|context| context.kind != ExecutionContextKind::Local)
                .ok_or_else(|| format!("Execution context not found: {context_id}"))?,
        )
    };
    let mut summary = CodexImportSummary::default();
    for path in paths {
        let loaded = match context.clone() {
            Some(context) => {
                let remote_path = path.clone();
                tokio::task::spawn_blocking(move || {
                    let mut runner = crate::context_probe::ProcessProbeRunner;
                    read_context_rollout_with_runner(&context, &remote_path, &mut runner)
                })
                .await
                .map_err(|e| format!("Codex import task failed: {e}"))?
            }
            None => std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}")),
        };
        let source_path = if context_id == "local" {
            path.clone()
        } else {
            format!("{context_id}:{path}")
        };
        let outcome = match loaded {
            Ok(jsonl) => {
                import_codex_jsonl(&state.store, &ap.id, &model_id, &source_path, &jsonl).await
            }
            Err(error) => Err(error),
        };
        match outcome {
            Ok("imported") => {
                summary.imported += 1;
                summary.synced_paths.push(path);
            }
            Ok("updated") => {
                summary.updated += 1;
                summary.synced_paths.push(path);
            }
            Ok(_) => {
                summary.skipped += 1;
                summary.synced_paths.push(path);
            }
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
    use crate::context_probe::{ProbeCommand, ProbeCommandOutput, ProbeRunner};

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

    struct FakeProbeRunner {
        output: ProbeCommandOutput,
        commands: Vec<ProbeCommand>,
    }

    impl ProbeRunner for FakeProbeRunner {
        fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
            self.commands.push(command.clone());
            Ok(self.output.clone())
        }
    }

    fn framed_scan(path: &str, jsonl: &str) -> String {
        format!(
            "login banner\n{CONTEXT_SCAN_PROTOCOL}{path}\0{}\0{jsonl}\0",
            jsonl.len()
        )
    }

    #[test]
    fn scans_wsl_and_ssh_sources_with_fake_commands() {
        let path = "/home/me/.codex/sessions/2026/05/rollout-codex-abc.jsonl";
        let output = ProbeCommandOutput {
            status: 0,
            stdout: framed_scan(path, CODEX_JSONL),
            stderr: String::new(),
        };

        let mut wsl = ExecutionContext::new("wsl:Ubuntu-24.04", "Ubuntu-24.04").unwrap();
        wsl.config_json = r#"{"distro":"Ubuntu-24.04"}"#.into();
        let mut wsl_runner = FakeProbeRunner {
            output: output.clone(),
            commands: vec![],
        };
        let candidates = context_codex_candidates_with_runner(&wsl, &mut wsl_runner).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, path);
        assert_eq!(candidates[0].parsed.session_id, "codex-abc");
        assert_eq!(wsl_runner.commands[0].program, "wsl.exe");

        let mut ssh = ExecutionContext::new("ssh:gpu-server", "gpu-server").unwrap();
        ssh.config_json = r#"{"alias":"gpu-server"}"#.into();
        ssh.last_probe_status = Some("ok".into());
        let mut ssh_runner = FakeProbeRunner {
            output,
            commands: vec![],
        };
        let candidates = context_codex_candidates_with_runner(&ssh, &mut ssh_runner).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(ssh_runner.commands[0].program, "ssh");
        assert!(ssh_runner.commands[0]
            .args
            .last()
            .is_some_and(|script| script.contains("$HOME/.codex/sessions")));
    }

    #[test]
    fn context_file_read_validates_path_and_strips_banner() {
        let path = "/home/me/.codex/sessions/2026/05/rollout-codex-abc.jsonl";
        let wsl = ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
        let mut runner = FakeProbeRunner {
            output: ProbeCommandOutput {
                status: 0,
                stdout: format!("banner\n{CONTEXT_FILE_PROTOCOL}{CODEX_JSONL}"),
                stderr: String::new(),
            },
            commands: vec![],
        };
        assert_eq!(
            read_context_rollout_with_runner(&wsl, path, &mut runner).unwrap(),
            CODEX_JSONL
        );
        assert!(
            read_context_rollout_with_runner(&wsl, "/etc/passwd", &mut runner)
                .unwrap_err()
                .contains("outside")
        );
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
        let folders = store.list_folders("p").await.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].1, "codex");
        assert_eq!(sessions[0].3.as_deref(), Some(folders[0].0.as_str()));

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
