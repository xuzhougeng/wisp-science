use crate::{acp_bridge_launch, ActiveProject, AgentEvent, AppState};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use uuid::Uuid;
use wisp_acp::{
    acp::schema::v1::{
        ContentBlock, McpServer, McpServerStdio, ResourceLink, SessionId, TextContent,
    },
    AcpAgentProfile as LaunchProfile, AcpPermissionKind, AcpPermissionRequest, AcpSessionEvent,
    AcpSessionHandle, AcpStopReason, AcpUpdateKind,
};
use wisp_llm::Message;

const PROFILES_KEY: &str = "acp_agent_profiles";
const ACP_READ_ONLY_TIMEOUT: Duration = Duration::from_secs(90);
const ACP_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);
const ACP_SHARED_CONTEXT_MAX_CHARS: usize = 24_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AcpAgentProfile {
    #[serde(default)]
    pub id: String,
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

// The webview DTOs (ui/src/dto.rs) deserialize with rename_all = "camelCase";
// without the matching attribute here `protocolVersion`/`authMethods` fall back
// to defaults (the "ACP v0" bug) and permission events fail to parse at all,
// hanging the turn (#200, #201).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcpAgentInfoDto {
    protocol_version: u16,
    implementation: Option<serde_json::Value>,
    capabilities: serde_json::Value,
    auth_methods: Vec<serde_json::Value>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PermissionEvent {
    request_id: String,
    frame_id: String,
    tool_call: serde_json::Value,
    options: Vec<serde_json::Value>,
}

pub(crate) struct AcpRuntime {
    pub profile_id: String,
    pub fingerprint: String,
    pub cwd: PathBuf,
    pub session_id: SessionId,
    pub session_state: Mutex<Option<wisp_acp::AcpSessionState>>,
    pub handle: Arc<AcpSessionHandle>,
}

pub(crate) type AcpRuntimeMap = Mutex<HashMap<String, Arc<AcpRuntime>>>;

fn validate(profile: &AcpAgentProfile) -> Result<(), String> {
    if profile.label.trim().is_empty() {
        return Err("ACP Agent label is required.".into());
    }
    if profile.command.trim().is_empty() {
        return Err("ACP Agent command is required.".into());
    }
    Ok(())
}

pub(crate) fn profile_available(profile: &AcpAgentProfile) -> bool {
    validate(profile).is_ok() && which::which(profile.command.trim()).is_ok()
}

pub(crate) fn fingerprint(profile: &AcpAgentProfile) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in serde_json::to_vec(&(profile.command.trim(), &profile.args)).unwrap_or_default() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

pub(crate) async fn profiles(store: &wisp_store::Store) -> Vec<AcpAgentProfile> {
    store
        .get_setting(PROFILES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

async fn save_profiles(
    store: &wisp_store::Store,
    profiles: &[AcpAgentProfile],
) -> Result<(), String> {
    let raw = serde_json::to_string(profiles).map_err(|error| error.to_string())?;
    store
        .set_setting(PROFILES_KEY, &raw)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) fn launch_profile(profile: &AcpAgentProfile) -> LaunchProfile {
    LaunchProfile::new(
        profile.id.clone(),
        profile.label.clone(),
        PathBuf::from(&profile.command),
        profile.args.clone(),
    )
}

fn info_dto(handle: &AcpSessionHandle) -> AcpAgentInfoDto {
    let info = handle.info();
    AcpAgentInfoDto {
        protocol_version: info.protocol_version,
        implementation: info.implementation.as_ref().map(|implementation| {
            serde_json::json!({
                "name": implementation.name,
                "title": implementation.title,
                "version": implementation.version,
            })
        }),
        capabilities: info.capabilities.clone(),
        auth_methods: info
            .auth_methods
            .iter()
            .map(|method| {
                serde_json::json!({
                    "id": method.id,
                    "name": method.name,
                    "description": method.description,
                })
            })
            .collect(),
    }
}

#[tauri::command]
pub(crate) async fn list_acp_agents(
    state: State<'_, AppState>,
) -> Result<Vec<AcpAgentProfile>, String> {
    Ok(profiles(&state.store).await)
}

#[tauri::command]
pub(crate) async fn get_acp_session_agent(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    frame_id: String,
) -> Result<Option<String>, String> {
    let project = state.active(window.label());
    if state
        .store
        .frame_project_id(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
        != Some(project.id.as_str())
    {
        return Err("Session does not belong to the active project.".into());
    }
    Ok(state
        .store
        .get_acp_session(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .map(|binding| binding.agent_profile_id))
}

/// The mode controls need `availableModes`, but nothing launches the agent until
/// the first turn, so after an app restart they would stay hidden until the user
/// sent a message. Serve the cached list for the bound profile instead.
///
/// `currentModeId` is deliberately left out: it belongs to a session whose agent
/// process is gone, and the caller seeds this without overwriting a live one.
#[tauri::command]
pub(crate) async fn get_acp_session_state(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    frame_id: String,
) -> Result<Option<serde_json::Value>, String> {
    let project = state.active(window.label());
    if state
        .store
        .frame_project_id(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
        != Some(project.id.as_str())
    {
        return Err("Session does not belong to the active project.".into());
    }
    let Some(binding) = state
        .store
        .get_acp_session(&frame_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    Ok(state
        .store
        .get_setting(&available_modes_key(&binding.profile_fingerprint))
        .await
        .map_err(|error| error.to_string())?
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .map(|modes| serde_json::json!({ "availableModes": modes })))
}

#[tauri::command]
pub(crate) async fn save_acp_agent(
    state: State<'_, AppState>,
    mut profile: AcpAgentProfile,
) -> Result<Vec<AcpAgentProfile>, String> {
    validate(&profile)?;
    let mut all = profiles(&state.store).await;
    if profile.id.trim().is_empty() {
        profile.id = Uuid::new_v4().to_string();
    }
    if let Some(existing) = all.iter_mut().find(|candidate| candidate.id == profile.id) {
        *existing = profile;
    } else {
        all.push(profile);
    }
    save_profiles(&state.store, &all).await?;
    Ok(all)
}

#[tauri::command]
pub(crate) async fn remove_acp_agent(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<AcpAgentProfile>, String> {
    let mut all = profiles(&state.store).await;
    all.retain(|profile| profile.id != id);
    save_profiles(&state.store, &all).await?;
    Ok(all)
}

#[tauri::command]
pub(crate) async fn test_acp_agent(
    state: State<'_, AppState>,
    id: String,
) -> Result<AcpAgentInfoDto, String> {
    let profile = profiles(&state.store)
        .await
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| "Unknown ACP Agent.".to_string())?;
    let handle = AcpSessionHandle::launch(launch_profile(&profile))
        .await
        .map_err(|error| error.to_string())?;
    let info = info_dto(&handle);
    handle.shutdown(Duration::from_secs(2)).await;
    Ok(info)
}

#[tauri::command]
pub(crate) async fn authenticate_acp_agent(
    state: State<'_, AppState>,
    id: String,
    method_id: String,
) -> Result<(), String> {
    let profile = profiles(&state.store)
        .await
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| "Unknown ACP Agent.".to_string())?;
    let handle = AcpSessionHandle::launch(launch_profile(&profile))
        .await
        .map_err(|error| error.to_string())?;
    let result = handle
        .authenticate(method_id)
        .await
        .map_err(|error| error.to_string());
    handle.shutdown(Duration::from_secs(2)).await;
    result
}

/// One-shot, read-only ACP answer: launch a throwaway session without MCP
/// servers, prompt once, collect the reply, and shut the agent down. Tool-use
/// requests are auto-rejected so side chat and independent review cannot mutate
/// the workspace or block waiting on approval.
pub(crate) async fn acp_read_only_once(
    state: &AppState,
    cwd: &Path,
    profile_id: &str,
    prompt_text: &str,
    cancel: Option<&AtomicBool>,
) -> Result<String, String> {
    let profile = profiles(&state.store)
        .await
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| "Unknown ACP Agent.".to_string())?;
    let handle = AcpSessionHandle::launch(launch_profile(&profile))
        .await
        .map_err(|error| error.to_string())?;
    let result = await_read_only_result(
        acp_read_only_turn(&handle, cwd, prompt_text),
        ACP_READ_ONLY_TIMEOUT,
        cancel,
    )
    .await;
    handle.shutdown(Duration::from_secs(2)).await;
    result
}

async fn await_read_only_result<F>(
    future: F,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<String, String>
where
    F: Future<Output = Result<String, String>>,
{
    tokio::pin!(future);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let cancellation = wait_for_cancel(cancel);
    tokio::pin!(cancellation);
    tokio::select! {
        result = &mut future => result,
        _ = &mut deadline => Err(format!(
            "The ACP read-only session timed out after {} seconds.",
            timeout.as_secs()
        )),
        _ = &mut cancellation => Err("The ACP read-only session was cancelled.".into()),
    }
}

async fn wait_for_cancel(cancel: Option<&AtomicBool>) {
    let Some(cancel) = cancel else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(ACP_CANCEL_POLL_INTERVAL).await;
    }
}

async fn acp_read_only_turn(
    handle: &AcpSessionHandle,
    cwd: &Path,
    prompt_text: &str,
) -> Result<String, String> {
    let start = handle
        .new_session(cwd, vec![])
        .await
        .map_err(|error| error.to_string())?;
    let content = vec![ContentBlock::Text(TextContent::new(
        prompt_text.to_string(),
    ))];
    let prompt = handle.prompt(start.session_id, content);
    tokio::pin!(prompt);
    let mut answer = String::new();
    loop {
        tokio::select! {
            result = &mut prompt => {
                result.map_err(|error| error.to_string())?;
                break;
            }
            event = handle.next_event() => match event {
                Some(AcpSessionEvent::Update { kind, payload, .. }) => {
                    if kind == AcpUpdateKind::AgentMessage {
                        if let Some(text) = text_from_payload(&payload) {
                            answer.push_str(text);
                        }
                    }
                }
                Some(AcpSessionEvent::Permission(request)) => {
                    // Read-only session: reject the tool without cancelling the
                    // turn when the agent offers a reject option; else cancel.
                    let reject = request
                        .options
                        .iter()
                        .find(|option| {
                            matches!(
                                option.kind,
                                AcpPermissionKind::RejectOnce | AcpPermissionKind::RejectAlways
                            )
                        })
                        .map(|option| option.id.clone());
                    let _ = handle.respond_permission(request.request_id, reject);
                }
                Some(AcpSessionEvent::Exited { error }) => {
                    return Err(error.unwrap_or_else(|| "ACP Agent exited.".into()));
                }
                None => return Err("ACP Agent event stream closed.".into()),
            }
        }
    }
    let answer = answer.trim();
    if answer.is_empty() {
        Err("The ACP Agent returned no answer.".into())
    } else {
        Ok(answer.to_string())
    }
}

pub(crate) async fn acp_side_chat_once(
    state: &AppState,
    cwd: &Path,
    profile_id: &str,
    prompt_text: &str,
) -> Result<String, String> {
    acp_read_only_once(state, cwd, profile_id, prompt_text, None).await
}

pub(crate) async fn profile_label(store: &wisp_store::Store, profile_id: &str) -> Option<String> {
    profiles(store)
        .await
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| profile.label)
}

pub(crate) fn mcp_server(
    state: &AppState,
    project: &ActiveProject,
    frame_id: &str,
    allowed_tools: Option<&[String]>,
) -> Result<McpServer, String> {
    project_mcp_server(&state.app_data, project, frame_id, allowed_tools)
}

pub(crate) fn project_mcp_server(
    app_data: &Path,
    project: &ActiveProject,
    frame_id: &str,
    allowed_tools: Option<&[String]>,
) -> Result<McpServer, String> {
    let (command, args) = acp_bridge_launch(app_data, project, frame_id, allowed_tools)?;
    Ok(McpServer::Stdio(
        McpServerStdio::new("wisp-science", PathBuf::from(command)).args(args),
    ))
}

fn available_modes_key(profile_fingerprint: &str) -> String {
    format!("acp.available_modes.{profile_fingerprint}")
}

async fn runtime_for(
    state: &AppState,
    project: &ActiveProject,
    frame_id: &str,
    requested_profile_id: Option<&str>,
) -> Result<Arc<AcpRuntime>, String> {
    if let Some(runtime) = state.acp_sessions.lock().await.get(frame_id).cloned() {
        if runtime.handle.is_alive() {
            let profile = profiles(&state.store)
                .await
                .into_iter()
                .find(|profile| profile.id == runtime.profile_id)
                .ok_or_else(|| "The attached ACP Agent profile no longer exists.".to_string())?;
            if requested_profile_id.is_some_and(|id| id != runtime.profile_id)
                || fingerprint(&profile) != runtime.fingerprint
                || project.root != runtime.cwd
            {
                return Err("The ACP Agent selection, launch command, or project path changed; start a new session.".into());
            }
            return Ok(runtime);
        }
        // The agent process died (crash, host reboot mid-run). Evict the dead
        // runtime and fall through to relaunch + resume from the saved binding
        // instead of failing every turn until the user starts a new session.
        state.acp_sessions.lock().await.remove(frame_id);
    }
    let binding = state
        .store
        .get_acp_session(frame_id)
        .await
        .map_err(|error| error.to_string())?;
    let profile_id = requested_profile_id
        .map(str::to_owned)
        .or_else(|| {
            binding
                .as_ref()
                .map(|binding| binding.agent_profile_id.clone())
        })
        .ok_or_else(|| "No ACP Agent is attached to this session.".to_string())?;
    let profile = profiles(&state.store)
        .await
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| "The attached ACP Agent profile no longer exists.".to_string())?;
    let profile_fingerprint = fingerprint(&profile);
    let cwd = project.root.clone();
    if let Some(binding) = &binding {
        if binding.agent_profile_id != profile.id
            || binding.profile_fingerprint != profile_fingerprint
            || PathBuf::from(&binding.cwd) != cwd
        {
            return Err(
                "This ACP Agent profile or project path changed; start a new ACP session.".into(),
            );
        }
    } else if !state
        .store
        .load_messages(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err("An ACP Agent can only be attached to an empty session.".into());
    }

    let handle = Arc::new(
        AcpSessionHandle::launch(launch_profile(&profile))
            .await
            .map_err(|error| error.to_string())?,
    );
    let bridge = vec![mcp_server(state, project, frame_id, None)?];
    let (session_id, session_state) = if let Some(binding) = &binding {
        let id = SessionId::new(binding.agent_session_id.clone());
        match handle
            .resume_session(id.clone(), &cwd, bridge.clone())
            .await
        {
            Ok(state) => (id, state),
            Err(wisp_acp::AcpError::Unsupported(_)) => {
                match handle.load_session(id.clone(), &cwd, bridge).await {
                    Ok(state) => (id, state),
                    Err(wisp_acp::AcpError::Unsupported(_)) => {
                        return Err("This ACP Agent cannot resume or load the saved session.".into())
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    } else {
        let start = handle
            .new_session(&cwd, bridge)
            .await
            .map_err(|error| error.to_string())?;
        (start.session_id, start.state)
    };
    // `availableModes` decides whether the plan toggle and the plan card actions
    // render at all, and it is a property of the profile rather than of this
    // session — cache it so those controls survive an app restart, which leaves
    // no agent process to ask. Keyed by fingerprint so a profile whose launch
    // command changed re-learns its modes instead of reusing stale ones.
    if let Some(modes) = session_state
        .modes
        .as_ref()
        .and_then(|modes| modes.get("availableModes"))
    {
        let _ = state
            .store
            .set_setting(
                &available_modes_key(&profile_fingerprint),
                &modes.to_string(),
            )
            .await;
    }
    let runtime = Arc::new(AcpRuntime {
        profile_id: profile.id.clone(),
        fingerprint: profile_fingerprint.clone(),
        cwd: cwd.clone(),
        session_id: session_id.clone(),
        session_state: Mutex::new(Some(session_state)),
        handle,
    });
    if binding.is_none() {
        let info = info_dto(&runtime.handle);
        let now = chrono::Utc::now().timestamp();
        state
            .store
            .save_acp_session(&wisp_store::AcpSessionBinding {
                frame_id: frame_id.to_string(),
                agent_profile_id: profile.id,
                profile_fingerprint,
                agent_session_id: session_id.to_string(),
                cwd: cwd.to_string_lossy().into_owned(),
                protocol_version: 1,
                agent_info_json: serde_json::to_string(&info.implementation).unwrap_or_default(),
                capabilities_json: info.capabilities.to_string(),
                created_at: now,
                updated_at: now,
            })
            .await
            .map_err(|error| error.to_string())?;
    }
    state
        .acp_sessions
        .lock()
        .await
        .insert(frame_id.to_string(), runtime.clone());
    Ok(runtime)
}

fn text_from_payload(payload: &serde_json::Value) -> Option<&str> {
    payload
        .get("content")
        .and_then(|content| content.get("text"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| payload.get("text").and_then(serde_json::Value::as_str))
}

/// Durable ACP tool snapshot stored as a `Message::tool` body so reloads can
/// rebuild the live `AcpTool` transcript rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AcpToolEnvelope {
    pub v: u8,
    pub call_id: String,
    pub title: String,
    #[serde(default)]
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub locations: String,
    /// Optional ACP tool-call fields. Codex and other agents may provide these
    /// even when the content block itself only contains a terminal handle.
    #[serde(default)]
    pub raw_input: String,
    #[serde(default)]
    pub raw_output: String,
}

impl AcpToolEnvelope {
    pub(crate) fn to_message(&self) -> Message {
        let body = serde_json::to_string(self).unwrap_or_else(|_| "{}".into());
        Message::tool(&self.call_id, format!("acp:{}", self.title), body)
    }

    pub(crate) fn from_tool_message(name: Option<&str>, body: &str) -> Option<Self> {
        if !name.is_some_and(|name| name.starts_with("acp:")) {
            return None;
        }
        let envelope: Self = serde_json::from_str(body).ok()?;
        (envelope.v == 1 && !envelope.call_id.is_empty()).then_some(envelope)
    }
}

/// Tool name marking a persisted plan snapshot. Deliberately outside the `acp:`
/// prefix so plan rows never land in the ACP tool transcript or review evidence.
pub(crate) const PLAN_TOOL_NAME: &str = "wisp:plan";

/// The turn's final plan, stored in the ACP entry shape so the UI parses live
/// and reloaded plans with one function.
///
/// ponytail: one plan row per turn that produced one, so a reloaded session
/// shows how the plan evolved. Collapsing to "latest only" would need a
/// cross-turn delete pass and would lose that history.
fn plan_message(seq: i64, payload: &serde_json::Value) -> Option<Message> {
    let entries = payload.get("entries").filter(|value| value.is_array())?;
    Some(Message::tool(
        format!("plan-{seq}"),
        PLAN_TOOL_NAME,
        serde_json::json!({ "v": 1, "source": "acp", "entries": entries }).to_string(),
    ))
}

fn json_value_text(value: Option<&serde_json::Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        value => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn upsert_acp_tool_envelope(tools: &mut Vec<AcpToolEnvelope>, payload: &serde_json::Value) {
    let Some(call_id) = payload
        .get("toolCallId")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let patch = |tool: &mut AcpToolEnvelope| {
        if let Some(value) = payload.get("title").and_then(serde_json::Value::as_str) {
            tool.title = value.to_string();
        }
        if let Some(value) = payload.get("kind").and_then(serde_json::Value::as_str) {
            tool.kind = value.to_string();
        }
        if let Some(value) = payload.get("status").and_then(serde_json::Value::as_str) {
            tool.status = value.to_string();
        }
        if payload.get("content").is_some() {
            tool.content = json_value_text(payload.get("content"));
        }
        if payload.get("locations").is_some() {
            tool.locations = json_value_text(payload.get("locations"));
        }
        if payload.get("rawInput").is_some() {
            tool.raw_input = json_value_text(payload.get("rawInput"));
        }
        if payload.get("rawOutput").is_some() {
            tool.raw_output = json_value_text(payload.get("rawOutput"));
        }
    };
    if let Some(tool) = tools.iter_mut().find(|tool| tool.call_id == call_id) {
        patch(tool);
        return;
    }
    let mut tool = AcpToolEnvelope {
        v: 1,
        call_id: call_id.to_string(),
        title: payload
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("ACP tool")
            .to_string(),
        kind: payload
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: payload
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("pending")
            .to_string(),
        content: json_value_text(payload.get("content")),
        locations: json_value_text(payload.get("locations")),
        raw_input: json_value_text(payload.get("rawInput")),
        raw_output: json_value_text(payload.get("rawOutput")),
    };
    patch(&mut tool);
    tools.push(tool);
}

fn permission_event(frame_id: &str, request: &AcpPermissionRequest) -> PermissionEvent {
    PermissionEvent {
        request_id: request.request_id.clone(),
        frame_id: frame_id.to_string(),
        tool_call: request.tool_call.clone(),
        options: request
            .options
            .iter()
            .map(|option| {
                serde_json::json!({
                    "id": option.id,
                    "name": option.name,
                    "kind": match option.kind {
                        wisp_acp::AcpPermissionKind::AllowOnce => "allow_once",
                        wisp_acp::AcpPermissionKind::AllowAlways => "allow_always",
                        wisp_acp::AcpPermissionKind::RejectOnce => "reject_once",
                        wisp_acp::AcpPermissionKind::RejectAlways => "reject_always",
                        wisp_acp::AcpPermissionKind::Unknown => "unknown",
                    },
                })
            })
            .collect(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcpTurnKind {
    User,
    Internal,
}

async fn begin_acp_turn(
    store: &wisp_store::Store,
    frame_id: &str,
    message: &str,
    kind: AcpTurnKind,
) -> Result<i64, String> {
    let seq = store
        .load_messages(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .len() as i64;
    if kind == AcpTurnKind::User {
        store
            .append_message(frame_id, seq + 1, &Message::user(message))
            .await
            .map_err(|error| error.to_string())?;
        Ok(seq + 2)
    } else {
        Ok(seq + 1)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_acp_turn(
    state: &AppState,
    app: &AppHandle,
    window_label: Option<&str>,
    project: &ActiveProject,
    frame_id: &str,
    profile_id: Option<&str>,
    message: &str,
    attachments: &[String],
    injected_context: &[String],
    artifact_references: &[PathBuf],
) -> Result<String, String> {
    run_acp_turn_with_kind(
        state,
        app,
        window_label,
        project,
        frame_id,
        profile_id,
        message,
        attachments,
        injected_context,
        artifact_references,
        None,
        AcpTurnKind::User,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_acp_participant_turn(
    state: &AppState,
    app: &AppHandle,
    window_label: Option<&str>,
    project: &ActiveProject,
    parent_frame_id: &str,
    profile_id: &str,
    message: &str,
    attachments: &[String],
    injected_context: &[String],
    artifact_references: &[PathBuf],
) -> Result<String, String> {
    let profile = profiles(&state.store)
        .await
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| "The selected ACP participant no longer exists.".to_string())?;
    let participant = match state
        .store
        .get_acp_conversation_participant(parent_frame_id, profile_id)
        .await
        .map_err(|error| error.to_string())?
    {
        Some(participant) => participant,
        None => state
            .store
            .create_acp_conversation_participant(
                parent_frame_id,
                &project.id,
                profile_id,
                &profile.label,
                &Uuid::new_v4().to_string(),
            )
            .await
            .map_err(|error| error.to_string())?,
    };

    // Launch/resume before the parent accepts the durable user row. Invalid
    // commands, changed fingerprints, and non-resumable sessions therefore do
    // not leave a ghost request in the visible shared transcript.
    runtime_for(
        state,
        project,
        &participant.child_frame_id,
        Some(profile_id),
    )
    .await?;

    let parent_messages = state
        .store
        .load_messages_with_seq(parent_frame_id)
        .await
        .map_err(|error| error.to_string())?;
    let own_response_ranges = state
        .store
        .acp_conversation_response_ranges(parent_frame_id, profile_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut participant_context = injected_context.to_vec();
    if let Some(context) = shared_conversation_delta(
        &parent_messages,
        participant.synced_parent_seq,
        &own_response_ranges,
        ACP_SHARED_CONTEXT_MAX_CHARS,
    ) {
        participant_context.push(context);
    }
    let child_before = state
        .store
        .load_messages_with_seq(&participant.child_frame_id)
        .await
        .map_err(|error| error.to_string())?
        .last()
        .map_or(0, |(seq, _)| *seq);
    let stop_reason = run_acp_turn_with_kind(
        state,
        app,
        window_label,
        project,
        &participant.child_frame_id,
        Some(profile_id),
        message,
        attachments,
        &participant_context,
        artifact_references,
        Some(parent_frame_id),
        AcpTurnKind::User,
    )
    .await?;

    let user_message_seq = state
        .store
        .load_messages_with_seq(parent_frame_id)
        .await
        .map_err(|error| error.to_string())?
        .last()
        .map(|(seq, _)| *seq)
        .ok_or_else(|| "ACP participant user message was not persisted.".to_string())?;
    let child_responses = state
        .store
        .load_messages_with_seq(&participant.child_frame_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|(seq, message)| *seq > child_before && message.role != wisp_llm::Role::User)
        .collect::<Vec<_>>();
    let child_response_start = child_responses
        .first()
        .map(|(seq, _)| *seq)
        .ok_or_else(|| "ACP participant produced no durable response.".to_string())?;
    let child_response_end = child_responses
        .last()
        .map(|(seq, _)| *seq)
        .unwrap_or(child_response_start);
    let binding = state
        .store
        .get_acp_session(&participant.child_frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "ACP participant session binding disappeared.".to_string())?;
    let responses = child_responses
        .into_iter()
        .map(|(_, message)| message)
        .collect::<Vec<_>>();
    let mirrored = state
        .store
        .record_acp_conversation_turn(
            parent_frame_id,
            &participant.child_frame_id,
            profile_id,
            &profile.label,
            &binding.profile_fingerprint,
            &binding.agent_session_id,
            user_message_seq,
            child_response_start,
            child_response_end,
            &responses,
        )
        .await
        .map_err(|error| error.to_string())?;
    for (seq, response) in mirrored {
        if response.role != wisp_llm::Role::Assistant {
            continue;
        }
        let resources = crate::resource_refs::bind_new_message_resources(
            &state.store,
            &project.root,
            &project.id,
            parent_frame_id,
            seq,
            &response.content.as_text(),
        )
        .await;
        if !resources.is_empty() {
            crate::emit_agent_event(
                app,
                AgentEvent::Resources {
                    frame_id: parent_frame_id.to_string(),
                    seq,
                    resources: resources.iter().map(Into::into).collect(),
                },
            );
        }
    }
    Ok(stop_reason)
}

pub(crate) async fn run_acp_internal_turn(
    state: &AppState,
    app: &AppHandle,
    project: &ActiveProject,
    frame_id: &str,
    message: &str,
) -> Result<String, String> {
    run_acp_turn_with_kind(
        state,
        app,
        None,
        project,
        frame_id,
        None,
        message,
        &[],
        &[],
        &[],
        None,
        AcpTurnKind::Internal,
    )
    .await
}

/// Render only discussion rows a participant has not already received. Its own
/// mirrored replies are excluded because the external ACP session already owns
/// that context; other participants' replies remain visible.
fn shared_conversation_delta(
    messages: &[(i64, Message)],
    after_seq: i64,
    own_response_ranges: &[(i64, i64)],
    max_chars: usize,
) -> Option<String> {
    let in_own_response = |seq: i64| {
        own_response_ranges
            .iter()
            .any(|(start, end)| seq >= *start && seq <= *end)
    };
    let mut rows = Vec::new();
    for (seq, message) in messages {
        if *seq <= after_seq || in_own_response(*seq) {
            continue;
        }
        let speaker = match message.role {
            wisp_llm::Role::User => "User".to_string(),
            wisp_llm::Role::Assistant => message
                .model_name
                .clone()
                .unwrap_or_else(|| "Assistant".to_string()),
            wisp_llm::Role::System | wisp_llm::Role::Tool => continue,
        };
        let text = message.content.as_text();
        let text = text.trim();
        if !text.is_empty() {
            rows.push(format!("[msg:{seq}] {speaker}: {text}"));
        }
    }
    if rows.is_empty() {
        return None;
    }
    let body = rows.join("\n\n");
    let char_count = body.chars().count();
    let body = if char_count > max_chars {
        let tail = body
            .chars()
            .skip(char_count.saturating_sub(max_chars))
            .collect::<String>();
        format!("[Earlier shared updates omitted to fit the context window]\n{tail}")
    } else {
        body
    };
    Some(format!(
        "Shared Wisp discussion updates since your previous turn follow. \
Treat them as context from other participants; answer only the new user request.\n\n{body}"
    ))
}

/// `emit_to` the turn's own window when it has one; internal turns (review
/// correction, channels) have no window and fall back to a broadcast.
fn emit_ask_event(
    app: &AppHandle,
    window_label: Option<&str>,
    event: &str,
    payload: serde_json::Value,
) {
    match window_label {
        Some(label) => {
            let _ = app.emit_to(label, event, payload);
        }
        None => {
            let _ = app.emit(event, payload);
        }
    }
}

/// Surface new pending bridge `ask_user` rows to the UI. `acp_asks` doubles as
/// the seen-set: a row already registered was already emitted, and an answered
/// row leaves the pending query before it leaves the map.
async fn surface_pending_asks(
    state: &AppState,
    app: &AppHandle,
    window_label: Option<&str>,
    storage_frame_id: &str,
    event_frame_id: &str,
) {
    let pending = state
        .store
        .pending_ask_user_requests(storage_frame_id)
        .await
        .unwrap_or_default();
    for (request_id, payload_json) in pending {
        let mut asks = state.acp_asks.lock().await;
        if asks.contains_key(&request_id) {
            continue;
        }
        asks.insert(request_id.clone(), storage_frame_id.to_string());
        drop(asks);
        state
            .awaiting_confirm
            .lock()
            .unwrap()
            .insert(event_frame_id.to_string());
        state.device_hub.mark_needs_user(event_frame_id, None);
        let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap_or_default();
        emit_ask_event(
            app,
            window_label,
            "ask-user-request",
            serde_json::json!({
                "frameId": event_frame_id,
                "requestId": request_id,
                "payload": payload,
            }),
        );
    }
}

/// Turn-end sweep: a pending ask that outlived its turn can never be answered
/// (the bridge poll dies with the agent), so expire it and settle the card.
async fn settle_expired_asks(
    state: &AppState,
    app: &AppHandle,
    window_label: Option<&str>,
    storage_frame_id: &str,
    event_frame_id: &str,
) {
    let expired = state
        .store
        .expire_ask_user_requests_except(storage_frame_id, &HashSet::new())
        .await
        .unwrap_or_default();
    state
        .acp_asks
        .lock()
        .await
        .retain(|_, owner| owner != storage_frame_id);
    if !state
        .acp_permissions
        .lock()
        .await
        .values()
        .any(|owner| owner == storage_frame_id)
    {
        state
            .awaiting_confirm
            .lock()
            .unwrap()
            .remove(event_frame_id);
        state.device_hub.resolve_needs_user(event_frame_id);
    }
    for (request_id, _) in expired {
        emit_ask_event(
            app,
            window_label,
            "ask-user-resolved",
            serde_json::json!({
                "frameId": event_frame_id,
                "requestId": request_id,
                "expired": true,
            }),
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_acp_turn_with_kind(
    state: &AppState,
    app: &AppHandle,
    window_label: Option<&str>,
    project: &ActiveProject,
    frame_id: &str,
    profile_id: Option<&str>,
    message: &str,
    attachments: &[String],
    injected_context: &[String],
    artifact_references: &[PathBuf],
    event_frame_id: Option<&str>,
    turn_kind: AcpTurnKind,
) -> Result<String, String> {
    let event_frame_id = event_frame_id.unwrap_or(frame_id);
    let result = run_acp_turn_inner(
        state,
        app,
        window_label,
        project,
        frame_id,
        profile_id,
        message,
        attachments,
        injected_context,
        artifact_references,
        Some(event_frame_id),
        turn_kind,
    )
    .await;
    // Runs on every exit path, success or error — asks must never outlive
    // their turn as live cards.
    settle_expired_asks(state, app, window_label, frame_id, event_frame_id).await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_acp_turn_inner(
    state: &AppState,
    app: &AppHandle,
    window_label: Option<&str>,
    project: &ActiveProject,
    frame_id: &str,
    profile_id: Option<&str>,
    message: &str,
    attachments: &[String],
    injected_context: &[String],
    artifact_references: &[PathBuf],
    event_frame_id: Option<&str>,
    turn_kind: AcpTurnKind,
) -> Result<String, String> {
    let event_frame_id = event_frame_id.unwrap_or(frame_id);
    let runtime = runtime_for(state, project, frame_id, profile_id).await?;
    if let Some(session_state) = runtime.session_state.lock().await.take() {
        let _ = app.emit(
            "acp-session-state",
            serde_json::json!({
                "frameId": event_frame_id,
                "modes": session_state.modes,
                "configOptions": session_state.config_options,
            }),
        );
    }
    if let Some(requested) = profile_id {
        if requested != runtime.profile_id {
            return Err("The ACP Agent selection is locked after the first prompt.".into());
        }
    }
    let mut content = acp_text_content(message, injected_context);
    let mut linked_paths = HashSet::new();
    for attachment in attachments {
        let path = wisp_tools::safety::validate_file_path(project.root.as_path(), attachment)
            .map_err(|_| format!("Attachment '{attachment}' is outside the active project."))?;
        if linked_paths.insert(path.clone()) {
            content.push(acp_resource_link(&path)?);
        }
    }
    for path in artifact_references {
        if linked_paths.insert(path.clone()) {
            content.push(acp_resource_link(path)?);
        }
    }
    let mut next_seq = begin_acp_turn(&state.store, frame_id, message, turn_kind).await?;
    if turn_kind == AcpTurnKind::User {
        if event_frame_id != frame_id {
            let parent_seq = state
                .store
                .load_messages_with_seq(event_frame_id)
                .await
                .map_err(|error| error.to_string())?
                .last()
                .map_or(1, |(seq, _)| seq.saturating_add(1));
            state
                .store
                .append_message(event_frame_id, parent_seq, &Message::user(message))
                .await
                .map_err(|error| error.to_string())?;
        }
        crate::emit_agent_event(
            app,
            AgentEvent::User {
                frame_id: event_frame_id.to_string(),
                text: message.to_string(),
            },
        );
    }
    let prompt = runtime.handle.prompt(runtime.session_id.clone(), content);
    tokio::pin!(prompt);
    let mut assistant = String::new();
    let mut reasoning = String::new();
    let mut tools: Vec<AcpToolEnvelope> = Vec::new();
    // Plans are revised in place during a turn; only the last one is persisted.
    let mut plan: Option<serde_json::Value> = None;
    // The bridge's ask_user runs in a separate process whose only channel is
    // the store, so pendings are discovered by polling while the turn runs.
    let mut ask_tick = tokio::time::interval(Duration::from_millis(500));
    let outcome = loop {
        tokio::select! {
            result = &mut prompt => break result.map_err(|error| error.to_string())?,
            _ = ask_tick.tick() => {
                surface_pending_asks(state, app, window_label, frame_id, event_frame_id).await;
            }
            event = runtime.handle.next_event() => match event {
                Some(AcpSessionEvent::Update { kind, payload, .. }) => {
                    if matches!(kind, AcpUpdateKind::AgentMessage | AcpUpdateKind::AgentThought) {
                        if let Some(text) = text_from_payload(&payload) {
                            let target = if kind == AcpUpdateKind::AgentMessage { &mut assistant } else { &mut reasoning };
                            target.push_str(text);
                            let event = if kind == AcpUpdateKind::AgentMessage {
                                AgentEvent::Text { frame_id: event_frame_id.to_string(), delta: text.to_string() }
                            } else {
                                AgentEvent::Reasoning { frame_id: event_frame_id.to_string(), delta: text.to_string() }
                            };
                            crate::emit_agent_event(app, event);
                        }
                    } else {
                        if matches!(kind, AcpUpdateKind::ToolCall | AcpUpdateKind::ToolCallUpdate) {
                            upsert_acp_tool_envelope(&mut tools, &payload);
                        }
                        if kind == AcpUpdateKind::Plan {
                            plan = Some(payload.clone());
                        }
                        let _ = app.emit("acp-session-update", serde_json::json!({
                            "frameId": event_frame_id,
                            "kind": format!("{kind:?}"),
                            "payload": payload,
                        }));
                    }
                }
                Some(AcpSessionEvent::Permission(request)) => {
                    state.acp_permissions.lock().await.insert(request.request_id.clone(), frame_id.to_string());
                    state.awaiting_confirm.lock().unwrap().insert(event_frame_id.to_string());
                    state.device_hub.mark_needs_user(event_frame_id, Some(&project.id));
                    let _ = app.emit("permission-request", permission_event(event_frame_id, &request));
                }
                Some(AcpSessionEvent::Exited { error }) => return Err(error.unwrap_or_else(|| "ACP Agent exited.".into())),
                None => return Err("ACP Agent event stream closed.".into()),
            }
        }
    };
    // ACP permits final notifications to race with the prompt response. Drain
    // the already-buffered tail before persisting and emitting Done.
    let drain_deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while tokio::time::Instant::now() < drain_deadline {
        let event = match tokio::time::timeout(
            Duration::from_millis(75),
            runtime.handle.next_event(),
        )
        .await
        {
            Ok(Some(event)) => event,
            Ok(None) | Err(_) => break,
        };
        match event {
            AcpSessionEvent::Update { kind, payload, .. } => {
                if let Some(text) = text_from_payload(&payload) {
                    if kind == AcpUpdateKind::AgentMessage {
                        assistant.push_str(text);
                        crate::emit_agent_event(
                            app,
                            AgentEvent::Text {
                                frame_id: event_frame_id.to_string(),
                                delta: text.to_string(),
                            },
                        );
                    } else if kind == AcpUpdateKind::AgentThought {
                        reasoning.push_str(text);
                        crate::emit_agent_event(
                            app,
                            AgentEvent::Reasoning {
                                frame_id: event_frame_id.to_string(),
                                delta: text.to_string(),
                            },
                        );
                    }
                }
                if !matches!(
                    kind,
                    AcpUpdateKind::AgentMessage | AcpUpdateKind::AgentThought
                ) {
                    if matches!(
                        kind,
                        AcpUpdateKind::ToolCall | AcpUpdateKind::ToolCallUpdate
                    ) {
                        upsert_acp_tool_envelope(&mut tools, &payload);
                    }
                    if kind == AcpUpdateKind::Plan {
                        plan = Some(payload.clone());
                    }
                    let _ = app.emit(
                        "acp-session-update",
                        serde_json::json!({
                            "frameId": event_frame_id,
                            "kind": format!("{kind:?}"),
                            "payload": payload,
                        }),
                    );
                }
            }
            AcpSessionEvent::Permission(request) => {
                let _ = runtime.handle.respond_permission(request.request_id, None);
            }
            AcpSessionEvent::Exited { error: Some(error) } => return Err(error),
            AcpSessionEvent::Exited { error: None } => break,
        }
    }
    for tool in &tools {
        state
            .store
            .append_message(frame_id, next_seq, &tool.to_message())
            .await
            .map_err(|error| error.to_string())?;
        next_seq += 1;
    }
    if let Some(message) = plan
        .as_ref()
        .and_then(|payload| plan_message(next_seq, payload))
    {
        state
            .store
            .append_message(frame_id, next_seq, &message)
            .await
            .map_err(|error| error.to_string())?;
        next_seq += 1;
    }
    let mut persisted = Message::assistant(assistant);
    persisted.reasoning = (!reasoning.is_empty()).then_some(reasoning);
    persisted.model_name = profiles(&state.store)
        .await
        .into_iter()
        .find(|profile| profile.id == runtime.profile_id)
        .map(|profile| profile.label);
    state
        .store
        .append_message(frame_id, next_seq, &persisted)
        .await
        .map_err(|error| error.to_string())?;
    let resources = crate::resource_refs::bind_new_message_resources(
        &state.store,
        &project.root,
        &project.id,
        frame_id,
        next_seq,
        &persisted.content.as_text(),
    )
    .await;
    // A routed participant turn is persisted in its hidden child first; the
    // caller rebinds and emits resources after it knows the mirrored parent seq.
    if event_frame_id == frame_id && !resources.is_empty() {
        crate::emit_agent_event(
            app,
            AgentEvent::Resources {
                frame_id: event_frame_id.to_string(),
                seq: next_seq,
                resources: resources.iter().map(Into::into).collect(),
            },
        );
    }
    cancel_pending_permissions(state, frame_id, &runtime).await;
    Ok(stop_reason(outcome.stop_reason).into())
}

/// ACP has no Wisp-reference block type. Render trusted, host-resolved Wisp
/// context as ordinary text blocks, which every ACP v1 Agent accepts.
fn acp_text_content(message: &str, injected_context: &[String]) -> Vec<ContentBlock> {
    let mut content = vec![ContentBlock::Text(TextContent::new(message.to_string()))];
    content.extend(
        injected_context
            .iter()
            .map(|text| ContentBlock::Text(TextContent::new(text.clone()))),
    );
    content
}

fn acp_resource_link(path: &Path) -> Result<ContentBlock, String> {
    let uri = url::Url::from_file_path(path).map_err(|_| {
        format!(
            "Attachment path '{}' cannot be represented as a file URI.",
            path.display()
        )
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    Ok(ContentBlock::ResourceLink(ResourceLink::new(
        name,
        uri.to_string(),
    )))
}

fn stop_reason(reason: AcpStopReason) -> &'static str {
    match reason {
        AcpStopReason::EndTurn => "end_turn",
        AcpStopReason::MaxTokens => "max_tokens",
        AcpStopReason::MaxTurnRequests => "max_turn_requests",
        AcpStopReason::Refusal => "refusal",
        AcpStopReason::Cancelled => "cancelled",
        AcpStopReason::Unknown => "unknown",
    }
}

async fn visible_frame_id(state: &AppState, storage_frame_id: &str) -> String {
    state
        .store
        .acp_conversation_parent_for_child(storage_frame_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| storage_frame_id.to_string())
}

#[tauri::command]
pub(crate) async fn respond_acp_permission(
    state: State<'_, AppState>,
    app: AppHandle,
    request_id: String,
    option_id: Option<String>,
) -> Result<(), String> {
    let frame_id = state
        .acp_permissions
        .lock()
        .await
        .get(&request_id)
        .cloned()
        .ok_or_else(|| "ACP permission request is no longer pending.".to_string())?;
    let visible_frame_id = visible_frame_id(&state, &frame_id).await;
    let runtime = state
        .acp_sessions
        .lock()
        .await
        .get(&frame_id)
        .cloned()
        .ok_or_else(|| "ACP session is no longer active.".to_string())?;
    runtime
        .handle
        .respond_permission(request_id.clone(), option_id)
        .map_err(|error| error.to_string())?;
    state.acp_permissions.lock().await.remove(&request_id);
    let frame_has_permissions = state
        .acp_permissions
        .lock()
        .await
        .values()
        .any(|owner| owner == &frame_id);
    let frame_has_asks = state
        .acp_asks
        .lock()
        .await
        .values()
        .any(|owner| owner == &frame_id);
    if !frame_has_permissions && !frame_has_asks {
        state
            .awaiting_confirm
            .lock()
            .unwrap()
            .remove(&visible_frame_id);
        state.device_hub.resolve_needs_user(&visible_frame_id);
    }
    let _ = app.emit(
        "permission-resolved",
        serde_json::json!({
            "frameId": visible_frame_id,
            "requestId": request_id,
        }),
    );
    Ok(())
}

/// Resolve a pending bridge `ask_user` request: write the answer for the
/// bridge's poll loop to consume and settle the card. The permission flow's
/// shape, with the store standing in for the oneshot — the bridge lives in
/// another process.
#[tauri::command]
pub(crate) async fn respond_ask_user(
    state: State<'_, AppState>,
    app: AppHandle,
    window: tauri::WebviewWindow,
    request_id: String,
    answer: String,
) -> Result<(), String> {
    let answer = answer.trim().to_string();
    if answer.is_empty() {
        return Err("The answer is empty.".into());
    }
    let frame_id = state
        .acp_asks
        .lock()
        .await
        .get(&request_id)
        .cloned()
        .ok_or_else(|| "This question is no longer pending.".to_string())?;
    let visible_frame_id = visible_frame_id(&state, &frame_id).await;
    if !state
        .store
        .answer_ask_user_request(&request_id, &answer)
        .await
        .map_err(|error| error.to_string())?
    {
        state.acp_asks.lock().await.remove(&request_id);
        return Err("This question is no longer pending.".into());
    }
    state.acp_asks.lock().await.remove(&request_id);
    let frame_has_asks = state
        .acp_asks
        .lock()
        .await
        .values()
        .any(|owner| owner == &frame_id);
    let frame_has_permissions = state
        .acp_permissions
        .lock()
        .await
        .values()
        .any(|owner| owner == &frame_id);
    if !frame_has_asks && !frame_has_permissions {
        state
            .awaiting_confirm
            .lock()
            .unwrap()
            .remove(&visible_frame_id);
        state.device_hub.resolve_needs_user(&visible_frame_id);
    }
    let _ = app.emit_to(
        window.label(),
        "ask-user-resolved",
        serde_json::json!({
            "frameId": visible_frame_id,
            "requestId": request_id,
            "expired": false,
        }),
    );
    Ok(())
}

#[tauri::command]
pub(crate) async fn set_acp_session_config(
    state: State<'_, AppState>,
    app: AppHandle,
    window: tauri::WebviewWindow,
    frame_id: String,
    config_id: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let project = state.active(window.label());
    if state
        .store
        .frame_project_id(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
        != Some(project.id.as_str())
    {
        return Err("Session does not belong to the active project.".into());
    }
    let runtime = state
        .acp_sessions
        .lock()
        .await
        .get(&frame_id)
        .cloned()
        .ok_or_else(|| "ACP session is not active.".to_string())?;
    let value = serde_json::from_value(value).map_err(|error| error.to_string())?;
    let options = runtime
        .handle
        .set_config(runtime.session_id.clone(), config_id, value)
        .await
        .map_err(|error| error.to_string())?;
    let value = serde_json::to_value(&options).map_err(|error| error.to_string())?;
    let _ = app.emit(
        "acp-session-state",
        serde_json::json!({
            "frameId": frame_id,
            "configOptions": value,
        }),
    );
    Ok(value)
}

/// Set the ACP session mode (e.g. Codex approval mode: read-only / agent /
/// full-access). `session/set_mode` returns no state, so the caller applies the
/// selected `mode_id` optimistically; the agent will confirm with a
/// `CurrentModeUpdate` notification during the next turn if it disagrees.
#[tauri::command]
pub(crate) async fn set_acp_session_mode(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    frame_id: String,
    mode_id: String,
) -> Result<String, String> {
    let project = state.active(window.label());
    if state
        .store
        .frame_project_id(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
        != Some(project.id.as_str())
    {
        return Err("Session does not belong to the active project.".into());
    }
    // Not `acp_sessions` directly: the mode controls are visible from the cached
    // `availableModes` before the session's first turn, so switching mode has to
    // launch and resume the agent the same way sending a message would.
    let runtime = runtime_for(&state, &project, &frame_id, None).await?;
    runtime
        .handle
        .set_mode(runtime.session_id.clone(), mode_id.clone())
        .await
        .map_err(|error| error.to_string())?;
    Ok(mode_id)
}

pub(crate) async fn cancel_frame(state: &AppState, frame_id: &str) {
    let visible_frame_id = visible_frame_id(state, frame_id).await;
    if let Some(runtime) = state.acp_sessions.lock().await.remove(frame_id) {
        let _ = runtime.handle.cancel(runtime.session_id.clone());
        cancel_pending_permissions(state, frame_id, &runtime).await;
        let handle = runtime.handle.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            handle.shutdown(Duration::from_secs(1)).await;
        });
    }
    state
        .acp_permissions
        .lock()
        .await
        .retain(|_, owner| owner != frame_id);
    state
        .acp_asks
        .lock()
        .await
        .retain(|_, owner| owner != frame_id);
    state
        .awaiting_confirm
        .lock()
        .unwrap()
        .remove(&visible_frame_id);
    state.device_hub.resolve_needs_user(&visible_frame_id);
}

pub(crate) async fn close_frame(state: &AppState, frame_id: &str) {
    let visible_frame_id = visible_frame_id(state, frame_id).await;
    if let Some(runtime) = state.acp_sessions.lock().await.remove(frame_id) {
        let _ = runtime
            .handle
            .close_session(runtime.session_id.clone())
            .await;
        if let Ok(runtime) = Arc::try_unwrap(runtime) {
            if let Ok(handle) = Arc::try_unwrap(runtime.handle) {
                handle.shutdown(Duration::from_secs(2)).await;
            }
        }
    }
    state
        .acp_permissions
        .lock()
        .await
        .retain(|_, owner| owner != frame_id);
    state
        .acp_asks
        .lock()
        .await
        .retain(|_, owner| owner != frame_id);
    state
        .awaiting_confirm
        .lock()
        .unwrap()
        .remove(&visible_frame_id);
    state.device_hub.resolve_needs_user(&visible_frame_id);
}

async fn cancel_pending_permissions(state: &AppState, frame_id: &str, runtime: &AcpRuntime) {
    let visible_frame_id = visible_frame_id(state, frame_id).await;
    let request_ids = {
        let mut pending = state.acp_permissions.lock().await;
        let request_ids = pending
            .iter()
            .filter(|(_, owner)| owner.as_str() == frame_id)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        pending.retain(|_, owner| owner != frame_id);
        request_ids
    };
    for request_id in request_ids {
        let _ = runtime.handle.respond_permission(request_id, None);
    }
    state
        .awaiting_confirm
        .lock()
        .unwrap()
        .remove(&visible_frame_id);
    state.device_hub.resolve_needs_user(&visible_frame_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_validation_preserves_argument_boundaries() {
        let profile = AcpAgentProfile {
            id: "agent".into(),
            label: "Agent".into(),
            command: "agent binary".into(),
            args: vec!["--flag=value with spaces".into()],
        };
        validate(&profile).unwrap();
        assert_eq!(launch_profile(&profile).args, profile.args);
    }

    #[test]
    fn profile_fingerprint_locks_command_and_argument_vector() {
        let base = AcpAgentProfile {
            id: "agent".into(),
            label: "Agent".into(),
            command: "agent".into(),
            args: vec!["one argument".into(), "two".into()],
        };
        let mut changed = base.clone();
        changed.args = vec!["one".into(), "argument".into(), "two".into()];
        assert_ne!(fingerprint(&base), fingerprint(&changed));
        changed = base.clone();
        changed.command = "other-agent".into();
        assert_ne!(fingerprint(&base), fingerprint(&changed));
    }

    #[test]
    fn acp_wire_dtos_serialize_as_camel_case() {
        let info = serde_json::to_value(AcpAgentInfoDto {
            protocol_version: 1,
            implementation: None,
            capabilities: serde_json::json!({}),
            auth_methods: vec![],
        })
        .unwrap();
        assert!(info.get("protocolVersion").is_some());
        assert!(info.get("authMethods").is_some());
        let event = serde_json::to_value(permission_event(
            "frame-1",
            &AcpPermissionRequest {
                request_id: "permission-1".into(),
                session_id: "session-1".into(),
                tool_call: serde_json::json!({}),
                options: vec![],
            },
        ))
        .unwrap();
        assert!(event.get("requestId").is_some());
        assert!(event.get("frameId").is_some());
        assert!(event.get("toolCall").is_some());
    }

    #[test]
    fn update_text_mapping_is_tolerant() {
        assert_eq!(
            text_from_payload(&serde_json::json!({"content":{"text":"a"}})),
            Some("a")
        );
        assert_eq!(text_from_payload(&serde_json::json!({"future":true})), None);
    }

    #[test]
    fn explicit_wisp_context_becomes_standard_acp_text() {
        let content = acp_text_content(
            "analyse this",
            &["The user explicitly selected these skills:\n# Skill: bear-map".into()],
        );
        let json = serde_json::to_value(content).unwrap().to_string();
        assert!(json.contains("analyse this"));
        assert!(json.contains("bear-map"));
    }

    #[test]
    fn shared_context_sends_only_unseen_other_participant_rows() {
        let mut codex_reply = Message::assistant("Use a two-column hierarchy.");
        codex_reply.model_name = Some("Codex".into());
        let mut claude_reply = Message::assistant("Check the mobile collapse order.");
        claude_reply.model_name = Some("Claude".into());
        let messages = vec![
            (1, Message::user("Design a homepage")),
            (2, codex_reply),
            (3, Message::user("Claude, review it")),
            (4, claude_reply),
        ];

        let first = shared_conversation_delta(&messages, 0, &[], 24_000).unwrap();
        assert!(first.contains("[msg:1] User: Design a homepage"));
        assert!(first.contains("[msg:2] Codex: Use a two-column hierarchy."));
        assert!(first.contains("[msg:4] Claude: Check the mobile collapse order."));

        let codex_next = shared_conversation_delta(&messages, 1, &[(2, 2)], 24_000).unwrap();
        assert!(!codex_next.contains("two-column hierarchy"));
        assert!(codex_next.contains("[msg:3] User: Claude, review it"));
        assert!(codex_next.contains("[msg:4] Claude: Check the mobile collapse order."));

        assert!(shared_conversation_delta(&messages, 4, &[(2, 2)], 24_000).is_none());
    }

    #[test]
    fn shared_context_tail_is_bounded() {
        let messages = vec![
            (1, Message::user("first message")),
            (2, Message::assistant("second message")),
        ];
        let delta = shared_conversation_delta(&messages, 0, &[], 12).unwrap();
        assert!(delta.contains("Earlier shared updates omitted"));
        assert!(delta.ends_with("cond message"));
    }

    #[tokio::test]
    async fn read_only_wait_is_bounded_and_cancellable() {
        let timed_out = await_read_only_result(
            std::future::pending::<Result<String, String>>(),
            Duration::from_millis(5),
            None,
        )
        .await
        .unwrap_err();
        assert!(timed_out.contains("timed out"));

        let cancel = AtomicBool::new(true);
        let cancelled = await_read_only_result(
            std::future::pending::<Result<String, String>>(),
            Duration::from_secs(1),
            Some(&cancel),
        )
        .await
        .unwrap_err();
        assert!(cancelled.contains("cancelled"));

        let completed = await_read_only_result(
            async { Ok("review complete".to_string()) },
            Duration::from_secs(1),
            None,
        )
        .await
        .unwrap();
        assert_eq!(completed, "review complete");
    }

    #[tokio::test]
    async fn internal_turn_does_not_persist_a_user_authored_message() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_acp_internal_turn_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store
            .create_frame("f", "p", "Agent", "model")
            .await
            .unwrap();

        let next_seq = begin_acp_turn(&store, "f", "user request", AcpTurnKind::User)
            .await
            .unwrap();
        assert_eq!(next_seq, 2);
        let internal_seq = begin_acp_turn(
            &store,
            "f",
            "generated reviewer correction",
            AcpTurnKind::Internal,
        )
        .await
        .unwrap();
        assert_eq!(internal_seq, 2);
        store
            .append_message("f", internal_seq, &Message::assistant("corrected answer"))
            .await
            .unwrap();

        let messages = store.load_messages("f").await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content.as_text(), "user request");
        assert_eq!(messages[1].content.as_text(), "corrected answer");
        assert!(messages
            .iter()
            .all(|message| message.content.as_text() != "generated reviewer correction"));
        drop(store);
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn acp_tool_envelope_round_trips_through_tool_message() {
        let mut tools = Vec::new();
        upsert_acp_tool_envelope(
            &mut tools,
            &serde_json::json!({
                "toolCallId": "call-1",
                "title": "Get-ChildItem -Force",
                "kind": "execute",
                "status": "in_progress",
            }),
        );
        upsert_acp_tool_envelope(
            &mut tools,
            &serde_json::json!({
                "toolCallId": "call-1",
                "status": "completed",
                "content": [{"type":"terminal","terminalId":"t1"}],
                "rawInput": {"cmd":"pwd"},
                "rawOutput": {"stdout":"/workspace","exitCode":0},
            }),
        );
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].status, "completed");
        assert!(tools[0].content.contains("terminalId"));
        assert!(tools[0].raw_input.contains("pwd"));
        assert!(tools[0].raw_output.contains("/workspace"));
        let message = tools[0].to_message();
        let restored = AcpToolEnvelope::from_tool_message(
            message.tool_name.as_deref(),
            &message.content.as_text(),
        )
        .unwrap();
        assert_eq!(restored, tools[0]);
    }

    #[test]
    fn plan_message_keeps_entries_structured() {
        let payload = serde_json::json!({
            "entries": [{ "content": "read", "status": "in_progress", "priority": "high" }]
        });
        let message = plan_message(7, &payload).unwrap();
        assert_eq!(message.tool_name.as_deref(), Some(PLAN_TOOL_NAME));
        // Not an `acp:` tool row, so it never re-enters the ACP tool transcript.
        assert!(AcpToolEnvelope::from_tool_message(
            message.tool_name.as_deref(),
            &message.content.as_text()
        )
        .is_none());
        let body: serde_json::Value =
            serde_json::from_str(&message.content.as_text()).expect("plan body is JSON");
        assert_eq!(body["source"], "acp");
        assert_eq!(body["entries"][0]["status"], "in_progress");
        assert_eq!(body["entries"][0]["priority"], "high");
        // A plan update without entries is not worth a transcript row.
        assert!(plan_message(7, &serde_json::json!({})).is_none());
    }
}
