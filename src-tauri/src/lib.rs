//! Tauri v2 desktop shell: commands that drive the Wisp agent and stream
//! events to the webview, plus a settings/confirm surface.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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

/// A reloaded transcript row for the UI to render (role in
/// user|assistant|reasoning|tool).
#[derive(Serialize, Clone)]
struct UiItem {
    role: String,
    text: String,
    tool_name: Option<String>,
    ok: Option<bool>,
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
                out.push(UiItem { role: "tool".into(), text: m.content.as_text(), tool_name: m.tool_name.clone(), ok: Some(true) });
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
}

struct SessionState {
    frame_id: Option<String>,
    last_seq: i64,
}

struct AppState {
    root: PathBuf,
    store: Store,
    project_id: String,
    skills: Arc<SkillIndex>,
    memory: Arc<MemoryManager>,
    agent: tokio::sync::Mutex<Option<Agent>>,
    session: tokio::sync::Mutex<SessionState>,
    confirm: Arc<StdMutex<Option<std::sync::mpsc::Sender<bool>>>>,
}

/// `wisp_core::Output` backed by Tauri events. `confirm` blocks on a std
/// channel satisfied by the `confirm_response` command.
struct TauriOutput {
    app: AppHandle,
    frame_id: String,
    confirm: Arc<StdMutex<Option<std::sync::mpsc::Sender<bool>>>>,
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
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn normalized_provider(provider: &str) -> String {
    match provider.trim() {
        "anthropic" => "anthropic".into(),
        _ => "openai".into(),
    }
}

fn non_empty_setting(value: Option<String>, fallback: impl FnOnce() -> String) -> String {
    value.filter(|v| !v.trim().is_empty()).unwrap_or_else(fallback)
}

async fn load_settings(store: &Store) -> (String, String, String, String) {
    let provider = normalized_provider(&non_empty_setting(
        store.get_setting("provider").await.ok().flatten(),
        || env_or("WISP_PROVIDER", "openai"),
    ));
    let api_url = non_empty_setting(store.get_setting("api_url").await.ok().flatten(), || {
        env_or("WISP_API_URL", if provider == "anthropic" { "https://api.anthropic.com" } else { "https://api.deepseek.com" })
    });
    let model = non_empty_setting(store.get_setting("model").await.ok().flatten(), || {
        env_or("WISP_MODEL", if provider == "anthropic" { "claude-3-5-sonnet-latest" } else { "deepseek-chat" })
    });
    let api_key = wisp_store::secrets::Secret::get("api_key").ok().unwrap_or_else(|| env_or("WISP_API_KEY", ""));
    (provider, api_url, model, api_key)
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

/// Wire the Python REPL (bundled kernel worker) and an MCP server into a
/// freshly built agent. Mirrors the CLI wiring; best-effort, never fatal.
async fn wire_python_and_mcp(agent: &mut wisp_core::Agent, root: &std::path::Path) {
    let py_env = wisp_python::PythonEnv::ensure(root).ok();

    let worker = std::env::var("WISP_KERNEL_WORKER")
        .ok()
        .or_else(|| wisp_python::bundled_worker_path().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_default();
    let worker_path = std::path::PathBuf::from(&worker);
    if worker_path.is_file() {
        if let Some(env) = &py_env {
            if let Ok(client) = wisp_python::KernelClient::spawn(&env.python(), &worker_path) {
                agent.add_tool(Box::new(wisp_python::ReplTool::new(client)));
            }
        }
    }

    if let Ok(cmdline) = std::env::var("WISP_MCP_COMMAND") {
        let parts: Vec<&str> = cmdline.split_whitespace().collect();
        if parts.len() >= 2 {
            let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
            if let Ok(client) = wisp_mcp::McpClient::launch(parts[0], &args).await {
                register_mcp(agent, std::sync::Arc::new(client)).await;
            }
        }
    } else if let Ok(pkg) = std::env::var("WISP_MCP_PKG") {
        if let Some(env) = &py_env {
            if let Ok(client) = wisp_mcp::McpClient::launch_bio_tools(&env.python(), &pkg).await {
                register_mcp(agent, std::sync::Arc::new(client)).await;
            }
        }
    }
}

async fn register_mcp(agent: &mut wisp_core::Agent, client: std::sync::Arc<wisp_mcp::McpClient>) {
    if let Ok(tools) = client.tools_list().await {
        for t in tools {
            agent.add_tool(Box::new(wisp_mcp::McpTool::new(t, client.clone())));
        }
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

    let output = TauriOutput { app: app.clone(), frame_id: turn_id.clone(), confirm: state.confirm.clone() };

    let mut guard = state.agent.lock().await;
    if guard.is_none() {
        let mut agent = Agent::new(cfg.clone(), state.skills.clone(), state.memory.clone(), state.root.clone(), max_context, max_iter);
        // Load the persisted session from SQLite (replaces the JSON session file).
        let frame_id = {
            let mut sess = state.session.lock().await;
            ensure_frame(&state.store, &state.project_id, &mut sess).await.map_err(|e| format!("{e}"))?
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
            agent.seed_system_prompt(&state.skills);
        }
        wire_python_and_mcp(&mut agent, &state.root).await;
        *guard = Some(agent);
    }
    let agent = guard.as_mut().unwrap();

    let result = agent.run(&message, &output).await;
    agent.ctx.clear_runtime_injections();

    // Persist only the messages added this turn to SQLite.
    {
        let mut sess = state.session.lock().await;
        if let Some(frame_id) = sess.frame_id.clone() {
            let start = sess.last_seq as usize;
            let mut seq = sess.last_seq;
            for m in &agent.ctx.messages[start..] {
                seq += 1;
                if let Err(e) = state.store.append_message(&frame_id, seq, m).await {
                    tracing::warn!("persist message {seq} failed: {e}");
                    break;
                }
            }
            sess.last_seq = agent.ctx.messages.len() as i64;
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
async fn new_session(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.agent.lock().await;
    if let Some(agent) = guard.as_mut() {
        agent.ctx.clear();
        let mut sess = state.session.lock().await;
        sess.frame_id = None;
        sess.last_seq = 0;
        let frame_id = ensure_frame(&state.store, &state.project_id, &mut sess).await.map_err(|e| format!("{e}"))?;
        agent.ctx.messages = state.store.load_messages(&frame_id).await.unwrap_or_default();
        sess.last_seq = agent.ctx.messages.len() as i64;
        if agent.ctx.is_empty() {
            agent.seed_system_prompt(&state.skills);
        }
    }
    Ok(())
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, String> {
    let rows = state.store.list_sessions(&state.project_id).await.map_err(|e| format!("{e}"))?;
    Ok(rows.into_iter().map(|(id, title, ts)| SessionInfo { id, title, ts }).collect())
}

/// Switch the active session to `id`, load its transcript, and return the
/// rendered rows so the UI can repopulate the conversation view.
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
    state.skills.all().iter().map(|s| SkillInfo { name: s.name.clone(), description: s.description.clone() }).collect()
}

#[tauri::command]
fn list_demos() -> Vec<seed::DemoInfo> {
    seed::list_demos()
}

#[tauri::command]
fn load_demo(id: String) -> Result<seed::Demo, String> {
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
    Ok(Settings { provider, api_url, model, has_api_key: !api_key.is_empty() })
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
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        provider.complete(&[Message::user("Reply with OK.")], &[]),
    )
    .await
    .map_err(|_| "Validation timed out after 30s".to_string())?
    .map_err(|e| format!("{e}"))?;

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
    let rel = path.unwrap_or_else(|| ".".into());
    let dir = wisp_tools::safety::resolve_under_root(&state.root, &rel)?;
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
    let real = wisp_tools::safety::validate_file_path(&state.root, &path)?;
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

fn mcp_lib_dir(root: &std::path::Path) -> Option<PathBuf> {
    for lib in [
        root.join("mcp-servers").join("bio-tools").join("lib"),
        root.join("..").join("mcp-servers").join("bio-tools").join("lib"),
    ] {
        if lib.is_dir() { return Some(lib); }
    }
    None
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
    let (_, _, _, api_key) = load_settings(&state.store).await;
    let mcp = list_mcp_servers(&state.root);
    let name = state.root.file_name().and_then(|n| n.to_str()).unwrap_or("Workspace").to_string();
    ProjectInfo {
        name,
        root: state.root.to_string_lossy().into_owned(),
        skill_count: state.skills.all().len(),
        mcp_server_count: mcp.len(),
        memory_file_count: count_memory_files(&state.memory),
        has_api_key: !api_key.is_empty(),
    }
}

#[tauri::command]
async fn get_project_info(state: State<'_, AppState>) -> Result<ProjectInfo, String> {
    Ok(build_project_info(&state).await)
}

#[tauri::command]
async fn get_capabilities(state: State<'_, AppState>) -> Result<Capabilities, String> {
    let project = build_project_info(&state).await;
    let skills = state.skills.all().iter().map(|s| SkillInfo { name: s.name.clone(), description: s.description.clone() }).collect();
    Ok(Capabilities {
        skills,
        mcp_servers: list_mcp_servers(&state.root),
        memory_files: list_memory_files(&state.memory),
        project,
    })
}

#[tauri::command]
fn list_memory(state: State<'_, AppState>) -> Result<Vec<MemoryFile>, String> {
    Ok(list_memory_files(&state.memory))
}

#[tauri::command]
async fn get_onboarding_state(state: State<'_, AppState>) -> Result<OnboardingState, String> {
    let (_, _, _, api_key) = load_settings(&state.store).await;
    let done = state.store.get_setting("onboarding_done").await.ok().flatten().is_some();
    Ok(OnboardingState { show: !done, has_api_key: !api_key.is_empty() })
}

#[tauri::command]
async fn dismiss_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    state.store.set_setting("onboarding_done", "1").await.map_err(|e| format!("{e}"))
}

#[tauri::command]
async fn register_artifact(
    state: State<'_, AppState>,
    path: String,
    content_type: Option<String>,
) -> Result<ArtifactInfo, String> {
    let real = wisp_tools::safety::validate_file_path(&state.root, &path)?;
    let mut sess = state.session.lock().await;
    let frame_id = ensure_frame(&state.store, &state.project_id, &mut sess).await.map_err(|e| format!("{e}"))?;
    let id = Uuid::new_v4().to_string();
    let filename = real.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    let mime = content_type.unwrap_or_else(|| mime_for_path(&real).to_string());
    let storage = real.to_string_lossy().into_owned();
    state.store.save_artifact(&id, &state.project_id, &frame_id, &filename, &mime, &storage).await.map_err(|e| format!("{e}"))?;
    let ts = chrono::Utc::now().timestamp();
    Ok(ArtifactInfo { id, name: filename, kind: mime, path: storage, ts })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("wisp=info".parse().unwrap()))
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let db_path = root.join(".wisp").join("wisp.sqlite");
            let store = tauri::async_runtime::block_on(Store::open(&db_path)).expect("open store");
            let _ = tauri::async_runtime::block_on(store.create_project("default", "Workspace"));
            let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
            let memory = Arc::new(MemoryManager::new(&root));
            let state = AppState {
                root,
                store,
                project_id: "default".into(),
                skills,
                memory,
                agent: tokio::sync::Mutex::new(None),
                session: tokio::sync::Mutex::new(SessionState { frame_id: None, last_seq: 0 }),
                confirm: Arc::new(StdMutex::new(None)),
            };
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_message,
            new_session,
            list_sessions,
            load_session,
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
            register_artifact,
            get_project_info,
            get_capabilities,
            list_memory,
            get_onboarding_state,
            dismiss_onboarding,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Wisp");
}
