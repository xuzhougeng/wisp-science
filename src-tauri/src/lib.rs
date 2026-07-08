//! Tauri v2 desktop shell: commands that drive the Wisp agent and stream
//! events to the webview, plus a settings/confirm surface.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;
use wisp_core::{Agent, MemoryManager, Output};
use wisp_llm::{Message, ProviderConfig};
use wisp_skills::SkillIndex;
use wisp_store::Store;

mod models;
mod review;
mod seed;
mod ssh_hosts;

/// One streamed agent event, tagged for the frontend to match on.
#[derive(Serialize, Clone)]
#[serde(tag = "kind")]
enum AgentEvent {
    User {
        frame_id: String,
        text: String,
    },
    Text {
        frame_id: String,
        delta: String,
    },
    Reasoning {
        frame_id: String,
        delta: String,
    },
    ToolCall {
        frame_id: String,
        name: String,
        preview: String,
    },
    ToolResult {
        frame_id: String,
        name: String,
        ok: bool,
        content: String,
        duration_ms: u64,
    },
    Usage {
        frame_id: String,
        round: u64,
        input: u64,
        output: u64,
        ctx_tokens: usize,
        max_context: usize,
    },
    Compaction {
        frame_id: String,
        before: usize,
        after: usize,
        strategy: String,
    },
    Diff {
        frame_id: String,
        path: String,
    },
    Stdout {
        frame_id: String,
        chunk: String,
    },
    Done {
        frame_id: String,
    },
    Error {
        frame_id: String,
        message: String,
    },
    /// One-shot reviewer findings (Markdown) for the current session.
    Review {
        frame_id: String,
        markdown: String,
    },
}

#[derive(Serialize, Clone)]
struct ConfirmRequest {
    frame_id: String,
    message: String,
    /// Tool name when known (`python`, `shell`, …).
    #[serde(default)]
    tool: String,
    /// Code / command preview for the inline approval card.
    #[serde(default)]
    preview: String,
}

/// Parse a blocking-confirm message into (tool, preview) for the UI card.
fn parse_confirm_payload(message: &str) -> (String, String) {
    // Plan-approval pause: the checklist rides in the message behind a marker so
    // the UI renders the dedicated plan card (preview = the checklist).
    if let Some(rest) = message.strip_prefix(wisp_tools::plan::PLAN_APPROVAL_PREFIX) {
        return ("update_plan".to_string(), rest.to_string());
    }
    if let Some(rest) = message.strip_prefix("Run tool '") {
        if let Some((tool, _)) = rest.split_once("'?") {
            return (tool.to_string(), String::new());
        }
    }
    if message.starts_with("Dangerous command detected") {
        if let Some((_, cmd)) = message.rsplit_once(": ") {
            return ("shell".into(), cmd.to_string());
        }
    }
    (String::new(), String::new())
}

#[derive(Serialize, Clone)]
struct SkillInfo {
    name: String,
    description: String,
    tags: Vec<String>,
    enabled: bool,
    builtin: bool,
    dir: String,
}

#[derive(Serialize, Clone)]
struct DirEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize, Clone)]
struct FileContent {
    path: String,
    mime: String,
    text: Option<String>,
    /// Base64 payload for binary files (images, pdf, pdb, …).
    base64: Option<String>,
}

#[derive(Serialize, Clone)]
struct ArtifactInfo {
    id: String,
    name: String,
    kind: String,
    path: String,
    ts: i64,
}

#[derive(Serialize, Clone)]
struct ProjectInfo {
    id: String,
    name: String,
    root: String,
    skill_count: usize,
    mcp_server_count: usize,
    memory_file_count: usize,
    has_api_key: bool,
}

#[derive(Serialize, Clone)]
struct MemoryFile {
    name: String,
    preview: String,
    bytes: u64,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
        #[serde(default)]
        cwd: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
}

/// A user-configured MCP server connection.
#[derive(Serialize, Deserialize, Clone)]
struct McpConnection {
    id: String,
    name: String,
    enabled: bool,
    transport: McpTransport,
}

// ── Connectors (multi-level) + per-tool approval ────────────────────────────
//
// The bundled `mcp_bio` aggregate serves ~247 tools; `mcp_bio/domains.json`
// (domain slug -> tool names) partitions them into 23 "connectors". That file
// is the static connector↔tool map — no server launch needed to build the tree.
// User `McpConnection`s are extra "custom" connectors (their tools aren't
// statically known, so per-tool approval only applies to the bundled ones).

/// Per-tool approval mode. `Allow` is the default (silent auto-run, matching the
/// old behaviour); `Ask` shows the confirm card; `Deny` blocks the call.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ApprovalMode {
    Allow,
    Ask,
    Deny,
}

impl ApprovalMode {
    fn as_str(self) -> &'static str {
        match self {
            ApprovalMode::Allow => "allow",
            ApprovalMode::Ask => "ask",
            ApprovalMode::Deny => "deny",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "ask" => ApprovalMode::Ask,
            "deny" => ApprovalMode::Deny,
            _ => ApprovalMode::Allow,
        }
    }
    fn to_tools(self) -> wisp_tools::Approval {
        match self {
            ApprovalMode::Allow => wisp_tools::Approval::Allow,
            ApprovalMode::Ask => wisp_tools::Approval::Ask,
            ApprovalMode::Deny => wisp_tools::Approval::Deny,
        }
    }
}

/// Global approval scope — the master knob layered over the per-tool policy.
/// `Ask` (default) keeps the existing per-tool + dangerous-command prompting.
/// `Auto` silences per-tool prompts but a dangerous command still asks. `Full`
/// auto-approves everything, dangerous commands included. An explicit per-tool
/// `Deny` survives every scope: it's a hard block, not a prompt.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Scope {
    Full,
    Auto,
    #[default]
    Ask,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Full => "full",
            Scope::Auto => "auto",
            Scope::Ask => "ask",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "full" => Scope::Full,
            "auto" => Scope::Auto,
            _ => Scope::Ask,
        }
    }
}

/// Live approval policy read by `TauriOutput::approval_mode` on every tool call.
/// `tool_connector` is static (built once from `domains.json`); `tools`/`skip`/
/// `scope` mirror the persisted settings and are refreshed by the approval
/// commands.
#[derive(Default)]
struct ApprovalPolicy {
    /// Global scope layered over the per-tool modes below.
    scope: Scope,
    /// Tool name -> mode. Absent = `Allow`.
    tools: HashMap<String, ApprovalMode>,
    /// Connector keys whose tools are force-allowed ("Skip approvals" on).
    skip: HashSet<String>,
    /// Tool name -> bundled connector (domain slug), for resolving `skip`.
    tool_connector: HashMap<String, String>,
}

impl ApprovalPolicy {
    /// The per-tool mode before the global scope is applied.
    fn base_mode(&self, tool: &str) -> ApprovalMode {
        if let Some(conn) = self.tool_connector.get(tool) {
            if self.skip.contains(conn) {
                return ApprovalMode::Allow;
            }
        }
        self.tools.get(tool).copied().unwrap_or(ApprovalMode::Allow)
    }

    fn mode_for(&self, tool: &str) -> wisp_tools::Approval {
        let base = self.base_mode(tool);
        match self.scope {
            // Current behaviour: honour the per-tool mode as configured.
            Scope::Ask => base.to_tools(),
            // Auto/Full silence per-tool prompts, but an explicit Deny is a hard
            // block that survives (dangerous commands are gated separately in
            // the shell tool via `full()`).
            Scope::Auto | Scope::Full => match base {
                ApprovalMode::Deny => wisp_tools::Approval::Deny,
                _ => wisp_tools::Approval::Allow,
            },
        }
    }

    /// Whether dangerous shell commands should auto-approve (scope == Full).
    fn full(&self) -> bool {
        self.scope == Scope::Full
    }
}

/// One bundled bio-tools connector (a domain from `mcp_bio/domains.json`).
#[derive(Clone)]
struct BioDomain {
    slug: String,
    name: String,
    tools: Vec<String>,
}

/// Read the static `mcp_bio/domains.json` connector map. Empty if the bundle is
/// absent (dev checkouts without the vendored bio-tools).
fn bio_domains() -> Vec<BioDomain> {
    let Some(dir) = wisp_paths::bio_tools_dir() else {
        return vec![];
    };
    let path = dir.join("lib").join("mcp_bio").join("domains.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    let Ok(map) = serde_json::from_str::<BTreeMap<String, Vec<String>>>(&text) else {
        return vec![];
    };
    map.into_iter()
        .map(|(slug, tools)| BioDomain {
            name: domain_display_name(&slug),
            slug,
            tools,
        })
        .collect()
}

/// Human label for a domain slug, matching the reference casing for the common
/// ones and title-casing the rest.
fn domain_display_name(slug: &str) -> String {
    match slug {
        "biomart" => return "BioMart".into(),
        "biorxiv" => return "bioRxiv".into(),
        "cellguide" => return "CellGuide".into(),
        "chembl" => return "ChEMBL".into(),
        "pubmed" => return "PubMed".into(),
        "rna" => return "RNA".into(),
        "zinc" => return "ZINC".into(),
        _ => {}
    }
    slug.split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Serialize, Clone)]
struct Capabilities {
    skills: Vec<SkillInfo>,
    mcp_servers: Vec<String>,
    memory_files: Vec<MemoryFile>,
    project: ProjectInfo,
}

#[derive(Serialize, Clone)]
struct OnboardingState {
    show: bool,
    has_api_key: bool,
}

/// One saved conversation for the history sidebar.
#[derive(Serialize, Clone)]
struct SessionInfo {
    id: String,
    title: String,
    ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    folder_id: Option<String>,
}

#[derive(Serialize, Clone)]
struct FolderInfo {
    id: String,
    name: String,
}

#[derive(Serialize, Clone)]
struct ProjectSummary {
    id: String,
    name: String,
    description: String,
    workspace_dir: String,
    session_count: i64,
    updated_at: i64,
    running_count: i64,
    needs_you_count: i64,
}

async fn build_project_summary(state: &AppState, id: &str) -> ProjectSummary {
    let running = state.running_turns.lock().await.clone();
    let awaiting = state.awaiting_confirm.lock().unwrap().clone();
    let Some((id, name, ws, _c, upd, cnt, desc)) = state
        .store
        .list_projects()
        .await
        .ok()
        .and_then(|v| v.into_iter().find(|r| r.0 == id))
    else {
        return ProjectSummary {
            id: id.into(),
            name: String::new(),
            description: String::new(),
            workspace_dir: String::new(),
            session_count: 0,
            updated_at: 0,
            running_count: 0,
            needs_you_count: 0,
        };
    };
    let (running_count, needs_you_count) =
        project_status_counts(&state.store, &id, &running, &awaiting).await;
    ProjectSummary {
        id,
        name,
        description: desc,
        workspace_dir: ws,
        session_count: cnt,
        updated_at: upd,
        running_count,
        needs_you_count,
    }
}

fn session_runtime_status(
    id: &str,
    last_role: Option<&str>,
    running: &HashSet<String>,
    awaiting: &HashSet<String>,
) -> &'static str {
    if awaiting.contains(id) {
        "needs_you"
    } else if running.contains(id) {
        "running"
    } else if last_role == Some("assistant") {
        "needs_you"
    } else {
        "complete"
    }
}

async fn project_status_counts(
    store: &wisp_store::Store,
    project_id: &str,
    running: &HashSet<String>,
    awaiting: &HashSet<String>,
) -> (i64, i64) {
    let Ok(rows) = store.list_session_last_roles(project_id).await else {
        return (0, 0);
    };
    let mut running_count = 0i64;
    let mut needs_you_count = 0i64;
    for (id, role) in rows {
        if awaiting.contains(&id) {
            needs_you_count += 1;
        } else if running.contains(&id) {
            running_count += 1;
        } else if role.as_deref() == Some("assistant") {
            needs_you_count += 1;
        }
    }
    (running_count, needs_you_count)
}

/// A reloaded transcript row for the UI to render (role in
/// user|assistant|reasoning|tool).
#[derive(Serialize, Clone)]
struct UiItem {
    role: String,
    text: String,
    tool_name: Option<String>,
    ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_name: Option<String>,
}

/// Index in `msgs` where the `user_index`‑th user turn starts (0-based user count).
fn user_message_start(msgs: &[wisp_llm::Message], user_index: usize) -> usize {
    let mut seen = 0usize;
    for (i, m) in msgs.iter().enumerate() {
        if m.role == wisp_llm::Role::User && !m.content.as_text().trim().is_empty() {
            if seen == user_index {
                return i;
            }
            seen += 1;
        }
    }
    msgs.len()
}

/// Flatten persisted messages into UI transcript items (skips system turns,
/// splits assistant reasoning into its own row).
fn messages_to_items(msgs: &[wisp_llm::Message]) -> Vec<UiItem> {
    let mut out = vec![];
    for m in msgs {
        match m.role {
            wisp_llm::Role::User => {
                let t = m.content.as_text();
                if !t.trim().is_empty() {
                    out.push(UiItem {
                        role: "user".into(),
                        text: t,
                        tool_name: None,
                        ok: None,
                        model_name: None,
                    });
                }
            }
            wisp_llm::Role::Assistant => {
                if let Some(r) = &m.reasoning {
                    if !r.trim().is_empty() {
                        out.push(UiItem {
                            role: "reasoning".into(),
                            text: r.clone(),
                            tool_name: None,
                            ok: None,
                            model_name: None,
                        });
                    }
                }
                let t = m.content.as_text();
                if !t.trim().is_empty() {
                    out.push(UiItem {
                        role: "assistant".into(),
                        text: t,
                        tool_name: None,
                        ok: None,
                        model_name: m.model_name.clone(),
                    });
                }
            }
            wisp_llm::Role::Tool => {
                let text = m.content.as_text();
                if m.tool_name.as_deref() == Some("attempt_completion") {
                    if !text.trim().is_empty() {
                        out.push(UiItem {
                            role: "assistant".into(),
                            text,
                            tool_name: None,
                            ok: None,
                            model_name: m.model_name.clone(),
                        });
                    }
                } else {
                    out.push(UiItem {
                        role: "tool".into(),
                        text,
                        tool_name: m.tool_name.clone(),
                        ok: Some(true),
                        model_name: None,
                    });
                }
            }
            wisp_llm::Role::System => {}
        }
    }
    out
}

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    provider: String,
    api_url: String,
    model: String,
    /// User-facing alias for the active model profile (composer picker label).
    #[serde(default)]
    label: String,
    has_api_key: bool,
    #[serde(default = "default_locale")]
    locale: String,
    /// Where the workspace/data root lives. Empty = platform default
    /// (Documents/wisp-science). Applied on next launch (#6, #13).
    #[serde(default)]
    workspace_dir: String,
    /// Max output tokens per LLM turn. 0 = provider default.
    #[serde(default)]
    max_tokens: u64,
    /// OpenAI reasoning effort (none/minimal/low/medium/high/xhigh). Empty = provider default.
    #[serde(default)]
    reasoning_effort: String,
}

/// Drop cached per-session agents so the next turn picks up new model settings.
async fn clear_idle_agents(state: &AppState) {
    let runtimes = state
        .sessions
        .lock()
        .await
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for rt in runtimes {
        if let Ok(mut guard) = rt.agent.try_lock() {
            *guard = None;
        }
    }
}

fn default_locale() -> String {
    "en".into()
}

#[derive(Serialize, Clone)]
struct BootstrapStatus {
    skills_loaded: usize,
    python_ok: bool,
    mcp_catalog: usize,
    uv_ok: bool,
    node_ok: bool,
    npm_ok: bool,
    sci_ok: bool,
    pixi_ok: bool,
    app_version: String,
    workspace: String,
    errors: Vec<String>,
}

/// Per-session runtime: one agent (with its own Python kernel + MCP), one
/// cancel flag, and the persisted-seq cursor. Keyed by frame id in
/// `AppState.sessions`, so different conversations run concurrently on
/// independent mutexes.
struct SessionRuntime {
    agent: tokio::sync::Mutex<Option<Agent>>,
    cancel: Arc<AtomicBool>,
    last_seq: StdMutex<i64>,
}

impl SessionRuntime {
    fn new() -> Self {
        Self {
            agent: tokio::sync::Mutex::new(None),
            cancel: Arc::new(AtomicBool::new(false)),
            last_seq: StdMutex::new(0),
        }
    }
    fn last_seq(&self) -> i64 {
        *self.last_seq.lock().unwrap()
    }
    fn set_last_seq(&self, v: i64) {
        *self.last_seq.lock().unwrap() = v;
    }
}

#[derive(Clone)]
struct ActiveProject {
    id: String,
    root: PathBuf,
    skills: Arc<SkillIndex>,
    memory: Arc<MemoryManager>,
}

struct AppState {
    app_data: PathBuf,
    store: Store,
    active: std::sync::RwLock<HashMap<String, ActiveProject>>,
    /// One runtime per conversation frame id. Locked only briefly to clone the
    /// `Arc`; the per-session `agent` mutex is what serializes turns *within*
    /// one conversation — different conversations never block each other.
    sessions: tokio::sync::Mutex<HashMap<String, Arc<SessionRuntime>>>,
    /// Session ids with an in-flight agent turn (for the projects dashboard).
    running_turns: tokio::sync::Mutex<HashSet<String>>,
    /// The frame id the UI is currently viewing. Drives artifact attachment
    /// (`upload_file`/`register_artifact`) and `list_artifacts` fallback.
    active_frame: std::sync::RwLock<HashMap<String, String>>,
    /// Per-session confirm channels, keyed by frame id.
    confirms: Arc<StdMutex<HashMap<String, std::sync::mpsc::Sender<bool>>>>,
    /// Sessions blocked on an inline approval card (Projects dashboard → Needs you).
    awaiting_confirm: Arc<StdMutex<HashSet<String>>>,
    /// Live per-tool approval policy, read on every tool call by `TauriOutput`.
    approvals: Arc<StdRwLock<ApprovalPolicy>>,
    bootstrap: StdMutex<BootstrapStatus>,
    /// Guards against a second `review_session` running concurrently.
    reviewing: Arc<AtomicBool>,
}

impl AppState {
    /// Snapshot a window's active project. Falls back to the "main" window's
    /// project (always initialized at startup) for un-scoped or early calls.
    fn active(&self, label: &str) -> ActiveProject {
        let map = self.active.read().unwrap();
        map.get(label)
            .or_else(|| map.get("main"))
            .cloned()
            .expect("main window active project is initialized at startup")
    }
    fn set_active(&self, label: &str, ap: ActiveProject) {
        self.active.write().unwrap().insert(label.to_string(), ap);
    }
    /// The frame this window is viewing (artifact upload target), if any.
    fn active_frame(&self, label: &str) -> Option<String> {
        self.active_frame.read().unwrap().get(label).cloned()
    }
    fn set_active_frame(&self, label: &str, frame: Option<String>) {
        match frame {
            Some(f) => {
                self.active_frame.write().unwrap().insert(label.to_string(), f);
            }
            None => {
                self.active_frame.write().unwrap().remove(label);
            }
        }
    }
}

/// Ensure `dir` exists and is usable; fall back to `app_data/workspace` if not.
/// Never panics unless even the fallback can't be created.
fn ensure_writable(dir: PathBuf, app_data: &std::path::Path) -> PathBuf {
    if std::fs::create_dir_all(&dir).is_ok() {
        dir
    } else {
        let fallback = app_data.join("workspace");
        tracing::warn!("workspace {:?} not writable; using {:?}", dir, fallback);
        std::fs::create_dir_all(&fallback).expect("create fallback workspace dir");
        fallback
    }
}

/// `wisp_core::Output` backed by Tauri events. `confirm` blocks on a std
/// channel satisfied by the `confirm_response` command. `frame_id` is the
/// session frame id (carried on every event so the UI can route by session).
struct TauriOutput {
    app: AppHandle,
    frame_id: String,
    confirms: Arc<StdMutex<HashMap<String, std::sync::mpsc::Sender<bool>>>>,
    awaiting_confirm: Arc<StdMutex<HashSet<String>>>,
    /// Shared live approval policy (see `AppState::approvals`).
    approvals: Arc<StdRwLock<ApprovalPolicy>>,
    /// Incremental-persistence sink: each message the turn produces is sent here
    /// and written to SQLite by a background task, so a crash or mid-turn "new
    /// session" no longer discards the whole turn. `None` disables it.
    persist: Option<tokio::sync::mpsc::UnboundedSender<Message>>,
    /// Provenance sink: each tool-execution record the turn produces is sent here
    /// and persisted as an `execution_log` row by a background drain task.
    /// `None` disables it.
    prov: Option<tokio::sync::mpsc::UnboundedSender<wisp_core::ProvenanceRecord>>,
}

impl TauriOutput {
    fn emit(&self, event: AgentEvent) {
        let _ = self.app.emit("agent", event);
    }
}

impl Output for TauriOutput {
    fn assistant_text(&self, delta: &str) {
        self.emit(AgentEvent::Text {
            frame_id: self.frame_id.clone(),
            delta: delta.into(),
        });
    }
    fn reasoning(&self, delta: &str) {
        self.emit(AgentEvent::Reasoning {
            frame_id: self.frame_id.clone(),
            delta: delta.into(),
        });
    }
    fn tool_call(&self, name: &str, preview: &str) {
        self.emit(AgentEvent::ToolCall {
            frame_id: self.frame_id.clone(),
            name: name.into(),
            preview: preview.into(),
        });
    }
    fn tool_result(&self, name: &str, ok: bool, content: &str, duration_ms: u64) {
        let clipped: String = content.chars().take(4000).collect();
        self.emit(AgentEvent::ToolResult {
            frame_id: self.frame_id.clone(),
            name: name.into(),
            ok,
            content: clipped,
            duration_ms,
        });
    }
    fn usage(&self, round: usize, input: u64, output: u64, ctx_tokens: usize, max_context: usize) {
        self.emit(AgentEvent::Usage {
            frame_id: self.frame_id.clone(),
            round: round as u64,
            input,
            output,
            ctx_tokens,
            max_context,
        });
    }
    fn compaction(&self, before: usize, after: usize, strategy: &str) {
        self.emit(AgentEvent::Compaction {
            frame_id: self.frame_id.clone(),
            before,
            after,
            strategy: strategy.into(),
        });
    }
    fn diff(&self, path: &str, _old: &str, _new: &str) {
        self.emit(AgentEvent::Diff {
            frame_id: self.frame_id.clone(),
            path: path.into(),
        });
    }
    fn stdout_chunk(&self, chunk: &str) {
        self.emit(AgentEvent::Stdout {
            frame_id: self.frame_id.clone(),
            chunk: chunk.into(),
        });
    }
    fn confirm(&self, message: &str) -> bool {
        let (tool, preview) = parse_confirm_payload(message);
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        self.confirms
            .lock()
            .unwrap()
            .insert(self.frame_id.clone(), tx);
        self.awaiting_confirm
            .lock()
            .unwrap()
            .insert(self.frame_id.clone());
        let _ = self.app.emit(
            "confirm-request",
            ConfirmRequest {
                frame_id: self.frame_id.clone(),
                message: message.into(),
                tool,
                preview,
            },
        );
        let approved = rx
            .recv_timeout(std::time::Duration::from_secs(180))
            .unwrap_or(false);
        self.confirms.lock().unwrap().remove(&self.frame_id);
        self.awaiting_confirm
            .lock()
            .unwrap()
            .remove(&self.frame_id);
        approved
    }
    fn approval_mode(&self, tool: &str) -> wisp_tools::Approval {
        self.approvals
            .read()
            .map(|p| p.mode_for(tool))
            .unwrap_or(wisp_tools::Approval::Allow)
    }
    fn danger_auto_approve(&self) -> bool {
        self.approvals.read().map(|p| p.full()).unwrap_or(false)
    }
    fn on_message(&self, msg: &Message) {
        if msg.role == wisp_llm::Role::User {
            self.emit(AgentEvent::User {
                frame_id: self.frame_id.clone(),
                text: msg.content.as_text(),
            });
        }
        if let Some(tx) = &self.persist {
            let _ = tx.send(msg.clone());
        }
    }
    fn provenance(&self, rec: &wisp_core::ProvenanceRecord) {
        if let Some(tx) = &self.prov {
            let _ = tx.send(rec.clone());
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn normalized_provider(provider: &str) -> String {
    match provider.trim() {
        "anthropic" => "anthropic".into(),
        "openai_responses" | "openai-responses" | "responses" => "openai_responses".into(),
        _ => "openai".into(),
    }
}

fn non_empty_setting(value: Option<String>, fallback: impl FnOnce() -> String) -> String {
    value
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(fallback)
}

/// Pick the workspace root: env override, then the saved setting, then the
/// platform default — the first non-empty candidate we can create wins.
fn resolve_workspace(env: Option<String>, stored: Option<String>, default: PathBuf) -> PathBuf {
    for cand in [env, stored].into_iter().flatten() {
        let cand = cand.trim();
        if cand.is_empty() {
            continue;
        }
        let p = PathBuf::from(cand);
        if std::fs::create_dir_all(&p).is_ok() {
            return p;
        }
    }
    default
}

async fn load_locale(store: &Store) -> String {
    let raw = store.get_setting("locale").await.ok().flatten();
    match raw.as_deref().map(str::trim) {
        Some("zh") | Some("zh-CN") | Some("zh-TW") => "zh".into(),
        Some(other) if !other.is_empty() => other.to_string(),
        _ => "en".into(),
    }
}

fn default_max_tokens(provider: &str) -> u64 {
    match normalized_provider(provider).as_str() {
        "anthropic" => 8192,
        _ => 8192,
    }
}

fn effective_max_tokens(configured: u64, provider: &str) -> u64 {
    let v = if configured >= 16 {
        configured
    } else {
        default_max_tokens(provider)
    };
    v.max(16)
}

fn effective_reasoning_effort(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s == "default" {
        None
    } else {
        Some(s.to_string())
    }
}

fn apply_llm_advanced(
    cfg: &mut ProviderConfig,
    max_tokens: u64,
    reasoning_effort: &str,
    provider: &str,
) {
    cfg.max_tokens = effective_max_tokens(max_tokens, provider);
    cfg.reasoning_effort = effective_reasoning_effort(reasoning_effort);
}

async fn load_settings(store: &Store) -> (String, String, String, String) {
    // Resolve through the active model profile (migrates legacy single-model
    // installs on first read), then apply env/default fallbacks so a blank
    // field still produces a usable config.
    let (provider, api_url, model, api_key) = models::active_config(store).await;
    let provider = normalized_provider(&non_empty_setting(Some(provider), || {
        env_or("WISP_PROVIDER", "openai")
    }));
    let api_url = non_empty_setting(Some(api_url), || {
        env_or("WISP_API_URL", default_api_url(&provider))
    });
    let model = non_empty_setting(Some(model), || {
        env_or("WISP_MODEL", default_model(&provider))
    });
    let api_key = if api_key.trim().is_empty() {
        env_or("WISP_API_KEY", "")
    } else {
        api_key
    };
    (provider, api_url, model, api_key)
}

fn parse_disabled_skills(raw: Option<&str>) -> HashSet<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

async fn load_disabled_skills(store: &Store) -> HashSet<String> {
    let raw = store.get_setting("disabled_skills").await.ok().flatten();
    parse_disabled_skills(raw.as_deref())
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    tags.into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn parse_skill_tags(raw: Option<String>) -> BTreeMap<String, Vec<String>> {
    let Some(raw) = raw else {
        return BTreeMap::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return BTreeMap::new();
    };
    let Some(obj) = value.as_object() else {
        return BTreeMap::new();
    };
    obj.iter()
        .filter_map(|(name, tags)| {
            let tags = tags
                .as_array()?
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>();
            let tags = normalize_tags(tags);
            if tags.is_empty() {
                None
            } else {
                Some((name.clone(), tags))
            }
        })
        .collect()
}

fn parse_enabled_skill_names(raw: Option<String>) -> Option<HashSet<String>> {
    let raw = raw?;
    serde_json::from_str::<Vec<String>>(&raw)
        .ok()
        .map(|names| {
            names
                .into_iter()
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .or_else(|| Some(HashSet::new()))
}

fn enabled_skill_names_key(project_id: &str) -> String {
    format!("project_enabled_skills:{project_id}")
}

async fn load_skill_tags(store: &Store) -> BTreeMap<String, Vec<String>> {
    parse_skill_tags(store.get_setting("skill_tags").await.ok().flatten())
}

async fn save_skill_tags(
    store: &Store,
    tags: &BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    store
        .set_setting(
            "skill_tags",
            &serde_json::to_string(tags).map_err(|e| format!("{e}"))?,
        )
        .await
        .map_err(|e| format!("{e}"))
}

async fn load_enabled_skill_names(store: &Store, project_id: &str) -> Option<HashSet<String>> {
    parse_enabled_skill_names(
        store
            .get_setting(&enabled_skill_names_key(project_id))
            .await
            .ok()
            .flatten(),
    )
}

async fn save_enabled_skill_names(
    store: &Store,
    project_id: &str,
    names: &HashSet<String>,
) -> Result<(), String> {
    let mut names = names.iter().cloned().collect::<Vec<_>>();
    names.sort();
    store
        .set_setting(
            &enabled_skill_names_key(project_id),
            &serde_json::to_string(&names).map_err(|e| format!("{e}"))?,
        )
        .await
        .map_err(|e| format!("{e}"))
}

async fn effective_enabled_skill_names(
    store: &Store,
    ap: &ActiveProject,
) -> Option<HashSet<String>> {
    if let Some(enabled) = load_enabled_skill_names(store, &ap.id).await {
        return Some(enabled);
    }
    let disabled = load_disabled_skills(store).await;
    if disabled.is_empty() {
        None
    } else {
        Some(
            ap.skills
                .all()
                .iter()
                .filter(|s| !disabled.contains(&s.name))
                .map(|s| s.name.clone())
                .collect(),
        )
    }
}

fn skill_infos(
    skills: &SkillIndex,
    tags: &BTreeMap<String, Vec<String>>,
    enabled: Option<&HashSet<String>>,
) -> Vec<SkillInfo> {
    let bundled = wisp_skills::bundled_dir();
    skills
        .all()
        .iter()
        .map(|s| {
            let builtin = bundled
                .as_ref()
                .map(|b| s.dir.starts_with(b))
                .unwrap_or(false);
            SkillInfo {
                name: s.name.clone(),
                description: s.description.clone(),
                tags: tags.get(&s.name).cloned().unwrap_or_default(),
                enabled: enabled.is_none_or(|names| names.contains(&s.name)),
                builtin,
                dir: s.dir.to_string_lossy().to_string(),
            }
        })
        .collect()
}

async fn active_skill_index(store: &Store, ap: &ActiveProject) -> Arc<SkillIndex> {
    Arc::new(
        ap.skills
            .filtered_by_names(effective_enabled_skill_names(store, ap).await.as_ref()),
    )
}

async fn load_mcp_connections(store: &Store) -> Vec<McpConnection> {
    store
        .get_setting("mcp_connections")
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<McpConnection>>(&s).ok())
        .unwrap_or_default()
}

async fn save_mcp_connections(store: &Store, conns: &[McpConnection]) -> Result<(), String> {
    let json = serde_json::to_string(conns).map_err(|e| format!("{e}"))?;
    store
        .set_setting("mcp_connections", &json)
        .await
        .map_err(|e| format!("{e}"))
}

async fn load_json_setting<T: serde::de::DeserializeOwned + Default>(store: &Store, key: &str) -> T {
    store
        .get_setting(key)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<T>(&s).ok())
        .unwrap_or_default()
}

async fn save_json_setting<T: Serialize>(store: &Store, key: &str, val: &T) -> Result<(), String> {
    let json = serde_json::to_string(val).map_err(|e| format!("{e}"))?;
    store
        .set_setting(key, &json)
        .await
        .map_err(|e| format!("{e}"))
}

/// Disabled bundled connectors (domain slugs). Custom connections carry their
/// own `enabled` flag instead.
async fn load_disabled_connectors(store: &Store) -> HashSet<String> {
    load_json_setting::<Vec<String>>(store, "disabled_connectors")
        .await
        .into_iter()
        .collect()
}

/// Persisted per-tool approvals (tool name -> "ask"/"deny"; "allow" omitted).
async fn load_tool_approvals(store: &Store) -> HashMap<String, String> {
    load_json_setting(store, "tool_approvals").await
}

/// Persisted global approval scope ("full" | "auto" | "ask"; default "ask").
async fn load_approval_scope(store: &Store) -> Scope {
    Scope::parse(&load_json_setting::<String>(store, "approval_scope").await)
}

/// Connector keys with "Skip approvals" on.
async fn load_skip_connectors(store: &Store) -> HashSet<String> {
    load_json_setting::<Vec<String>>(store, "skip_approval_connectors")
        .await
        .into_iter()
        .collect()
}

/// tool name -> bundled connector (domain slug). Static; built from domains.json.
fn build_tool_connector_map() -> HashMap<String, String> {
    let mut m = HashMap::new();
    for d in bio_domains() {
        for t in d.tools {
            m.insert(t, d.slug.clone());
        }
    }
    m
}

/// Snapshot the persisted approval state into a fresh `ApprovalPolicy`.
async fn build_approval_policy(store: &Store) -> ApprovalPolicy {
    ApprovalPolicy {
        scope: load_approval_scope(store).await,
        tools: load_tool_approvals(store)
            .await
            .into_iter()
            .map(|(k, v)| (k, ApprovalMode::parse(&v)))
            .collect(),
        skip: load_skip_connectors(store).await,
        tool_connector: build_tool_connector_map(),
    }
}

/// Reload the live approval policy after a settings change so running sessions
/// see it on their next tool call (approval is enforced live, not per session).
async fn refresh_approval_policy(state: &AppState) {
    let policy = build_approval_policy(&state.store).await;
    if let Ok(mut guard) = state.approvals.write() {
        *guard = policy;
    }
}

async fn load_memory_enabled(store: &Store) -> bool {
    store
        .get_setting("memory_enabled")
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<bool>(&s).ok())
        .unwrap_or(true)
}

async fn save_memory_enabled(store: &Store, on: bool) -> Result<(), String> {
    store
        .set_setting("memory_enabled", &on.to_string())
        .await
        .map_err(|e| format!("{e}"))
}

fn memory_file_path(memory: &MemoryManager, name: &str) -> Result<std::path::PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err("invalid memory file name".into());
    }
    if std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        != Some("md")
    {
        return Err("memory file must be .md".into());
    }
    let path = memory.dir().join(name);
    if !path.starts_with(memory.dir()) {
        return Err("invalid memory path".into());
    }
    Ok(path)
}

/// Build an `McpClient` from a user-configured connection. Stdio connections
/// carry their own command/env/cwd (unrelated to the bundled Python venv).
async fn connect_mcp(conn: &McpConnection) -> anyhow::Result<wisp_mcp::McpClient> {
    match &conn.transport {
        McpTransport::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args);
            for (k, v) in env {
                cmd.env(k, v);
            }
            if let Some(dir) = cwd {
                if !dir.is_empty() {
                    cmd.current_dir(dir);
                }
            }
            wisp_mcp::McpClient::launch_with_command(cmd).await
        }
        McpTransport::Http { url, headers } => {
            wisp_mcp::McpClient::connect_http(url, headers).await
        }
    }
}

fn default_api_url(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "https://api.anthropic.com",
        "openai_responses" => "https://api.openai.com/v1",
        _ => "https://api.deepseek.com",
    }
}

fn default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-5",
        "openai_responses" => "gpt-5.5",
        _ => "deepseek-v4-pro",
    }
}

fn build_provider_config(
    provider: &str,
    api_url: &str,
    api_key: &str,
    model: &str,
    max_tokens: u64,
    reasoning_effort: &str,
) -> Result<ProviderConfig, String> {
    let provider = normalized_provider(provider);
    let api_url = api_url.trim();
    let api_key = api_key.trim();
    let model = model.trim();
    if api_url.is_empty() {
        return Err("API URL is required.".into());
    }
    if model.is_empty() {
        return Err("Model is required.".into());
    }
    if api_key.is_empty() {
        return Err("No API key set. Open Settings and paste your provider API key.".into());
    }
    let mut cfg = match provider.as_str() {
        "anthropic" => ProviderConfig::anthropic(api_url, api_key, model),
        "openai_responses" => ProviderConfig::openai_responses(api_url, api_key, model),
        "openai" => ProviderConfig::openai(api_url, api_key, model),
        _ => return Err(format!("Unsupported provider: {provider}")),
    };
    apply_llm_advanced(&mut cfg, max_tokens, reasoning_effort, &provider);
    Ok(cfg)
}

fn effective_api_key(new_key: Option<String>, stored_key: String) -> String {
    let key = new_key.unwrap_or_default();
    if key.trim().is_empty() || key.starts_with("(stored") {
        stored_key
    } else {
        key
    }
}

fn skill_paths(root: &std::path::Path) -> Vec<PathBuf> {
    let mut paths = vec![];
    if let Some(b) = wisp_skills::bundled_dir() {
        paths.push(b);
    }
    paths.push(root.join(".wisp").join("skills"));
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".wisp").join("skills"));
    }
    if let Ok(extra) = std::env::var("WISP_SKILLS_PATH") {
        for p in extra.split([':', ';']).filter(|s| !s.is_empty()) {
            paths.push(PathBuf::from(p));
        }
    }
    paths
}

/// Wire Python REPL, bundled bio-tools MCP, and user-configured MCP
/// connections into a freshly built agent.
async fn wire_python_and_mcp(
    agent: &mut wisp_core::Agent,
    app_data: &std::path::Path,
    store: &Store,
) -> Vec<String> {
    let mut errors = vec![];
    let py_env = match wisp_python::PythonEnv::ensure(app_data) {
        Ok(env) => Some(env),
        Err(e) => {
            errors.push(format!("Python environment: {e}"));
            None
        }
    };

    let worker = std::env::var("WISP_KERNEL_WORKER")
        .ok()
        .or_else(|| wisp_python::bundled_worker_path().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_default();
    let service_env = models::service_env();
    let worker_path = wisp_python::resolve_bundled_script(&worker);
    if worker_path.is_file() {
        if let Some(env) = &py_env {
            match wisp_python::KernelClient::spawn(&env.python(), &worker_path, &service_env) {
                Ok(client) => agent.add_tool(Box::new(wisp_python::ReplTool::new(client))),
                Err(e) => errors.push(format!("Python REPL: {e}")),
            }
        }
    } else {
        errors.push(format!(
            "Kernel worker not found at {}",
            worker_path.display()
        ));
    }

    // Bundled bio-tools. Per-connector (domain) enable is the only gate now:
    // the `WISP_MCP_COMMAND` dev override always applies; otherwise mcp_bio
    // launches unless every domain is disabled.
    if let Ok(cmdline) = std::env::var("WISP_MCP_COMMAND") {
        let parts: Vec<String> = cmdline
            .split_whitespace()
            .map(|s| {
                if s.ends_with(".py") {
                    wisp_python::resolve_bundled_script(s)
                        .to_string_lossy()
                        .to_string()
                } else {
                    s.to_string()
                }
            })
            .collect();
        if parts.len() >= 2 {
            let args: Vec<String> = parts[1..].to_vec();
            match wisp_mcp::McpClient::launch(&parts[0], &args).await {
                Ok(client) => register_mcp(agent, std::sync::Arc::new(client)).await,
                Err(e) => errors.push(format!("MCP command: {e}")),
            }
        }
    } else if let Some(env) = &py_env {
        let pkg = std::env::var("WISP_MCP_PKG").unwrap_or_else(|_| "mcp_bio".into());
        // mcp_bio serves all 247 tools; drop disabled domains' tools at
        // registration. Skip the launch entirely if every domain is off.
        let disabled = load_disabled_connectors(store).await;
        let domains = bio_domains();
        let all_off = !domains.is_empty() && domains.iter().all(|d| disabled.contains(&d.slug));
        let skip: HashSet<String> = domains
            .iter()
            .filter(|d| disabled.contains(&d.slug))
            .flat_map(|d| d.tools.iter().cloned())
            .collect();
        if !all_off {
            match wisp_mcp::McpClient::launch_bio_tools(&env.python(), &pkg, &service_env).await {
                Ok(client) => {
                    register_mcp_filtered(agent, std::sync::Arc::new(client), &skip).await
                }
                Err(e) => errors.push(format!("MCP {pkg}: {e}")),
            }
        }
    }

    // User-configured connections. Connect concurrently: each HTTP server has
    // a 10s connect timeout, so a sequential loop could stall first-message
    // startup by 10s per unreachable server (#67). Registration stays in
    // config order so tool ordering is deterministic.
    let conns: Vec<McpConnection> = load_mcp_connections(store)
        .await
        .into_iter()
        .filter(|c| c.enabled)
        .collect();
    let mut set = tokio::task::JoinSet::new();
    for (i, conn) in conns.into_iter().enumerate() {
        set.spawn(async move {
            let res = connect_mcp(&conn).await;
            (i, conn.name, res)
        });
    }
    let mut results = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(r) = joined {
            results.push(r);
        }
    }
    results.sort_by_key(|(i, _, _)| *i);
    for (_, name, res) in results {
        match res {
            Ok(client) => register_mcp(agent, std::sync::Arc::new(client)).await,
            Err(e) => errors.push(format!("MCP '{name}': {e}")),
        }
    }
    errors
}

async fn register_mcp(agent: &mut wisp_core::Agent, client: std::sync::Arc<wisp_mcp::McpClient>) {
    register_mcp_filtered(agent, client, &HashSet::new()).await
}

/// Like `register_mcp`, but skips any tool whose name is in `skip` (used to drop
/// disabled bio-tools domains from the shared `mcp_bio` aggregate).
async fn register_mcp_filtered(
    agent: &mut wisp_core::Agent,
    client: std::sync::Arc<wisp_mcp::McpClient>,
    skip: &HashSet<String>,
) {
    match client.tools_list().await {
        Ok(tools) => {
            for t in tools {
                if skip.contains(&t.name) {
                    continue;
                }
                agent.add_tool(Box::new(wisp_mcp::McpTool::new(t, client.clone())));
            }
        }
        Err(e) => tracing::warn!("mcp tools_list failed: {e}"),
    }
}

/// Get the active session frame id, creating a new SQLite frame if none.
/// Create a brand-new SQLite frame for the active project and return its id.
/// Used by `new_session` (and the lazy first-send path) to hand the UI a
/// concrete session id before streaming starts.
async fn create_session_frame(store: &Store, project_id: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    store
        .create_frame(&id, project_id, "OPERON", "wisp")
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(id)
}

/// Return the active frame id, creating one if the UI hasn't picked a session
/// yet. Used by artifact registration so uploads attach to the conversation
/// the user is composing in.
async fn ensure_active_frame(
    state: &AppState,
    label: &str,
    ap: &ActiveProject,
) -> Result<String, String> {
    if let Some(id) = state.active_frame(label) {
        return Ok(id);
    }
    let id = create_session_frame(&state.store, &ap.id).await?;
    state.set_active_frame(label, Some(id.clone()));
    Ok(id)
}

#[tauri::command]
async fn send_message(
    state: State<'_, AppState>,
    app: AppHandle,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
    message: String,
    resume: Option<bool>,
) -> Result<String, String> {
    let resume = resume.unwrap_or(false);
    if !resume && message.trim().is_empty() {
        return Err("message is empty".into());
    }
    let (provider, api_url, model, api_key) = load_settings(&state.store).await;
    let (max_tokens, reasoning_effort) = models::active_llm_advanced(&state.store).await;
    let model_label = models::active_label(&state.store).await;
    let cfg = build_provider_config(
        &provider,
        &api_url,
        &api_key,
        &model,
        max_tokens,
        &reasoning_effort,
    )?;

    let max_context = state
        .store
        .get_setting("max_context")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1_000_000);
    let max_iter = state
        .store
        .get_setting("max_iter")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100);

    let ap = state.active(window.label());

    // Resolve the target session frame: an explicit id wins, else lazily create
    // one (mirrors the legacy first-send behavior). The frame id is what every
    // streamed event carries, so the UI can route by session.
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => create_session_frame(&state.store, &ap.id).await?,
    };
    state.set_active_frame(window.label(), Some(frame_id.clone()));

    // Get or create this session's runtime. The map mutex is dropped here —
    // the per-session `agent` mutex (not this map) is what the turn holds,
    // so a turn in session A never blocks a turn in session B.
    let rt = {
        let mut sessions = state.sessions.lock().await;
        sessions
            .entry(frame_id.clone())
            .or_insert_with(|| Arc::new(SessionRuntime::new()))
            .clone()
    };

    let mut guard = rt.agent.lock().await;
    if guard.is_none() {
        let skills = active_skill_index(&state.store, &ap).await;
        let mut agent = Agent::new(
            cfg.clone(),
            skills.clone(),
            ap.memory.clone(),
            ap.root.clone(),
            max_context,
            max_iter,
            load_memory_enabled(&state.store).await,
        );
        match state.store.load_messages(&frame_id).await {
            Ok(msgs) => agent.ctx.messages = msgs,
            Err(e) => tracing::warn!("load session from sqlite failed: {e}"),
        }
        rt.set_last_seq(agent.ctx.messages.len() as i64);
        if agent.ctx.is_empty() {
            let hosts = ssh_hosts::stored_hosts(&state.store).await;
            agent.seed_system_prompt(&skills, ssh_hosts::render_hosts_section(&hosts));
        }
        let wire_errors = wire_python_and_mcp(&mut agent, &state.app_data, &state.store).await;
        if !wire_errors.is_empty() {
            state.bootstrap.lock().unwrap().errors.extend(wire_errors);
        }
        *guard = Some(agent);
    }
    let agent = guard.as_mut().unwrap();
    rt.cancel.store(false, Ordering::Relaxed);

    // Incremental persistence: a background task appends each message the turn
    // produces to SQLite as it arrives (via TauriOutput::on_message), so a crash
    // no longer loses the whole turn. The task owns the running seq, so it stays
    // correct even if the in-memory context is compacted mid-turn.
    //
    // First flush any messages already in the context but not yet persisted
    // (e.g. a system prompt seeded here), so the incremental seq lines up with
    // what a later reload expects.
    let start_seq = {
        let start = rt.last_seq() as usize;
        if start < agent.ctx.messages.len() {
            let mut seq = rt.last_seq();
            for m in &agent.ctx.messages[start..] {
                seq += 1;
                let _ = state.store.append_message(&frame_id, seq, m).await;
            }
            rt.set_last_seq(agent.ctx.messages.len() as i64);
        }
        rt.last_seq()
    };

    let (persist_handle, persist_tx) = {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
        let store = state.store.clone();
        let fid = frame_id.clone();
        let stamp = model_label.clone();
        let mut seq = start_seq;
        let handle = tokio::spawn(async move {
            while let Some(mut msg) = rx.recv().await {
                if msg.role == wisp_llm::Role::Assistant && msg.model_name.is_none() {
                    msg.model_name = Some(stamp.clone());
                }
                seq += 1;
                if let Err(e) = store.append_message(&fid, seq, &msg).await {
                    tracing::warn!("incremental persist seq {seq} failed: {e}");
                }
            }
            seq
        });
        (handle, tx)
    };

    let (prov_handle, prov_tx) = {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<wisp_core::ProvenanceRecord>();
        let store = state.store.clone();
        let app_data = state.app_data.clone();
        let fid = frame_id.clone();
        let handle = tokio::spawn(async move {
            let mut env_hash: Option<String> = None;
            while let Some(rec) = rx.recv().await {
                if env_hash.is_none() {
                    env_hash = capture_env(&store, &app_data).await;
                }
                let cell_index = store.next_cell_index(&fid).await.unwrap_or(0);
                let e = wisp_store::ExecLog {
                    id: Uuid::new_v4().to_string(),
                    frame_id: fid.clone(),
                    cell_index,
                    tool: rec.tool,
                    language: rec.language,
                    source: rec.source,
                    stdout: rec.output,
                    stderr: String::new(),
                    exit_status: if rec.success { "ok".into() } else { "error".into() },
                    wall_s: None,
                    files_written: rec.files_written,
                    files_read: rec.files_read,
                    env_hash: env_hash.clone(),
                };
                if let Err(e) = store.insert_execution_log(&e).await {
                    tracing::warn!("provenance persist failed: {e}");
                }
            }
        });
        (handle, tx)
    };

    let output = TauriOutput {
        app: app.clone(),
        frame_id: frame_id.clone(),
        confirms: state.confirms.clone(),
        awaiting_confirm: state.awaiting_confirm.clone(),
        approvals: state.approvals.clone(),
        persist: Some(persist_tx),
        prov: Some(prov_tx),
    };

    state.running_turns.lock().await.insert(frame_id.clone());
    let result = if resume {
        agent.run_resume(&output, Some(&rt.cancel)).await
    } else {
        agent.run(&message, &output, Some(&rt.cancel)).await
    };
    state.running_turns.lock().await.remove(&frame_id);
    agent.ctx.clear_runtime_injections();

    // Close the persist channel and wait for the task to flush; its final seq is
    // the authoritative persisted count.
    drop(output);
    match tokio::time::timeout(std::time::Duration::from_secs(5), persist_handle).await {
        Ok(Ok(final_seq)) => rt.set_last_seq(final_seq),
        other => {
            tracing::warn!("persist task did not finish cleanly: {other:?}");
            rt.set_last_seq(agent.ctx.messages.len() as i64);
        }
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), prov_handle).await;
    drop(guard);

    match result {
        Ok(_) => {
            let _ = app.emit(
                "agent",
                AgentEvent::Done {
                    frame_id: frame_id.clone(),
                },
            );
            Ok(frame_id)
        }
        Err(e) => {
            let _ = app.emit(
                "agent",
                AgentEvent::Error {
                    frame_id: frame_id.clone(),
                    message: format!("{e}"),
                },
            );
            Err(format!("{e}"))
        }
    }
}

#[tauri::command]
async fn stop_agent(state: State<'_, AppState>, session_id: Option<String>) -> Result<(), String> {
    // Cancel only the named session's turn; other conversations keep running.
    let targets: Vec<Arc<SessionRuntime>> = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => state
            .sessions
            .lock()
            .await
            .get(id)
            .cloned()
            .into_iter()
            .collect(),
        None => state.sessions.lock().await.values().cloned().collect(),
    };
    for rt in targets {
        rt.cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// L1 session review: one read-only reviewer LLM call over the current
/// transcript. No sub-agent, no tools — traces claims and reports findings.
#[tauri::command]
async fn review_session(
    state: State<'_, AppState>,
    app: AppHandle,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
) -> Result<(), String> {
    if state.reviewing.swap(true, Ordering::SeqCst) {
        return Err("A review is already running.".into());
    }
    let out: Result<(), String> = async {
        // Refuse only if *that* session has a turn mid-flight — a parallel
        // conversation running elsewhere must not block the review.
        let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(id) => id.to_string(),
            None => state
                .active_frame(window.label())
                .ok_or_else(|| "No active session to review.".to_string())?,
        };
        if let Some(rt) = state.sessions.lock().await.get(&frame_id).cloned() {
            if rt.agent.try_lock().is_err() {
                return Err("Session is busy — wait for the current turn to finish.".to_string());
            }
        }

        let msgs = state
            .store
            .load_messages(&frame_id)
            .await
            .map_err(|e| format!("{e}"))?;
        if msgs
            .iter()
            .all(|m| matches!(m.role, wisp_llm::Role::System))
        {
            return Err("Nothing to review yet.".into());
        }
        let transcript = review::serialize_transcript(&msgs);

        let (provider, api_url, model, api_key) = load_settings(&state.store).await;
        let (max_tokens, reasoning_effort) = models::active_llm_advanced(&state.store).await;
        let cfg = build_provider_config(
            &provider,
            &api_url,
            &api_key,
            &model,
            max_tokens,
            &reasoning_effort,
        )?;
        let llm = wisp_llm::build(cfg);

        let review_msgs = vec![
            Message::system(review::REVIEWER_RUBRIC),
            Message::user(transcript),
        ];
        let completion = llm
            .complete(&review_msgs, &[])
            .await
            .map_err(|e| format!("{e}"))?;

        app.emit(
            "agent",
            AgentEvent::Review {
                frame_id,
                markdown: completion.content,
            },
        )
        .map_err(|e| format!("{e}"))?;
        Ok(())
    }
    .await;
    state.reviewing.store(false, Ordering::SeqCst);
    out
}

#[tauri::command]
async fn new_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<String, String> {
    // Create a fresh frame and hand its id to the UI up front, so the UI can
    // route streamed events to the right transcript *before* the first delta
    // arrives. Does NOT cancel any running turn — parallel conversations keep
    // running. Empty frames are filtered out of the sidebar until they get a
    // user message.
    let ap = state.active(window.label());
    let id = create_session_frame(&state.store, &ap.id).await?;
    state.set_active_frame(window.label(), Some(id.clone()));
    Ok(id)
}

#[tauri::command]
async fn list_sessions(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<SessionInfo>, String> {
    let ap = state.active(window.label());
    let rows = state
        .store
        .list_sessions(&ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|(id, title, ts, folder_id)| SessionInfo {
            id,
            title,
            ts,
            folder_id,
        })
        .collect())
}

#[tauri::command]
async fn list_folders(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<FolderInfo>, String> {
    let ap = state.active(window.label());
    let rows = state
        .store
        .list_folders(&ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|(id, name, _)| FolderInfo { id, name })
        .collect())
}

#[tauri::command]
async fn create_folder(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
) -> Result<FolderInfo, String> {
    let ap = state.active(window.label());
    let id = Uuid::new_v4().to_string();
    state
        .store
        .create_folder(&id, &ap.id, &name)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(FolderInfo {
        id,
        name: name.trim().to_string(),
    })
}

#[tauri::command]
async fn rename_folder(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    name: String,
) -> Result<(), String> {
    let ap = state.active(window.label());
    state
        .store
        .rename_folder(&id, &ap.id, &name)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
async fn delete_folder(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    let ap = state.active(window.label());
    state
        .store
        .delete_folder(&id, &ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
async fn move_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    folder_id: Option<String>,
) -> Result<(), String> {
    let ap = state.active(window.label());
    state
        .store
        .move_session_to_folder(&id, &ap.id, folder_id.as_deref())
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
async fn delete_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    let ap = state.active(window.label());
    if let Some(rt) = state.sessions.lock().await.get(&id) {
        rt.cancel.store(true, Ordering::Relaxed);
    }
    state.sessions.lock().await.remove(&id);
    if state.active_frame(window.label()).as_deref() == Some(id.as_str()) {
        state.set_active_frame(window.label(), None);
    }
    state
        .store
        .delete_session(&id, &ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
async fn rename_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    title: String,
) -> Result<(), String> {
    let ap = state.active(window.label());
    state
        .store
        .rename_session(&id, &ap.id, &title)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

/// How many sessions appear on the Projects landing "Recent sessions" column.
const RECENT_SESSIONS_LIMIT: i64 = 5;

#[tauri::command]
async fn list_recent_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let running = state.running_turns.lock().await.clone();
    let awaiting = state.awaiting_confirm.lock().unwrap().clone();
    let rows = state
        .store
        .list_recent_sessions_detail(RECENT_SESSIONS_LIMIT)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let status =
                session_runtime_status(&r.id, r.last_role.as_deref(), &running, &awaiting);
            serde_json::json!({
                "id": r.id,
                "project_id": r.project_id,
                "title": r.title,
                "ts": r.created_at,
                "status": status,
            })
        })
        .collect())
}

#[tauri::command]
async fn list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectSummary>, String> {
    let running = state.running_turns.lock().await.clone();
    let awaiting = state.awaiting_confirm.lock().unwrap().clone();
    let rows = state
        .store
        .list_projects()
        .await
        .map_err(|e| format!("{e}"))?;
    let mut out = vec![];
    for (id, name, ws, _c, upd, cnt, desc) in rows {
        let (running_count, needs_you_count) =
            project_status_counts(&state.store, &id, &running, &awaiting).await;
        out.push(ProjectSummary {
            id,
            name,
            description: desc,
            workspace_dir: ws,
            session_count: cnt,
            updated_at: upd,
            running_count,
            needs_you_count,
        });
    }
    Ok(out)
}

#[tauri::command]
async fn pick_directory(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p);
    });
    let picked = rx.await.map_err(|e| format!("{e}"))?;
    Ok(picked.map(|fp| fp.to_string()))
}

/// Copy a workspace file to a user-chosen location via the native save dialog.
/// Returns the saved path, or `None` if the user cancelled.
#[tauri::command]
async fn download_file(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    path: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    // Resolve + validate against the active workspace root, same as read_file.
    let real = {
        let ap = state.active(window.label());
        wisp_tools::safety::validate_file_path(&ap.root, &path)?
    };
    if !real.is_file() {
        return Err(format!("file not found: {path}"));
    }
    let default_name = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(dest) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None); // user cancelled
    };
    let dest_path = std::path::PathBuf::from(dest.to_string());
    std::fs::copy(&real, &dest_path).map_err(|e| format!("copy failed: {e}"))?;
    Ok(Some(dest_path.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn create_project(
    state: State<'_, AppState>,
    name: String,
    workspace_dir: String,
    description: String,
    agent_context: String,
) -> Result<ProjectSummary, String> {
    if name.trim().is_empty() {
        return Err("Project name is required".into());
    }
    let dir = workspace_dir.trim();
    if dir.is_empty() {
        return Err("A working directory is required".into());
    }
    let path = PathBuf::from(dir);
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Failed to create working directory: {e}"))?;
    // Writability probe: create + remove a temp marker.
    let marker = path.join(".wisp-write-test");
    std::fs::write(&marker, b"").map_err(|e| format!("Working directory is not writable: {e}"))?;
    let _ = std::fs::remove_file(&marker);

    let id = Uuid::new_v4().to_string();
    state
        .store
        .create_project(&id, name.trim(), dir)
        .await
        .map_err(|e| format!("{e}"))?;
    // Description (DB) + Agent Context (.wisp/WISP.md) — same storage as update_project.
    let desc = description.trim();
    if !desc.is_empty() {
        state
            .store
            .update_project(&id, name.trim(), desc)
            .await
            .map_err(|e| format!("{e}"))?;
    }
    let ctx = agent_context.trim();
    if !ctx.is_empty() {
        let wisp_dir = path.join(".wisp");
        std::fs::create_dir_all(&wisp_dir)
            .map_err(|e| format!("Failed to write Agent Context: {e}"))?;
        std::fs::write(wisp_dir.join("WISP.md"), ctx)
            .map_err(|e| format!("Failed to write Agent Context: {e}"))?;
    }
    Ok(build_project_summary(&state, &id).await)
}

/// Cancel and drop every in-memory runtime belonging to `project_id`'s sessions
/// (e.g. the project is being deleted). Other projects' sessions keep running —
/// switching/closing a project must not stop unrelated work (#52). Call this
/// *before* the project's frames are removed from the store.
async fn cancel_project_sessions(state: &AppState, project_id: &str) {
    let frame_ids: Vec<String> = state
        .store
        .list_sessions(project_id)
        .await
        .map(|rows| rows.into_iter().map(|(id, ..)| id).collect())
        .unwrap_or_default();
    {
        let mut sessions = state.sessions.lock().await;
        for fid in &frame_ids {
            if let Some(rt) = sessions.remove(fid) {
                rt.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
    let mut running = state.running_turns.lock().await;
    for fid in &frame_ids {
        running.remove(fid);
    }
}

/// Point the backend's active project at `id`, rebuilding its skills/memory.
/// Returns the resolved `(name, workspace_dir)`. `id` must exist in the store.
///
/// Switching projects no longer tears down the previous project's sessions —
/// each session's agent already captured its own root/skills/memory at creation,
/// so cross-project turns run in parallel and stay monitorable on the dashboard
/// (#52). Deleting a project stops only *its* sessions (see `delete_project`).
async fn set_active_project(
    state: &AppState,
    label: &str,
    id: &str,
) -> Result<(String, String), String> {
    let (name, ws) = state
        .store
        .get_project(id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Project not found".to_string())?;
    let root = ensure_writable(PathBuf::from(&ws), &state.app_data);
    let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
    let memory = Arc::new(MemoryManager::new(&root));
    state.set_active(
        label,
        ActiveProject {
            id: id.to_string(),
            root: root.clone(),
            skills,
            memory,
        },
    );
    state.set_active_frame(label, None);
    {
        state.bootstrap.lock().unwrap().workspace = root.to_string_lossy().into_owned();
    }
    let _ = state.store.set_setting("active_project_id", id).await;
    Ok((name, ws))
}

#[tauri::command]
async fn open_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<ProjectSummary, String> {
    let (name, ws) = set_active_project(state.inner(), window.label(), &id).await?;
    let _ = state.store.create_project(&id, &name, &ws).await; // touch updated_at → sorts to top
    Ok(build_project_summary(&state, &id).await)
}

/// Project ids that currently have their own window, persisted so the set can be
/// restored on the next launch (#52, Phase 3). Stored as a JSON array setting.
async fn persisted_windows(store: &Store) -> Vec<String> {
    store
        .get_setting("open_project_windows")
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

async fn update_persisted_windows(store: &Store, id: &str, present: bool) {
    let mut v = persisted_windows(store).await;
    let had = v.iter().any(|x| x == id);
    if present && !had {
        v.push(id.to_string());
    } else if !present && had {
        v.retain(|x| x != id);
    } else {
        return;
    }
    let _ = store
        .set_setting(
            "open_project_windows",
            &serde_json::to_string(&v).unwrap_or_default(),
        )
        .await;
}

fn project_window_label(id: &str) -> String {
    format!("proj-{id}") // project ids are UUIDs or "default" — label-safe
}

/// Open a project in its own window (or focus the existing one), wiring up
/// cleanup on close. Shared by the `open_project_window` command and the
/// startup restore (#52).
async fn spawn_project_window(
    app: &AppHandle,
    state: &AppState,
    id: &str,
) -> Result<String, String> {
    let label = project_window_label(id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
        return Ok(label);
    }
    // Pre-set this window's active project so its first commands resolve correctly
    // even before the window's frontend calls open_project.
    set_active_project(state, &label, id).await?;
    let url = tauri::WebviewUrl::App(format!("index.html?project={id}").into());
    let win = tauri::WebviewWindowBuilder::new(app, &label, url)
        .title("wisp-science")
        .inner_size(1100.0, 760.0)
        .resizable(true)
        .build()
        .map_err(|e| e.to_string())?;
    let evt_app = app.clone();
    let evt_label = label.clone();
    let evt_id = id.to_string();
    win.on_window_event(move |ev| {
        if matches!(ev, tauri::WindowEvent::Destroyed) {
            // Drop this window's per-window project context and stop persisting
            // it for restore. Its running sessions are tracked globally and keep
            // going until they finish or are stopped.
            let st = evt_app.state::<AppState>();
            st.active.write().unwrap().remove(&evt_label);
            st.active_frame.write().unwrap().remove(&evt_label);
            let store = st.store.clone();
            let id = evt_id.clone();
            tauri::async_runtime::spawn(async move {
                update_persisted_windows(&store, &id, false).await;
            });
        }
    });
    update_persisted_windows(&state.store, id, true).await;
    Ok(label)
}

/// Open a project in its own window (or focus the existing one). Each window
/// carries its own active project, keyed by window label (#52).
#[tauri::command]
async fn open_project_window(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    spawn_project_window(&app, state.inner(), &id).await
}

#[tauri::command]
async fn delete_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    // The delete ✕ is only reachable from the projects list, so a project may
    // legitimately be deleted while it's still the backend's *active* one
    // (returning to the list is a frontend-only nav — it never told the backend
    // to leave). Delete it, then fall back to the always-present "default"
    // workspace so `active` never dangles at a deleted project.
    let was_active = state.active(window.label()).id == id;
    // Stop the deleted project's own running sessions (gather frame ids before
    // the store cascade removes them); other projects keep running (#52).
    cancel_project_sessions(state.inner(), &id).await;
    state
        .store
        .delete_project(&id)
        .await
        .map_err(|e| format!("{e}"))?;
    if was_active {
        let _ = set_active_project(state.inner(), window.label(), "default").await;
    }
    Ok(())
}

#[derive(Serialize, Clone)]
struct ProjectSettings {
    id: String,
    name: String,
    description: String,
    agent_context: String,
}

/// Read the active project's editable settings for the Project Settings modal.
/// Agent Context is `.wisp/WISP.md`, injected into every seeded system prompt.
#[tauri::command]
async fn get_project_settings(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<ProjectSettings, String> {
    let ap = state.active(window.label());
    let (name, description, _ws) = state
        .store
        .get_project_meta(&ap.id)
        .await
        .map_err(|e| format!("{e}"))?
        .unwrap_or_default();
    let agent_context =
        std::fs::read_to_string(ap.root.join(".wisp").join("WISP.md")).unwrap_or_default();
    Ok(ProjectSettings {
        id: ap.id.clone(),
        name,
        description,
        agent_context,
    })
}

/// Save the active project's name/description (DB) and Agent Context (.wisp/WISP.md).
/// An empty Agent Context removes WISP.md so the prompt falls back to "no rules".
/// Takes effect on the next seeded session; already-running agents keep their prompt.
#[tauri::command]
async fn update_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    description: String,
    agent_context: String,
) -> Result<ProjectSummary, String> {
    if name.trim().is_empty() {
        return Err("Project name is required".into());
    }
    let ap = state.active(window.label());
    state
        .store
        .update_project(&ap.id, name.trim(), description.trim())
        .await
        .map_err(|e| format!("{e}"))?;
    let wisp_dir = ap.root.join(".wisp");
    let wisp_md = wisp_dir.join("WISP.md");
    let ctx = agent_context.trim();
    if ctx.is_empty() {
        let _ = std::fs::remove_file(&wisp_md);
    } else {
        std::fs::create_dir_all(&wisp_dir)
            .map_err(|e| format!("Failed to write Agent Context: {e}"))?;
        std::fs::write(&wisp_md, ctx).map_err(|e| format!("Failed to write Agent Context: {e}"))?;
    }
    Ok(build_project_summary(&state, &ap.id).await)
}

/// Switch the active session to `id`, load its transcript, and return the
/// rendered rows so the UI can repopulate the conversation view.
/// Rewind the named session to just before the given user turn (for message
/// edit). Only touches that session's agent context and DB rows.
#[tauri::command]
async fn rewind_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
    user_index: usize,
) -> Result<(), String> {
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => state
            .active_frame(window.label())
            .ok_or_else(|| "No active session to rewind.".to_string())?,
    };
    let rt = state.sessions.lock().await.get(&frame_id).cloned();
    let keep = if let Some(rt) = rt {
        let mut guard = rt.agent.lock().await;
        if let Some(agent) = guard.as_mut() {
            let k = user_message_start(&agent.ctx.messages, user_index);
            agent.ctx.messages.truncate(k);
            k
        } else {
            user_index_to_keep_after_db(&state.store, &frame_id, user_index).await?
        }
    } else {
        user_index_to_keep_after_db(&state.store, &frame_id, user_index).await?
    };
    state
        .store
        .truncate_messages(&frame_id, keep as i64)
        .await
        .map_err(|e| format!("{e}"))?;
    if let Some(rt) = state.sessions.lock().await.get(&frame_id) {
        rt.set_last_seq(keep as i64);
    }
    Ok(())
}

/// Compute the `keep` index purely from persisted messages when no in-memory
/// agent exists for the session yet.
async fn user_index_to_keep_after_db(
    store: &Store,
    frame_id: &str,
    user_index: usize,
) -> Result<usize, String> {
    let msgs = store
        .load_messages(frame_id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(user_message_start(&msgs, user_index))
}

#[tauri::command]
async fn load_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Vec<UiItem>, String> {
    let msgs = state
        .store
        .load_messages(&id)
        .await
        .map_err(|e| format!("{e}"))?;
    // Track which session the UI is viewing. If a runtime exists for it (e.g.
    // it's mid-stream), keep the in-memory agent context authoritative — the UI
    // will render the cached streaming transcript instead of this DB snapshot.
    state.set_active_frame(window.label(), Some(id.clone()));
    if let Some(rt) = state.sessions.lock().await.get(&id).cloned() {
        rt.set_last_seq(msgs.len() as i64);
    }
    Ok(messages_to_items(&msgs))
}

#[tauri::command]
async fn list_skills(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<SkillInfo>, String> {
    let ap = state.active(window.label());
    let tags = load_skill_tags(&state.store).await;
    let enabled = effective_enabled_skill_names(&state.store, &ap).await;
    Ok(skill_infos(&ap.skills, &tags, enabled.as_ref()))
}

#[tauri::command]
async fn set_skill_tags(
    state: State<'_, AppState>,
    name: String,
    tags: Vec<String>,
) -> Result<(), String> {
    let mut all_tags = load_skill_tags(&state.store).await;
    let tags = normalize_tags(tags);
    if tags.is_empty() {
        all_tags.remove(&name);
    } else {
        all_tags.insert(name, tags);
    }
    save_skill_tags(&state.store, &all_tags).await
}

async fn update_skills_enabled(
    state: &AppState,
    label: &str,
    names: Vec<String>,
    enabled: bool,
) -> Result<(), String> {
    let ap = state.active(label);
    let mut current = effective_enabled_skill_names(&state.store, &ap)
        .await
        .unwrap_or_else(|| ap.skills.all().iter().map(|s| s.name.clone()).collect());
    let known = ap
        .skills
        .all()
        .iter()
        .map(|s| s.name.as_str())
        .collect::<HashSet<_>>();
    for name in names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && known.contains(n.as_str()))
    {
        if enabled {
            current.insert(name);
        } else {
            current.remove(&name);
        }
    }
    save_enabled_skill_names(&state.store, &ap.id, &current).await?;
    clear_idle_agents(state).await;
    Ok(())
}

#[tauri::command]
async fn set_skill_enabled(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    update_skills_enabled(&state, window.label(), vec![name], enabled).await
}

#[tauri::command]
async fn set_skills_enabled(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    names: Vec<String>,
    enabled: bool,
) -> Result<(), String> {
    update_skills_enabled(&state, window.label(), names, enabled).await
}

#[derive(Serialize, Clone)]
struct McpConnectionsView {
    connections: Vec<McpConnection>,
}

#[tauri::command]
async fn list_mcp_connections(state: State<'_, AppState>) -> Result<McpConnectionsView, String> {
    Ok(McpConnectionsView {
        connections: load_mcp_connections(&state.store).await,
    })
}

#[tauri::command]
async fn add_mcp_connection(state: State<'_, AppState>, conn: McpConnection) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    conns.push(conn);
    save_mcp_connections(&state.store, &conns).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn update_mcp_connection(
    state: State<'_, AppState>,
    conn: McpConnection,
) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    match conns.iter_mut().find(|c| c.id == conn.id) {
        Some(slot) => *slot = conn,
        None => return Err("connection not found".into()),
    }
    save_mcp_connections(&state.store, &conns).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn delete_mcp_connection(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    conns.retain(|c| c.id != id);
    save_mcp_connections(&state.store, &conns).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn set_mcp_connection_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut conns = load_mcp_connections(&state.store).await;
    if let Some(c) = conns.iter_mut().find(|c| c.id == id) {
        c.enabled = enabled;
    }
    save_mcp_connections(&state.store, &conns).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

// ── Connectors tree (multi-level Connections UI) ────────────────────────────

#[derive(Serialize, Clone)]
struct ConnectorTool {
    name: String,
    /// Effective approval mode: "allow" | "ask" | "deny".
    mode: String,
}

#[derive(Serialize, Clone)]
struct ConnectorInfo {
    /// Domain slug (bundled) or connection id (custom).
    key: String,
    name: String,
    /// "bundled" | "custom".
    kind: String,
    enabled: bool,
    skip_approvals: bool,
    /// "stdio" | "http" for custom connectors; empty for bundled.
    transport: String,
    /// Command/URL line for custom connectors; empty for bundled.
    subtitle: String,
    /// Tools for bundled connectors (static from domains.json). Custom
    /// connectors list none — their tools aren't known without launching.
    tools: Vec<ConnectorTool>,
}

#[derive(Serialize, Clone)]
struct ConnectorsView {
    connectors: Vec<ConnectorInfo>,
    /// Global approval scope ("full" | "auto" | "ask").
    scope: String,
}

#[tauri::command]
async fn list_connectors(state: State<'_, AppState>) -> Result<ConnectorsView, String> {
    let store = &state.store;
    let disabled = load_disabled_connectors(store).await;
    let approvals = load_tool_approvals(store).await;
    let skip = load_skip_connectors(store).await;

    let mut connectors = vec![];
    for d in bio_domains() {
        let skip_on = skip.contains(&d.slug);
        let tools = d
            .tools
            .iter()
            .map(|t| ConnectorTool {
                mode: if skip_on {
                    "allow".into()
                } else {
                    approvals.get(t).cloned().unwrap_or_else(|| "allow".into())
                },
                name: t.clone(),
            })
            .collect();
        connectors.push(ConnectorInfo {
            enabled: !disabled.contains(&d.slug),
            key: d.slug,
            name: d.name,
            kind: "bundled".into(),
            skip_approvals: skip_on,
            transport: String::new(),
            subtitle: String::new(),
            tools,
        });
    }
    for c in load_mcp_connections(store).await {
        let (transport, subtitle) = match &c.transport {
            McpTransport::Stdio { command, .. } => ("stdio", command.clone()),
            McpTransport::Http { url, .. } => ("http", url.clone()),
        };
        connectors.push(ConnectorInfo {
            key: c.id,
            name: c.name,
            kind: "custom".into(),
            enabled: c.enabled,
            skip_approvals: false,
            transport: transport.into(),
            subtitle,
            tools: vec![],
        });
    }
    let scope = load_approval_scope(store).await.as_str().to_string();
    Ok(ConnectorsView { connectors, scope })
}

/// Enable/disable a bundled connector (domain). Custom connectors use
/// `set_mcp_connection_enabled` instead.
#[tauri::command]
async fn set_connector_enabled(
    state: State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<(), String> {
    let mut disabled = load_disabled_connectors(&state.store).await;
    if enabled {
        disabled.remove(&key);
    } else {
        disabled.insert(key);
    }
    let list: Vec<String> = disabled.into_iter().collect();
    save_json_setting(&state.store, "disabled_connectors", &list).await?;
    clear_idle_agents(&state).await;
    Ok(())
}

/// Set the approval mode ("allow" | "ask" | "deny") for a single tool. Enforced
/// live on the next tool call — no session rebuild needed.
#[tauri::command]
async fn set_tool_approval(
    state: State<'_, AppState>,
    tool: String,
    mode: String,
) -> Result<(), String> {
    let mut approvals = load_tool_approvals(&state.store).await;
    // Store only overrides; "allow" is the default, so drop it to stay compact.
    if ApprovalMode::parse(&mode) == ApprovalMode::Allow {
        approvals.remove(&tool);
    } else {
        approvals.insert(tool, ApprovalMode::parse(&mode).as_str().into());
    }
    save_json_setting(&state.store, "tool_approvals", &approvals).await?;
    refresh_approval_policy(&state).await;
    Ok(())
}

/// Set the global approval scope ("full" | "auto" | "ask"). Enforced live on
/// the next tool call — no session rebuild needed.
#[tauri::command]
async fn set_approval_scope(state: State<'_, AppState>, scope: String) -> Result<(), String> {
    // Normalize through `Scope` so only the three valid values ever persist.
    save_json_setting(&state.store, "approval_scope", &Scope::parse(&scope).as_str()).await?;
    refresh_approval_policy(&state).await;
    Ok(())
}

/// Toggle "Skip approvals" for a connector (force-allow all its tools).
#[tauri::command]
async fn set_connector_skip_approvals(
    state: State<'_, AppState>,
    key: String,
    enabled: bool,
) -> Result<(), String> {
    let mut skip = load_skip_connectors(&state.store).await;
    if enabled {
        skip.insert(key);
    } else {
        skip.remove(&key);
    }
    let list: Vec<String> = skip.into_iter().collect();
    save_json_setting(&state.store, "skip_approval_connectors", &list).await?;
    refresh_approval_policy(&state).await;
    Ok(())
}

#[tauri::command]
async fn test_mcp_connection(
    _state: State<'_, AppState>,
    conn: McpConnection,
) -> Result<usize, String> {
    let client = connect_mcp(&conn).await.map_err(|e| format!("{e}"))?;
    let tools = client.tools_list().await.map_err(|e| format!("{e}"))?;
    Ok(tools.len())
}

#[tauri::command]
async fn pick_skill_source(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    // Let the user pick a SKILL.md; folder picking is offered via a second button
    // in the UI that calls pick_directory (existing command).
    app.dialog()
        .file()
        .add_filter("SKILL.md", &["md"])
        .pick_file(move |p| {
            let _ = tx.send(p);
        });
    let picked = rx.await.map_err(|e| format!("{e}"))?;
    Ok(picked.map(|fp| fp.to_string()))
}

fn user_skills_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".wisp").join("skills"))
        .ok_or_else(|| "no home directory".to_string())
}

/// Reject skill names that could escape the skills directory. A valid name is a
/// single path component: no separators, no `..`, non-empty.
fn validate_skill_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("skill name is empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!("invalid skill name '{name}'"));
    }
    // Must be exactly one path component (defends against platform-specific tricks).
    if std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        != Some(name)
    {
        return Err(format!("invalid skill name '{name}'"));
    }
    Ok(())
}

#[tauri::command]
async fn install_skill(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    src_path: String,
) -> Result<String, String> {
    let src = PathBuf::from(&src_path);
    // Resolve the skill's source dir + the SKILL.md path.
    let (skill_dir, skill_md) = if src.is_dir() {
        let md = src.join("SKILL.md");
        if !md.is_file() {
            return Err("selected folder has no SKILL.md".into());
        }
        (src.clone(), md)
    } else if src.file_name().map(|n| n == "SKILL.md").unwrap_or(false) {
        (
            src.parent().map(PathBuf::from).unwrap_or_default(),
            src.clone(),
        )
    } else {
        return Err("select a skill folder or a SKILL.md file".into());
    };
    // Parse name from frontmatter (fall back to dir name), validate description.
    let skill = wisp_skills::parse_skill_file(&skill_md)
        .ok_or_else(|| "could not parse SKILL.md frontmatter".to_string())?;
    if skill.description.trim().is_empty() {
        return Err("SKILL.md is missing a description".into());
    }
    validate_skill_name(&skill.name)?;
    let dest = user_skills_dir()?.join(&skill.name);
    if dest.exists() {
        return Err(format!("a skill named '{}' already exists", skill.name));
    }
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| format!("{e}"))?;
    copy_dir_recursive(&skill_dir, &dest).map_err(|e| format!("{e}"))?;
    reload_skills(&state, window.label());
    let ap = state.active(window.label());
    if let Some(mut enabled) = load_enabled_skill_names(&state.store, &ap.id).await {
        enabled.insert(skill.name.clone());
        save_enabled_skill_names(&state.store, &ap.id, &enabled).await?;
    }
    clear_idle_agents(&state).await;
    Ok(skill.name)
}

#[tauri::command]
async fn remove_skill(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
) -> Result<(), String> {
    validate_skill_name(&name)?;
    let dir = user_skills_dir()?.join(&name);
    if !dir.is_dir() {
        return Err("only user-added skills can be removed".into());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("{e}"))?;
    let ap = state.active(window.label());
    if let Some(mut enabled) = load_enabled_skill_names(&state.store, &ap.id).await {
        enabled.remove(&name);
        let _ = save_enabled_skill_names(&state.store, &ap.id, &enabled).await;
    }
    let mut tags = load_skill_tags(&state.store).await;
    tags.remove(&name);
    let _ = save_skill_tags(&state.store, &tags).await;
    reload_skills(&state, window.label());
    clear_idle_agents(&state).await;
    Ok(())
}

fn reload_skills(state: &AppState, label: &str) {
    let mut ap = state.active(label);
    ap.skills = Arc::new(SkillIndex::load(&skill_paths(&ap.root)));
    state.set_active(label, ap);
}

fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

#[tauri::command]
fn list_demos() -> Vec<seed::DemoInfo> {
    seed::list_demos()
}

#[tauri::command]
fn load_demo(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<seed::Demo, String> {
    let ap = state.active(window.label());
    seed::extract_demo_assets(&id, &ap.root)?;
    seed::load_demo(&id).ok_or_else(|| format!("demo '{id}' not found"))
}

#[tauri::command]
fn confirm_response(
    state: State<'_, AppState>,
    session_id: String,
    approved: bool,
) -> Result<(), String> {
    if let Some(tx) = state.confirms.lock().unwrap().remove(&session_id) {
        let _ = tx.send(approved);
        Ok(())
    } else {
        Err("no pending confirmation".into())
    }
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let (provider, api_url, model, _api_key) = load_settings(&state.store).await;
    let locale = load_locale(&state.store).await;
    let workspace_dir = state
        .store
        .get_setting("workspace_dir")
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let (max_tokens, reasoning_effort) = models::active_llm_advanced(&state.store).await;
    let has_api_key = models::active_has_key(&state.store).await;
    let label = models::active_label(&state.store).await;
    Ok(Settings {
        provider,
        api_url,
        model,
        label,
        has_api_key,
        locale,
        workspace_dir,
        max_tokens,
        reasoning_effort,
    })
}

#[tauri::command]
async fn set_settings(state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    let provider = normalized_provider(&settings.provider);
    let api_url = settings.api_url.trim();
    let model = settings.model.trim();
    if api_url.is_empty() {
        return Err("API URL is required.".into());
    }
    if model.is_empty() {
        return Err("Model is required.".into());
    }
    tracing::info!(
        target: "wisp",
        provider = %provider,
        api_url = %api_url,
        model = %model,
        "saving settings"
    );
    // provider/api_url/model belong to the *active* model profile now, not a
    // single global config — the classic form edits whichever model is active.
    models::set_active_fields(
        &state.store,
        &provider,
        api_url,
        model,
        settings.label.trim(),
    )
    .await?;
    let locale = match settings.locale.trim() {
        "zh" | "zh-CN" | "zh-TW" => "zh",
        other if !other.is_empty() => other,
        _ => "en",
    };
    state
        .store
        .set_setting("locale", locale)
        .await
        .map_err(|e| format!("{e}"))?;

    // Workspace directory: persist an absolute, creatable path. Takes effect on
    // next launch (AppState.root is fixed at startup — restart, not hot-swap).
    let workspace_dir = settings.workspace_dir.trim();
    if workspace_dir.is_empty() {
        // Empty clears the override → back to the platform default next launch.
        state
            .store
            .set_setting("workspace_dir", "")
            .await
            .map_err(|e| format!("{e}"))?;
    } else {
        let ws = Path::new(workspace_dir);
        if !ws.is_absolute() {
            return Err("Workspace directory must be an absolute path.".into());
        }
        // Don't create the dir here. It only takes effect next launch, where
        // `ensure_writable` creates it (with a fallback). Creating it eagerly
        // during save can block the whole command on a bad/removable path —
        // e.g. Windows pops a modal "insert a disk in drive D:" — wedging the
        // UI at "Saving…" forever (#40). Just persist the string.
        state
            .store
            .set_setting("workspace_dir", workspace_dir)
            .await
            .map_err(|e| format!("{e}"))?;
    }

    // Reset cached agents so the next turn picks up the new provider.
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn set_api_key(state: State<'_, AppState>, key: String) -> Result<(), String> {
    tracing::info!(target: "wisp", has_api_key = !key.is_empty(), "saving api key");
    // The key belongs to the active model profile.
    models::set_active_key(&state.store, &key).await
}

#[tauri::command]
async fn credential_status() -> Result<Vec<(String, bool)>, String> {
    Ok(models::credential_status())
}

#[tauri::command]
async fn set_credential(
    state: State<'_, AppState>,
    id: String,
    value: String,
) -> Result<(), String> {
    let value = value.trim().to_string();
    // OpenAlex is the one service with a cheap online key probe: GET
    // /rate-limit carrying only api_key. 2xx or 429 (= authenticated but over
    // budget) means the key works; any other 4xx means OpenAlex rejected it.
    // Network trouble is treated like success (soft-degrade) — don't block
    // saving a key offline. Other credentials (NCBI key/email) have no cheap
    // standalone probe, so they're stored as-is.
    if id == "openalex_api_key" && !value.is_empty() {
        let resp = reqwest::Client::builder()
            .user_agent("wisp-science")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?
            .get("https://api.openalex.org/rate-limit")
            .query(&[("api_key", value.as_str())])
            .send()
            .await;
        if let Ok(r) = resp {
            let s = r.status();
            if s.is_client_error() && s.as_u16() != 429 {
                return Err("OpenAlex rejected this API key.".into());
            }
        }
    }
    tracing::info!(target: "wisp", id = %id, present = !value.is_empty(), "saving credential");
    models::store_credential(&id, &value)?;
    // Respawn kernels/MCP on the next turn so they inherit the new env.
    clear_idle_agents(&state).await;
    Ok(())
}

#[tauri::command]
async fn validate_settings(
    state: State<'_, AppState>,
    settings: Settings,
    key: Option<String>,
) -> Result<String, String> {
    let (_, _, _, stored_key) = load_settings(&state.store).await;
    let api_key = effective_api_key(key, stored_key);
    let provider_name = normalized_provider(&settings.provider);
    let mut cfg = build_provider_config(
        &settings.provider,
        &settings.api_url,
        &api_key,
        &settings.model,
        settings.max_tokens,
        &settings.reasoning_effort,
    )?;
    // Keep the ping cheap but respect API minimum (Responses API needs >= 16).
    cfg.max_tokens = cfg.max_tokens.min(64).max(16);

    tracing::info!(
        target: "wisp",
        provider = %provider_name,
        api_url = %settings.api_url,
        model = %settings.model,
        "validating settings"
    );
    let provider = wisp_llm::build(cfg);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        provider.complete(&[Message::user("Reply with OK.")], &[]),
    )
    .await
    .map_err(|_| {
        tracing::warn!(target: "wisp", "settings validation timed out");
        "Validation timed out after 30s".to_string()
    })?;
    if let Err(e) = result {
        tracing::warn!(target: "wisp", error = %e, "settings validation failed");
        return Err(format!("{e}"));
    }

    tracing::info!(target: "wisp", "settings validation succeeded");
    Ok(format!(
        "Validated {} with {}",
        provider_name, settings.model
    ))
}

fn mime_for_path(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("csv") => "text/csv",
        Some("tsv") => "text/tab-separated-values",
        Some("json") => "application/json",
        Some("md") => "text/markdown",
        Some("fasta" | "fa") => "text/x-fasta",
        Some("pdb") | Some("mol2") | Some("cif") => "chemical/x-pdb",
        Some("sdf" | "mol") => "chemical/x-mdl-molfile",
        _ => "application/octet-stream",
    }
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/") || mime == "application/json" || mime == "text/markdown"
}

#[tauri::command]
fn list_dir(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    path: Option<String>,
) -> Result<Vec<DirEntry>, String> {
    let ap = state.active(window.label());
    let rel = path.unwrap_or_else(|| ".".into());
    let dir = wisp_tools::safety::resolve_under_root(&ap.root, &rel)?;
    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", rel));
    }
    let mut entries = vec![];
    for ent in std::fs::read_dir(&dir).map_err(|e| format!("{e}"))? {
        let ent = ent.map_err(|e| format!("{e}"))?;
        let meta = ent.metadata().map_err(|e| format!("{e}"))?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        entries.push(DirEntry {
            name,
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

fn read_file_at(
    state: &AppState,
    label: &str,
    path: String,
    max_bytes: Option<u64>,
) -> Result<FileContent, String> {
    let ap = state.active(label);
    let real = wisp_tools::safety::validate_file_path(&ap.root, &path)?;
    let mime = mime_for_path(&real);
    let cap = max_bytes.unwrap_or(8 * 1024 * 1024).min(32 * 1024 * 1024);
    let bytes = std::fs::read(&real).map_err(|e| format!("{e}"))?;
    if bytes.len() as u64 > cap {
        return Err(format!("file exceeds {cap} byte limit"));
    }
    let path_str = real.to_string_lossy().into_owned();
    if is_text_mime(mime)
        || mime == "text/csv"
        || mime == "text/tab-separated-values"
        || mime == "text/x-fasta"
        || mime == "chemical/x-pdb"
        || mime == "chemical/x-mdl-molfile"
    {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(FileContent {
            path: path_str,
            mime: mime.into(),
            text: Some(text),
            base64: None,
        })
    } else {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(FileContent {
            path: path_str,
            mime: mime.into(),
            text: None,
            base64: Some(b64),
        })
    }
}

#[tauri::command]
fn read_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    path: String,
    max_bytes: Option<u64>,
) -> Result<FileContent, String> {
    read_file_at(&state, window.label(), path, max_bytes)
}

#[tauri::command]
async fn list_artifacts(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
) -> Result<Vec<ArtifactInfo>, String> {
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => Some(id.to_string()),
        None => state.active_frame(window.label()),
    };
    let Some(fid) = frame_id else {
        return Ok(vec![]);
    };
    let rows = state
        .store
        .list_artifacts(&fid)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|(id, name, ct, path, ts)| ArtifactInfo {
            id,
            name: name.clone(),
            kind: ct,
            path,
            ts,
        })
        .collect())
}

/// Given candidate artifact file paths (as they appear in chat), return the
/// subset that can't be previewed: resolved against the project root and
/// missing on disk, or outside the root. The UI drops these so a stale
/// intermediate file doesn't linger as an artifact that 404s on click (#41).
#[tauri::command]
fn missing_files(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let ap = state.active(window.label());
    Ok(paths
        .into_iter()
        .filter(|p| {
            wisp_tools::safety::validate_file_path(&ap.root, p)
                .map(|real| !real.exists())
                .unwrap_or(true)
        })
        .collect())
}

#[tauri::command]
async fn read_artifact(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<FileContent, String> {
    let row = state
        .store
        .get_artifact(&id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{id}' not found"))?;
    let (_name, _ct, storage_path, _frame) = row;
    read_file_at(&state, window.label(), storage_path, None)
}

fn mcp_lib_dir(_root: &std::path::Path) -> Option<PathBuf> {
    wisp_paths::bio_tools_dir().map(|d| d.join("lib"))
}

fn list_mcp_servers(root: &std::path::Path) -> Vec<String> {
    let Some(lib) = mcp_lib_dir(root) else {
        return vec![];
    };
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(&lib) {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with("mcp_") && ent.path().join("server.py").is_file() {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

fn count_memory_files(memory: &MemoryManager) -> usize {
    let Ok(rd) = std::fs::read_dir(memory.dir()) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .count()
}

fn list_memory_files(memory: &MemoryManager) -> Vec<MemoryFile> {
    let Ok(rd) = std::fs::read_dir(memory.dir()) else {
        return vec![];
    };
    let mut paths: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .map(|e| e.path())
        .collect();
    paths.sort_by(|a, b| b.cmp(a));
    paths
        .into_iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(&path).ok()?;
            let text = std::fs::read_to_string(&path).ok()?;
            let preview: String = text.chars().take(240).collect();
            Some(MemoryFile {
                name: path.file_name()?.to_string_lossy().into_owned(),
                preview,
                bytes: meta.len(),
            })
        })
        .collect()
}

async fn build_project_info(state: &AppState, label: &str) -> ProjectInfo {
    let ap = state.active(label);
    let (_, _, _, api_key) = load_settings(&state.store).await;
    let mcp = list_mcp_servers(&ap.root);
    // Prefer the user-set project name (Project Settings) over the folder name.
    let db_name = state
        .store
        .get_project(&ap.id)
        .await
        .ok()
        .flatten()
        .map(|(n, _)| n)
        .unwrap_or_default();
    let name = if db_name.trim().is_empty() {
        ap.root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Workspace")
            .to_string()
    } else {
        db_name
    };
    ProjectInfo {
        id: ap.id.clone(),
        name,
        root: ap.root.to_string_lossy().into_owned(),
        skill_count: ap.skills.all().len(),
        mcp_server_count: mcp.len(),
        memory_file_count: count_memory_files(&ap.memory),
        has_api_key: !api_key.is_empty(),
    }
}

#[tauri::command]
async fn get_project_info(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<ProjectInfo, String> {
    Ok(build_project_info(&state, window.label()).await)
}

#[tauri::command]
async fn get_capabilities(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Capabilities, String> {
    let ap = state.active(window.label());
    let project = build_project_info(&state, window.label()).await;
    let tags = load_skill_tags(&state.store).await;
    let enabled = effective_enabled_skill_names(&state.store, &ap).await;
    let skills = skill_infos(&ap.skills, &tags, enabled.as_ref());
    Ok(Capabilities {
        skills,
        mcp_servers: list_mcp_servers(&ap.root),
        memory_files: list_memory_files(&ap.memory),
        project,
    })
}

#[derive(Serialize, Clone)]
struct MemoryView {
    enabled: bool,
    today_file: String,
    files: Vec<MemoryFile>,
}

#[tauri::command]
fn list_memory(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active(window.label());
    Ok(list_memory_files(&ap.memory))
}

#[tauri::command]
async fn get_memory_view(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<MemoryView, String> {
    let ap = state.active(window.label());
    Ok(MemoryView {
        enabled: load_memory_enabled(&state.store).await,
        today_file: chrono::Local::now().format("%Y-%m-%d.md").to_string(),
        files: list_memory_files(&ap.memory),
    })
}

#[tauri::command]
async fn set_memory_enabled(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    enabled: bool,
) -> Result<MemoryView, String> {
    save_memory_enabled(&state.store, enabled).await?;
    clear_idle_agents(&state).await;
    let ap = state.active(window.label());
    Ok(MemoryView {
        enabled,
        today_file: chrono::Local::now().format("%Y-%m-%d.md").to_string(),
        files: list_memory_files(&ap.memory),
    })
}

#[tauri::command]
fn read_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
) -> Result<String, String> {
    let ap = state.active(window.label());
    let path = memory_file_path(&ap.memory, &name)?;
    if !path.is_file() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("{e}"))
}

#[tauri::command]
fn write_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    content: String,
) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active(window.label());
    let path = memory_file_path(&ap.memory, &name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("{e}"))?;
    Ok(list_memory_files(&ap.memory))
}

#[tauri::command]
fn delete_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active(window.label());
    let path = memory_file_path(&ap.memory, &name)?;
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|e| format!("{e}"))?;
    }
    Ok(list_memory_files(&ap.memory))
}

#[tauri::command]
fn clear_memory(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active(window.label());
    let Ok(rd) = std::fs::read_dir(ap.memory.dir()) else {
        return Ok(vec![]);
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(list_memory_files(&ap.memory))
}

#[tauri::command]
async fn get_onboarding_state(state: State<'_, AppState>) -> Result<OnboardingState, String> {
    let (_, _, _, api_key) = load_settings(&state.store).await;
    let done = state
        .store
        .get_setting("onboarding_done")
        .await
        .ok()
        .flatten()
        .is_some();
    Ok(OnboardingState {
        show: !done,
        has_api_key: !api_key.is_empty(),
    })
}

fn initial_bootstrap(
    app_data: &std::path::Path,
    workspace: &std::path::Path,
    skills: usize,
) -> BootstrapStatus {
    let mut status = BootstrapStatus {
        skills_loaded: skills,
        python_ok: false,
        mcp_catalog: list_mcp_servers(workspace).len(),
        uv_ok: wisp_python::PythonEnv::find_uv().is_some(),
        node_ok: wisp_python::PythonEnv::find_node().is_some(),
        npm_ok: wisp_python::PythonEnv::find_npm().is_some(),
        sci_ok: wisp_python::PythonEnv::find_sci().is_some(),
        pixi_ok: wisp_python::PythonEnv::find_pixi().is_some(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        workspace: workspace.to_string_lossy().into_owned(),
        errors: vec![],
    };
    if status.skills_loaded == 0 {
        status
            .errors
            .push("No bundled skills found in install resources.".into());
    }
    if !status.uv_ok {
        status
            .errors
            .push("uv not found on PATH; install uv or set UV_PATH.".into());
    }
    if !status.node_ok {
        status
            .errors
            .push("Node.js not found on PATH; bear-* literature skills need Node >= 20.".into());
    } else if !status.npm_ok {
        status.errors.push(
            "npm not found on PATH; install Node.js (includes npm) for scimaster-cli.".into(),
        );
    } else if !status.sci_ok {
        status.errors.push(
            "scimaster-cli (`sci`) not found; run `npm install -g scimaster-cli` then `sci init`."
                .into(),
        );
    }
    if !status.pixi_ok {
        status.errors.push(
            "pixi not found on PATH; optional for local bioinformatics multi-env workflows.".into(),
        );
    }
    match wisp_python::PythonEnv::ensure(app_data) {
        Ok(_) => status.python_ok = true,
        Err(e) => status.errors.push(format!("Python environment: {e}")),
    }
    if wisp_paths::bio_tools_dir().is_none() {
        status
            .errors
            .push("Bundled bio-tools MCP catalog not found.".into());
    }
    status
}

#[tauri::command]
fn get_bootstrap_status(state: State<'_, AppState>) -> BootstrapStatus {
    state.bootstrap.lock().unwrap().clone()
}

#[tauri::command]
fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_for_updates() -> Result<String, String> {
    Ok("In-app auto-update is disabled until release signing is configured. Download new builds from GitHub Releases.".into())
}

#[tauri::command]
async fn dismiss_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    state
        .store
        .set_setting("onboarding_done", "1")
        .await
        .map_err(|e| format!("{e}"))
}

fn sanitize_upload_name(name: &str) -> Result<String, String> {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "invalid filename".to_string())?;
    if base.is_empty() || base == "." || base == ".." || base.contains('\0') {
        return Err("invalid filename".into());
    }
    Ok(base.to_string())
}

fn unique_upload_path(root: &std::path::Path, dir: &str, name: &str) -> std::path::PathBuf {
    let mut path = root.join(dir).join(name);
    if !path.exists() {
        return path;
    }
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str());
    for i in 1..1000 {
        let candidate = match ext {
            Some(e) => format!("{stem}_{i}.{e}"),
            None => format!("{stem}_{i}"),
        };
        path = root.join(dir).join(&candidate);
        if !path.exists() {
            return path;
        }
    }
    root.join(dir).join(name)
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct PipPkg {
    name: String,
    #[serde(default)]
    version: String,
}

#[derive(serde::Serialize)]
struct ProvInput {
    path: String,
    produced_here: bool,
}

#[derive(serde::Serialize)]
struct ProvEnv {
    name: Option<String>,
    packages: Vec<PipPkg>,
}

#[derive(serde::Serialize)]
struct ArtifactProvenance {
    code: String,
    language: String,
    output: String,
    exit_status: String,
    inputs: Vec<ProvInput>,
    env: Option<ProvEnv>,
}

#[derive(serde::Serialize)]
struct ExportToolResult {
    tool_call_id: String,
    tool_name: String,
    content: String,
}

#[derive(serde::Serialize)]
struct ExportToolCall {
    id: String,
    name: String,
    arguments: serde_json::Value,
    arguments_raw: String,
    result: Option<ExportToolResult>,
}

#[derive(serde::Serialize)]
struct ExportArtifactManifest {
    source_path: String,
    workspace_path: String,
    zip_path: String,
    mime: String,
    bytes: usize,
    provenance_path: Option<String>,
}

struct ExportArtifactFile {
    source_path: String,
    workspace_path: String,
    zip_path: String,
    mime: String,
    bytes: Vec<u8>,
}

#[derive(serde::Serialize)]
struct MissingExportArtifact {
    path: String,
    error: String,
}

#[derive(serde::Serialize)]
struct ExportManifest {
    session_id: String,
    exported_at: String,
    message_count: usize,
    tool_call_count: usize,
    artifacts: Vec<ExportArtifactManifest>,
    missing_artifacts: Vec<MissingExportArtifact>,
}

/// Normalize a UI path (absolute or relative) to the workspace-relative form used
/// in `execution_log.files_written`.
fn to_workspace_rel(root: &std::path::Path, path: &str) -> String {
    let p = std::path::Path::new(path);
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

fn zip_component(raw: &str) -> String {
    let s = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let s = s.trim_matches(['.', '_', '-']);
    if s.is_empty() {
        "file".into()
    } else {
        s.to_string()
    }
}

fn markdown_fence(lang: &str, body: &str) -> String {
    format!("```{lang}\n{body}\n```\n")
}

fn render_export_transcript(messages: &[Message]) -> String {
    let mut out = String::from("# wisp-science session export\n\n");
    for (idx, msg) in messages.iter().enumerate() {
        match msg.role {
            wisp_llm::Role::System => {}
            wisp_llm::Role::User => {
                out.push_str(&format!(
                    "## User {}\n\n{}\n\n",
                    idx + 1,
                    msg.content.as_text()
                ));
            }
            wisp_llm::Role::Assistant => {
                if let Some(reasoning) = msg.reasoning.as_deref().filter(|s| !s.trim().is_empty()) {
                    out.push_str("### Reasoning\n\n");
                    out.push_str(&markdown_fence("text", reasoning));
                    out.push('\n');
                }
                let text = msg.content.as_text();
                if !text.trim().is_empty() {
                    let model = msg
                        .model_name
                        .as_deref()
                        .map(|m| format!(" ({m})"))
                        .unwrap_or_default();
                    out.push_str(&format!("## Assistant{model}\n\n{text}\n\n"));
                }
                if !msg.tool_calls.is_empty() {
                    out.push_str("### Tool calls\n\n");
                    for tc in &msg.tool_calls {
                        out.push_str(&format!("- `{}` `{}`\n", tc.function.name, tc.id));
                        out.push_str(&markdown_fence("json", &tc.function.arguments));
                    }
                    out.push('\n');
                }
            }
            wisp_llm::Role::Tool => {
                let name = msg.tool_name.as_deref().unwrap_or("tool");
                out.push_str(&format!("## Tool result: {name}\n\n"));
                out.push_str(&markdown_fence("text", &msg.content.as_text()));
                out.push('\n');
            }
        }
    }
    out
}

fn export_tool_calls(messages: &[Message]) -> Vec<ExportToolCall> {
    let mut results = HashMap::<String, ExportToolResult>::new();
    for msg in messages {
        if msg.role != wisp_llm::Role::Tool {
            continue;
        }
        let Some(id) = msg.tool_call_id.clone() else {
            continue;
        };
        results.insert(
            id.clone(),
            ExportToolResult {
                tool_call_id: id,
                tool_name: msg.tool_name.clone().unwrap_or_else(|| "tool".into()),
                content: msg.content.as_text(),
            },
        );
    }

    let mut calls = vec![];
    for msg in messages {
        if msg.role != wisp_llm::Role::Assistant {
            continue;
        }
        for tc in &msg.tool_calls {
            let raw = tc.function.arguments.clone();
            let arguments = if raw.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&raw)
                    .unwrap_or_else(|_| serde_json::Value::String(raw.clone()))
            };
            calls.push(ExportToolCall {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments,
                arguments_raw: raw,
                result: results.remove(&tc.id),
            });
        }
    }
    calls
}

fn collect_export_artifacts(
    root: &std::path::Path,
    artifact_paths: Vec<String>,
    stored_artifacts: Vec<(String, String, String, String, i64)>,
) -> (Vec<ExportArtifactFile>, Vec<MissingExportArtifact>) {
    let mut candidates = artifact_paths;
    candidates.extend(stored_artifacts.into_iter().map(|(_, _, _, path, _)| path));

    let mut seen = HashSet::<String>::new();
    let mut files = vec![];
    let mut missing = vec![];
    for source_path in candidates {
        let real = match wisp_tools::safety::validate_file_path(root, &source_path) {
            Ok(real) => real,
            Err(error) => {
                missing.push(MissingExportArtifact {
                    path: source_path,
                    error,
                });
                continue;
            }
        };
        let workspace_path = to_workspace_rel(root, &real.to_string_lossy());
        if !seen.insert(workspace_path.clone()) {
            continue;
        }
        let bytes = match std::fs::read(&real) {
            Ok(bytes) => bytes,
            Err(e) => {
                missing.push(MissingExportArtifact {
                    path: source_path,
                    error: format!("{e}"),
                });
                continue;
            }
        };
        let name = real
            .file_name()
            .and_then(|n| n.to_str())
            .map(zip_component)
            .unwrap_or_else(|| "artifact".into());
        let zip_path = format!("artifacts/{:03}-{name}", files.len() + 1);
        files.push(ExportArtifactFile {
            source_path,
            workspace_path,
            zip_path,
            mime: mime_for_path(&real).into(),
            bytes,
        });
    }
    (files, missing)
}

fn zip_text<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    body: &str,
) -> Result<(), String> {
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    zip.start_file(path, opts).map_err(|e| format!("{e}"))?;
    zip.write_all(body.as_bytes()).map_err(|e| format!("{e}"))
}

fn zip_bytes<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    zip.start_file(path, opts).map_err(|e| format!("{e}"))?;
    zip.write_all(bytes).map_err(|e| format!("{e}"))
}

fn zip_json<W: Write + std::io::Seek, T: serde::Serialize>(
    zip: &mut zip::ZipWriter<W>,
    path: &str,
    value: &T,
) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value).map_err(|e| format!("{e}"))?;
    zip_text(zip, path, &body)
}

/// Parse `uv pip list --format=json` / `pip list --format=json` output.
fn parse_pip_list(json: &str) -> Vec<PipPkg> {
    serde_json::from_str::<Vec<PipPkg>>(json).unwrap_or_default()
}

/// Capture the kernel venv's package list once; store it hashed; return the hash.
/// Non-fatal: any failure returns `None` and the Environment panel shows "unavailable".
async fn capture_env(store: &wisp_store::Store, app_data: &std::path::Path) -> Option<String> {
    let venv = app_data.join("python").join(".venv");
    let python = wisp_python::PythonEnv { venv }.python();
    let uv = wisp_python::PythonEnv::find_uv()?;
    let out = tokio::process::Command::new(&uv)
        .args(["pip", "list", "--format=json", "--python"])
        .arg(&python)
        .output()
        .await
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    let json = String::from_utf8_lossy(&out.stdout).into_owned();
    let packages = parse_pip_list(&json);
    if packages.is_empty() {
        return None;
    }
    let packages_json = serde_json::to_string(&packages).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&packages_json, &mut h);
    let hash = format!("{:016x}", std::hash::Hasher::finish(&h));
    store.record_env_snapshot(&hash, Some("kernel"), &packages_json).await.ok()?;
    Some(hash)
}

async fn register_artifact_at(
    state: &AppState,
    label: &str,
    ap: &ActiveProject,
    path: String,
    content_type: Option<String>,
) -> Result<ArtifactInfo, String> {
    let real = wisp_tools::safety::validate_file_path(&ap.root, &path)?;
    let frame_id = ensure_active_frame(state, label, ap).await?;
    let id = Uuid::new_v4().to_string();
    let filename = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let mime = content_type.unwrap_or_else(|| mime_for_path(&real).to_string());
    let storage = real.to_string_lossy().into_owned();
    state
        .store
        .save_artifact(&id, &ap.id, &frame_id, &filename, &mime, &storage)
        .await
        .map_err(|e| format!("{e}"))?;
    let ts = chrono::Utc::now().timestamp();
    Ok(ArtifactInfo {
        id,
        name: filename,
        kind: mime,
        path: storage,
        ts,
    })
}

#[tauri::command]
async fn upload_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    filename: String,
    data_base64: String,
) -> Result<ArtifactInfo, String> {
    use base64::Engine;
    let name = sanitize_upload_name(&filename)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|e| format!("invalid base64: {e}"))?;
    let cap = 32 * 1024 * 1024;
    if bytes.len() > cap {
        return Err(format!("file exceeds {cap} byte limit"));
    }
    let ap = state.active(window.label());
    let upload_dir = ap.root.join("uploads");
    std::fs::create_dir_all(&upload_dir).map_err(|e| format!("{e}"))?;
    let dest = unique_upload_path(&ap.root, "uploads", &name);
    std::fs::write(&dest, &bytes).map_err(|e| format!("{e}"))?;
    let rel = dest
        .strip_prefix(&ap.root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| dest.to_string_lossy().into_owned());
    register_artifact_at(&state, window.label(), &ap, rel, None).await
}

#[tauri::command]
async fn register_artifact(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    path: String,
    content_type: Option<String>,
) -> Result<ArtifactInfo, String> {
    let ap = state.active(window.label());
    register_artifact_at(&state, window.label(), &ap, path, content_type).await
}

/// Provenance for a produced artifact, addressed by workspace path. `None` when the
/// path has no recorded producing cell (uploads, pre-feature figures) → empty modal.
#[tauri::command]
async fn get_artifact_provenance(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
    path: String,
) -> Result<Option<ArtifactProvenance>, String> {
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => Some(id.to_string()),
        None => state.active_frame(window.label()),
    };
    let Some(fid) = frame_id else { return Ok(None) };
    let ap = state.active(window.label());
    artifact_provenance_for_path(&state.store, &fid, &ap.root, &path).await
}

async fn artifact_provenance_for_path(
    store: &Store,
    frame_id: &str,
    root: &std::path::Path,
    path: &str,
) -> Result<Option<ArtifactProvenance>, String> {
    let rel = to_workspace_rel(root, path);
    let Some(e) = store
        .find_provenance_by_path(frame_id, &rel)
        .await
        .map_err(|e| format!("{e}"))?
    else {
        return Ok(None);
    };
    let written = store
        .frame_written_paths(frame_id)
        .await
        .unwrap_or_default();
    let inputs = e
        .files_read
        .iter()
        .map(|p| ProvInput {
            path: p.clone(),
            produced_here: written.contains(p),
        })
        .collect();
    let env = match e.env_hash.as_deref() {
        Some(h) => store
            .get_env_snapshot(h)
            .await
            .ok()
            .flatten()
            .map(|(name, pj)| ProvEnv {
                name,
                packages: parse_pip_list(&pj),
            }),
        None => None,
    };
    Ok(Some(ArtifactProvenance {
        code: e.source,
        language: e.language,
        output: e.stdout,
        exit_status: e.exit_status,
        inputs,
        env,
    }))
}

#[tauri::command]
async fn export_session(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
    artifact_paths: Vec<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let messages = state
        .store
        .load_messages(&session_id)
        .await
        .map_err(|e| format!("{e}"))?;
    if messages.is_empty() {
        return Err("No messages to export.".into());
    }

    let ap = state.active(window.label());
    let stored_artifacts = state
        .store
        .list_artifacts(&session_id)
        .await
        .unwrap_or_default();
    let (files, missing_artifacts) =
        collect_export_artifacts(&ap.root, artifact_paths, stored_artifacts);
    let tool_calls = export_tool_calls(&messages);

    let mut artifact_manifest = Vec::<ExportArtifactManifest>::new();
    let mut provenance_files = Vec::<(String, ArtifactProvenance)>::new();
    for file in &files {
        let stem = std::path::Path::new(&file.zip_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(zip_component)
            .unwrap_or_else(|| "artifact".into());
        let provenance_path = match artifact_provenance_for_path(
            &state.store,
            &session_id,
            &ap.root,
            &file.workspace_path,
        )
        .await?
        {
            Some(prov) => {
                let path = format!("provenance/{stem}.json");
                provenance_files.push((path.clone(), prov));
                Some(path)
            }
            None => None,
        };
        artifact_manifest.push(ExportArtifactManifest {
            source_path: file.source_path.clone(),
            workspace_path: file.workspace_path.clone(),
            zip_path: file.zip_path.clone(),
            mime: file.mime.clone(),
            bytes: file.bytes.len(),
            provenance_path,
        });
    }

    let manifest = ExportManifest {
        session_id: session_id.clone(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        message_count: messages.len(),
        tool_call_count: tool_calls.len(),
        artifacts: artifact_manifest,
        missing_artifacts,
    };

    let default_name = format!("wisp-session-{}.zip", zip_component(&session_id));
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
    let out = std::fs::File::create(&dest_path).map_err(|e| format!("{e}"))?;
    let mut zip = zip::ZipWriter::new(out);

    zip_json(&mut zip, "manifest.json", &manifest)?;
    zip_text(
        &mut zip,
        "transcript.md",
        &render_export_transcript(&messages),
    )?;
    zip_json(&mut zip, "messages.json", &messages)?;
    zip_json(&mut zip, "tool-calls.json", &tool_calls)?;
    for file in &files {
        zip_bytes(&mut zip, &file.zip_path, &file.bytes)?;
    }
    for (path, provenance) in &provenance_files {
        zip_json(&mut zip, path, provenance)?;
    }
    zip.finish().map_err(|e| format!("{e}"))?;

    Ok(Some(dest_path.to_string_lossy().into_owned()))
}

/// Tell the webview whether we're in dev (keep native context menu / DevTools).
fn set_dev_flag(app: &tauri::AppHandle) {
    let dev = cfg!(debug_assertions);
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.eval(&format!("window.__WISP_DEV__ = {};", dev));
}

/// A macOS/Linux `.app` launched from Finder/Dock/Launchpad inherits a bare
/// `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), not the login-shell `PATH`. So
/// Homebrew tools (`/opt/homebrew/bin` on Apple Silicon), `~/.local/bin`,
/// `~/.cargo/bin`, nvm, etc. are invisible to `which::which` (capability
/// detection) *and* to the `sh -c` / uv / node / pixi child spawns — the
/// tools are installed and work in a terminal, but the app reports them
/// missing. Resolve the user's real login-shell `PATH` once, up front, and set
/// it on the process so every downstream consumer sees the same `PATH` the
/// terminal does. Runs before any threads spawn children (env mutation is safe
/// here). No-op on Windows.
#[cfg(not(target_os = "windows"))]
fn inherit_login_shell_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    // Markers survive noisy rc files that print to stdout (p10k instant prompt,
    // MOTD, a stray `echo` in .zshrc). `-ilc` sources both login (.zprofile,
    // where `brew shellenv` usually lives) and interactive (.zshrc) profiles.
    // ponytail: assumes a colon-PATH shell (zsh/bash/sh); fish joins list vars
    // with spaces and would parse wrong — fish users set UV_PATH/PIXI_PATH or
    // launch from a terminal. Widen to fish only if someone reports it.
    let script = r#"printf '__WISP_PATH__%s__WISP_END__' "$PATH""#;
    let Ok(out) = std::process::Command::new(&shell)
        .args(["-ilc", script])
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Some(path) = stdout
        .split_once("__WISP_PATH__")
        .and_then(|(_, rest)| rest.split_once("__WISP_END__"))
        .map(|(p, _)| p.trim())
        .filter(|p| !p.is_empty())
    {
        std::env::set_var("PATH", path);
    }
}

#[cfg(target_os = "windows")]
fn inherit_login_shell_path() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    inherit_login_shell_path();
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("wisp=info".parse().unwrap());
    let subscriber = tracing_subscriber::fmt().with_env_filter(filter);
    #[cfg(all(not(debug_assertions), target_os = "windows"))]
    subscriber.with_writer(std::io::sink).init();
    #[cfg(not(all(not(debug_assertions), target_os = "windows")))]
    subscriber.init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Ok(res) = app.path().resource_dir() {
                wisp_paths::set_resource_root(res);
            }
            let app_data = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from(".wisp"))
                .join("wisp-science");
            std::fs::create_dir_all(&app_data).expect("create app data dir");
            let db_path = app_data.join("wisp.sqlite");
            let store = tauri::async_runtime::block_on(Store::open(&db_path)).expect("open store");

            let (active_id, ws) = tauri::async_runtime::block_on(async {
                // Legacy single-workspace installs stored one global `workspace_dir`
                // setting. Backfill the `default` project's dir from it (or the
                // platform default) so its existing sessions stay reachable. Env
                // override is applied to the *root* below, not persisted here.
                let default_workspace = app
                    .path()
                    .document_dir()
                    .map(|d| d.join("wisp-science"))
                    .unwrap_or_else(|_| app_data.join("workspace"));
                let legacy_ws = store
                    .get_setting("workspace_dir")
                    .await
                    .ok()
                    .flatten()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| default_workspace.to_string_lossy().into_owned());
                store
                    .create_project("default", "Workspace", &legacy_ws)
                    .await
                    .ok();
                let active_id = match store.get_setting("active_project_id").await.ok().flatten() {
                    Some(id) if store.get_project(&id).await.ok().flatten().is_some() => id,
                    _ => "default".to_string(),
                };
                let (_, dir) = store
                    .get_project(&active_id)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| ("Workspace".into(), legacy_ws.clone()));
                (active_id, dir)
            });

            // Env override wins for the active root only (dev escape hatch; not persisted).
            let default_workspace = app
                .path()
                .document_dir()
                .map(|d| d.join("wisp-science"))
                .unwrap_or_else(|_| app_data.join("workspace"));
            let root = resolve_workspace(
                std::env::var("WISP_WORKSPACE").ok(),
                Some(ws),
                default_workspace,
            );
            let root = ensure_writable(root, &app_data);

            let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
            let memory = Arc::new(MemoryManager::new(&root));
            let bootstrap = StdMutex::new(initial_bootstrap(&app_data, &root, skills.all().len()));
            let approvals = Arc::new(StdRwLock::new(tauri::async_runtime::block_on(
                build_approval_policy(&store),
            )));
            let state = AppState {
                app_data,
                store,
                active: std::sync::RwLock::new(HashMap::from([(
                    "main".to_string(),
                    ActiveProject {
                        id: active_id,
                        root,
                        skills,
                        memory,
                    },
                )])),
                sessions: tokio::sync::Mutex::new(HashMap::new()),
                running_turns: tokio::sync::Mutex::new(HashSet::new()),
                active_frame: std::sync::RwLock::new(HashMap::new()),
                confirms: Arc::new(StdMutex::new(HashMap::new())),
                awaiting_confirm: Arc::new(StdMutex::new(HashSet::new())),
                approvals,
                bootstrap,
                reviewing: Arc::new(AtomicBool::new(false)),
            };
            app.manage(state);
            set_dev_flag(app.handle());
            // Restore the project windows open when the app last quit (#52). The
            // "main" window comes from tauri.conf; these are the extra per-project
            // ones. A project that was since deleted simply fails to spawn.
            {
                let handle = app.handle().clone();
                let ids = tauri::async_runtime::block_on(async {
                    persisted_windows(&handle.state::<AppState>().store).await
                });
                for id in ids {
                    let handle = app.handle().clone();
                    tauri::async_runtime::block_on(async {
                        let st = handle.state::<AppState>();
                        let _ = spawn_project_window(&handle, st.inner(), &id).await;
                    });
                }
            }
            // dev runs the bare debug binary, which doesn't grab focus on macOS —
            // pull the window to the front so it doesn't hide behind the terminal.
            // release launches from the .app bundle and activates normally.
            #[cfg(debug_assertions)]
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_message,
            stop_agent,
            review_session,
            ssh_hosts::list_ssh_hosts,
            ssh_hosts::add_ssh_host,
            ssh_hosts::remove_ssh_host,
            ssh_hosts::list_ssh_config_aliases,
            ssh_hosts::import_ssh_config_hosts,
            new_session,
            list_sessions,
            delete_session,
            rename_session,
            list_folders,
            create_folder,
            rename_folder,
            delete_folder,
            move_session,
            list_recent_sessions,
            list_projects,
            pick_directory,
            download_file,
            export_session,
            create_project,
            open_project,
            open_project_window,
            delete_project,
            get_project_settings,
            update_project,
            load_session,
            rewind_session,
            list_skills,
            set_skill_tags,
            set_skills_enabled,
            set_skill_enabled,
            pick_skill_source,
            install_skill,
            remove_skill,
            list_demos,
            load_demo,
            confirm_response,
            get_settings,
            set_settings,
            set_api_key,
            credential_status,
            set_credential,
            models::list_models,
            models::save_model,
            models::remove_model,
            models::set_active_model,
            validate_settings,
            list_dir,
            read_file,
            list_artifacts,
            read_artifact,
            missing_files,
            upload_file,
            register_artifact,
            get_artifact_provenance,
            get_project_info,
            get_capabilities,
            list_memory,
            get_memory_view,
            set_memory_enabled,
            read_memory_file,
            write_memory_file,
            delete_memory_file,
            clear_memory,
            get_onboarding_state,
            dismiss_onboarding,
            get_bootstrap_status,
            check_for_updates,
            open_external_url,
            list_mcp_connections,
            add_mcp_connection,
            update_mcp_connection,
            delete_mcp_connection,
            set_mcp_connection_enabled,
            test_mcp_connection,
            list_connectors,
            set_connector_enabled,
            set_tool_approval,
            set_approval_scope,
            set_connector_skip_approvals,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Wisp");
}

#[cfg(test)]
mod tests {
    use super::{
        copy_dir_recursive, parse_disabled_skills, parse_enabled_skill_names, parse_skill_tags,
        resolve_workspace, session_runtime_status, McpConnection, McpTransport,
    };
    use std::collections::HashSet;
    use std::path::PathBuf;

    #[test]
    fn session_runtime_status_labels() {
        let mut running = HashSet::new();
        running.insert("s1".into());
        let awaiting = HashSet::new();
        assert_eq!(
            session_runtime_status("s1", Some("user"), &running, &awaiting),
            "running"
        );
        assert_eq!(
            session_runtime_status("s2", Some("assistant"), &running, &awaiting),
            "needs_you"
        );
        assert_eq!(
            session_runtime_status("s3", Some("user"), &running, &awaiting),
            "complete"
        );
        let mut awaiting = HashSet::new();
        awaiting.insert("s1".into());
        assert_eq!(
            session_runtime_status("s1", Some("user"), &running, &awaiting),
            "needs_you"
        );
    }

    #[test]
    fn scope_gates_per_tool_modes() {
        use super::{ApprovalMode, ApprovalPolicy, Scope};
        use std::collections::HashMap;
        use wisp_tools::Approval;

        let policy = |scope: Scope| {
            let mut tools = HashMap::new();
            tools.insert("asker".to_string(), ApprovalMode::Ask);
            tools.insert("blocked".to_string(), ApprovalMode::Deny);
            ApprovalPolicy {
                scope,
                tools,
                ..Default::default()
            }
        };

        // Ask (current behaviour): per-tool modes pass through unchanged.
        let ask = policy(Scope::Ask);
        assert_eq!(ask.mode_for("asker"), Approval::Ask);
        assert_eq!(ask.mode_for("blocked"), Approval::Deny);
        assert_eq!(ask.mode_for("unset"), Approval::Allow);
        assert!(!ask.full());

        // Auto: per-tool Ask is silenced to Allow, but an explicit Deny still
        // blocks and dangerous commands are NOT auto-approved.
        let auto = policy(Scope::Auto);
        assert_eq!(auto.mode_for("asker"), Approval::Allow);
        assert_eq!(auto.mode_for("blocked"), Approval::Deny);
        assert!(!auto.full());

        // Full: everything Allow except an explicit Deny; dangerous commands
        // auto-approve (full() == true).
        let full = policy(Scope::Full);
        assert_eq!(full.mode_for("asker"), Approval::Allow);
        assert_eq!(full.mode_for("blocked"), Approval::Deny);
        assert!(full.full());
    }

    #[test]
    fn copy_dir_recursive_copies_nested_files() {
        let base = std::env::temp_dir().join(format!(
            "wisp_copy_dir_test_{}_{}",
            std::process::id(),
            line!()
        ));
        let from = base.join("from");
        let to = base.join("to");
        std::fs::create_dir_all(from.join("scripts")).unwrap();
        std::fs::write(from.join("SKILL.md"), "---\nname: x\n---\nbody").unwrap();
        std::fs::write(from.join("scripts").join("run.py"), "print(1)").unwrap();

        copy_dir_recursive(&from, &to).unwrap();

        assert!(to.join("SKILL.md").is_file());
        assert!(to.join("scripts").join("run.py").is_file());
        assert_eq!(
            std::fs::read_to_string(to.join("SKILL.md")).unwrap(),
            "---\nname: x\n---\nbody"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn validate_skill_name_rejects_traversal() {
        use super::validate_skill_name;
        for bad in [
            "",
            "  ",
            "..",
            "../../etc",
            "/etc/passwd",
            "a/b",
            "..\\x",
            "foo/../bar",
        ] {
            assert!(validate_skill_name(bad).is_err(), "should reject {bad:?}");
        }
        for ok in ["alphafold2", "my-skill", "Skill_1"] {
            assert!(validate_skill_name(ok).is_ok(), "should accept {ok:?}");
        }
    }

    #[test]
    fn parse_disabled_skills_handles_missing_and_valid() {
        assert!(parse_disabled_skills(None).is_empty());
        assert!(parse_disabled_skills(Some("not json")).is_empty());
        let s = parse_disabled_skills(Some(r#"["alphafold2","boltz"]"#));
        assert!(s.contains("alphafold2") && s.contains("boltz") && s.len() == 2);
    }

    #[test]
    fn resolve_workspace_prefers_env_then_setting_then_default() {
        let default = PathBuf::from("/nonexistent/wisp/default");
        // Blank/whitespace candidates are skipped → default wins (never created).
        assert_eq!(
            resolve_workspace(Some("   ".into()), Some(String::new()), default.clone()),
            default
        );
        assert!(!default.exists());

        let base = std::env::temp_dir().join(format!("wisp_ws_test_{}", std::process::id()));
        let env_dir = base.join("env");
        let set_dir = base.join("set");
        // A creatable env path wins over the setting, and gets created.
        assert_eq!(
            resolve_workspace(
                Some(env_dir.to_string_lossy().into_owned()),
                Some(set_dir.to_string_lossy().into_owned()),
                default.clone(),
            ),
            env_dir
        );
        assert!(env_dir.exists());
        // Falls through to the setting when env is absent.
        assert_eq!(
            resolve_workspace(None, Some(set_dir.to_string_lossy().into_owned()), default),
            set_dir
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn parse_skill_tags_normalizes_global_tag_json() {
        let tags = parse_skill_tags(Some(
            serde_json::json!({
                "alpha": [" compute ", "protein", "compute", ""],
                "beta": [],
                "gamma": "bad"
            })
            .to_string(),
        ));

        assert_eq!(
            tags.get("alpha").unwrap(),
            &vec!["compute".to_string(), "protein".to_string()]
        );
        assert!(!tags.contains_key("beta"));
        assert!(!tags.contains_key("gamma"));
    }

    #[test]
    fn parse_enabled_skill_names_uses_none_as_all_enabled() {
        assert!(parse_enabled_skill_names(None).is_none());

        let enabled =
            parse_enabled_skill_names(Some(r#"["alpha", " beta ", "", "alpha"]"#.into())).unwrap();
        assert!(enabled.contains("alpha"));
        assert!(enabled.contains("beta"));
        assert_eq!(enabled.len(), 2);

        assert!(parse_enabled_skill_names(Some("not json".into()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn mcp_connection_serde_roundtrip() {
        let stdio = McpConnection {
            id: "1".into(),
            name: "local".into(),
            enabled: true,
            transport: McpTransport::Stdio {
                command: "python".into(),
                args: vec!["s.py".into()],
                env: vec![("K".into(), "V".into())],
                cwd: None,
            },
        };
        let http = McpConnection {
            id: "2".into(),
            name: "remote".into(),
            enabled: false,
            transport: McpTransport::Http {
                url: "https://x/mcp".into(),
                headers: vec![("Authorization".into(), "Bearer t".into())],
            },
        };
        for c in [stdio, http] {
            let json = serde_json::to_string(&c).unwrap();
            let back: McpConnection = serde_json::from_str(&json).unwrap();
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
        // tag shape
        let j = serde_json::to_value(&McpConnection {
            id: "3".into(),
            name: "n".into(),
            enabled: true,
            transport: McpTransport::Http {
                url: "u".into(),
                headers: vec![],
            },
        })
        .unwrap();
        assert_eq!(j["transport"]["kind"], "http");
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    #[test]
    fn export_tool_calls_matches_results_by_id() {
        let mut assistant = Message::assistant("");
        assistant.tool_calls = vec![wisp_llm::ToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: wisp_llm::FunctionCall {
                name: "python".into(),
                arguments: r#"{"code":"print(1)"}"#.into(),
            },
        }];
        let tool = Message::tool("call_1", "python", "ok");

        let calls = export_tool_calls(&[assistant, tool]);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "python");
        assert_eq!(calls[0].arguments["code"], "print(1)");
        assert_eq!(calls[0].result.as_ref().unwrap().content, "ok");
    }

    #[test]
    fn parse_pip_list_reads_name_version() {
        let json = r#"[{"name":"numpy","version":"1.26.0"},{"name":"pandas","version":"2.2.0"}]"#;
        let pkgs = parse_pip_list(json);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "numpy");
        assert_eq!(pkgs[1].version, "2.2.0");
        assert!(parse_pip_list("not json").is_empty());
    }

    #[test]
    fn to_workspace_rel_normalizes_absolute_and_passes_relative() {
        use std::path::Path;
        let root = Path::new("/proj");
        // absolute path under root → stripped to workspace-relative
        assert_eq!(to_workspace_rel(root, "/proj/out/fig.png"), "out/fig.png");
        // already-relative path → passed through unchanged
        assert_eq!(to_workspace_rel(root, "out/fig.png"), "out/fig.png");
        // path not under root → left as-is (strip_prefix fails, falls through)
        assert_eq!(to_workspace_rel(root, "/other/x.png"), "/other/x.png");
    }
}
