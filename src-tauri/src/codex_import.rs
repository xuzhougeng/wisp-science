//! Import Codex CLI and Claude Code JSONL conversations into Wisp sessions.
//! Re-imports are idempotent via the existing `codex_imports` table; Claude
//! session ids are namespaced so they cannot collide with Codex thread ids.

use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use tauri::State;
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
const MAX_METADATA_FRAME_BYTES: u64 = METADATA_PREFIX_BYTES + CODEX_TITLE_PREVIEW_BYTES as u64 + 1;
const MAX_SCAN_FILES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportProvider {
    Codex,
    Claude,
}

impl ImportProvider {
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
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = vec![];
    file.take(METADATA_PREFIX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if provider == ImportProvider::Codex {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut reader = BufReader::new(file.take(CODEX_TITLE_SEARCH_BYTES));
        let mut line = vec![];
        while reader
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("{}: {e}", path.display()))?
            > 0
        {
            let is_user_event = [
                b"\"type\":\"event_msg\"".as_slice(),
                b"\"type\":\"user_message\"".as_slice(),
            ]
            .into_iter()
            .all(|needle| line.windows(needle.len()).any(|window| window == needle));
            if is_user_event
                && std::str::from_utf8(&line)
                    .ok()
                    .and_then(codex_event_title)
                    .is_some()
            {
                bytes.push(b'\n');
                bytes.extend(line.iter().take(CODEX_TITLE_PREVIEW_BYTES));
                break;
            }
            line.clear();
        }
    }
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
        .find(|message| message.role == Role::User && !message.text.trim().is_empty())
        .map(|message| message.text.trim().chars().take(120).collect::<String>())
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
        if value.get("type").and_then(serde_json::Value::as_str) != Some("event_msg") {
            return None;
        }
        let payload = value.get("payload")?;
        if payload.get("type").and_then(serde_json::Value::as_str) != Some("user_message") {
            return None;
        }
        payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    })
}

fn metadata_from_jsonl(
    provider: ImportProvider,
    jsonl: &str,
    stamp: &FileStamp,
) -> Option<SessionMetadata> {
    let supplemental_title = (provider == ImportProvider::Codex)
        .then(|| codex_event_title(jsonl))
        .flatten();
    metadata_from_parsed(
        provider,
        &parse_jsonl(provider, jsonl),
        stamp,
        supplemental_title.as_deref(),
    )
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

fn candidates_from_stamps(
    provider: ImportProvider,
    stamps: Vec<FileStamp>,
    cached: &[ExternalSessionCacheRecord],
) -> Vec<SessionCandidate> {
    let cached = cached
        .into_iter()
        .map(|record| (record.source_path.as_str(), record))
        .collect::<HashMap<_, _>>();
    stamps
        .into_iter()
        .filter_map(|stamp| {
            let previous = cached.get(stamp.path.as_str()).copied();
            if let Some(record) = previous.filter(|record| {
                record.file_size == stamp.size && record.modified_at_ms == stamp.modified_at_ms
            }) {
                return Some(candidate_from_cache(record));
            }
            let metadata = read_metadata_preview(provider, Path::new(&stamp.path))
                .ok()
                .and_then(|jsonl| metadata_from_jsonl(provider, &jsonl, &stamp))
                .or_else(|| previous.map(|record| metadata_from_cache(record)))?;
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
    index($0, "\"type\":\"event_msg\"") && index($0, "\"type\":\"user_message\"") {{
      print substr($0, 1, {CODEX_TITLE_PREVIEW_BYTES})
      exit
    }}
  ' "$file" 2>/dev/null
)
fi
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
        validate_context_path(provider, &path)?;
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
        if let Some(record) = previous
            .filter(|record| record.file_size == size && record.modified_at_ms == modified_at_ms)
        {
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
    if stamps.iter().all(|stamp| {
        cache.get(stamp.path.as_str()).is_some_and(|record| {
            stamped
                && record.file_size == stamp.size
                && record.modified_at_ms == stamp.modified_at_ms
        })
    }) {
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

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(super) struct ExternalSessionInfo {
    pub path: String,
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub message_count: usize,
    /// Unix seconds of the last source activity.
    pub last_active_at: i64,
    /// "new" (never imported), "imported" (up to date), or "updatable"
    /// (the source has messages the imported frame does not).
    pub state: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(super) struct ExternalSessionPreviewLine {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub(super) struct ExternalImportSummary {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
    pub failed: usize,
    /// Paths whose final state is known without rescanning the source.
    pub synced_paths: Vec<String>,
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
    candidates: Vec<SessionCandidate>,
) -> Vec<ExternalSessionInfo> {
    let mut out = vec![];
    for candidate in candidates {
        let import_key = provider.import_key(&candidate.metadata.session_id);
        let state = match store.find_codex_import(&import_key).await.ok().flatten() {
            Some(frame_id) => {
                let stored = store.message_count(&frame_id).await.unwrap_or(0);
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
    out
}

#[cfg(test)]
async fn list_sessions_in(
    provider: ImportProvider,
    store: &Store,
    root: &Path,
) -> Vec<ExternalSessionInfo> {
    list_candidates(provider, store, local_candidates(provider, root, &[])).await
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
    let parsed = parse_jsonl(provider, jsonl);
    if parsed.session_id.is_empty() || parsed.messages.is_empty() {
        return Err(format!("{source_path}: no importable messages"));
    }
    let result = |outcome| ImportResult {
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

    if let Some(frame_id) = store
        .find_codex_import(&import_key)
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
            return Ok(result("skipped"));
        }
        store
            .replace_messages(&frame_id, &to_wisp_messages(provider, &parsed))
            .await
            .map_err(|e| e.to_string())?;
        store
            .set_frame_timestamps(&frame_id, created_at, updated_at)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(result("updated"));
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
        .record_codex_import(&import_key, &frame_id, source_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result("imported"))
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
    if !refresh.unwrap_or(false) && !cached.is_empty() {
        return Ok(list_candidates(
            provider,
            &state.store,
            cached.iter().map(candidate_from_cache).collect(),
        )
        .await);
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
    Ok(list_candidates(provider, &state.store, candidates).await)
}

#[tauri::command]
pub(super) async fn list_codex_sessions(
    state: State<'_, AppState>,
    context_id: Option<String>,
    refresh: Option<bool>,
) -> Result<Vec<ExternalSessionInfo>, String> {
    list_sessions(&state, context_id, refresh, ImportProvider::Codex).await
}

#[tauri::command]
pub(super) async fn list_claude_sessions(
    state: State<'_, AppState>,
    context_id: Option<String>,
    refresh: Option<bool>,
) -> Result<Vec<ExternalSessionInfo>, String> {
    list_sessions(&state, context_id, refresh, ImportProvider::Claude).await
}

fn checked_local_session_path(provider: ImportProvider, path: &str) -> Result<PathBuf, String> {
    let root = provider
        .root()
        .ok_or_else(|| "Home directory is unavailable".to_string())?
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
    window: tauri::WebviewWindow,
    paths: Vec<String>,
    context_id: Option<String>,
    provider: ImportProvider,
) -> Result<ExternalImportSummary, String> {
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

#[tauri::command]
pub(super) async fn import_codex_sessions(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    paths: Vec<String>,
    context_id: Option<String>,
) -> Result<ExternalImportSummary, String> {
    import_sessions(state, window, paths, context_id, ImportProvider::Codex).await
}

#[tauri::command]
pub(super) async fn import_claude_sessions(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    paths: Vec<String>,
    context_id: Option<String>,
) -> Result<ExternalImportSummary, String> {
    import_sessions(state, window, paths, context_id, ImportProvider::Claude).await
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
        let frame_id = store
            .find_codex_import("claude:claude-abc")
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
