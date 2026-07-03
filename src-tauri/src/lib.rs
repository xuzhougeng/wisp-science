//! Tauri v2 desktop shell: commands that drive the Wisp agent and stream
//! events to the webview, plus a settings/confirm surface.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;
use wisp_core::{Agent, MemoryManager, Output};
use wisp_llm::{Message, ProviderConfig};
use wisp_skills::SkillIndex;
use wisp_store::Store;

mod seed;

/// One streamed agent event, tagged for the frontend to match on.
#[derive(Serialize, Clone)]
#[serde(tag = "kind")]
enum AgentEvent {
    Text { frame_id: String, delta: String },
    Reasoning { frame_id: String, delta: String },
    ToolCall { frame_id: String, name: String, preview: String },
    ToolResult { frame_id: String, name: String, ok: bool, content: String },
    Usage { frame_id: String, round: u64, input: u64, output: u64, ctx_tokens: usize, max_context: usize },
    Compaction { frame_id: String, before: usize, after: usize, strategy: String },
    Diff { frame_id: String, path: String },
    Stdout { frame_id: String, chunk: String },
    Done { frame_id: String },
    Error { frame_id: String, message: String },
}

#[derive(Serialize, Clone)]
struct ConfirmRequest {
    frame_id: String,
    message: String,
}

#[derive(Serialize, Clone)]
struct SkillInfo {
    name: String,
    description: String,
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
}

#[derive(Serialize, Clone)]
struct ProjectSummary {
    id: String,
    name: String,
    workspace_dir: String,
    session_count: i64,
    updated_at: i64,
}

async fn build_project_summary(state: &AppState, id: &str) -> ProjectSummary {
    // Project counts are tiny; filtering the full list is fine.
    state.store.list_projects().await.ok()
        .and_then(|v| v.into_iter().find(|r| r.0 == id))
        .map(|(id, name, ws, _c, upd, cnt)| ProjectSummary { id, name, workspace_dir: ws, session_count: cnt, updated_at: upd })
        .unwrap_or(ProjectSummary { id: id.into(), name: String::new(), workspace_dir: String::new(), session_count: 0, updated_at: 0 })
}

/// A reloaded transcript row for the UI to render (role in
/// user|assistant|reasoning|tool).
#[derive(Serialize, Clone)]
struct UiItem {
    role: String,
    text: String,
    tool_name: Option<String>,
    ok: Option<bool>,
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
                    out.push(UiItem { role: "user".into(), text: t, tool_name: None, ok: None });
                }
            }
            wisp_llm::Role::Assistant => {
                if let Some(r) = &m.reasoning {
                    if !r.trim().is_empty() {
                        out.push(UiItem { role: "reasoning".into(), text: r.clone(), tool_name: None, ok: None });
                    }
                }
                let t = m.content.as_text();
                if !t.trim().is_empty() {
                    out.push(UiItem { role: "assistant".into(), text: t, tool_name: None, ok: None });
                }
            }
            wisp_llm::Role::Tool => {
                let text = m.content.as_text();
                if m.tool_name.as_deref() == Some("attempt_completion") {
                    if !text.trim().is_empty() {
                        out.push(UiItem { role: "assistant".into(), text, tool_name: None, ok: None });
                    }
                } else {
                    out.push(UiItem { role: "tool".into(), text, tool_name: m.tool_name.clone(), ok: Some(true) });
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
    has_api_key: bool,
    #[serde(default = "default_locale")]
    locale: String,
    /// Where the workspace/data root lives. Empty = platform default
    /// (Documents/wisp-science). Applied on next launch (#6, #13).
    #[serde(default)]
    workspace_dir: String,
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
    app_version: String,
    workspace: String,
    errors: Vec<String>,
}

struct SessionState {
    frame_id: Option<String>,
    last_seq: i64,
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
    active: std::sync::RwLock<ActiveProject>,
    agent: tokio::sync::Mutex<Option<Agent>>,
    session: tokio::sync::Mutex<SessionState>,
    confirm: Arc<StdMutex<Option<std::sync::mpsc::Sender<bool>>>>,
    bootstrap: StdMutex<BootstrapStatus>,
    cancel: Arc<AtomicBool>,
}

impl AppState {
    /// Snapshot the active project. Cheap: two `Arc` clones + a `String`/`PathBuf`.
    /// Take the guard, clone, drop — never held across `.await`.
    fn active(&self) -> ActiveProject {
        self.active.read().unwrap().clone()
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
/// channel satisfied by the `confirm_response` command.
struct TauriOutput {
    app: AppHandle,
    frame_id: String,
    confirm: Arc<StdMutex<Option<std::sync::mpsc::Sender<bool>>>>,
    /// Incremental-persistence sink: each message the turn produces is sent here
    /// and written to SQLite by a background task, so a crash or mid-turn "new
    /// session" no longer discards the whole turn. `None` disables it.
    persist: Option<tokio::sync::mpsc::UnboundedSender<Message>>,
}

impl TauriOutput {
    fn emit(&self, event: AgentEvent) {
        let _ = self.app.emit("agent", event);
    }
}

impl Output for TauriOutput {
    fn assistant_text(&self, delta: &str) {
        self.emit(AgentEvent::Text { frame_id: self.frame_id.clone(), delta: delta.into() });
    }
    fn reasoning(&self, delta: &str) {
        self.emit(AgentEvent::Reasoning { frame_id: self.frame_id.clone(), delta: delta.into() });
    }
    fn tool_call(&self, name: &str, preview: &str) {
        self.emit(AgentEvent::ToolCall { frame_id: self.frame_id.clone(), name: name.into(), preview: preview.into() });
    }
    fn tool_result(&self, name: &str, ok: bool, content: &str) {
        let clipped: String = content.chars().take(4000).collect();
        self.emit(AgentEvent::ToolResult { frame_id: self.frame_id.clone(), name: name.into(), ok, content: clipped });
    }
    fn usage(&self, round: usize, input: u64, output: u64, ctx_tokens: usize, max_context: usize) {
        self.emit(AgentEvent::Usage { frame_id: self.frame_id.clone(), round: round as u64, input, output, ctx_tokens, max_context });
    }
    fn compaction(&self, before: usize, after: usize, strategy: &str) {
        self.emit(AgentEvent::Compaction { frame_id: self.frame_id.clone(), before, after, strategy: strategy.into() });
    }
    fn diff(&self, path: &str, _old: &str, _new: &str) {
        self.emit(AgentEvent::Diff { frame_id: self.frame_id.clone(), path: path.into() });
    }
    fn stdout_chunk(&self, chunk: &str) {
        self.emit(AgentEvent::Stdout { frame_id: self.frame_id.clone(), chunk: chunk.into() });
    }
    fn confirm(&self, message: &str) -> bool {
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        *self.confirm.lock().unwrap() = Some(tx);
        let _ = self.app.emit("confirm-request", ConfirmRequest { frame_id: self.frame_id.clone(), message: message.into() });
        // Block until the UI responds; default to deny on timeout/missing.
        rx.recv_timeout(std::time::Duration::from_secs(180)).unwrap_or(false)
    }
    fn on_message(&self, msg: &Message) {
        if let Some(tx) = &self.persist {
            let _ = tx.send(msg.clone());
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
    value.filter(|v| !v.trim().is_empty()).unwrap_or_else(fallback)
}

/// Pick the workspace root: env override, then the saved setting, then the
/// platform default — the first non-empty candidate we can create wins.
fn resolve_workspace(env: Option<String>, stored: Option<String>, default: PathBuf) -> PathBuf {
    for cand in [env, stored].into_iter().flatten() {
        let cand = cand.trim();
        if cand.is_empty() { continue; }
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

async fn load_settings(store: &Store) -> (String, String, String, String) {
    let provider = normalized_provider(&non_empty_setting(
        store.get_setting("provider").await.ok().flatten(),
        || env_or("WISP_PROVIDER", "openai"),
    ));
    let api_url = non_empty_setting(store.get_setting("api_url").await.ok().flatten(), || env_or("WISP_API_URL", default_api_url(&provider)));
    let model = non_empty_setting(store.get_setting("model").await.ok().flatten(), || env_or("WISP_MODEL", default_model(&provider)));
    let api_key = wisp_store::secrets::Secret::get("api_key").ok().unwrap_or_else(|| env_or("WISP_API_KEY", ""));
    (provider, api_url, model, api_key)
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

fn build_provider_config(provider: &str, api_url: &str, api_key: &str, model: &str) -> Result<ProviderConfig, String> {
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
    Ok(match provider.as_str() {
        "anthropic" => ProviderConfig::anthropic(api_url, api_key, model),
        "openai_responses" => ProviderConfig::openai_responses(api_url, api_key, model),
        "openai" => ProviderConfig::openai(api_url, api_key, model),
        _ => return Err(format!("Unsupported provider: {provider}")),
    })
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
    if let Some(b) = wisp_skills::bundled_dir() { paths.push(b); }
    paths.push(root.join(".wisp").join("skills"));
    if let Some(home) = dirs::home_dir() { paths.push(home.join(".wisp").join("skills")); }
    if let Ok(extra) = std::env::var("WISP_SKILLS_PATH") {
        for p in extra.split([':', ';']).filter(|s| !s.is_empty()) { paths.push(PathBuf::from(p)); }
    }
    paths
}

/// Wire Python REPL and bundled bio-tools MCP into a freshly built agent.
async fn wire_python_and_mcp(agent: &mut wisp_core::Agent, app_data: &std::path::Path) -> Vec<String> {
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
    let worker_path = wisp_python::resolve_bundled_script(&worker);
    if worker_path.is_file() {
        if let Some(env) = &py_env {
            match wisp_python::KernelClient::spawn(&env.python(), &worker_path) {
                Ok(client) => agent.add_tool(Box::new(wisp_python::ReplTool::new(client))),
                Err(e) => errors.push(format!("Python REPL: {e}")),
            }
        }
    } else {
        errors.push(format!("Kernel worker not found at {}", worker_path.display()));
    }

    if let Ok(cmdline) = std::env::var("WISP_MCP_COMMAND") {
        let parts: Vec<String> = cmdline
            .split_whitespace()
            .map(|s| {
                if s.ends_with(".py") {
                    wisp_python::resolve_bundled_script(s).to_string_lossy().to_string()
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
        match wisp_mcp::McpClient::launch_bio_tools(&env.python(), &pkg).await {
            Ok(client) => register_mcp(agent, std::sync::Arc::new(client)).await,
            Err(e) => errors.push(format!("MCP {pkg}: {e}")),
        }
    }
    errors
}

async fn register_mcp(agent: &mut wisp_core::Agent, client: std::sync::Arc<wisp_mcp::McpClient>) {
    match client.tools_list().await {
        Ok(tools) => {
            for t in tools {
                agent.add_tool(Box::new(wisp_mcp::McpTool::new(t, client.clone())));
            }
        }
        Err(e) => tracing::warn!("mcp tools_list failed: {e}"),
    }
}

/// Get the active session frame id, creating a new SQLite frame if none.
async fn ensure_frame(store: &Store, project_id: &str, sess: &mut SessionState) -> anyhow::Result<String> {
    if let Some(id) = sess.frame_id.clone() {
        return Ok(id);
    }
    let id = Uuid::new_v4().to_string();
    store.create_frame(&id, project_id, "OPERON", "wisp").await?;
    sess.frame_id = Some(id.clone());
    sess.last_seq = 0;
    Ok(id)
}

#[tauri::command]
async fn send_message(state: State<'_, AppState>, app: AppHandle, message: String) -> Result<String, String> {
    let turn_id = Uuid::new_v4().to_string();
    let (provider, api_url, model, api_key) = load_settings(&state.store).await;
    let cfg = build_provider_config(&provider, &api_url, &api_key, &model)?;

    let max_context = state.store.get_setting("max_context").await.ok().flatten().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1_000_000);
    let max_iter = state.store.get_setting("max_iter").await.ok().flatten().and_then(|s| s.parse::<usize>().ok()).unwrap_or(100);

    let ap = state.active();
    let mut guard = state.agent.lock().await;
    if guard.is_none() {
        let mut agent = Agent::new(cfg.clone(), ap.skills.clone(), ap.memory.clone(), ap.root.clone(), max_context, max_iter);
        // Load the persisted session from SQLite (replaces the JSON session file).
        let frame_id = {
            let mut sess = state.session.lock().await;
            ensure_frame(&state.store, &ap.id, &mut sess).await.map_err(|e| format!("{e}"))?
        };
        match state.store.load_messages(&frame_id).await {
            Ok(msgs) => agent.ctx.messages = msgs,
            Err(e) => tracing::warn!("load session from sqlite failed: {e}"),
        }
        {
            let mut sess = state.session.lock().await;
            sess.last_seq = agent.ctx.messages.len() as i64;
        }
        if agent.ctx.is_empty() {
            agent.seed_system_prompt(&ap.skills);
        }
        let wire_errors = wire_python_and_mcp(&mut agent, &state.app_data).await;
        if !wire_errors.is_empty() {
            state.bootstrap.lock().unwrap().errors.extend(wire_errors);
        }
        *guard = Some(agent);
    }
    let agent = guard.as_mut().unwrap();
    state.cancel.store(false, Ordering::Relaxed);

    // Incremental persistence: a background task appends each message the turn
    // produces to SQLite as it arrives (via TauriOutput::on_message), so a crash
    // or a mid-turn "new session" no longer loses the whole turn (#14, #15). The
    // task owns the running seq, so it stays correct even if the in-memory
    // context is compacted mid-turn.
    //
    // First flush any messages already in the context but not yet persisted
    // (e.g. a system prompt seeded here or by `new_session`), so the incremental
    // seq lines up with what a later reload expects.
    let (persist_frame, start_seq) = {
        let mut sess = state.session.lock().await;
        let frame = sess.frame_id.clone();
        if let Some(frame_id) = &frame {
            let start = sess.last_seq as usize;
            if start < agent.ctx.messages.len() {
                let mut seq = sess.last_seq;
                for m in &agent.ctx.messages[start..] {
                    seq += 1;
                    let _ = state.store.append_message(frame_id, seq, m).await;
                }
                sess.last_seq = agent.ctx.messages.len() as i64;
            }
        }
        (frame, sess.last_seq)
    };
    let (persist_handle, persist_tx) = match persist_frame {
        Some(frame_id) => {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
            let store = state.store.clone();
            let mut seq = start_seq;
            let handle = tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    seq += 1;
                    if let Err(e) = store.append_message(&frame_id, seq, &msg).await {
                        tracing::warn!("incremental persist seq {seq} failed: {e}");
                    }
                }
                seq
            });
            (Some(handle), Some(tx))
        }
        None => (None, None),
    };

    let output = TauriOutput {
        app: app.clone(),
        frame_id: turn_id.clone(),
        confirm: state.confirm.clone(),
        persist: persist_tx,
    };

    let result = agent.run(&message, &output, Some(state.cancel.as_ref())).await;
    agent.ctx.clear_runtime_injections();

    // Close the persist channel and wait for the task to flush; its final seq is
    // the authoritative persisted count.
    drop(output);
    if let Some(handle) = persist_handle {
        match tokio::time::timeout(std::time::Duration::from_secs(5), handle).await {
            Ok(Ok(final_seq)) => { state.session.lock().await.last_seq = final_seq; }
            other => {
                tracing::warn!("persist task did not finish cleanly: {other:?}");
                let mut sess = state.session.lock().await;
                sess.last_seq = agent.ctx.messages.len() as i64;
            }
        }
    }
    drop(guard);

    match result {
        Ok(_) => {
            let _ = app.emit("agent", AgentEvent::Done { frame_id: turn_id.clone() });
            Ok(turn_id)
        }
        Err(e) => {
            let _ = app.emit("agent", AgentEvent::Error { frame_id: turn_id.clone(), message: format!("{e}") });
            Err(format!("{e}"))
        }
    }
}

#[tauri::command]
fn stop_agent(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
async fn new_session(state: State<'_, AppState>) -> Result<(), String> {
    // If a turn is mid-flight it holds the agent lock for its whole duration.
    // Signal cancellation first so it interrupts (killing any running child)
    // and releases the lock, instead of this command blocking for minutes (#15).
    let ap = state.active();
    state.cancel.store(true, Ordering::Relaxed);
    let mut guard = state.agent.lock().await;
    // Reset even when the agent hasn't been created yet (lazy init in
    // send_message), otherwise the next message lands in the old frame.
    let mut sess = state.session.lock().await;
    sess.frame_id = None;
    sess.last_seq = 0;
    if let Some(agent) = guard.as_mut() {
        agent.ctx.clear();
        let frame_id = ensure_frame(&state.store, &ap.id, &mut sess).await.map_err(|e| format!("{e}"))?;
        agent.ctx.messages = state.store.load_messages(&frame_id).await.unwrap_or_default();
        sess.last_seq = agent.ctx.messages.len() as i64;
        if agent.ctx.is_empty() {
            agent.seed_system_prompt(&ap.skills);
        }
    }
    Ok(())
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, String> {
    let ap = state.active();
    let rows = state.store.list_sessions(&ap.id).await.map_err(|e| format!("{e}"))?;
    Ok(rows.into_iter().map(|(id, title, ts)| SessionInfo { id, title, ts }).collect())
}

#[tauri::command]
async fn list_recent_sessions(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let rows = state.store.list_recent_sessions(12).await.map_err(|e| format!("{e}"))?;
    Ok(rows.into_iter().map(|(id, pid, title, ts)| serde_json::json!({
        "id": id, "project_id": pid, "title": title, "ts": ts
    })).collect())
}

#[tauri::command]
async fn list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectSummary>, String> {
    let rows = state.store.list_projects().await.map_err(|e| format!("{e}"))?;
    Ok(rows.into_iter()
        .map(|(id, name, ws, _c, upd, cnt)| ProjectSummary { id, name, workspace_dir: ws, session_count: cnt, updated_at: upd })
        .collect())
}

#[tauri::command]
async fn pick_directory(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |p| { let _ = tx.send(p); });
    let picked = rx.await.map_err(|e| format!("{e}"))?;
    Ok(picked.map(|fp| fp.to_string()))
}

#[tauri::command]
async fn create_project(state: State<'_, AppState>, name: String, workspace_dir: String) -> Result<ProjectSummary, String> {
    if name.trim().is_empty() { return Err("Project name is required".into()); }
    let dir = workspace_dir.trim();
    if dir.is_empty() { return Err("A working directory is required".into()); }
    let path = PathBuf::from(dir);
    std::fs::create_dir_all(&path).map_err(|e| format!("Failed to create working directory: {e}"))?;
    // Writability probe: create + remove a temp marker.
    let marker = path.join(".wisp-write-test");
    std::fs::write(&marker, b"").map_err(|e| format!("Working directory is not writable: {e}"))?;
    let _ = std::fs::remove_file(&marker);

    let id = Uuid::new_v4().to_string();
    state.store.create_project(&id, name.trim(), dir).await.map_err(|e| format!("{e}"))?;
    Ok(build_project_summary(&state, &id).await)
}

#[tauri::command]
async fn open_project(state: State<'_, AppState>, id: String) -> Result<ProjectSummary, String> {
    let (name, ws) = state.store.get_project(&id).await.map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Project not found".to_string())?;
    // Interrupt any running turn and hold the agent lock across the whole
    // project swap (mirrors new_session; #11/#15). Holding the lock the entire
    // time — not just for the `= None` reset — closes a race where a concurrent
    // send_message snapshots `active` (old project) before taking this same
    // lock, finds the agent empty, and runs a turn against the old root while
    // we swap `active` underneath it.
    state.cancel.store(true, Ordering::Relaxed);
    let mut guard = state.agent.lock().await;
    *guard = None;
    let root = ensure_writable(PathBuf::from(&ws), &state.app_data);
    let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
    let memory = Arc::new(MemoryManager::new(&root));
    { *state.active.write().unwrap() = ActiveProject { id: id.clone(), root: root.clone(), skills, memory }; }
    { *state.session.lock().await = SessionState { frame_id: None, last_seq: 0 }; }
    state.cancel.store(false, Ordering::Relaxed);
    drop(guard);
    { state.bootstrap.lock().unwrap().workspace = root.to_string_lossy().into_owned(); }
    let _ = state.store.set_setting("active_project_id", &id).await;
    let _ = state.store.create_project(&id, &name, &ws).await; // touch updated_at → sorts to top
    Ok(build_project_summary(&state, &id).await)
}

#[tauri::command]
async fn delete_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if state.active().id == id {
        return Err("Return to the projects list before deleting the active project".into());
    }
    state.store.delete_project(&id).await.map_err(|e| format!("{e}"))?;
    Ok(())
}

/// Switch the active session to `id`, load its transcript, and return the
/// rendered rows so the UI can repopulate the conversation view.
/// Rewind the active session to just before the given user turn (for message edit).
#[tauri::command]
async fn rewind_session(state: State<'_, AppState>, user_index: usize) -> Result<(), String> {
    let keep = {
        let guard = state.agent.lock().await;
        guard
            .as_ref()
            .map(|agent| user_message_start(&agent.ctx.messages, user_index))
            .unwrap_or(0)
    };
    {
        let mut guard = state.agent.lock().await;
        if let Some(agent) = guard.as_mut() {
            agent.ctx.messages.truncate(keep);
        }
    }
    let mut sess = state.session.lock().await;
    if let Some(frame_id) = sess.frame_id.clone() {
        state
            .store
            .truncate_messages(&frame_id, keep as i64)
            .await
            .map_err(|e| format!("{e}"))?;
        sess.last_seq = keep as i64;
    }
    Ok(())
}

#[tauri::command]
async fn load_session(state: State<'_, AppState>, id: String) -> Result<Vec<UiItem>, String> {
    let msgs = state.store.load_messages(&id).await.map_err(|e| format!("{e}"))?;
    {
        let mut sess = state.session.lock().await;
        sess.frame_id = Some(id.clone());
        sess.last_seq = msgs.len() as i64;
    }
    if let Some(agent) = state.agent.lock().await.as_mut() {
        agent.ctx.messages = msgs.clone();
    }
    Ok(messages_to_items(&msgs))
}

#[tauri::command]
fn list_skills(state: State<'_, AppState>) -> Vec<SkillInfo> {
    let ap = state.active();
    ap.skills.all().iter().map(|s| SkillInfo { name: s.name.clone(), description: s.description.clone() }).collect()
}

#[tauri::command]
fn list_demos() -> Vec<seed::DemoInfo> {
    seed::list_demos()
}

#[tauri::command]
fn load_demo(state: State<'_, AppState>, id: String) -> Result<seed::Demo, String> {
    let ap = state.active();
    seed::extract_demo_assets(&id, &ap.root)?;
    seed::load_demo(&id).ok_or_else(|| format!("demo '{id}' not found"))
}

#[tauri::command]
fn confirm_response(state: State<'_, AppState>, approved: bool) -> Result<(), String> {
    if let Some(tx) = state.confirm.lock().unwrap().take() {
        let _ = tx.send(approved);
        Ok(())
    } else {
        Err("no pending confirmation".into())
    }
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let (provider, api_url, model, api_key) = load_settings(&state.store).await;
    let locale = load_locale(&state.store).await;
    let workspace_dir = state.store.get_setting("workspace_dir").await.ok().flatten().unwrap_or_default();
    Ok(Settings { provider, api_url, model, has_api_key: !api_key.is_empty(), locale, workspace_dir })
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
    state.store.set_setting("provider", &provider).await.map_err(|e| format!("{e}"))?;
    state.store.set_setting("api_url", api_url).await.map_err(|e| format!("{e}"))?;
    state.store.set_setting("model", model).await.map_err(|e| format!("{e}"))?;
    let locale = match settings.locale.trim() {
        "zh" | "zh-CN" | "zh-TW" => "zh",
        other if !other.is_empty() => other,
        _ => "en",
    };
    state.store.set_setting("locale", locale).await.map_err(|e| format!("{e}"))?;

    // Workspace directory: persist an absolute, creatable path. Takes effect on
    // next launch (AppState.root is fixed at startup — restart, not hot-swap).
    let workspace_dir = settings.workspace_dir.trim();
    if workspace_dir.is_empty() {
        // Empty clears the override → back to the platform default next launch.
        state.store.set_setting("workspace_dir", "").await.map_err(|e| format!("{e}"))?;
    } else {
        let ws = Path::new(workspace_dir);
        if !ws.is_absolute() {
            return Err("Workspace directory must be an absolute path.".into());
        }
        std::fs::create_dir_all(ws).map_err(|e| format!("Failed to create workspace directory: {e}"))?;
        state.store.set_setting("workspace_dir", workspace_dir).await.map_err(|e| format!("{e}"))?;
    }

    // Reset the cached agent so the next turn picks up the new provider.
    *state.agent.lock().await = None;
    Ok(())
}

#[tauri::command]
fn set_api_key(_state: State<'_, AppState>, key: String) -> Result<(), String> {
    tracing::info!(target: "wisp", has_api_key = !key.is_empty(), "saving api key");
    if key.is_empty() {
        wisp_store::secrets::Secret::delete("api_key").map_err(|e| format!("{e}"))
    } else {
        wisp_store::secrets::Secret::set("api_key", &key).map_err(|e| format!("{e}"))
    }
}

#[tauri::command]
async fn validate_settings(state: State<'_, AppState>, settings: Settings, key: Option<String>) -> Result<String, String> {
    let (_, _, _, stored_key) = load_settings(&state.store).await;
    let api_key = effective_api_key(key, stored_key);
    let provider_name = normalized_provider(&settings.provider);
    let mut cfg = build_provider_config(&settings.provider, &settings.api_url, &api_key, &settings.model)?;
    cfg.max_tokens = 1;

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
    Ok(format!("Validated {} with {}", provider_name, settings.model))
}

fn mime_for_path(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
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
fn list_dir(state: State<'_, AppState>, path: Option<String>) -> Result<Vec<DirEntry>, String> {
    let ap = state.active();
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
        if name.starts_with('.') { continue; }
        entries.push(DirEntry { name, is_dir: meta.is_dir(), size: meta.len() });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

fn read_file_at(state: &AppState, path: String, max_bytes: Option<u64>) -> Result<FileContent, String> {
    let ap = state.active();
    let real = wisp_tools::safety::validate_file_path(&ap.root, &path)?;
    let mime = mime_for_path(&real);
    let cap = max_bytes.unwrap_or(8 * 1024 * 1024).min(32 * 1024 * 1024);
    let bytes = std::fs::read(&real).map_err(|e| format!("{e}"))?;
    if bytes.len() as u64 > cap {
        return Err(format!("file exceeds {cap} byte limit"));
    }
    let path_str = real.to_string_lossy().into_owned();
    if is_text_mime(mime) || mime == "text/csv" || mime == "text/tab-separated-values" || mime == "text/x-fasta" || mime == "chemical/x-pdb" || mime == "chemical/x-mdl-molfile" {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(FileContent { path: path_str, mime: mime.into(), text: Some(text), base64: None })
    } else {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(FileContent { path: path_str, mime: mime.into(), text: None, base64: Some(b64) })
    }
}

#[tauri::command]
fn read_file(state: State<'_, AppState>, path: String, max_bytes: Option<u64>) -> Result<FileContent, String> {
    read_file_at(&state, path, max_bytes)
}

#[tauri::command]
async fn list_artifacts(state: State<'_, AppState>) -> Result<Vec<ArtifactInfo>, String> {
    let frame_id = state.session.lock().await.frame_id.clone();
    let Some(fid) = frame_id else { return Ok(vec![]); };
    let rows = state.store.list_artifacts(&fid).await.map_err(|e| format!("{e}"))?;
    Ok(rows.into_iter().map(|(id, name, ct, path, ts)| ArtifactInfo {
        id,
        name: name.clone(),
        kind: ct,
        path,
        ts,
    }).collect())
}

#[tauri::command]
async fn read_artifact(state: State<'_, AppState>, id: String) -> Result<FileContent, String> {
    let row = state.store.get_artifact(&id).await.map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{id}' not found"))?;
    let (_name, _ct, storage_path, _frame) = row;
    read_file_at(&state, storage_path, None)
}

fn mcp_lib_dir(_root: &std::path::Path) -> Option<PathBuf> {
    wisp_paths::bio_tools_dir().map(|d| d.join("lib"))
}

fn list_mcp_servers(root: &std::path::Path) -> Vec<String> {
    let Some(lib) = mcp_lib_dir(root) else { return vec![]; };
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
    let Ok(rd) = std::fs::read_dir(memory.dir()) else { return 0; };
    rd.flatten().filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md")).count()
}

fn list_memory_files(memory: &MemoryManager) -> Vec<MemoryFile> {
    let Ok(rd) = std::fs::read_dir(memory.dir()) else { return vec![]; };
    let mut paths: Vec<PathBuf> = rd.flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .map(|e| e.path())
        .collect();
    paths.sort_by(|a, b| b.cmp(a));
    paths.into_iter().filter_map(|path| {
        let meta = std::fs::metadata(&path).ok()?;
        let text = std::fs::read_to_string(&path).ok()?;
        let preview: String = text.chars().take(240).collect();
        Some(MemoryFile {
            name: path.file_name()?.to_string_lossy().into_owned(),
            preview,
            bytes: meta.len(),
        })
    }).collect()
}

async fn build_project_info(state: &AppState) -> ProjectInfo {
    let ap = state.active();
    let (_, _, _, api_key) = load_settings(&state.store).await;
    let mcp = list_mcp_servers(&ap.root);
    let name = ap.root.file_name().and_then(|n| n.to_str()).unwrap_or("Workspace").to_string();
    ProjectInfo {
        name,
        root: ap.root.to_string_lossy().into_owned(),
        skill_count: ap.skills.all().len(),
        mcp_server_count: mcp.len(),
        memory_file_count: count_memory_files(&ap.memory),
        has_api_key: !api_key.is_empty(),
    }
}

#[tauri::command]
async fn get_project_info(state: State<'_, AppState>) -> Result<ProjectInfo, String> {
    Ok(build_project_info(&state).await)
}

#[tauri::command]
async fn get_capabilities(state: State<'_, AppState>) -> Result<Capabilities, String> {
    let ap = state.active();
    let project = build_project_info(&state).await;
    let skills = ap.skills.all().iter().map(|s| SkillInfo { name: s.name.clone(), description: s.description.clone() }).collect();
    Ok(Capabilities {
        skills,
        mcp_servers: list_mcp_servers(&ap.root),
        memory_files: list_memory_files(&ap.memory),
        project,
    })
}

#[tauri::command]
fn list_memory(state: State<'_, AppState>) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active();
    Ok(list_memory_files(&ap.memory))
}

#[tauri::command]
async fn get_onboarding_state(state: State<'_, AppState>) -> Result<OnboardingState, String> {
    let (_, _, _, api_key) = load_settings(&state.store).await;
    let done = state.store.get_setting("onboarding_done").await.ok().flatten().is_some();
    Ok(OnboardingState { show: !done, has_api_key: !api_key.is_empty() })
}

fn initial_bootstrap(app_data: &std::path::Path, workspace: &std::path::Path, skills: usize) -> BootstrapStatus {
    let mut status = BootstrapStatus {
        skills_loaded: skills,
        python_ok: false,
        mcp_catalog: list_mcp_servers(workspace).len(),
        uv_ok: wisp_python::PythonEnv::find_uv().is_some(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        workspace: workspace.to_string_lossy().into_owned(),
        errors: vec![],
    };
    if status.skills_loaded == 0 {
        status.errors.push("No bundled skills found in install resources.".into());
    }
    if !status.uv_ok {
        status.errors.push("uv not found on PATH; install uv or set UV_PATH.".into());
    }
    match wisp_python::PythonEnv::ensure(app_data) {
        Ok(_) => status.python_ok = true,
        Err(e) => status.errors.push(format!("Python environment: {e}")),
    }
    if wisp_paths::bio_tools_dir().is_none() {
        status.errors.push("Bundled bio-tools MCP catalog not found.".into());
    }
    status
}

#[tauri::command]
fn get_bootstrap_status(state: State<'_, AppState>) -> BootstrapStatus {
    state.bootstrap.lock().unwrap().clone()
}

#[tauri::command]
async fn check_for_updates() -> Result<String, String> {
    Ok("In-app auto-update is disabled until release signing is configured. Download new builds from GitHub Releases.".into())
}

#[tauri::command]
async fn dismiss_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    state.store.set_setting("onboarding_done", "1").await.map_err(|e| format!("{e}"))
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
    let ext = std::path::Path::new(name).extension().and_then(|s| s.to_str());
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

async fn register_artifact_at(
    state: &AppState,
    ap: &ActiveProject,
    path: String,
    content_type: Option<String>,
) -> Result<ArtifactInfo, String> {
    let real = wisp_tools::safety::validate_file_path(&ap.root, &path)?;
    let mut sess = state.session.lock().await;
    let frame_id = ensure_frame(&state.store, &ap.id, &mut sess).await.map_err(|e| format!("{e}"))?;
    let id = Uuid::new_v4().to_string();
    let filename = real.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    let mime = content_type.unwrap_or_else(|| mime_for_path(&real).to_string());
    let storage = real.to_string_lossy().into_owned();
    state.store.save_artifact(&id, &ap.id, &frame_id, &filename, &mime, &storage).await.map_err(|e| format!("{e}"))?;
    let ts = chrono::Utc::now().timestamp();
    Ok(ArtifactInfo { id, name: filename, kind: mime, path: storage, ts })
}

#[tauri::command]
async fn upload_file(
    state: State<'_, AppState>,
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
    let ap = state.active();
    let upload_dir = ap.root.join("uploads");
    std::fs::create_dir_all(&upload_dir).map_err(|e| format!("{e}"))?;
    let dest = unique_upload_path(&ap.root, "uploads", &name);
    std::fs::write(&dest, &bytes).map_err(|e| format!("{e}"))?;
    let rel = dest
        .strip_prefix(&ap.root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| dest.to_string_lossy().into_owned());
    register_artifact_at(&state, &ap, rel, None).await
}

#[tauri::command]
async fn register_artifact(
    state: State<'_, AppState>,
    path: String,
    content_type: Option<String>,
) -> Result<ArtifactInfo, String> {
    let ap = state.active();
    register_artifact_at(&state, &ap, path, content_type).await
}

/// Tell the webview whether we're in dev (keep native context menu / DevTools).
fn set_dev_flag(app: &tauri::AppHandle) {
    let dev = cfg!(debug_assertions);
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.eval(&format!("window.__WISP_DEV__ = {};", dev));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("wisp=info".parse().unwrap());
    let subscriber = tracing_subscriber::fmt().with_env_filter(filter);
    #[cfg(all(not(debug_assertions), target_os = "windows"))]
    subscriber.with_writer(std::io::sink).init();
    #[cfg(not(all(not(debug_assertions), target_os = "windows")))]
    subscriber.init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
                let default_workspace = app.path().document_dir()
                    .map(|d| d.join("wisp-science"))
                    .unwrap_or_else(|_| app_data.join("workspace"));
                let legacy_ws = store.get_setting("workspace_dir").await.ok().flatten()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| default_workspace.to_string_lossy().into_owned());
                store.create_project("default", "Workspace", &legacy_ws).await.ok();
                let active_id = match store.get_setting("active_project_id").await.ok().flatten() {
                    Some(id) if store.get_project(&id).await.ok().flatten().is_some() => id,
                    _ => "default".to_string(),
                };
                let (_, dir) = store.get_project(&active_id).await.ok().flatten()
                    .unwrap_or_else(|| ("Workspace".into(), legacy_ws.clone()));
                (active_id, dir)
            });

            // Env override wins for the active root only (dev escape hatch; not persisted).
            let default_workspace = app.path().document_dir()
                .map(|d| d.join("wisp-science"))
                .unwrap_or_else(|_| app_data.join("workspace"));
            let root = resolve_workspace(std::env::var("WISP_WORKSPACE").ok(), Some(ws), default_workspace);
            let root = ensure_writable(root, &app_data);

            let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
            let memory = Arc::new(MemoryManager::new(&root));
            let bootstrap = StdMutex::new(initial_bootstrap(&app_data, &root, skills.all().len()));
            let state = AppState {
                app_data,
                store,
                active: std::sync::RwLock::new(ActiveProject { id: active_id, root, skills, memory }),
                agent: tokio::sync::Mutex::new(None),
                session: tokio::sync::Mutex::new(SessionState { frame_id: None, last_seq: 0 }),
                confirm: Arc::new(StdMutex::new(None)),
                bootstrap,
                cancel: Arc::new(AtomicBool::new(false)),
            };
            app.manage(state);
            set_dev_flag(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_message,
            stop_agent,
            new_session,
            list_sessions,
            list_recent_sessions,
            list_projects,
            pick_directory,
            create_project,
            open_project,
            delete_project,
            load_session,
            rewind_session,
            list_skills,
            list_demos,
            load_demo,
            confirm_response,
            get_settings,
            set_settings,
            set_api_key,
            validate_settings,
            list_dir,
            read_file,
            list_artifacts,
            read_artifact,
            upload_file,
            register_artifact,
            get_project_info,
            get_capabilities,
            list_memory,
            get_onboarding_state,
            dismiss_onboarding,
            get_bootstrap_status,
            check_for_updates,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Wisp");
}

#[cfg(test)]
mod tests {
    use super::resolve_workspace;
    use std::path::PathBuf;

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
}
