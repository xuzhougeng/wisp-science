//! Conversations had in Codex CLI or Claude Code on this computer, found in
//! their own folders and brought into a project as ordinary conversations.
//! Ported from the WebView's `src-tauri/src/codex_import.rs` (its parsers,
//! limits and import keys), for this computer only: the WebView also reads
//! them from SSH and WSL contexts, which is not ported. Importing the same
//! conversation again brings in what was added since and nothing else.
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};
use wisp_llm::{Content, FunctionCall, Message, Role, ToolCall};

/// A conversation file larger than this is not read.
const MAX_BYTES: u64 = 32 * 1024 * 1024;
/// How much of a file the list reads for its title.
const PREFIX_BYTES: u64 = 256 * 1024;
/// Files listed per source, newest first.
const MAX_FILES: usize = 200;

/// Where a conversation was had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Codex,
    Claude,
}

impl Source {
    pub const ALL: [Source; 2] = [Source::Codex, Source::Claude];

    pub fn label(self) -> &'static str {
        match self {
            Source::Codex => "Codex",
            Source::Claude => "Claude Code",
        }
    }

    fn root(self, home: &Path) -> PathBuf {
        match self {
            Source::Codex => home.join(".codex").join("sessions"),
            Source::Claude => home.join(".claude").join("projects"),
        }
    }

    fn valid_filename(self, name: &str) -> bool {
        match self {
            Source::Codex => name.starts_with("rollout-") && name.ends_with(".jsonl"),
            Source::Claude => name.ends_with(".jsonl"),
        }
    }

    /// The key an import is recorded under; Claude's ids are namespaced so
    /// they cannot collide with Codex threads. The WebView's.
    pub(crate) fn import_key(self, session_id: &str) -> String {
        match self {
            Source::Codex => session_id.to_string(),
            Source::Claude => format!("claude:{session_id}"),
        }
    }

    /// The group imported conversations are put in.
    pub(crate) fn folder(self) -> &'static str {
        match self {
            Source::Codex => "codex",
            Source::Claude => "claude",
        }
    }

    fn model_name(self) -> &'static str {
        match self {
            Source::Codex => "Codex CLI",
            Source::Claude => "Claude Code",
        }
    }

    fn parse(self, jsonl: &str) -> Parsed {
        match self {
            Source::Codex => parse_codex(jsonl),
            Source::Claude => parse_claude(jsonl),
        }
    }
}

/// The key an import is recorded under in a project. The WebView's.
pub(crate) fn project_key(project: &str, key: &str) -> String {
    format!("project:{}:{project}:{key}", project.len())
}

/// The home folder these tools keep their conversations under.
pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub session_id: String,
    pub cwd: String,
    pub created_at_ms: i64,
    pub last_active_at_ms: i64,
    pub messages: Vec<ParsedMessage>,
}

#[derive(Debug)]
pub(crate) struct ParsedMessage {
    role: Role,
    text: String,
    ts_ms: i64,
    tool_calls: Vec<ToolCall>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
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

/// All `text` fields of a message content value, joined.
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

fn push(
    session: &mut Parsed,
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

fn update_chronology(session: &mut Parsed, ts_ms: i64) {
    if ts_ms <= 0 {
        return;
    }
    if session.created_at_ms == 0 {
        session.created_at_ms = ts_ms;
    }
    session.last_active_at_ms = session.last_active_at_ms.max(ts_ms);
}

pub(crate) fn parse_codex(jsonl: &str) -> Parsed {
    let mut session = Parsed::default();
    for line in jsonl.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        let ts_ms = timestamp_ms(value.get("timestamp"));
        let kind = value.get("type").and_then(serde_json::Value::as_str);
        // Older Codex versions wrote fields at the top level; current ones
        // wrap them in `payload`.
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
                push(&mut session, role, text, ts_ms, vec![], None, None);
            }
            _ => continue,
        }
        update_chronology(&mut session, ts_ms);
    }
    session
}

pub(crate) fn parse_claude(jsonl: &str) -> Parsed {
    let mut session = Parsed::default();
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
                push(&mut session, role, text.clone(), ts_ms, vec![], None, None);
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
                push(&mut session, role, text, ts_ms, calls, None, None);
                for (id, name, result) in results {
                    push(
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

/// The messages as Wisp keeps them.
pub(crate) fn messages(source: Source, parsed: &Parsed) -> Vec<Message> {
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
            model_name: (m.role == Role::Assistant).then(|| source.model_name().to_string()),
        })
        .collect()
}

/// A conversation found on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct External {
    pub source: Source,
    pub path: String,
    /// Its first request, or the file's name.
    pub title: String,
    /// The folder it was had in.
    pub folder: String,
    pub modified_ms: i64,
    pub size: u64,
    /// The id it is recorded under, when the start of the file says it.
    pub(crate) key: Option<String>,
}

fn modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn read_prefix(path: &Path, limit: u64) -> std::io::Result<String> {
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut bytes)?;
    // A cut in the middle of a character, or of a line, is dropped.
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The first thing asked, short, for the list.
fn title_of(parsed: &Parsed) -> Option<String> {
    let first = parsed
        .messages
        .iter()
        .find(|message| message.role == Role::User)?;
    let line = first.text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!line.is_empty()).then(|| line.chars().take(120).collect())
}

/// The conversations each tool keeps under `home`, newest first.
pub(crate) fn list(home: &Path) -> Vec<External> {
    let mut found = Vec::new();
    for source in Source::ALL {
        let root = source.root(home);
        let mut walker = walkdir::WalkDir::new(&root).min_depth(1);
        if source == Source::Claude {
            // One folder per project, its conversations directly in it.
            walker = walker.min_depth(2).max_depth(2);
        }
        let mut files: Vec<(PathBuf, std::fs::Metadata)> = walker
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| source.valid_filename(name))
            })
            .filter_map(|entry| Some((entry.path().to_owned(), entry.metadata().ok()?)))
            .collect();
        files.sort_by_key(|(_, metadata)| std::cmp::Reverse(modified_ms(metadata)));
        files.truncate(MAX_FILES);
        for (path, metadata) in files {
            let parsed = read_prefix(&path, PREFIX_BYTES)
                .map(|text| source.parse(&text))
                .unwrap_or_default();
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            found.push(External {
                source,
                title: title_of(&parsed).unwrap_or(name),
                folder: parsed.cwd.clone(),
                modified_ms: modified_ms(&metadata),
                size: metadata.len(),
                key: (!parsed.session_id.is_empty()).then(|| source.import_key(&parsed.session_id)),
                path: path.to_string_lossy().into_owned(),
            });
        }
    }
    found.sort_by_key(|external| std::cmp::Reverse(external.modified_ms));
    found
}

/// Reads one conversation under `home` for importing. Only files in the
/// tools' own folders are read.
pub(crate) fn read(home: &Path, path: &str) -> Result<(Source, Parsed)> {
    let path = Path::new(path);
    let file = std::fs::canonicalize(path).context("That conversation file is no longer there")?;
    let source = Source::ALL
        .into_iter()
        .find(|source| {
            std::fs::canonicalize(source.root(home)).is_ok_and(|root| file.starts_with(root))
        })
        .context("Only conversations in Codex's or Claude Code's own folders can be imported")?;
    ensure!(
        file.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| source.valid_filename(name)),
        "That is not a conversation file"
    );
    let size = std::fs::metadata(&file)?.len();
    if size > MAX_BYTES {
        bail!("That conversation is larger than 32 MB and cannot be imported");
    }
    let text = std::fs::read_to_string(&file).context("That conversation file is not UTF-8")?;
    let parsed = source.parse(&text);
    ensure!(
        !parsed.session_id.is_empty() && !parsed.messages.is_empty(),
        "That conversation has nothing to import"
    );
    Ok((source, parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const CODEX: &str = r#"{"timestamp":"2026-11-14T08:00:00Z","type":"session_meta","payload":{"id":"thread-1","cwd":"/work/rnaseq"}}
{"timestamp":"2026-11-14T08:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}
{"timestamp":"2026-11-14T08:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Align the reads to GRCh38"}]}}
{"timestamp":"2026-11-14T08:00:05Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Aligned 12 samples."}]}}
"#;

    pub(crate) const CLAUDE: &str = r#"{"sessionId":"s-9","cwd":"/work/qc","timestamp":"2026-11-13T10:00:00Z","message":{"role":"user","content":"Check the QC table"}}
{"sessionId":"s-9","timestamp":"2026-11-13T10:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"Reading it."},{"type":"tool_use","id":"t1","name":"Read","input":{"path":"qc.csv"}}]}}
{"sessionId":"s-9","timestamp":"2026-11-13T10:00:04Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"sample,reads"}]}}
{"sessionId":"s-9","isMeta":true,"message":{"role":"user","content":"ignored"}}
"#;

    #[test]
    fn both_tools_conversations_read_as_messages() {
        let codex = parse_codex(CODEX);
        assert_eq!(
            (codex.session_id.as_str(), codex.cwd.as_str()),
            ("thread-1", "/work/rnaseq")
        );
        let said: Vec<_> = codex.messages.iter().map(|m| m.text.as_str()).collect();
        // The environment wrapper is plumbing, not conversation.
        assert_eq!(said, ["Align the reads to GRCh38", "Aligned 12 samples."]);
        assert!(codex.created_at_ms > 0 && codex.last_active_at_ms > codex.created_at_ms);

        let claude = parse_claude(CLAUDE);
        assert_eq!(claude.session_id, "s-9");
        let kept = messages(Source::Claude, &claude);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[1].tool_calls[0].function.name, "Read");
        assert_eq!(
            (kept[2].role, kept[2].tool_name.as_deref()),
            (Role::Tool, Some("Read"))
        );
        assert_eq!(kept[1].model_name.as_deref(), Some("Claude Code"));
    }

    #[test]
    fn conversations_are_found_in_the_tools_own_folders_newest_first() {
        let home = tempfile::tempdir().unwrap();
        let codex = home.path().join(".codex/sessions/2026/11/14");
        let claude = home.path().join(".claude/projects/-work-qc");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(codex.join("rollout-1.jsonl"), CODEX).unwrap();
        std::fs::write(codex.join("notes.jsonl"), CODEX).unwrap();
        std::fs::write(claude.join("s-9.jsonl"), CLAUDE).unwrap();
        let found = list(home.path());
        let titles: Vec<_> = found.iter().map(|f| (f.source, f.title.as_str())).collect();
        assert_eq!(titles.len(), 2, "{titles:?}");
        assert!(titles.contains(&(Source::Codex, "Align the reads to GRCh38")));
        assert!(titles.contains(&(Source::Claude, "Check the QC table")));
        assert!(found.iter().any(|f| f.key.as_deref() == Some("claude:s-9")));

        let (source, parsed) = read(home.path(), &found[0].path).unwrap();
        assert!(!parsed.messages.is_empty() && Source::ALL.contains(&source));
        // A file elsewhere, or not a conversation, is not read.
        let elsewhere = home.path().join("rollout-2.jsonl");
        std::fs::write(&elsewhere, CODEX).unwrap();
        assert!(read(home.path(), &elsewhere.to_string_lossy()).is_err());
        assert!(read(home.path(), &codex.join("notes.jsonl").to_string_lossy()).is_err());
        // Nothing there at all: nothing found.
        assert!(list(tempfile::tempdir().unwrap().path()).is_empty());
    }
}
