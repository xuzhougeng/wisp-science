//! Import Codex CLI and Claude Code JSONL conversations into Wisp sessions.
//! Re-imports are idempotent via the existing `codex_imports` table; Claude
//! session ids are namespaced so they cannot collide with Codex thread ids.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use tauri::State;
pub(super) use wisp_dto::{ExternalImportSummary, ExternalSessionInfo, ExternalSessionPreviewLine};
use wisp_llm::{Content, FunctionCall, Message, Role, ToolCall};
use wisp_store::{ExecutionContext, ExecutionContextKind, ExternalSessionCacheRecord, Store};

use super::AppState;

const CONTEXT_SCAN_PROTOCOL: &str = "WISP_SESSION_SCAN_V2\0";
const CONTEXT_METADATA_PROTOCOL: &str = "WISP_SESSION_META_V1\0";
const CONTEXT_FILE_PROTOCOL: &str = "WISP_CODEX_FILE_V1\0";
const CONTEXT_PREVIEW_PROTOCOL: &str = "WISP_SESSION_PREVIEW_V1\0";
const CONTEXT_ROLLOUT_MAX_BYTES: u64 = 32 * 1024 * 1024;
const CONTEXT_PREVIEW_MAX_BYTES: u64 = 2 * 1024 * 1024;
const PREVIEW_MESSAGE_LIMIT: usize = 4;
const PREVIEW_MESSAGE_CHARS: usize = 600;
// Keep a cold 500-session SSH scan below 16 MiB while leaving room for Codex's
// comparatively large session_meta line. If the first real prompt is later,
// the UI falls back to the project directory for the provisional title.
const METADATA_PREFIX_BYTES: u64 = 32 * 1024;
const CODEX_TITLE_PREVIEW_BYTES: usize = 8 * 1024;
const CODEX_TITLE_SEARCH_BYTES: u64 = 2 * 1024 * 1024;
const CODEX_METADATA_HINT_BYTES: u64 = 128;
const MAX_METADATA_FRAME_BYTES: u64 =
    METADATA_PREFIX_BYTES + CODEX_TITLE_PREVIEW_BYTES as u64 + CODEX_METADATA_HINT_BYTES;
const MAX_SCAN_FILES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportProvider {
    Codex,
    Claude,
}

impl ImportProvider {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            _ => Err("Unknown external conversation provider".into()),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }

    fn root(self) -> Option<PathBuf> {
        dirs::home_dir().map(|home| match self {
            Self::Codex => home.join(".codex").join("sessions"),
            Self::Claude => home.join(".claude").join("projects"),
        })
    }

    fn remote_root(self) -> &'static str {
        match self {
            Self::Codex => ".codex/sessions",
            Self::Claude => ".claude/projects",
        }
    }

    fn path_marker(self) -> &'static str {
        match self {
            Self::Codex => "/.codex/sessions/",
            Self::Claude => "/.claude/projects/",
        }
    }

    fn valid_filename(self, name: &str) -> bool {
        match self {
            Self::Codex => name.starts_with("rollout-") && name.ends_with(".jsonl"),
            Self::Claude => name.ends_with(".jsonl"),
        }
    }

    fn import_key(self, session_id: &str) -> String {
        match self {
            Self::Codex => session_id.to_string(),
            Self::Claude => format!("claude:{session_id}"),
        }
    }

    fn cache_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn frame_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }

    fn model_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex CLI",
            Self::Claude => "Claude Code",
        }
    }
}

#[derive(Debug, Default)]
struct ParsedSession {
    session_id: String,
    cwd: String,
    created_at_ms: i64,
    last_active_at_ms: i64,
    messages: Vec<ParsedMessage>,
}

#[derive(Debug)]
struct ParsedMessage {
    role: Role,
    text: String,
    ts_ms: i64,
    tool_calls: Vec<ToolCall>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionMetadata {
    session_id: String,
    title: String,
    cwd: String,
    message_count: usize,
    created_at_ms: i64,
    last_active_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    path: String,
    size: i64,
    modified_at_ms: i64,
}

#[derive(Debug)]
struct SessionCandidate {
    path: String,
    file_size: i64,
    modified_at_ms: i64,
    metadata: SessionMetadata,
    changed_since_import: bool,
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

fn push_message(
    session: &mut ParsedSession,
    role: Role,
    text: String,
    ts_ms: i64,
    tool_calls: Vec<ToolCall>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
) {
    if text.trim().is_empty() && tool_calls.is_empty() && tool_call_id.is_none() {
        return;
    }
    session.messages.push(ParsedMessage {
        role,
        text,
        ts_ms,
        tool_calls,
        tool_call_id,
        tool_name,
    });
}

fn update_chronology(session: &mut ParsedSession, ts_ms: i64) {
    if ts_ms <= 0 {
        return;
    }
    if session.created_at_ms == 0 {
        session.created_at_ms = ts_ms;
    }
    session.last_active_at_ms = session.last_active_at_ms.max(ts_ms);
}

fn parse_codex_jsonl(jsonl: &str) -> ParsedSession {
    let mut session = ParsedSession::default();
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
                push_message(&mut session, role, text, ts_ms, vec![], None, None);
            }
            _ => continue,
        }
        update_chronology(&mut session, ts_ms);
    }
    session
}

fn parse_claude_jsonl(jsonl: &str) -> ParsedSession {
    let mut session = ParsedSession::default();
    let mut tool_names = HashMap::<String, String>::new();
    for line in jsonl.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if value.get("isMeta").and_then(serde_json::Value::as_bool) == Some(true) {
            continue;
        }
        if session.session_id.is_empty() {
            if let Some(id) = value.get("sessionId").and_then(serde_json::Value::as_str) {
                session.session_id = id.to_string();
            }
        }
        if session.cwd.is_empty() {
            if let Some(cwd) = value.get("cwd").and_then(serde_json::Value::as_str) {
                session.cwd = cwd.to_string();
            }
        }
        let Some(message) = value.get("message").and_then(serde_json::Value::as_object) else {
            continue;
        };
        let role = match message.get("role").and_then(serde_json::Value::as_str) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => continue,
        };
        let Some(content) = message.get("content") else {
            continue;
        };
        let ts_ms = timestamp_ms(value.get("timestamp"));
        let before = session.messages.len();
        match content {
            serde_json::Value::String(text) => {
                push_message(&mut session, role, text.clone(), ts_ms, vec![], None, None);
            }
            serde_json::Value::Array(items) => {
                let text = content_text(content);
                let mut calls = vec![];
                let mut results = vec![];
                for item in items {
                    match item.get("type").and_then(serde_json::Value::as_str) {
                        Some("tool_use") if role == Role::Assistant => {
                            let Some(id) = item.get("id").and_then(serde_json::Value::as_str)
                            else {
                                continue;
                            };
                            let name = item
                                .get("name")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("tool")
                                .to_string();
                            let arguments = item
                                .get("input")
                                .map(serde_json::Value::to_string)
                                .unwrap_or_else(|| "{}".into());
                            tool_names.insert(id.to_string(), name.clone());
                            calls.push(ToolCall {
                                id: id.to_string(),
                                kind: "function".into(),
                                function: FunctionCall { name, arguments },
                            });
                        }
                        Some("tool_result") if role == Role::User => {
                            let Some(id) =
                                item.get("tool_use_id").and_then(serde_json::Value::as_str)
                            else {
                                continue;
                            };
                            let result = item.get("content").map(content_text).unwrap_or_default();
                            results.push((
                                id.to_string(),
                                tool_names.get(id).cloned().unwrap_or_else(|| "tool".into()),
                                result,
                            ));
                        }
                        _ => {}
                    }
                }
                push_message(&mut session, role, text, ts_ms, calls, None, None);
                for (id, name, result) in results {
                    push_message(
                        &mut session,
                        Role::Tool,
                        result,
                        ts_ms,
                        vec![],
                        Some(id),
                        Some(name),
                    );
                }
            }
            _ => {}
        }
        if session.messages.len() > before {
            update_chronology(&mut session, ts_ms);
        }
    }
    session
}

fn parse_jsonl(provider: ImportProvider, jsonl: &str) -> ParsedSession {
    match provider {
        ImportProvider::Codex => parse_codex_jsonl(jsonl),
        ImportProvider::Claude => parse_claude_jsonl(jsonl),
    }
}

/// Return the first real user message from a Codex record. Codex has emitted
/// this in both `response_item` message records and older `event_msg` records;
/// the import parser keeps the canonical response items, while metadata uses
/// this helper so a title remains available when the response item is outside
/// a bounded preview window.
fn codex_user_message_text(value: &serde_json::Value) -> Option<String> {
    let kind = value.get("type").and_then(serde_json::Value::as_str)?;
    let payload = value.get("payload")?;
    match kind {
        "response_item"
            if payload.get("type").and_then(serde_json::Value::as_str) == Some("message")
                && payload.get("role").and_then(serde_json::Value::as_str) == Some("user") =>
        {
            payload.get("content").map(content_text)
        }
        "event_msg"
            if payload.get("type").and_then(serde_json::Value::as_str) == Some("user_message") =>
        {
            payload
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        }
        "event_msg"
            if payload.get("type").and_then(serde_json::Value::as_str)
                == Some("item_completed") =>
        {
            let item = payload.get("item")?;
            if item.get("type").and_then(serde_json::Value::as_str) != Some("UserMessage") {
                return None;
            }
            item.get("content").map(content_text).or_else(|| {
                item.get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
        }
        _ => None,
    }
}

/// Codex desktop wraps some user turns with generated plugin, workspace, and
/// attachment context. Those wrappers belong to the transcript, but they are
/// not useful as the short title shown in the import list.
fn codex_title_text(text: &str) -> Option<String> {
    let text = text.trim();
    let has_request_marker = text.contains("## My request:");
    let candidate = text
        .split_once("## My request:")
        .map(|(_, request)| request)
        .unwrap_or(text);
    let candidate = candidate
        .split_once("\n<image name=")
        .map(|(request, _)| request)
        .or_else(|| {
            candidate
                .split_once("<image name=")
                .map(|(request, _)| request)
        })
        .unwrap_or(candidate)
        .trim();
    if candidate.is_empty() {
        return None;
    }
    if !has_request_marker
        && (text.contains("<recommended_plugins>")
            || text.contains("<environment_context>")
            || text.contains("<multi_agent_role>")
            || text.contains("<multi_agent_mode>")
            || text.contains("<skills_instructions>")
            || text.contains("<user_instructions>")
            || text.contains("AGENTS.md instructions")
            || text.starts_with("# Files mentioned by the user:"))
    {
        return None;
    }
    Some(candidate.chars().take(120).collect())
}

fn scan_session_files(provider: ImportProvider, root: &Path) -> Vec<PathBuf> {
    let mut walker = walkdir::WalkDir::new(root).min_depth(1);
    if provider == ImportProvider::Claude {
        walker = walker.max_depth(2).min_depth(2);
    }
    walker
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| provider.valid_filename(name))
        })
        .map(|entry| entry.into_path())
        .collect()
}

fn modified_at_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn local_file_stamps(provider: ImportProvider, root: &Path) -> Vec<FileStamp> {
    let mut stamps = scan_session_files(provider, root)
        .into_iter()
        .filter_map(|path| {
            let metadata = path.metadata().ok()?;
            Some(FileStamp {
                path: path.display().to_string(),
                size: i64::try_from(metadata.len()).ok()?,
                modified_at_ms: modified_at_ms(&metadata),
            })
        })
        .collect::<Vec<_>>();
    stamps.sort_by(|a, b| {
        b.modified_at_ms
            .cmp(&a.modified_at_ms)
            .then_with(|| a.path.cmp(&b.path))
    });
    stamps.truncate(MAX_SCAN_FILES);
    stamps
}

fn read_bounded_jsonl(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = vec![];
    file.take(CONTEXT_ROLLOUT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > CONTEXT_ROLLOUT_MAX_BYTES {
        return Err(format!(
            "{} exceeds the {} byte import limit",
            path.display(),
            CONTEXT_ROLLOUT_MAX_BYTES
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8", path.display()))
}

fn read_metadata_preview(provider: ImportProvider, path: &Path) -> Result<String, String> {
    if provider == ImportProvider::Codex {
        // A Codex session_meta line can contain the full instruction bundle,
        // and the first real prompt commonly follows a large developer line.
        // Read complete JSONL records up to the title search budget instead of
        // cutting the byte stream in the middle of the first records.
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut reader = BufReader::new(file);
        let mut bytes = Vec::new();
        let mut line = Vec::new();
        while (bytes.len() as u64) < CODEX_TITLE_SEARCH_BYTES {
            line.clear();
            let read = reader
                .read_until(b'\n', &mut line)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&line);
        }
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = vec![];
    file.take(METADATA_PREFIX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn metadata_from_parsed(
    provider: ImportProvider,
    parsed: &ParsedSession,
    stamp: &FileStamp,
    supplemental_title: Option<&str>,
) -> Option<SessionMetadata> {
    if parsed.session_id.is_empty() {
        return None;
    }
    let title = parsed
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .find_map(|message| {
            if provider == ImportProvider::Codex {
                codex_title_text(&message.text)
            } else {
                (!message.text.trim().is_empty())
                    .then(|| message.text.trim().chars().take(120).collect::<String>())
            }
        })
        .or_else(|| {
            supplemental_title
                .filter(|title| !title.trim().is_empty() && !is_noise_user_text(title))
                .map(|title| title.trim().chars().take(120).collect::<String>())
        })
        .or_else(|| {
            parsed
                .cwd
                .rsplit(['/', '\\'])
                .find(|part| !part.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| provider.frame_name().to_string());
    let partial = stamp.size > METADATA_PREFIX_BYTES as i64;
    Some(SessionMetadata {
        session_id: parsed.session_id.clone(),
        title,
        cwd: parsed.cwd.clone(),
        message_count: parsed.messages.len(),
        created_at_ms: parsed.created_at_ms,
        last_active_at_ms: if partial && stamp.modified_at_ms > 0 {
            stamp.modified_at_ms
        } else if parsed.last_active_at_ms > 0 {
            parsed.last_active_at_ms
        } else {
            stamp.modified_at_ms
        },
    })
}

fn codex_event_title(jsonl: &str) -> Option<String> {
    jsonl.lines().find_map(|line| {
        let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
        codex_user_message_text(&value).and_then(|text| codex_title_text(&text))
    })
}

fn codex_message_count_hint(jsonl: &str) -> Option<usize> {
    jsonl.lines().find_map(|line| {
        let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
        (value.get("type").and_then(serde_json::Value::as_str) == Some("wisp_metadata"))
            .then(|| {
                value
                    .get("message_count")
                    .and_then(serde_json::Value::as_u64)
            })
            .flatten()
            .and_then(|count| usize::try_from(count).ok())
    })
}

/// Scan a local Codex rollout without retaining its transcript in memory.
/// Metadata must cover the complete file: a prefix is insufficient for the
/// message count once the initial instruction bundle grows beyond the prefix.
fn metadata_from_codex_file(
    path: &Path,
    stamp: &FileStamp,
) -> Result<Option<SessionMetadata>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut session_id = String::new();
    let mut cwd = String::new();
    let mut first_user = None;
    let mut created_at_ms = 0;
    let mut last_active_at_ms = 0;
    let mut message_count = 0_usize;

    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .map_err(|e| format!("{}: {e}", path.display()))?
            == 0
        {
            break;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        let ts_ms = timestamp_ms(value.get("timestamp"));
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("session_meta") => {
                let payload = value
                    .get("payload")
                    .filter(|p| p.is_object())
                    .unwrap_or(&value);
                if let Some(id) = payload.get("id").and_then(serde_json::Value::as_str) {
                    session_id = id.to_string();
                }
                if let Some(value) = payload.get("cwd").and_then(serde_json::Value::as_str) {
                    cwd = value.to_string();
                }
            }
            Some("response_item") => {
                let payload = value
                    .get("payload")
                    .filter(|p| p.is_object())
                    .unwrap_or(&value);
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
                let text = payload.get("content").map(content_text).unwrap_or_default();
                if text.trim().is_empty() || (role == Role::User && is_noise_user_text(&text)) {
                    continue;
                }
                message_count += 1;
                if role == Role::User && first_user.is_none() {
                    first_user = codex_title_text(&text);
                }
            }
            _ => {}
        }
        if ts_ms > 0 {
            if created_at_ms == 0 {
                created_at_ms = ts_ms;
            }
            last_active_at_ms = last_active_at_ms.max(ts_ms);
        }
        if first_user.is_none() {
            if let Some(text) =
                codex_user_message_text(&value).and_then(|text| codex_title_text(&text))
            {
                first_user = Some(text);
            }
        }
    }

    if session_id.is_empty() {
        return Ok(None);
    }
    let title = first_user
        .or_else(|| {
            cwd.rsplit(['/', '\\'])
                .find(|part| !part.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| ImportProvider::Codex.frame_name().to_string());
    Ok(Some(SessionMetadata {
        session_id,
        title,
        cwd,
        message_count,
        created_at_ms,
        last_active_at_ms: if last_active_at_ms > 0 {
            last_active_at_ms
        } else {
            stamp.modified_at_ms
        },
    }))
}

fn metadata_from_jsonl(
    provider: ImportProvider,
    jsonl: &str,
    stamp: &FileStamp,
) -> Option<SessionMetadata> {
    let supplemental_title = (provider == ImportProvider::Codex)
        .then(|| codex_event_title(jsonl))
        .flatten();
    let mut metadata = metadata_from_parsed(
        provider,
        &parse_jsonl(provider, jsonl),
        stamp,
        supplemental_title.as_deref(),
    )?;
    if provider == ImportProvider::Codex {
        if let Some(count) = codex_message_count_hint(jsonl) {
            metadata.message_count = count;
        }
    }
    Some(metadata)
}

fn metadata_from_cache(record: &ExternalSessionCacheRecord) -> SessionMetadata {
    SessionMetadata {
        session_id: record.session_id.clone(),
        title: record.title.clone(),
        cwd: record.cwd.clone(),
        message_count: record.message_count.max(0) as usize,
        created_at_ms: record.created_at_ms,
        last_active_at_ms: record.last_active_at_ms,
    }
}

fn candidate_from_cache(record: &ExternalSessionCacheRecord) -> SessionCandidate {
    SessionCandidate {
        path: record.source_path.clone(),
        file_size: record.file_size,
        modified_at_ms: record.modified_at_ms,
        metadata: metadata_from_cache(record),
        changed_since_import: record.changed_since_import,
    }
}

fn cache_record(
    source_id: &str,
    provider: ImportProvider,
    candidate: &SessionCandidate,
) -> ExternalSessionCacheRecord {
    ExternalSessionCacheRecord {
        source_id: source_id.to_string(),
        provider: provider.cache_name().to_string(),
        source_path: candidate.path.clone(),
        file_size: candidate.file_size,
        modified_at_ms: candidate.modified_at_ms,
        session_id: candidate.metadata.session_id.clone(),
        title: candidate.metadata.title.clone(),
        cwd: candidate.metadata.cwd.clone(),
        message_count: candidate.metadata.message_count as i64,
        created_at_ms: candidate.metadata.created_at_ms,
        last_active_at_ms: candidate.metadata.last_active_at_ms,
        changed_since_import: candidate.changed_since_import,
    }
}

fn cached_title_is_fallback(record: &ExternalSessionCacheRecord) -> bool {
    let cwd_name = record.cwd.rsplit(['/', '\\']).find(|part| !part.is_empty());
    record.title.trim() == "Codex" || cwd_name.is_some_and(|name| record.title.trim() == name)
}

fn cached_metadata_needs_repair(record: &ExternalSessionCacheRecord) -> bool {
    record.provider == "codex"
        && (record.message_count <= 0
            || record.title.trim().is_empty()
            || cached_title_is_fallback(record))
}

fn candidates_from_stamps(
    provider: ImportProvider,
    stamps: Vec<FileStamp>,
    cached: &[ExternalSessionCacheRecord],
) -> Vec<SessionCandidate> {
    let cached = cached
        .iter()
        .map(|record| (record.source_path.as_str(), record))
        .collect::<HashMap<_, _>>();
    stamps
        .into_iter()
        .filter_map(|stamp| {
            let previous = cached.get(stamp.path.as_str()).copied();
            if let Some(record) = previous.filter(|record| {
                record.file_size == stamp.size
                    && record.modified_at_ms == stamp.modified_at_ms
                    && !cached_metadata_needs_repair(record)
            }) {
                return Some(candidate_from_cache(record));
            }
            let metadata = if provider == ImportProvider::Codex
                && stamp.size <= CONTEXT_ROLLOUT_MAX_BYTES as i64
            {
                metadata_from_codex_file(Path::new(&stamp.path), &stamp)
                    .ok()
                    .flatten()
                    .or_else(|| {
                        read_metadata_preview(provider, Path::new(&stamp.path))
                            .ok()
                            .and_then(|jsonl| metadata_from_jsonl(provider, &jsonl, &stamp))
                    })
            } else {
                read_metadata_preview(provider, Path::new(&stamp.path))
                    .ok()
                    .and_then(|jsonl| metadata_from_jsonl(provider, &jsonl, &stamp))
            }
            .or_else(|| previous.map(metadata_from_cache))?;
            Some(SessionCandidate {
                path: stamp.path,
                file_size: stamp.size,
                modified_at_ms: stamp.modified_at_ms,
                metadata,
                changed_since_import: previous.is_some()
                    || stamp.size > METADATA_PREFIX_BYTES as i64,
            })
        })
        .collect()
}

fn local_candidates(
    provider: ImportProvider,
    root: &Path,
    cached: &[ExternalSessionCacheRecord],
) -> Vec<SessionCandidate> {
    candidates_from_stamps(provider, local_file_stamps(provider, root), cached)
}

fn context_scan_script(provider: ImportProvider) -> String {
    let root = provider.remote_root();
    let pattern = match provider {
        ImportProvider::Codex => "rollout-*.jsonl",
        ImportProvider::Claude => "*.jsonl",
    };
    let depth = match provider {
        ImportProvider::Codex => "",
        ImportProvider::Claude => "-mindepth 2 -maxdepth 2",
    };
    format!(
        r#"LC_ALL=C
root=$HOME/{root}
printf 'WISP_SESSION_SCAN_V2\000'
if [ ! -d "$root" ]; then
  exit 0
fi
listing=$(find "$root" {depth} -type f -name '{pattern}' -printf '%T@\t%s\t%p\n' 2>/dev/null | sort -rn | head -{MAX_SCAN_FILES})
if [ -n "$listing" ]; then
  printf '%s\n' "$listing"
else
  find "$root" {depth} -type f -name '{pattern}' -print 2>/dev/null | head -{MAX_SCAN_FILES}
fi"#
    )
}

fn context_metadata_script(provider: ImportProvider, stamped: bool) -> String {
    let root = provider.remote_root();
    let pattern = match provider {
        ImportProvider::Codex => "rollout-*.jsonl",
        ImportProvider::Claude => "*.jsonl",
    };
    let depth = match provider {
        ImportProvider::Codex => "",
        ImportProvider::Claude => "-mindepth 2 -maxdepth 2",
    };
    let listing = if stamped {
        format!(
            "find \"$root\" {depth} -type f -name '{pattern}' -printf '%T@\\t%s\\t%p\\n' 2>/dev/null | sort -rn | head -{MAX_SCAN_FILES}"
        )
    } else {
        format!(
            "find \"$root\" {depth} -type f -name '{pattern}' -print 2>/dev/null | head -{MAX_SCAN_FILES}"
        )
    };
    let read_loop = if stamped {
        r#"while IFS="$(printf '\t')" read -r mtime size file; do"#
    } else {
        r#"while IFS= read -r file; do
  mtime=0
  size=$(wc -c < "$file" 2>/dev/null) || continue"#
    };
    let read_metadata = match provider {
        ImportProvider::Codex => format!(
            r#"if [ "$size" -le {METADATA_PREFIX_BYTES} ]; then
  metadata=$(head -c "$size" "$file" 2>/dev/null)
else
  metadata=$(
  awk '
    NR == 1 {{ print substr($0, 1, {METADATA_PREFIX_BYTES}) }}
    (index($0, "\"type\":\"event_msg\"") && index($0, "\"type\":\"user_message\"") \
      || index($0, "\"type\":\"response_item\"") && index($0, "\"role\":\"user\"")) {{
      print substr($0, 1, {CODEX_TITLE_PREVIEW_BYTES})
      exit
    }}
  ' "$file" 2>/dev/null
)
fi
message_count=$(
  awk '
    /"type":"response_item"/ && /"type":"message"/ && ( /"role":"user"/ || /"role":"assistant"/ ) {{
      if ( /"role":"user"/ && (index($0, "<environment_context>") || index($0, "AGENTS.md instructions") || index($0, "<user_instructions>")) ) next
      count++
    }}
    END {{ print count + 0 }}
  ' "$file" 2>/dev/null
)
case "$message_count" in ''|*[!0-9]*) message_count=0 ;; esac
metadata=$(printf '%s\n{{"type":"wisp_metadata","message_count":%s}}' "$metadata" "$message_count")
prefix=${{#metadata}}
case "$prefix" in ''|*[!0-9]*) continue ;; esac
if [ "$prefix" -gt {MAX_METADATA_FRAME_BYTES} ]; then continue; fi
printf '%s\000%s\000%s\000%s\000' "$file" "$size" "$mtime" "$prefix"
printf '%s' "$metadata""#
        ),
        ImportProvider::Claude => format!(
            r#"prefix=$size
if [ "$prefix" -gt {METADATA_PREFIX_BYTES} ]; then prefix={METADATA_PREFIX_BYTES}; fi
printf '%s\000%s\000%s\000%s\000' "$file" "$size" "$mtime" "$prefix"
head -c "$prefix" "$file" || exit 68"#
        ),
    };
    format!(
        r#"LC_ALL=C
root=$HOME/{root}
printf 'WISP_SESSION_META_V1\000'
if [ ! -d "$root" ]; then
  printf '\000'
  exit 0
fi
{listing} |
{read_loop}
  case "$size" in ''|*[!0-9]*) continue ;; esac
  if [ "$size" -gt {CONTEXT_ROLLOUT_MAX_BYTES} ]; then continue; fi
  {read_metadata}
done
printf '\000'"#
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn validate_context_path(provider: ImportProvider, path: &str) -> Result<(), String> {
    if path.len() > 4096 || path.contains(['\0', '\t', '\n', '\r']) {
        return Err(format!("Invalid {} session path", provider.label()));
    }
    if path.split('/').any(|part| part == "..") || !path.contains(provider.path_marker()) {
        return Err(format!(
            "{} session is outside ~/{}",
            provider.label(),
            provider.remote_root()
        ));
    }
    let relative = path
        .split_once(provider.path_marker())
        .map(|(_, relative)| relative)
        .unwrap_or_default();
    let name = relative.rsplit('/').next().unwrap_or_default();
    if !provider.valid_filename(name)
        || (provider == ImportProvider::Claude
            && (relative.split('/').count() != 2
                || relative.split('/').any(|part| part.is_empty())))
    {
        return Err(format!("Invalid {} session filename", provider.label()));
    }
    Ok(())
}

fn context_file_script(provider: ImportProvider, path: &str) -> Result<String, String> {
    validate_context_path(provider, path)?;
    let path = shell_single_quote(path);
    let root = provider.remote_root();
    let label = provider.label();
    Ok(format!(
        r#"LC_ALL=C
root=$HOME/{root}
file={path}
case "$file" in "$root"/*) ;; *)
  printf '{label} session is outside ~/{root}\n' >&2
  exit 66
esac
if [ ! -f "$file" ] || [ -L "$file" ]; then
  printf 'Cannot read {label} session: %s\n' "$file" >&2
  exit 66
fi
size=$(wc -c < "$file" 2>/dev/null) || exit 66
if [ "$size" -gt {CONTEXT_ROLLOUT_MAX_BYTES} ]; then
  printf '{label} session exceeds {CONTEXT_ROLLOUT_MAX_BYTES} byte limit\n' >&2
  exit 67
fi
printf 'WISP_CODEX_FILE_V1\000'
head -c "$size" "$file""#
    ))
}

fn context_preview_script(provider: ImportProvider, path: &str) -> Result<String, String> {
    validate_context_path(provider, path)?;
    let path = shell_single_quote(path);
    let root = provider.remote_root();
    let label = provider.label();
    Ok(format!(
        r#"LC_ALL=C
root=$HOME/{root}
file={path}
case "$file" in "$root"/*) ;; *)
  printf '{label} session is outside ~/{root}\n' >&2
  exit 66
esac
if [ ! -f "$file" ] || [ -L "$file" ]; then
  printf 'Cannot read {label} session: %s\n' "$file" >&2
  exit 66
fi
size=$(wc -c < "$file" 2>/dev/null) || exit 66
if [ "$size" -gt {CONTEXT_PREVIEW_MAX_BYTES} ]; then size={CONTEXT_PREVIEW_MAX_BYTES}; fi
printf 'WISP_SESSION_PREVIEW_V1\000'
head -c "$size" "$file""#
    ))
}

fn run_context_script(
    provider: ImportProvider,
    context: &ExecutionContext,
    script: &str,
    runner: &mut dyn crate::context_probe::ProbeRunner,
) -> Result<Vec<u8>, String> {
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
            "{} scan failed in {} (exit {}): {detail}",
            provider.label(),
            context.label,
            output.status
        ));
    }
    Ok(output.stdout)
}

fn protocol_payload<'a>(stdout: &'a [u8], marker: &str) -> Result<&'a [u8], String> {
    let marker = marker.as_bytes();
    stdout
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|start| &stdout[start + marker.len()..])
        .ok_or_else(|| "Session source returned an invalid response".into())
}

fn take_nul_field<'a>(payload: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], String> {
    let rest = payload
        .get(*cursor..)
        .ok_or_else(|| "Session source returned an incomplete response".to_string())?;
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| "Session source returned an incomplete response".to_string())?;
    *cursor += end + 1;
    Ok(&rest[..end])
}

fn timestamp_text_ms(value: &str) -> i64 {
    let (seconds, fraction) = value.trim().split_once('.').unwrap_or((value.trim(), ""));
    let seconds = seconds.parse::<i64>().unwrap_or(0);
    let millis = fraction
        .bytes()
        .take(3)
        .fold((0_i64, 100_i64), |(value, scale), digit| {
            if digit.is_ascii_digit() {
                (value + i64::from(digit - b'0') * scale, scale / 10)
            } else {
                (value, 0)
            }
        })
        .0;
    seconds.saturating_mul(1000).saturating_add(millis)
}

fn parse_context_listing(
    provider: ImportProvider,
    stdout: &[u8],
) -> Result<(Vec<FileStamp>, bool), String> {
    let payload = protocol_payload(stdout, CONTEXT_SCAN_PROTOCOL)?;
    let text =
        std::str::from_utf8(payload).map_err(|_| "Session source returned a non-UTF-8 listing")?;
    let mut stamped = false;
    let mut files = vec![];
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let parts = line.splitn(3, '\t').collect::<Vec<_>>();
        let (modified_at_ms, size, path) = if parts.len() == 3 {
            stamped = true;
            (
                timestamp_text_ms(parts[0]),
                parts[1]
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| "Session source returned an invalid file size")?,
                parts[2].trim(),
            )
        } else {
            (0, 0, line.trim())
        };
        if size > CONTEXT_ROLLOUT_MAX_BYTES as i64 {
            continue;
        }
        validate_context_path(provider, path)?;
        files.push(FileStamp {
            path: path.to_string(),
            size,
            modified_at_ms,
        });
    }
    Ok((files, stamped))
}

fn parse_context_metadata(
    provider: ImportProvider,
    stdout: &[u8],
    cached: &[ExternalSessionCacheRecord],
) -> Result<Vec<SessionCandidate>, String> {
    let payload = protocol_payload(stdout, CONTEXT_METADATA_PROTOCOL)?;
    let cached = cached
        .iter()
        .map(|record| (record.source_path.as_str(), record))
        .collect::<HashMap<_, _>>();
    let mut cursor = 0;
    let mut candidates = vec![];
    loop {
        let path = take_nul_field(payload, &mut cursor)?;
        if path.is_empty() {
            break;
        }
        let path = std::str::from_utf8(path)
            .map_err(|_| "Session source returned a non-UTF-8 path")?
            .to_string();
        validate_context_path(provider, &path)?;
        let size = std::str::from_utf8(take_nul_field(payload, &mut cursor)?)
            .map_err(|_| "Session source returned an invalid file size")?
            .parse::<i64>()
            .map_err(|_| "Session source returned an invalid file size")?;
        let modified_at_ms = timestamp_text_ms(
            std::str::from_utf8(take_nul_field(payload, &mut cursor)?)
                .map_err(|_| "Session source returned an invalid timestamp")?,
        );
        let prefix_size = std::str::from_utf8(take_nul_field(payload, &mut cursor)?)
            .map_err(|_| "Session source returned an invalid prefix size")?
            .parse::<usize>()
            .map_err(|_| "Session source returned an invalid prefix size")?;
        if prefix_size as u64 > MAX_METADATA_FRAME_BYTES {
            return Err("Session source returned an oversized metadata prefix".into());
        }
        let end = cursor
            .checked_add(prefix_size)
            .filter(|end| *end <= payload.len())
            .ok_or_else(|| "Session source returned incomplete metadata".to_string())?;
        let stamp = FileStamp {
            path: path.clone(),
            size,
            modified_at_ms,
        };
        let previous = cached.get(path.as_str()).copied();
        if let Some(record) = previous.filter(|record| {
            record.file_size == size
                && record.modified_at_ms == modified_at_ms
                && !cached_metadata_needs_repair(record)
        }) {
            candidates.push(candidate_from_cache(record));
            cursor = end;
            continue;
        }
        let jsonl = String::from_utf8_lossy(&payload[cursor..end]);
        let metadata = metadata_from_jsonl(provider, &jsonl, &stamp);
        cursor = end;
        let metadata = metadata.or_else(|| previous.map(metadata_from_cache));
        if let Some(metadata) = metadata {
            candidates.push(SessionCandidate {
                path,
                file_size: size,
                modified_at_ms,
                metadata,
                changed_since_import: previous.is_some() || size > METADATA_PREFIX_BYTES as i64,
            });
        }
    }
    Ok(candidates)
}

fn context_candidates_with_runner(
    provider: ImportProvider,
    context: &ExecutionContext,
    cached: &[ExternalSessionCacheRecord],
    runner: &mut dyn crate::context_probe::ProbeRunner,
) -> Result<Vec<SessionCandidate>, String> {
    let stdout = run_context_script(provider, context, &context_scan_script(provider), runner)?;
    let (stamps, stamped) = parse_context_listing(provider, &stdout)?;
    let cache = cached
        .iter()
        .map(|record| (record.source_path.as_str(), record))
        .collect::<HashMap<_, _>>();
    if !cached.iter().any(cached_metadata_needs_repair)
        && stamps.iter().all(|stamp| {
            cache.get(stamp.path.as_str()).is_some_and(|record| {
                stamped
                    && record.file_size == stamp.size
                    && record.modified_at_ms == stamp.modified_at_ms
            })
        })
    {
        return Ok(stamps
            .iter()
            .filter_map(|stamp| cache.get(stamp.path.as_str()).copied())
            .map(candidate_from_cache)
            .collect());
    }
    let stdout = run_context_script(
        provider,
        context,
        &context_metadata_script(provider, stamped),
        runner,
    )?;
    parse_context_metadata(provider, &stdout, cached)
}

fn read_context_jsonl_with_runner(
    provider: ImportProvider,
    context: &ExecutionContext,
    path: &str,
    runner: &mut dyn crate::context_probe::ProbeRunner,
) -> Result<String, String> {
    let stdout = run_context_script(
        provider,
        context,
        &context_file_script(provider, path)?,
        runner,
    )?;
    let payload = protocol_payload(&stdout, CONTEXT_FILE_PROTOCOL)?;
    if payload.len() as u64 > CONTEXT_ROLLOUT_MAX_BYTES {
        return Err("Session source returned an oversized transcript".into());
    }
    std::str::from_utf8(payload)
        .map(str::to_string)
        .map_err(|_| "Session source returned a non-UTF-8 transcript".into())
}

fn read_context_preview_with_runner(
    provider: ImportProvider,
    context: &ExecutionContext,
    path: &str,
    runner: &mut dyn crate::context_probe::ProbeRunner,
) -> Result<String, String> {
    let stdout = run_context_script(
        provider,
        context,
        &context_preview_script(provider, path)?,
        runner,
    )?;
    let payload = protocol_payload(&stdout, CONTEXT_PREVIEW_PROTOCOL)?;
    if payload.len() as u64 > CONTEXT_PREVIEW_MAX_BYTES {
        return Err("Session source returned an oversized preview".into());
    }
    Ok(String::from_utf8_lossy(payload).into_owned())
}

fn preview_lines(provider: ImportProvider, jsonl: &str) -> Vec<ExternalSessionPreviewLine> {
    let mut lines = parse_jsonl(provider, jsonl)
        .messages
        .into_iter()
        .filter(|message| matches!(message.role, Role::User | Role::Assistant))
        .filter_map(|message| {
            let text = message.text.trim();
            (!text.is_empty()).then(|| ExternalSessionPreviewLine {
                role: if message.role == Role::User {
                    "user"
                } else {
                    "assistant"
                }
                .into(),
                text: text.chars().take(PREVIEW_MESSAGE_CHARS).collect(),
            })
        })
        .take(PREVIEW_MESSAGE_LIMIT)
        .collect::<Vec<_>>();
    if lines.is_empty() && provider == ImportProvider::Codex {
        if let Some(text) = codex_event_title(jsonl) {
            lines.push(ExternalSessionPreviewLine {
                role: "user".into(),
                text: text.chars().take(PREVIEW_MESSAGE_CHARS).collect(),
            });
        }
    }
    lines
}

async fn list_candidates(
    provider: ImportProvider,
    store: &Store,
    project: &str,
    candidates: Vec<SessionCandidate>,
) -> Result<Vec<ExternalSessionInfo>, String> {
    let mut out = vec![];
    for candidate in candidates {
        let import_key = provider.import_key(&candidate.metadata.session_id);
        let state = match existing_import(store, project, &import_key).await? {
            Some(frame_id) => {
                let stored = store
                    .message_count(&frame_id)
                    .await
                    .map_err(|e| e.to_string())?;
                if candidate.changed_since_import
                    || candidate.metadata.message_count as i64 > stored
                {
                    "updatable"
                } else {
                    "imported"
                }
            }
            None => "new",
        };
        out.push(ExternalSessionInfo {
            path: candidate.path,
            session_id: candidate.metadata.session_id,
            title: candidate.metadata.title,
            cwd: candidate.metadata.cwd,
            message_count: candidate.metadata.message_count,
            last_active_at: candidate.metadata.last_active_at_ms / 1000,
            state: state.to_string(),
        });
    }
    out.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    Ok(out)
}

#[cfg(test)]
async fn list_sessions_in(
    provider: ImportProvider,
    store: &Store,
    root: &Path,
) -> Vec<ExternalSessionInfo> {
    list_candidates(provider, store, "p", local_candidates(provider, root, &[]))
        .await
        .unwrap()
}

fn scoped_import_key(project: &str, provider_key: &str) -> String {
    format!("project:{}:{project}:{provider_key}", project.len())
}

async fn existing_import(
    store: &Store,
    project: &str,
    provider_key: &str,
) -> Result<Option<String>, String> {
    for key in [
        scoped_import_key(project, provider_key),
        provider_key.to_owned(),
    ] {
        if let Some(frame) = store
            .find_codex_import(&key)
            .await
            .map_err(|e| e.to_string())?
        {
            if store
                .frame_project_id(&frame)
                .await
                .map_err(|e| e.to_string())?
                .as_deref()
                == Some(project)
            {
                return Ok(Some(frame));
            }
        }
    }
    Ok(None)
}

fn to_wisp_messages(provider: ImportProvider, parsed: &ParsedSession) -> Vec<Message> {
    parsed
        .messages
        .iter()
        .map(|m| Message {
            role: m.role,
            content: Content::text(m.text.clone()),
            tool_calls: m.tool_calls.clone(),
            tool_call_id: m.tool_call_id.clone(),
            tool_name: m.tool_name.clone(),
            reasoning: None,
            ts: m.ts_ms / 1000,
            model_name: (m.role == Role::Assistant).then(|| provider.model_name().to_string()),
        })
        .collect()
}

async fn ensure_import_folder(
    provider: ImportProvider,
    store: &Store,
    project_id: &str,
) -> Result<String, String> {
    if let Some((id, _, _)) = store
        .list_folders(project_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|(_, name, _)| name.eq_ignore_ascii_case(provider.folder()))
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    store
        .create_folder(&id, project_id, provider.folder())
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

struct ImportResult {
    frame_id: String,
    outcome: &'static str,
    message_count: i64,
    last_active_at_ms: i64,
}

async fn import_session_jsonl(
    provider: ImportProvider,
    store: &Store,
    project_id: &str,
    model_id: &str,
    source_path: &str,
    jsonl: &str,
) -> Result<ImportResult, String> {
    static IMPORTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _import = IMPORTS.lock().await;
    let parsed = parse_jsonl(provider, jsonl);
    if parsed.session_id.is_empty() || parsed.messages.is_empty() {
        return Err(format!("{source_path}: no importable messages"));
    }
    let result = |frame_id: String, outcome| ImportResult {
        frame_id,
        outcome,
        message_count: parsed.messages.len() as i64,
        last_active_at_ms: parsed.last_active_at_ms,
    };
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
    let import_key = provider.import_key(&parsed.session_id);

    if let Some(frame_id) = existing_import(store, project_id, &import_key).await? {
        let stored = store
            .message_count(&frame_id)
            .await
            .map_err(|e| e.to_string())?;
        // ponytail: only fast-forward. If the frame was continued inside Wisp
        // it can hold more turns than the rollout; merging diverged histories
        // is out of scope, so leave it untouched.
        if (parsed.messages.len() as i64) <= stored {
            return Ok(result(frame_id, "skipped"));
        }
        store
            .replace_messages(&frame_id, &to_wisp_messages(provider, &parsed))
            .await
            .map_err(|e| e.to_string())?;
        store
            .set_frame_timestamps(&frame_id, created_at, updated_at)
            .await
            .map_err(|e| e.to_string())?;
        store
            .record_codex_import(
                &scoped_import_key(project_id, &import_key),
                &frame_id,
                source_path,
            )
            .await
            .map_err(|e| e.to_string())?;
        return Ok(result(frame_id, "updated"));
    }

    let frame_id = uuid::Uuid::new_v4().to_string();
    let folder_id = ensure_import_folder(provider, store, project_id).await?;
    store
        .create_frame(&frame_id, project_id, provider.frame_name(), model_id)
        .await
        .map_err(|e| e.to_string())?;
    store
        .move_session_to_folder(&frame_id, project_id, Some(&folder_id))
        .await
        .map_err(|e| e.to_string())?;
    for (i, msg) in to_wisp_messages(provider, &parsed).iter().enumerate() {
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
        .record_codex_import(
            &scoped_import_key(project_id, &import_key),
            &frame_id,
            source_path,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(result(frame_id, "imported"))
}

#[cfg(test)]
async fn import_session_file(
    provider: ImportProvider,
    store: &Store,
    project_id: &str,
    model_id: &str,
    path: &Path,
) -> Result<&'static str, String> {
    let jsonl = read_bounded_jsonl(path)?;
    import_session_jsonl(
        provider,
        store,
        project_id,
        model_id,
        &path.display().to_string(),
        &jsonl,
    )
    .await
    .map(|result| result.outcome)
}

async fn list_sessions(
    state: &AppState,
    project: &str,
    context_id: Option<String>,
    refresh: Option<bool>,
    provider: ImportProvider,
) -> Result<Vec<ExternalSessionInfo>, String> {
    let context_id = context_id.unwrap_or_else(|| "local".into());
    let cached = state
        .store
        .list_external_session_cache(&context_id, provider.cache_name())
        .await
        .map_err(|e| e.to_string())?;
    // Older scans could only see the instruction prefix and persisted a
    // zero-message, directory-name placeholder. Rebuild those entries once
    // the complete metadata scanner is available.
    let needs_cache_repair = cached.iter().any(cached_metadata_needs_repair);
    if !refresh.unwrap_or(false) && !cached.is_empty() && !needs_cache_repair {
        return list_candidates(
            provider,
            &state.store,
            project,
            cached.iter().map(candidate_from_cache).collect(),
        )
        .await;
    }
    let candidates = if context_id == "local" {
        let Some(root) = provider.root().filter(|root| root.is_dir()) else {
            state
                .store
                .replace_external_session_cache(&context_id, provider.cache_name(), &[])
                .await
                .map_err(|e| e.to_string())?;
            return Ok(vec![]);
        };
        let cached_for_scan = cached.clone();
        tokio::task::spawn_blocking(move || local_candidates(provider, &root, &cached_for_scan))
            .await
            .map_err(|e| format!("{} scan task failed: {e}", provider.label()))?
    } else {
        let context = state
            .store
            .get_execution_context(&context_id)
            .await
            .map_err(|e| e.to_string())?
            .filter(|context| context.kind != ExecutionContextKind::Local)
            .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
        let cached_for_scan = cached.clone();
        tokio::task::spawn_blocking(move || {
            let mut runner = crate::context_probe::ProcessProbeRunner;
            context_candidates_with_runner(provider, &context, &cached_for_scan, &mut runner)
        })
        .await
        .map_err(|e| format!("{} scan task failed: {e}", provider.label()))??
    };
    let cache = candidates
        .iter()
        .map(|candidate| cache_record(&context_id, provider, candidate))
        .collect::<Vec<_>>();
    state
        .store
        .replace_external_session_cache(&context_id, provider.cache_name(), &cache)
        .await
        .map_err(|e| e.to_string())?;
    list_candidates(provider, &state.store, project, candidates).await
}

#[tauri::command]
pub(super) async fn list_codex_sessions(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    context_id: Option<String>,
    refresh: Option<bool>,
) -> Result<Vec<ExternalSessionInfo>, String> {
    let project = state.require_active(window.label())?;
    list_sessions(
        &state,
        &project.id,
        context_id,
        refresh,
        ImportProvider::Codex,
    )
    .await
}

#[tauri::command]
pub(super) async fn list_claude_sessions(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    context_id: Option<String>,
    refresh: Option<bool>,
) -> Result<Vec<ExternalSessionInfo>, String> {
    let project = state.require_active(window.label())?;
    list_sessions(
        &state,
        &project.id,
        context_id,
        refresh,
        ImportProvider::Claude,
    )
    .await
}

fn checked_local_session_path(provider: ImportProvider, path: &str) -> Result<PathBuf, String> {
    let root = provider
        .root()
        .ok_or_else(|| "Home directory is unavailable".to_string())?;
    checked_local_session_path_in(provider, &root, path)
}

fn checked_local_session_path_in(
    provider: ImportProvider,
    root: &Path,
    path: &str,
) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cannot open {} session root: {e}", provider.label()))?;
    let path = Path::new(path)
        .canonicalize()
        .map_err(|e| format!("{path}: {e}"))?;
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| format!("{} session is outside {}", provider.label(), root.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if !provider.valid_filename(name)
        || (provider == ImportProvider::Claude && relative.components().count() != 2)
    {
        return Err(format!("Invalid {} session path", provider.label()));
    }
    Ok(path)
}

fn read_local_session(provider: ImportProvider, path: &str) -> Result<String, String> {
    read_bounded_jsonl(&checked_local_session_path(provider, path)?)
}

fn read_local_preview(provider: ImportProvider, path: &str) -> Result<String, String> {
    let path = checked_local_session_path(provider, path)?;
    let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = vec![];
    file.take(CONTEXT_PREVIEW_MAX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn preview_session(
    state: State<'_, AppState>,
    path: String,
    context_id: Option<String>,
    provider: ImportProvider,
) -> Result<Vec<ExternalSessionPreviewLine>, String> {
    let context_id = context_id.unwrap_or_else(|| "local".into());
    let jsonl = if context_id == "local" {
        tokio::task::spawn_blocking(move || read_local_preview(provider, &path))
            .await
            .map_err(|e| format!("{} preview task failed: {e}", provider.label()))??
    } else {
        let context = state
            .store
            .get_execution_context(&context_id)
            .await
            .map_err(|e| e.to_string())?
            .filter(|context| context.kind != ExecutionContextKind::Local)
            .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
        tokio::task::spawn_blocking(move || {
            let mut runner = crate::context_probe::ProcessProbeRunner;
            read_context_preview_with_runner(provider, &context, &path, &mut runner)
        })
        .await
        .map_err(|e| format!("{} preview task failed: {e}", provider.label()))??
    };
    Ok(preview_lines(provider, &jsonl))
}

#[tauri::command]
pub(super) async fn preview_codex_session(
    state: State<'_, AppState>,
    path: String,
    context_id: Option<String>,
) -> Result<Vec<ExternalSessionPreviewLine>, String> {
    preview_session(state, path, context_id, ImportProvider::Codex).await
}

#[tauri::command]
pub(super) async fn preview_claude_session(
    state: State<'_, AppState>,
    path: String,
    context_id: Option<String>,
) -> Result<Vec<ExternalSessionPreviewLine>, String> {
    preview_session(state, path, context_id, ImportProvider::Claude).await
}

async fn import_sessions(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    paths: Vec<String>,
    context_id: Option<String>,
    provider: ImportProvider,
) -> Result<ExternalImportSummary, String> {
    let ap = state.require_active(window.label())?;
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
    let mut summary = ExternalImportSummary::default();
    for path in paths {
        let loaded = match context.clone() {
            Some(context) => {
                let remote_path = path.clone();
                tokio::task::spawn_blocking(move || {
                    let mut runner = crate::context_probe::ProcessProbeRunner;
                    read_context_jsonl_with_runner(provider, &context, &remote_path, &mut runner)
                })
                .await
                .map_err(|e| format!("{} import task failed: {e}", provider.label()))?
            }
            None => read_local_session(provider, &path),
        };
        let source_path = if context_id == "local" {
            path.clone()
        } else {
            format!("{context_id}:{path}")
        };
        let outcome = match loaded {
            Ok(jsonl) => {
                import_session_jsonl(
                    provider,
                    &state.store,
                    &ap.id,
                    &model_id,
                    &source_path,
                    &jsonl,
                )
                .await
            }
            Err(error) => Err(error),
        };
        match outcome {
            Ok(result) => {
                match result.outcome {
                    "imported" => summary.imported += 1,
                    "updated" => summary.updated += 1,
                    _ => summary.skipped += 1,
                }
                let _ = state
                    .store
                    .mark_external_session_cache_synced(
                        &context_id,
                        provider.cache_name(),
                        &path,
                        result.message_count,
                        result.last_active_at_ms,
                    )
                    .await;
                summary.synced_paths.push(path);
            }
            Err(error) => {
                tracing::warn!("{} import failed: {error}", provider.label());
                summary.failed += 1;
            }
        }
    }
    Ok(summary)
}

async fn source_context(store: &Store, id: &str) -> Result<Option<ExecutionContext>, String> {
    let kind = ExecutionContextKind::from_id(id).map_err(|e| e.to_string())?;
    if id == "local" {
        return Ok(None);
    }
    store
        .get_execution_context(id)
        .await
        .map_err(|e| e.to_string())?
        .filter(|c| c.kind == kind)
        .map(Some)
        .ok_or_else(|| "Session source is no longer registered".into())
}

fn native_import_sources(
    contexts: Vec<ExecutionContext>,
) -> Vec<wisp_dto::native_session_import::ExternalSource> {
    use wisp_dto::native_session_import::ExternalSource;
    let mut sources: Vec<_> = contexts
        .into_iter()
        .filter(|c| ExecutionContextKind::from_id(&c.id).ok() == Some(c.kind))
        .map(|c| ExternalSource {
            label: if c.label.trim().is_empty() {
                c.id.clone()
            } else {
                c.label
            },
            id: c.id,
            kind: c.kind.as_str().into(),
        })
        .collect();
    if !sources.iter().any(|s| s.id == "local") {
        sources.insert(
            0,
            ExternalSource {
                id: "local".into(),
                label: "Local".into(),
                kind: "local".into(),
            },
        );
    }
    sources
}

async fn read_native_source(
    store: &Store,
    provider: ImportProvider,
    context_id: &str,
    path: &str,
) -> Result<String, String> {
    let context = source_context(store, context_id).await?;
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || match context {
        Some(context) => read_context_jsonl_with_runner(
            provider,
            &context,
            &path,
            &mut crate::context_probe::ProcessProbeRunner,
        ),
        None => read_local_session(provider, &path),
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn native_preview_from_jsonl(
    store: &Store,
    project: &str,
    provider: ImportProvider,
    context: &str,
    path: &str,
    jsonl: &str,
) -> Result<wisp_dto::native_session_import::ExternalPreview, String> {
    use wisp_dto::native_session_import::{ExternalPreview, SCHEMA};
    let parsed = parse_jsonl(provider, jsonl);
    if parsed.session_id.trim().is_empty() || parsed.messages.is_empty() {
        return Err("The source contains no importable conversation messages".into());
    }
    Ok(ExternalPreview {
        schema: SCHEMA.into(),
        project_id: project.into(),
        provider: provider.cache_name().into(),
        context_id: context.into(),
        path: path.into(),
        existing_session_id: existing_import(
            store,
            project,
            &provider.import_key(&parsed.session_id),
        )
        .await?,
        source_session_id: parsed.session_id,
        sha256: wisp_sync::sha256_hex(jsonl.as_bytes()),
        message_count: parsed.messages.len(),
        messages: preview_lines(provider, jsonl),
    })
}

fn validate_native_review(
    preview: &wisp_dto::native_session_import::ExternalPreview,
    request: &wisp_dto::native_session_import::ExternalImportRequest,
) -> Result<(), String> {
    if preview.provider != request.provider
        || preview.context_id != request.context_id
        || preview.path != request.path
        || preview.source_session_id != request.source_session_id
        || preview.sha256 != request.sha256
    {
        return Err(
            "The conversation source changed after preview. Review it again before importing."
                .into(),
        );
    }
    Ok(())
}

/// Explicit project/source transport; no active WebView selection or implicit retargeting.
pub(crate) async fn execute_native(
    state: &AppState,
    request: &wisp_dto::native_settings::Request,
) -> Result<serde_json::Value, String> {
    use wisp_dto::native_session_import::*;
    let project = request
        .project_id
        .as_deref()
        .filter(|p| !p.is_empty() && p.trim() == *p)
        .ok_or("An explicit destination project is required")?;
    if wisp_store::is_assistant_project_id(project) {
        return Err("Choose a regular destination project".into());
    }
    state
        .store
        .get_project(project)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Destination project not found")?;
    match request.command.as_str() {
        "native_external_session_sources" => {
            let sources = native_import_sources(
                state
                    .store
                    .list_execution_contexts()
                    .await
                    .map_err(|e| e.to_string())?,
            );
            serde_json::to_value(ExternalSources {
                schema: SCHEMA.into(),
                project_id: project.into(),
                sources,
            })
            .map_err(|e| e.to_string())
        }
        "native_external_session_list" => {
            let input: ExternalListRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let provider = ImportProvider::parse(&input.provider)?;
            source_context(&state.store, &input.context_id).await?;
            let items = list_sessions(
                state,
                project,
                Some(input.context_id.clone()),
                Some(input.refresh),
                provider,
            )
            .await?;
            serde_json::to_value(ExternalList {
                schema: SCHEMA.into(),
                project_id: project.into(),
                provider: input.provider,
                context_id: input.context_id,
                items,
            })
            .map_err(|e| e.to_string())
        }
        "native_external_session_preview" => {
            let input: ExternalPreviewRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let provider = ImportProvider::parse(&input.provider)?;
            let jsonl =
                read_native_source(&state.store, provider, &input.context_id, &input.path).await?;
            serde_json::to_value(
                native_preview_from_jsonl(
                    &state.store,
                    project,
                    provider,
                    &input.context_id,
                    &input.path,
                    &jsonl,
                )
                .await?,
            )
            .map_err(|e| e.to_string())
        }
        "native_external_session_import" => {
            let input: ExternalImportRequest =
                serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
            let provider = ImportProvider::parse(&input.provider)?;
            let jsonl =
                read_native_source(&state.store, provider, &input.context_id, &input.path).await?;
            let reviewed = native_preview_from_jsonl(
                &state.store,
                project,
                provider,
                &input.context_id,
                &input.path,
                &jsonl,
            )
            .await?;
            validate_native_review(&reviewed, &input)?;
            let _project_guard = state.begin_project_exclusive_activity(project)?;
            state
                .store
                .get_project(project)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Destination project not found")?;
            crate::exploration_commands::require_writable_scope(
                &state.store,
                &wisp_store::StateScope::mainline(project),
            )
            .await?;
            let existing = existing_import(
                &state.store,
                project,
                &provider.import_key(&input.source_session_id),
            )
            .await?;
            let (runtime, _workflow) = crate::session_import::lock_native_import_target(
                state,
                project,
                existing.as_deref(),
            )
            .await?;
            let model = super::models::active_profile_id(&state.store).await;
            let path = if input.context_id == "local" {
                input.path.clone()
            } else {
                format!("{}:{}", input.context_id, input.path)
            };
            let result =
                import_session_jsonl(provider, &state.store, project, &model, &path, &jsonl)
                    .await?;
            if let Some(rt) = runtime {
                *rt.agent.lock().await = None;
                rt.sync_last_seq_from_store(&state.store, &result.frame_id)
                    .await?;
            }
            let _ = state
                .store
                .mark_external_session_cache_synced(
                    &input.context_id,
                    provider.cache_name(),
                    &input.path,
                    result.message_count,
                    result.last_active_at_ms,
                )
                .await;
            serde_json::to_value(ExternalResult {
                schema: SCHEMA.into(),
                project_id: project.into(),
                provider: input.provider,
                context_id: input.context_id,
                path: input.path,
                source_session_id: input.source_session_id,
                frame_id: result.frame_id,
                status: result.outcome.into(),
                message_count: result.message_count as usize,
            })
            .map_err(|e| e.to_string())
        }
        _ => Err("Unsupported external conversation command".into()),
    }
}

#[tauri::command]
pub(super) async fn import_codex_sessions(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    paths: Vec<String>,
    context_id: Option<String>,
) -> Result<ExternalImportSummary, String> {
    import_sessions(state, window, paths, context_id, ImportProvider::Codex).await
}

#[tauri::command]
pub(super) async fn import_claude_sessions(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    paths: Vec<String>,
    context_id: Option<String>,
) -> Result<ExternalImportSummary, String> {
    import_sessions(state, window, paths, context_id, ImportProvider::Claude).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_probe::{ProbeCommand, ProbeCommandOutput, ProbeRunner};

    #[test]
    fn native_external_sources_only_offer_contexts_the_reader_accepts() {
        let mut alias = ExecutionContext::new("local", "Display alias").unwrap();
        alias.id = "display-only-local".into();
        let mut mismatch = ExecutionContext::new("ssh:wrong-kind", "Wrong kind").unwrap();
        mismatch.kind = ExecutionContextKind::Wsl;
        let wsl = ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
        let mut ssh = ExecutionContext::new("ssh:analysis", "Analysis").unwrap();
        ssh.label.clear();
        let sources = native_import_sources(vec![alias, mismatch, wsl, ssh]);
        assert_eq!(
            sources.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["local", "wsl:Ubuntu", "ssh:analysis"]
        );
        assert_eq!(sources[2].label, "ssh:analysis");
        let local = ExecutionContext::new("local", "This computer").unwrap();
        let sources = native_import_sources(vec![local]);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].label, "This computer");
    }

    #[tokio::test]
    async fn native_external_preview_scopes_imports_and_rejects_changed_sources() {
        use wisp_dto::native_session_import::ExternalImportRequest;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
        for project in ["a", "b"] {
            store.create_project(project, project, "").await.unwrap();
        }
        for (provider, jsonl) in [
            (ImportProvider::Codex, CODEX_JSONL),
            (ImportProvider::Claude, CLAUDE_JSONL),
        ] {
            let preview =
                native_preview_from_jsonl(&store, "a", provider, "local", "synthetic.jsonl", jsonl)
                    .await
                    .unwrap();
            assert!(preview.existing_session_id.is_none());
            assert!(
                store.list_project_frame_ids("a").await.unwrap().is_empty()
                    || provider == ImportProvider::Claude
            );
            let input = ExternalImportRequest {
                provider: provider.cache_name().into(),
                context_id: "local".into(),
                path: "synthetic.jsonl".into(),
                source_session_id: preview.source_session_id.clone(),
                sha256: preview.sha256.clone(),
            };
            validate_native_review(&preview, &input).unwrap();
            let changed = native_preview_from_jsonl(
                &store,
                "a",
                provider,
                "local",
                "synthetic.jsonl",
                &format!("{jsonl}\n"),
            )
            .await
            .unwrap();
            assert!(validate_native_review(&changed, &input).is_err());
            let wrong_source = native_preview_from_jsonl(
                &store,
                "a",
                provider,
                "ssh:analysis",
                "synthetic.jsonl",
                jsonl,
            )
            .await
            .unwrap();
            assert!(validate_native_review(&wrong_source, &input).is_err());
            let first =
                import_session_jsonl(provider, &store, "a", "fake", "synthetic.jsonl", jsonl)
                    .await
                    .unwrap();
            let second =
                import_session_jsonl(provider, &store, "b", "fake", "synthetic.jsonl", jsonl)
                    .await
                    .unwrap();
            assert_eq!(first.outcome, "imported");
            assert_eq!(second.outcome, "imported");
            assert_ne!(first.frame_id, second.frame_id);
            assert_eq!(
                store
                    .frame_project_id(&second.frame_id)
                    .await
                    .unwrap()
                    .as_deref(),
                Some("b")
            );
            let again =
                import_session_jsonl(provider, &store, "a", "fake", "synthetic.jsonl", jsonl)
                    .await
                    .unwrap();
            assert_eq!(again.frame_id, first.frame_id);
            assert_eq!(again.outcome, "skipped");
        }
        assert_eq!(store.list_project_frame_ids("a").await.unwrap().len(), 2);
        assert_eq!(store.list_project_frame_ids("b").await.unwrap().len(), 2);
        assert!(source_context(&store, "ssh:missing").await.is_err());
        assert!(source_context(&store, " local").await.is_err());
        assert!(source_context(&store, "local").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn native_external_legacy_mapping_and_cached_status_respect_project_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.sqlite")).await.unwrap();
        for project in ["a", "b"] {
            store.create_project(project, project, "").await.unwrap();
        }
        store
            .create_frame("legacy", "a", "Codex", "fake")
            .await
            .unwrap();
        store
            .append_message("legacy", 1, &Message::user("Original legacy question"))
            .await
            .unwrap();
        store
            .record_codex_import("codex-abc", "legacy", "legacy.jsonl")
            .await
            .unwrap();
        let stamp = FileStamp {
            path: "/source/rollout-abc.jsonl".into(),
            size: CODEX_JSONL.len() as i64,
            modified_at_ms: 1,
        };
        let candidate = SessionCandidate {
            path: stamp.path.clone(),
            file_size: stamp.size,
            modified_at_ms: stamp.modified_at_ms,
            metadata: metadata_from_jsonl(ImportProvider::Codex, CODEX_JSONL, &stamp).unwrap(),
            changed_since_import: false,
        };
        let cached = cache_record("wsl:Ubuntu", ImportProvider::Codex, &candidate);
        store
            .replace_external_session_cache("wsl:Ubuntu", "codex", &[cached.clone()])
            .await
            .unwrap();
        let a = list_candidates(
            ImportProvider::Codex,
            &store,
            "a",
            vec![candidate_from_cache(&cached)],
        )
        .await
        .unwrap();
        let b = list_candidates(
            ImportProvider::Codex,
            &store,
            "b",
            vec![candidate_from_cache(&cached)],
        )
        .await
        .unwrap();
        assert_eq!(a[0].state, "updatable");
        assert_eq!(b[0].state, "new");
        let other = import_session_jsonl(
            ImportProvider::Codex,
            &store,
            "b",
            "fake",
            "new.jsonl",
            CODEX_JSONL,
        )
        .await
        .unwrap();
        assert_ne!(other.frame_id, "legacy");
        assert_eq!(store.message_count("legacy").await.unwrap(), 1);
        let legacy = import_session_jsonl(
            ImportProvider::Codex,
            &store,
            "a",
            "fake",
            "new.jsonl",
            CODEX_JSONL,
        )
        .await
        .unwrap();
        assert_eq!(legacy.frame_id, "legacy");
        assert_eq!(legacy.outcome, "updated");
        assert_eq!(store.message_count(&other.frame_id).await.unwrap(), 2);
        let a = list_candidates(
            ImportProvider::Codex,
            &store,
            "a",
            vec![candidate_from_cache(&cached)],
        )
        .await
        .unwrap();
        assert_eq!(a[0].state, "imported");
    }

    #[test]
    fn native_external_local_preview_cannot_escape_the_provider_root_or_import_subagents() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        std::fs::create_dir_all(root.join("project/subagents")).unwrap();
        let regular = root.join("project/chat.jsonl");
        let subagent = root.join("project/subagents/agent.jsonl");
        let outside = dir.path().join("rollout-outside.jsonl");
        for path in [&regular, &subagent, &outside] {
            std::fs::write(path, CLAUDE_JSONL).unwrap();
        }
        assert!(checked_local_session_path_in(
            ImportProvider::Claude,
            &root,
            regular.to_str().unwrap()
        )
        .is_ok());
        assert!(checked_local_session_path_in(
            ImportProvider::Claude,
            &root,
            subagent.to_str().unwrap()
        )
        .is_err());
        assert!(checked_local_session_path_in(
            ImportProvider::Codex,
            &root,
            outside.to_str().unwrap()
        )
        .is_err());
        assert!(checked_local_session_path_in(
            ImportProvider::Codex,
            &root,
            regular.to_str().unwrap()
        )
        .is_err());
    }

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
    const CLAUDE_JSONL: &str = concat!(
        r#"{"sessionId":"claude-abc","cwd":"/home/me/project","timestamp":"2026-05-31T10:00:00.000Z","type":"user","isMeta":true,"message":{"role":"user","content":"metadata"}}"#,
        "\n",
        r#"{"sessionId":"claude-abc","cwd":"/home/me/project","timestamp":"2026-05-31T10:01:00.000Z","type":"user","message":{"role":"user","content":"Fix tests"}}"#,
        "\n",
        r#"{"sessionId":"claude-abc","cwd":"/home/me/project","timestamp":"2026-05-31T10:02:00.000Z","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I will inspect it."},{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#,
        "\n",
        r#"{"sessionId":"claude-abc","cwd":"/home/me/project","timestamp":"2026-05-31T10:03:00.000Z","type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1","content":[{"type":"text","text":"file contents"}]}]}}"#,
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

    #[test]
    fn codex_title_strips_generated_request_wrappers() {
        let wrapped = concat!(
            "<recommended_plugins>\n- GitHub\n</recommended_plugins>\n",
            "# Files mentioned by the user:\n- screenshot.png\n",
            "## My request:\n修复导入标题并增加搜索\n",
            "<image name=[Image #1] path=\"screenshot.png\">\n</image>"
        );
        assert_eq!(
            codex_title_text(wrapped).as_deref(),
            Some("修复导入标题并增加搜索")
        );
        assert!(
            codex_title_text("<recommended_plugins>\n- GitHub\n</recommended_plugins>").is_none()
        );
    }

    #[test]
    fn parses_claude_text_tools_and_meta_lines() {
        let parsed = parse_claude_jsonl(CLAUDE_JSONL);
        assert_eq!(parsed.session_id, "claude-abc");
        assert_eq!(parsed.cwd, "/home/me/project");
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(parsed.messages[0].text, "Fix tests");
        assert_eq!(parsed.messages[1].role, Role::Assistant);
        assert_eq!(parsed.messages[1].tool_calls.len(), 1);
        assert_eq!(parsed.messages[1].tool_calls[0].function.name, "Read");
        assert_eq!(parsed.messages[2].role, Role::Tool);
        assert_eq!(parsed.messages[2].tool_call_id.as_deref(), Some("tool-1"));
        assert_eq!(parsed.messages[2].tool_name.as_deref(), Some("Read"));
        assert_eq!(parsed.messages[2].text, "file contents");
    }

    #[test]
    fn metadata_message_count_hint_survives_a_bounded_prefix() {
        let stamp = FileStamp {
            path: "/home/me/.codex/sessions/2026/05/rollout-hinted.jsonl".into(),
            size: 1,
            modified_at_ms: 2,
        };
        let jsonl = format!(
            "{}\n{{\"type\":\"wisp_metadata\",\"message_count\":17}}",
            CODEX_JSONL.lines().next().unwrap()
        );
        let metadata = metadata_from_jsonl(ImportProvider::Codex, &jsonl, &stamp).unwrap();
        assert_eq!(metadata.session_id, "codex-abc");
        assert_eq!(metadata.message_count, 17);
    }

    struct FakeProbeRunner {
        outputs: Vec<ProbeCommandOutput>,
        commands: Vec<ProbeCommand>,
    }

    impl ProbeRunner for FakeProbeRunner {
        fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
            self.commands.push(command.clone());
            if self.outputs.is_empty() {
                return Err("unexpected probe command".into());
            }
            Ok(self.outputs.remove(0))
        }
    }

    fn output(stdout: String) -> ProbeCommandOutput {
        ProbeCommandOutput {
            status: 0,
            stdout: stdout.into_bytes(),
            stderr: String::new(),
        }
    }

    fn framed_listing(path: &str, jsonl: &str) -> String {
        format!(
            "login banner\n{CONTEXT_SCAN_PROTOCOL}1780221780.0\t{}\t{path}\n",
            jsonl.len()
        )
    }

    fn framed_metadata(path: &str, jsonl: &str) -> String {
        let size = jsonl.len();
        let mtime = "1780221780.0";
        format!(
            "login banner\n{CONTEXT_METADATA_PROTOCOL}{path}\0{size}\0{mtime}\0{size}\0{jsonl}\0"
        )
    }

    #[test]
    fn metadata_frames_keep_alignment_when_utf8_is_cut_at_the_prefix_limit() {
        fn push_frame(bytes: &mut Vec<u8>, path: &str, metadata: &[u8]) {
            for field in [
                path.to_string(),
                metadata.len().to_string(),
                "1780221780.0".to_string(),
                metadata.len().to_string(),
            ] {
                bytes.extend_from_slice(field.as_bytes());
                bytes.push(0);
            }
            bytes.extend_from_slice(metadata);
        }

        let mut truncated = CLAUDE_JSONL.as_bytes().to_vec();
        truncated.resize(METADATA_PREFIX_BYTES as usize - 2, b' ');
        truncated.extend_from_slice(&[0xe4, 0xb8]);
        assert!(std::str::from_utf8(&truncated).is_err());

        let first_path = "/home/me/.claude/projects/-home-me-first/first.jsonl";
        let second_path = "/home/me/.claude/projects/-home-me-second/second.jsonl";
        let mut stdout = format!("banner\n{CONTEXT_METADATA_PROTOCOL}").into_bytes();
        push_frame(&mut stdout, first_path, &truncated);
        push_frame(&mut stdout, second_path, CLAUDE_JSONL.as_bytes());
        stdout.push(0);

        let candidates = parse_context_metadata(ImportProvider::Claude, &stdout, &[]).unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].path, first_path);
        assert_eq!(candidates[0].metadata.title, "Fix tests");
        assert_eq!(candidates[1].path, second_path);
    }

    #[cfg(unix)]
    #[test]
    fn remote_scan_scripts_are_valid_posix_shell() {
        for provider in [ImportProvider::Codex, ImportProvider::Claude] {
            for script in [
                context_scan_script(provider),
                context_metadata_script(provider, true),
                context_metadata_script(provider, false),
                context_preview_script(
                    provider,
                    match provider {
                        ImportProvider::Codex => {
                            "/home/me/.codex/sessions/2026/05/rollout-test.jsonl"
                        }
                        ImportProvider::Claude => "/home/me/.claude/projects/-home-me/test.jsonl",
                    },
                )
                .unwrap(),
            ] {
                assert!(std::process::Command::new("sh")
                    .args(["-n", "-c", &script])
                    .status()
                    .unwrap()
                    .success());
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn remote_metadata_script_extracts_title_without_transferring_context() {
        let home = std::env::temp_dir().join(format!("codex_remote_scan_{}", uuid::Uuid::new_v4()));
        let dir = home.join(".codex/sessions/2026/05/31");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-large.jsonl");
        let header = serde_json::json!({
            "type": "session_meta",
            "payload": {"id": "remote-large", "cwd": "/work/remote"}
        });
        let context = serde_json::json!({
            "type": "response_item",
            "payload": {"type": "message", "role": "developer", "content": [
                {"type": "input_text", "text": "x".repeat(128 * 1024)}
            ]}
        });
        let title = serde_json::json!({
            "type": "event_msg",
            "payload": {"type": "user_message", "message": "Remote real prompt"}
        });
        let transcript = format!("{header}\n{context}\n{title}\n");
        std::fs::write(&path, &transcript).unwrap();

        let output = std::process::Command::new("sh")
            .args(["-c", &context_metadata_script(ImportProvider::Codex, true)])
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.len() < transcript.len());
        let candidates =
            parse_context_metadata(ImportProvider::Codex, &output.stdout, &[]).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].metadata.session_id, "remote-large");
        assert_eq!(candidates[0].metadata.title, "Remote real prompt");
        assert!(candidates[0].changed_since_import);

        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn scans_wsl_and_ssh_sources_with_fake_commands() {
        let path = "/home/me/.codex/sessions/2026/05/rollout-codex-abc.jsonl";
        let outputs = vec![
            output(framed_listing(path, CODEX_JSONL)),
            output(framed_metadata(path, CODEX_JSONL)),
        ];

        let mut wsl = ExecutionContext::new("wsl:Ubuntu-24.04", "Ubuntu-24.04").unwrap();
        wsl.config_json = r#"{"distro":"Ubuntu-24.04"}"#.into();
        let mut wsl_runner = FakeProbeRunner {
            outputs: outputs.clone(),
            commands: vec![],
        };
        let candidates =
            context_candidates_with_runner(ImportProvider::Codex, &wsl, &[], &mut wsl_runner)
                .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, path);
        assert_eq!(candidates[0].metadata.session_id, "codex-abc");
        assert_eq!(wsl_runner.commands.len(), 2);
        assert_eq!(wsl_runner.commands[0].program, "wsl.exe");
        assert_eq!(wsl_runner.commands[0].args[2], "--exec");
        assert!(wsl_runner.commands[1]
            .args
            .last()
            .is_some_and(|script| script.contains("awk '") && !script.contains("cat ")));

        let mut ssh = ExecutionContext::new("ssh:gpu-server", "gpu-server").unwrap();
        ssh.config_json = r#"{"alias":"gpu-server"}"#.into();
        ssh.last_probe_status = Some("ok".into());
        let mut ssh_runner = FakeProbeRunner {
            outputs,
            commands: vec![],
        };
        let candidates =
            context_candidates_with_runner(ImportProvider::Codex, &ssh, &[], &mut ssh_runner)
                .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(ssh_runner.commands[0].program, "ssh");
        assert!(ssh_runner.commands[0]
            .args
            .last()
            .is_some_and(|script| script.contains("$HOME/.codex/sessions")));

        let claude_path = "/home/me/.claude/projects/-home-me-project/claude-abc.jsonl";
        let mut claude_runner = FakeProbeRunner {
            outputs: vec![
                output(framed_listing(claude_path, CLAUDE_JSONL)),
                output(framed_metadata(claude_path, CLAUDE_JSONL)),
            ],
            commands: vec![],
        };
        let candidates =
            context_candidates_with_runner(ImportProvider::Claude, &wsl, &[], &mut claude_runner)
                .unwrap();
        assert_eq!(candidates[0].metadata.title, "Fix tests");
        assert_eq!(claude_runner.commands[0].args[2], "--exec");
        assert!(claude_runner.commands[0]
            .args
            .last()
            .is_some_and(|script| script.contains("$HOME/.claude/projects")));
        assert!(validate_context_path(
            ImportProvider::Claude,
            "/home/me/.claude/projects/-home-me-project/subagents/agent-child.jsonl"
        )
        .is_err());
    }

    #[test]
    fn metadata_prefix_keeps_a_large_codex_session_header() {
        let dir = std::env::temp_dir().join(format!("codex_large_header_{}", uuid::Uuid::new_v4()));
        let sessions = dir.join("2026/05/31");
        std::fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("rollout-large.jsonl");
        let header = serde_json::json!({
            "type": "session_meta",
            "timestamp": "2026-05-31T10:00:00Z",
            "payload": {
                "id": "large-header",
                "cwd": "/work/large",
                "instructions": "x".repeat(20 * 1024),
            }
        });
        let developer = serde_json::json!({
            "type": "response_item",
            "timestamp": "2026-05-31T10:01:00Z",
            "payload": {
                "type": "message",
                "role": "developer",
                "content": [{"type": "input_text", "text": "y".repeat(40 * 1024)}],
            }
        });
        let title = serde_json::json!({
            "type": "event_msg",
            "timestamp": "2026-05-31T10:02:00Z",
            "payload": {
                "type": "user_message",
                "message": "Real prompt",
            }
        });
        std::fs::write(&path, format!("{header}\n{developer}\n{title}\n")).unwrap();

        let candidates = local_candidates(ImportProvider::Codex, &dir, &[]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].metadata.session_id, "large-header");
        assert_eq!(candidates[0].metadata.title, "Real prompt");
        assert!(candidates[0].changed_since_import);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn local_metadata_counts_current_codex_messages_after_large_context() {
        let dir =
            std::env::temp_dir().join(format!("codex_current_metadata_{}", uuid::Uuid::new_v4()));
        let sessions = dir.join("2026/09/21");
        std::fs::create_dir_all(&sessions).unwrap();
        let path = sessions.join("rollout-current.jsonl");
        let header = serde_json::json!({
            "type": "session_meta",
            "timestamp": "2026-09-21T10:00:00Z",
            "payload": {
                "id": "current-metadata",
                "cwd": "C:/work/project",
                "instructions": "x".repeat(40 * 1024),
            }
        });
        let user = serde_json::json!({
            "type": "response_item",
            "timestamp": "2026-09-21T10:01:00Z",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Find the root tip"}],
            }
        });
        let assistant = serde_json::json!({
            "type": "response_item",
            "timestamp": "2026-09-21T10:02:00Z",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "I found it."}],
            }
        });
        std::fs::write(&path, format!("{header}\n{user}\n{assistant}\n")).unwrap();

        let candidates = local_candidates(ImportProvider::Codex, &dir, &[]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].metadata.session_id, "current-metadata");
        assert_eq!(candidates[0].metadata.title, "Find the root tip");
        assert_eq!(candidates[0].metadata.message_count, 2);

        // Repair an unchanged file whose old prefix-only scan was cached.
        let mut stale = cache_record("local", ImportProvider::Codex, &candidates[0]);
        stale.title = "project".into();
        stale.message_count = 0;
        let repaired = local_candidates(ImportProvider::Codex, &dir, &[stale]);
        assert_eq!(repaired[0].metadata.title, "Find the root tip");
        assert_eq!(repaired[0].metadata.message_count, 2);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unchanged_remote_stamp_uses_cache_without_reading_metadata() {
        let path = "/home/me/.codex/sessions/2026/05/rollout-codex-abc.jsonl";
        let cache = ExternalSessionCacheRecord {
            source_id: "wsl:Ubuntu".into(),
            provider: "codex".into(),
            source_path: path.into(),
            file_size: CODEX_JSONL.len() as i64,
            modified_at_ms: 1780221780000,
            session_id: "codex-abc".into(),
            title: "Cached title".into(),
            cwd: "/work".into(),
            message_count: 2,
            created_at_ms: 1,
            last_active_at_ms: 2,
            changed_since_import: false,
        };
        let wsl = ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
        let mut runner = FakeProbeRunner {
            outputs: vec![output(framed_listing(path, CODEX_JSONL))],
            commands: vec![],
        };
        let candidates =
            context_candidates_with_runner(ImportProvider::Codex, &wsl, &[cache], &mut runner)
                .unwrap();
        assert_eq!(candidates[0].metadata.title, "Cached title");
        assert_eq!(runner.commands.len(), 1);
    }

    #[test]
    fn context_file_read_validates_path_and_strips_banner() {
        let path = "/home/me/.codex/sessions/2026/05/rollout-codex-abc.jsonl";
        let wsl = ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
        let mut runner = FakeProbeRunner {
            outputs: vec![output(format!(
                "banner\n{CONTEXT_FILE_PROTOCOL}{CODEX_JSONL}"
            ))],
            commands: vec![],
        };
        assert_eq!(
            read_context_jsonl_with_runner(ImportProvider::Codex, &wsl, path, &mut runner).unwrap(),
            CODEX_JSONL
        );
        assert!(read_context_jsonl_with_runner(
            ImportProvider::Codex,
            &wsl,
            "/etc/passwd",
            &mut runner
        )
        .unwrap_err()
        .contains("outside"));
    }

    #[test]
    fn preview_keeps_only_the_first_conversation_messages() {
        let codex = preview_lines(ImportProvider::Codex, CODEX_JSONL);
        assert_eq!(
            codex,
            vec![
                ExternalSessionPreviewLine {
                    role: "user".into(),
                    text: "Fix the renderer crash".into(),
                },
                ExternalSessionPreviewLine {
                    role: "assistant".into(),
                    text: "I found \nthe issue.".into(),
                },
            ]
        );

        let claude = preview_lines(ImportProvider::Claude, CLAUDE_JSONL);
        assert_eq!(claude.len(), 2);
        assert_eq!(claude[0].role, "user");
        assert_eq!(claude[0].text, "Fix tests");
        assert_eq!(claude[1].role, "assistant");
    }

    #[test]
    fn remote_preview_is_bounded_and_uses_the_selected_path() {
        let path = "/home/me/.codex/sessions/2026/05/rollout-codex-abc.jsonl";
        let wsl = ExecutionContext::new("wsl:Ubuntu", "Ubuntu").unwrap();
        let mut runner = FakeProbeRunner {
            outputs: vec![output(format!(
                "banner\n{CONTEXT_PREVIEW_PROTOCOL}{CODEX_JSONL}"
            ))],
            commands: vec![],
        };
        let preview =
            read_context_preview_with_runner(ImportProvider::Codex, &wsl, path, &mut runner)
                .unwrap();
        assert_eq!(preview, CODEX_JSONL);
        let script = runner.commands[0].args.last().unwrap();
        assert!(script.contains(path));
        assert!(script.contains(&CONTEXT_PREVIEW_MAX_BYTES.to_string()));
        assert!(script.contains("head -c"));
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
            import_session_file(ImportProvider::Codex, &store, "p", "m", &rollout)
                .await
                .unwrap(),
            "imported"
        );
        let frame_id = existing_import(&store, "p", "codex-abc")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.message_count(&frame_id).await.unwrap(), 2);
        let sessions = store.list_sessions("p").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].1, "Fix the renderer crash");
        assert_eq!(sessions[0].2, 1780221780); // latest imported Codex activity
        let folders = store.list_folders("p").await.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].1, "codex");
        assert_eq!(sessions[0].3.as_deref(), Some(folders[0].0.as_str()));

        // Unchanged rollout → idempotent skip.
        assert_eq!(
            import_session_file(ImportProvider::Codex, &store, "p", "m", &rollout)
                .await
                .unwrap(),
            "skipped"
        );

        // Codex side grew → fast-forward the frame.
        let grown = format!(
            "{CODEX_JSONL}{}\n",
            r#"{"type":"response_item","timestamp":"2026-05-31T10:10:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"thanks"}]}}"#
        );
        std::fs::write(&rollout, &grown).unwrap();
        assert_eq!(
            import_session_file(ImportProvider::Codex, &store, "p", "m", &rollout)
                .await
                .unwrap(),
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
            import_session_file(ImportProvider::Codex, &store, "p", "m", &rollout)
                .await
                .unwrap(),
            "skipped"
        );
        assert_eq!(store.message_count(&frame_id).await.unwrap(), 6);

        let listed = list_sessions_in(ImportProvider::Codex, &store, &dir).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "codex-abc");
        assert_eq!(listed[0].state, "imported");
        assert_eq!(listed[0].title, "Fix the renderer crash");

        drop(store);
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn claude_import_creates_namespaced_mapping_and_group() {
        let (store, db_path) = temp_store().await;
        let dir = std::env::temp_dir().join(format!("claude_projects_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("-home-me-project")).unwrap();
        std::fs::create_dir_all(dir.join("-home-me-project/subagents")).unwrap();
        let transcript = dir.join("-home-me-project/claude-abc.jsonl");
        std::fs::write(&transcript, CLAUDE_JSONL).unwrap();
        std::fs::write(
            dir.join("-home-me-project/subagents/agent-child.jsonl"),
            CLAUDE_JSONL,
        )
        .unwrap();

        assert_eq!(
            import_session_file(ImportProvider::Claude, &store, "p", "m", &transcript)
                .await
                .unwrap(),
            "imported"
        );
        let frame_id = existing_import(&store, "p", "claude:claude-abc")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(store.message_count(&frame_id).await.unwrap(), 3);
        let folders = store.list_folders("p").await.unwrap();
        assert_eq!(folders[0].1, "claude");
        let listed = list_sessions_in(ImportProvider::Claude, &store, &dir).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Fix tests");

        drop(store);
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_dir_all(dir);
    }
}
