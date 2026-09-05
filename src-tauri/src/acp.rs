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
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use uuid::Uuid;
use wisp_acp::{
    acp::schema::v1::{
        ContentBlock, McpServer, McpServerStdio, ResourceLink, SessionId, TextContent,
    },
    AcpAgentProfile as LaunchProfile, AcpAuthMethod, AcpAuthMethodKind, AcpPermissionKind,
    AcpPermissionRequest, AcpSessionEvent, AcpSessionHandle, AcpStopReason, AcpUpdateKind,
};
use wisp_llm::Message;

const PROFILES_KEY: &str = "acp_agent_profiles";
const ACP_READ_ONLY_TIMEOUT: Duration = Duration::from_secs(90);
const ACP_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);

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

#[derive(Clone, Debug)]
pub(crate) struct PendingAcpPermission {
    pub(crate) frame_id: String,
    request: AcpPermissionRequest,
    pub(crate) remote_request: crate::ConfirmRequest,
}

impl PendingAcpPermission {
    fn new(frame_id: &str, request: &AcpPermissionRequest) -> Self {
        let title = request
            .tool_call
            .get("title")
            .and_then(serde_json::Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("ACP tool")
            .to_string();
        let preview = json_value_text(request.tool_call.get("rawInput"));
        Self {
            frame_id: frame_id.to_string(),
            request: request.clone(),
            remote_request: crate::ConfirmRequest::new(
                frame_id,
                format!("ACP permission request: {title}"),
                title,
                preview,
            ),
        }
    }
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
                    "type": auth_method_kind(&method.kind),
                })
            })
            .collect(),
    }
}

fn auth_method_kind(kind: &AcpAuthMethodKind) -> &'static str {
    match kind {
        AcpAuthMethodKind::Agent => "agent",
        AcpAuthMethodKind::Terminal { .. } => "terminal",
        AcpAuthMethodKind::Environment => "env_var",
    }
}

fn terminal_auth_launch_spec(
    profile: &AcpAgentProfile,
    method: &AcpAuthMethod,
    cwd: &Path,
) -> Result<crate::terminal_sessions::TerminalLaunchSpec, String> {
    let AcpAuthMethodKind::Terminal { args, env } = &method.kind else {
        return Err("The selected ACP authentication method is not terminal-based.".into());
    };
    let mut launch_args = profile.args.clone();
    launch_args.extend(args.iter().cloned());
    Ok(crate::terminal_sessions::TerminalLaunchSpec {
        program: profile.command.clone(),
        args: launch_args,
        cwd: Some(cwd.to_path_buf()),
        display_cwd: cwd.to_string_lossy().into_owned(),
        envs: env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    })
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
    let project = state.require_active(window.label())?;
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
) -> Result<Option<wisp_dto::AcpSessionState>, String> {
    let project = state.require_active(window.label())?;
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
    let modes = state
        .store
        .get_setting(&available_modes_key(&binding.profile_fingerprint))
        .await
        .map_err(|error| error.to_string())?
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .map(|modes| serde_json::json!({ "availableModes": modes }));
    let options =
        cached_config_options(&state.store, &frame_id, &binding.profile_fingerprint).await?;
    Ok(Some(wisp_dto::AcpSessionState {
        frame_id,
        modes,
        config_options: options.and_then(|value| value.as_array().cloned()),
    }))
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
    terminals: State<'_, crate::terminal_sessions::TerminalManager>,
    window: tauri::WebviewWindow,
    id: String,
    method_id: String,
) -> Result<Option<crate::terminal_sessions::TerminalSessionSummary>, String> {
    let profile = profiles(&state.store)
        .await
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| "Unknown ACP Agent.".to_string())?;
    let handle = AcpSessionHandle::launch(launch_profile(&profile))
        .await
        .map_err(|error| error.to_string())?;
    let method = handle
        .info()
        .auth_methods
        .iter()
        .find(|method| method.id == method_id)
        .cloned();
    let Some(method) = method else {
        handle.shutdown(Duration::from_secs(2)).await;
        return Err("The ACP Agent no longer advertises this authentication method.".into());
    };
    match &method.kind {
        AcpAuthMethodKind::Agent => {
            let result = handle
                .authenticate(method_id)
                .await
                .map_err(|error| error.to_string());
            handle.shutdown(Duration::from_secs(2)).await;
            result.map(|_| None)
        }
        AcpAuthMethodKind::Terminal { .. } => {
            let (project, scope) = crate::exploration_commands::working_project_for_active_frame(
                &state,
                window.label(),
            )
            .await?;
            let spec = terminal_auth_launch_spec(&profile, &method, &project.root)?;
            handle.shutdown(Duration::from_secs(2)).await;
            terminals
                .open_spec(
                    &project.id,
                    scope.scope_key(),
                    &format!("acp-auth:{}", profile.id),
                    format!("{} — {}", profile.label, method.name),
                    "acp-auth",
                    spec,
                )
                .map(Some)
        }
        AcpAuthMethodKind::Environment => {
            handle.shutdown(Duration::from_secs(2)).await;
            Err("Environment-variable ACP authentication is not supported yet.".into())
        }
    }
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

fn config_options_key(frame_id: &str, fingerprint: &str) -> String {
    format!("acp.config_options.{frame_id}.{fingerprint}")
}

async fn cached_config_options(
    store: &wisp_store::Store,
    frame_id: &str,
    fingerprint: &str,
) -> Result<Option<serde_json::Value>, String> {
    Ok(store
        .get_setting(&config_options_key(frame_id, fingerprint))
        .await
        .map_err(|error| error.to_string())?
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .filter(serde_json::Value::is_array))
}

async fn cache_config_options(
    store: &wisp_store::Store,
    frame_id: &str,
    fingerprint: &str,
    options: &serde_json::Value,
) -> Result<(), String> {
    if !options.is_array() {
        return Ok(());
    }
    store
        .set_setting(
            &config_options_key(frame_id, fingerprint),
            &options.to_string(),
        )
        .await
        .map_err(|error| error.to_string())
}

async fn restore_config_options(
    store: &wisp_store::Store,
    frame_id: &str,
    fingerprint: &str,
    session: &mut wisp_acp::AcpSessionState,
) -> Result<(), String> {
    if let Some(options) = &session.config_options {
        cache_config_options(
            store,
            frame_id,
            fingerprint,
            &serde_json::to_value(options).map_err(|error| error.to_string())?,
        )
        .await?;
    } else if let Some(options) = cached_config_options(store, frame_id, fingerprint).await? {
        session.config_options = serde_json::from_value(options).ok();
    }
    Ok(())
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
    let (session_id, mut session_state) = if let Some(binding) = &binding {
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
    restore_config_options(
        &state.store,
        frame_id,
        &profile_fingerprint,
        &mut session_state,
    )
    .await?;
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

/// Full Permission remains owned by Wisp, so prefer the protocol's one-shot
/// allow option and auto-select it for every request. Falling back to
/// `AllowAlways` is necessary for agents that do not offer a one-shot choice;
/// requests without any allow option still surface to the user.
fn full_permission_option(request: &AcpPermissionRequest) -> Option<String> {
    request
        .options
        .iter()
        .find(|option| option.kind == AcpPermissionKind::AllowOnce)
        .or_else(|| {
            request
                .options
                .iter()
                .find(|option| option.kind == AcpPermissionKind::AllowAlways)
        })
        .map(|option| option.id.clone())
}

fn remote_permission_option(request: &AcpPermissionRequest, approved: bool) -> Option<String> {
    let kind = if approved {
        AcpPermissionKind::AllowOnce
    } else {
        AcpPermissionKind::RejectOnce
    };
    request
        .options
        .iter()
        .find(|option| option.kind == kind)
        .map(|option| option.id.clone())
}

pub(crate) async fn pending_remote_permission_requests(
    state: &AppState,
) -> Vec<crate::ConfirmRequest> {
    let mut requests = state
        .acp_permissions
        .lock()
        .await
        .values()
        .map(|pending| pending.remote_request.clone())
        .collect::<Vec<_>>();
    requests.sort_by(|left, right| left.approval_id.cmp(&right.approval_id));
    requests
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
        AcpTurnKind::User,
    )
    .await
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
        AcpTurnKind::Internal,
    )
    .await
}

/// Route ACP UI to windows of this conversation's project. Internal turns
/// (review, channels) have no originating window, but same-project peers
/// still need the cards; other projects must not see them.
fn emit_ask_event(
    app: &AppHandle,
    frame_id: &str,
    project_id: Option<&str>,
    event: &str,
    payload: serde_json::Value,
) {
    crate::emit_to_session_surfaces(app, frame_id, project_id, event, &payload);
}

/// Surface new pending bridge `ask_user` rows to the UI. `acp_asks` doubles as
/// the seen-set: a row already registered was already emitted, and an answered
/// row leaves the pending query before it leaves the map.
async fn surface_pending_asks(
    state: &AppState,
    app: &AppHandle,
    project_id: Option<&str>,
    frame_id: &str,
) {
    let pending = state
        .store
        .pending_ask_user_requests(frame_id)
        .await
        .unwrap_or_default();
    for (request_id, payload_json) in pending {
        let mut asks = state.acp_asks.lock().await;
        if asks.contains_key(&request_id) {
            continue;
        }
        asks.insert(request_id.clone(), frame_id.to_string());
        drop(asks);
        state
            .awaiting_confirm
            .lock()
            .unwrap()
            .insert(frame_id.to_string());
        state.device_hub.mark_needs_user(frame_id, project_id);
        let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap_or_default();
        emit_ask_event(
            app,
            frame_id,
            project_id,
            "ask-user-request",
            serde_json::json!({
                "frameId": frame_id,
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
    project_id: Option<&str>,
    frame_id: &str,
) {
    let expired = state
        .store
        .expire_ask_user_requests_except(frame_id, &HashSet::new())
        .await
        .unwrap_or_default();
    state
        .acp_asks
        .lock()
        .await
        .retain(|_, owner| owner != frame_id);
    if !state
        .acp_permissions
        .lock()
        .await
        .values()
        .any(|pending| pending.frame_id == frame_id)
    {
        state.awaiting_confirm.lock().unwrap().remove(frame_id);
        state.device_hub.resolve_needs_user(frame_id);
    }
    for (request_id, _) in expired {
        emit_ask_event(
            app,
            frame_id,
            project_id,
            "ask-user-resolved",
            serde_json::json!({
                "frameId": frame_id,
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
    turn_kind: AcpTurnKind,
) -> Result<String, String> {
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
        turn_kind,
    )
    .await;
    // Runs on every exit path, success or error — asks must never outlive
    // their turn as live cards.
    settle_expired_asks(state, app, Some(project.id.as_str()), frame_id).await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_acp_turn_inner(
    state: &AppState,
    app: &AppHandle,
    _window_label: Option<&str>,
    project: &ActiveProject,
    frame_id: &str,
    profile_id: Option<&str>,
    message: &str,
    attachments: &[String],
    injected_context: &[String],
    artifact_references: &[PathBuf],
    turn_kind: AcpTurnKind,
) -> Result<String, String> {
    let runtime = runtime_for(state, project, frame_id, profile_id).await?;
    if let Some(session_state) = runtime.session_state.lock().await.take() {
        crate::emit_to_session_surfaces(
            app,
            frame_id,
            Some(project.id.as_str()),
            "acp-session-state",
            &serde_json::json!({
                "frameId": frame_id,
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
        crate::emit_agent_event_in(
            app,
            AgentEvent::User {
                frame_id: frame_id.to_string(),
                text: message.to_string(),
            },
            Some(project.id.as_str()),
        );
        crate::emit_agent_event_in(
            app,
            AgentEvent::MessageBoundary {
                frame_id: frame_id.to_string(),
                seq: next_seq - 1,
            },
            Some(project.id.as_str()),
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
                surface_pending_asks(state, app, Some(project.id.as_str()), frame_id).await;
            }
            event = runtime.handle.next_event() => match event {
                Some(AcpSessionEvent::Update { kind, payload, .. }) => {
                    if kind == AcpUpdateKind::ConfigOptions {
                        if let Some(options) = payload.get("configOptions") {
                            cache_config_options(&state.store, frame_id, &runtime.fingerprint, options).await?;
                        }
                    }
                    if matches!(kind, AcpUpdateKind::AgentMessage | AcpUpdateKind::AgentThought) {
                        if let Some(text) = text_from_payload(&payload) {
                            let target = if kind == AcpUpdateKind::AgentMessage { &mut assistant } else { &mut reasoning };
                            target.push_str(text);
                            let event = if kind == AcpUpdateKind::AgentMessage {
                                AgentEvent::Text { frame_id: frame_id.to_string(), delta: text.to_string() }
                            } else {
                                AgentEvent::Reasoning { frame_id: frame_id.to_string(), delta: text.to_string() }
                            };
                            crate::emit_agent_event_in(app, event, Some(project.id.as_str()));
                        }
                    } else {
                        if matches!(kind, AcpUpdateKind::ToolCall | AcpUpdateKind::ToolCallUpdate) {
                            upsert_acp_tool_envelope(&mut tools, &payload);
                        }
                        if kind == AcpUpdateKind::Plan {
                            plan = Some(payload.clone());
                        }
                        crate::emit_to_session_surfaces(
                            app,
                            frame_id,
                            Some(project.id.as_str()),
                            "acp-session-update",
                            &serde_json::json!({
                                "frameId": frame_id,
                                "kind": format!("{kind:?}"),
                                "payload": payload,
                            }),
                        );
                    }
                }
                Some(AcpSessionEvent::Permission(request)) => {
                    if crate::approval_commands::session_full_permission(state, frame_id) {
                        if let Some(option_id) = full_permission_option(&request) {
                            if runtime
                                .handle
                                .respond_permission(request.request_id.clone(), Some(option_id))
                                .is_ok()
                            {
                                continue;
                            }
                        }
                    }
                    let pending = PendingAcpPermission::new(frame_id, &request);
                    state
                        .acp_permissions
                        .lock()
                        .await
                        .insert(request.request_id.clone(), pending.clone());
                    state.awaiting_confirm.lock().unwrap().insert(frame_id.to_string());
                    state.device_hub.mark_needs_user(frame_id, Some(&project.id));
                    crate::channels::publish_approval_request(&pending.remote_request);
                    crate::emit_to_session_surfaces(
                        app,
                        frame_id,
                        Some(project.id.as_str()),
                        "permission-request",
                        &permission_event(frame_id, &request),
                    );
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
                if kind == AcpUpdateKind::ConfigOptions {
                    if let Some(options) = payload.get("configOptions") {
                        cache_config_options(&state.store, frame_id, &runtime.fingerprint, options)
                            .await?;
                    }
                }
                if let Some(text) = text_from_payload(&payload) {
                    if kind == AcpUpdateKind::AgentMessage {
                        assistant.push_str(text);
                        crate::emit_agent_event_in(
                            app,
                            AgentEvent::Text {
                                frame_id: frame_id.to_string(),
                                delta: text.to_string(),
                            },
                            Some(project.id.as_str()),
                        );
                    } else if kind == AcpUpdateKind::AgentThought {
                        reasoning.push_str(text);
                        crate::emit_agent_event_in(
                            app,
                            AgentEvent::Reasoning {
                                frame_id: frame_id.to_string(),
                                delta: text.to_string(),
                            },
                            Some(project.id.as_str()),
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
                    crate::emit_to_session_surfaces(
                        app,
                        frame_id,
                        Some(project.id.as_str()),
                        "acp-session-update",
                        &serde_json::json!({
                            "frameId": frame_id,
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
    if !resources.is_empty() {
        crate::emit_agent_event_in(
            app,
            AgentEvent::Resources {
                frame_id: frame_id.to_string(),
                seq: next_seq,
                resources: resources.iter().map(Into::into).collect(),
            },
            Some(project.id.as_str()),
        );
    }
    cancel_pending_permissions(state, frame_id, &runtime).await;
    Ok(stop_reason(outcome.stop_reason).into())
}

/// ACP has no Wisp-reference block type. Render trusted, host-resolved Wisp
/// context as ordinary text blocks, which every ACP v1 Agent accepts.
fn acp_text_content(message: &str, injected_context: &[String]) -> Vec<ContentBlock> {
    let mut content = injected_context
        .iter()
        .map(|text| ContentBlock::Text(TextContent::new(text.clone())))
        .collect::<Vec<_>>();
    content.push(ContentBlock::Text(TextContent::new(message.to_string())));
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

#[tauri::command]
pub(crate) async fn respond_acp_permission(
    state: State<'_, AppState>,
    app: AppHandle,
    request_id: String,
    option_id: Option<String>,
) -> Result<(), String> {
    respond_acp_permission_inner(&state, &app, request_id, option_id)
        .await
        .map(|_| ())
}

async fn respond_acp_permission_inner(
    state: &AppState,
    app: &AppHandle,
    request_id: String,
    option_id: Option<String>,
) -> Result<PendingAcpPermission, String> {
    let pending = state
        .acp_permissions
        .lock()
        .await
        .remove(&request_id)
        .ok_or_else(|| "ACP permission request is no longer pending.".to_string())?;
    let frame_id = pending.frame_id.clone();
    let runtime = state.acp_sessions.lock().await.get(&frame_id).cloned();
    let Some(runtime) = runtime else {
        state
            .acp_permissions
            .lock()
            .await
            .insert(request_id, pending);
        return Err("ACP session is no longer active.".into());
    };
    if let Err(error) = runtime
        .handle
        .respond_permission(request_id.clone(), option_id)
    {
        state
            .acp_permissions
            .lock()
            .await
            .insert(request_id.clone(), pending.clone());
        return Err(error.to_string());
    }
    let frame_has_permissions = state
        .acp_permissions
        .lock()
        .await
        .values()
        .any(|pending| pending.frame_id == frame_id);
    let frame_has_asks = state
        .acp_asks
        .lock()
        .await
        .values()
        .any(|owner| owner == &frame_id);
    if !frame_has_permissions && !frame_has_asks {
        state.awaiting_confirm.lock().unwrap().remove(&frame_id);
        state.device_hub.resolve_needs_user(&frame_id);
    }
    let project_id = state.store.frame_project_id(&frame_id).await.ok().flatten();
    crate::emit_to_session_surfaces(
        app,
        &frame_id,
        project_id.as_deref(),
        "permission-resolved",
        &serde_json::json!({
            "frameId": frame_id,
            "requestId": request_id,
        }),
    );
    Ok(pending)
}

pub(crate) async fn respond_remote_permission(
    state: &AppState,
    app: &AppHandle,
    selector: &str,
    approved: bool,
) -> Result<crate::approval_commands::RemoteConfirmationResolution, String> {
    let selector = selector.trim().to_ascii_lowercase();
    if selector.len() < 6 || !selector.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("审批编号至少需要 6 位十六进制字符。".into());
    }
    let (request_id, option_id) = {
        let permissions = state.acp_permissions.lock().await;
        let matches = permissions
            .iter()
            .filter(|(_, pending)| pending.remote_request.approval_id.starts_with(&selector))
            .collect::<Vec<_>>();
        let [entry] = matches.as_slice() else {
            return if matches.is_empty() {
                Err("未找到该待审批请求；它可能已经处理或失效。".into())
            } else {
                Err("审批编号前缀不唯一，请输入更多位。".into())
            };
        };
        let option_id = remote_permission_option(&entry.1.request, approved).ok_or_else(|| {
            if approved {
                "该 ACP 请求没有可用的允许选项。".to_string()
            } else {
                "该 ACP 请求没有可用的拒绝选项。".to_string()
            }
        })?;
        (entry.0.clone(), option_id)
    };

    let pending = respond_acp_permission_inner(state, app, request_id, Some(option_id)).await?;
    Ok(crate::approval_commands::RemoteConfirmationResolution {
        approval_id: pending.remote_request.approval_id,
        frame_id: pending.frame_id,
        source: crate::approval_commands::RemoteConfirmationSource::Acp,
    })
}

/// Resolve a pending bridge `ask_user` request: write the answer for the
/// bridge's poll loop to consume and settle the card. The permission flow's
/// shape, with the store standing in for the oneshot — the bridge lives in
/// another process.
#[tauri::command]
pub(crate) async fn respond_ask_user(
    state: State<'_, AppState>,
    app: AppHandle,
    _window: tauri::WebviewWindow,
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
        .any(|pending| pending.frame_id == frame_id);
    if !frame_has_asks && !frame_has_permissions {
        state.awaiting_confirm.lock().unwrap().remove(&frame_id);
        state.device_hub.resolve_needs_user(&frame_id);
    }
    // Route by the conversation's project, not the answering window's current
    // binding. The window may have switched projects after the card appeared.
    let project_id = state.store.frame_project_id(&frame_id).await.ok().flatten();
    crate::emit_to_session_surfaces(
        &app,
        &frame_id,
        project_id.as_deref(),
        "ask-user-resolved",
        &serde_json::json!({
            "frameId": frame_id,
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
    let project = state.require_active(window.label())?;
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
    let runtime = runtime_for(&state, &project, &frame_id, None).await?;
    let value = serde_json::from_value(value).map_err(|error| error.to_string())?;
    let options = runtime
        .handle
        .set_config(runtime.session_id.clone(), config_id, value)
        .await
        .map_err(|error| error.to_string())?;
    let value = serde_json::to_value(&options).map_err(|error| error.to_string())?;
    cache_config_options(&state.store, &frame_id, &runtime.fingerprint, &value).await?;
    if let Some(pending) = runtime.session_state.lock().await.as_mut() {
        pending.config_options = Some(options);
    }
    crate::emit_to_session_surfaces(
        &app,
        &frame_id,
        Some(project.id.as_str()),
        "acp-session-state",
        &serde_json::json!({
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
    let project = state.require_active(window.label())?;
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
        .retain(|_, pending| pending.frame_id != frame_id);
    state
        .acp_asks
        .lock()
        .await
        .retain(|_, owner| owner != frame_id);
    state.awaiting_confirm.lock().unwrap().remove(frame_id);
    state.device_hub.resolve_needs_user(frame_id);
}

pub(crate) async fn close_frame(state: &AppState, frame_id: &str) {
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
        .retain(|_, pending| pending.frame_id != frame_id);
    state
        .acp_asks
        .lock()
        .await
        .retain(|_, owner| owner != frame_id);
    state.awaiting_confirm.lock().unwrap().remove(frame_id);
    state.device_hub.resolve_needs_user(frame_id);
}

async fn cancel_pending_permissions(state: &AppState, frame_id: &str, runtime: &AcpRuntime) {
    let request_ids = {
        let mut pending = state.acp_permissions.lock().await;
        let request_ids = pending
            .iter()
            .filter(|(_, permission)| permission.frame_id == frame_id)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        pending.retain(|_, permission| permission.frame_id != frame_id);
        request_ids
    };
    for request_id in request_ids {
        let _ = runtime.handle.respond_permission(request_id, None);
    }
    state.awaiting_confirm.lock().unwrap().remove(frame_id);
    state.device_hub.resolve_needs_user(frame_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permission_option(id: &str, kind: AcpPermissionKind) -> wisp_acp::AcpPermissionOption {
        wisp_acp::AcpPermissionOption {
            id: id.into(),
            name: id.into(),
            kind,
        }
    }

    #[test]
    fn remote_permission_accepts_only_one_shot_options() {
        let request = AcpPermissionRequest {
            request_id: "request".into(),
            session_id: "session".into(),
            tool_call: serde_json::json!({"title": "Run checks"}),
            options: vec![
                permission_option("allow-always", AcpPermissionKind::AllowAlways),
                permission_option("allow-once", AcpPermissionKind::AllowOnce),
                permission_option("reject-always", AcpPermissionKind::RejectAlways),
            ],
        };
        assert_eq!(
            remote_permission_option(&request, true).as_deref(),
            Some("allow-once")
        );
        assert_eq!(remote_permission_option(&request, false), None);
    }

    #[test]
    fn pending_acp_permission_builds_a_text_safe_remote_prompt() {
        let request = AcpPermissionRequest {
            request_id: "request".into(),
            session_id: "session".into(),
            tool_call: serde_json::json!({
                "title": "Run checks",
                "rawInput": {"command": "cargo test"}
            }),
            options: vec![permission_option(
                "allow-once",
                AcpPermissionKind::AllowOnce,
            )],
        };
        let pending = PendingAcpPermission::new("frame-123456", &request);
        assert_eq!(pending.frame_id, "frame-123456");
        assert_eq!(pending.remote_request.tool, "Run checks");
        assert!(pending.remote_request.preview.contains("cargo test"));
        assert_eq!(pending.request, request);
    }

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
    fn terminal_auth_launch_preserves_profile_and_method_argument_boundaries() {
        let profile = AcpAgentProfile {
            id: "claude".into(),
            label: "Claude ACP".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@agentclientprotocol/claude-agent-acp".into()],
        };
        let method = AcpAuthMethod {
            id: "claude-ai-login".into(),
            name: "Claude Subscription".into(),
            description: None,
            kind: AcpAuthMethodKind::Terminal {
                args: vec![
                    "--cli".into(),
                    "auth".into(),
                    "login".into(),
                    "argument with spaces".into(),
                ],
                env: [("ACP_AUTH_TEST".into(), "enabled".into())]
                    .into_iter()
                    .collect(),
            },
        };

        let spec = terminal_auth_launch_spec(&profile, &method, Path::new("/tmp/project"))
            .expect("terminal auth spec");
        assert_eq!(spec.program, "npx");
        assert_eq!(
            spec.args,
            [
                "-y",
                "@agentclientprotocol/claude-agent-acp",
                "--cli",
                "auth",
                "login",
                "argument with spaces",
            ]
        );
        assert_eq!(spec.envs, [("ACP_AUTH_TEST".into(), "enabled".into())]);
        assert_eq!(spec.cwd.as_deref(), Some(Path::new("/tmp/project")));
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
        assert_eq!(
            auth_method_kind(&AcpAuthMethodKind::Terminal {
                args: vec![],
                env: Default::default(),
            }),
            "terminal"
        );
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
    fn full_permission_prefers_one_shot_acp_allow() {
        let request = AcpPermissionRequest {
            request_id: "permission-1".into(),
            session_id: "session-1".into(),
            tool_call: serde_json::json!({}),
            options: vec![
                wisp_acp::AcpPermissionOption {
                    id: "always".into(),
                    name: "Always".into(),
                    kind: AcpPermissionKind::AllowAlways,
                },
                wisp_acp::AcpPermissionOption {
                    id: "once".into(),
                    name: "Once".into(),
                    kind: AcpPermissionKind::AllowOnce,
                },
            ],
        };
        assert_eq!(full_permission_option(&request).as_deref(), Some("once"));

        let rejected_only = AcpPermissionRequest {
            options: vec![wisp_acp::AcpPermissionOption {
                id: "reject".into(),
                name: "Reject".into(),
                kind: AcpPermissionKind::RejectOnce,
            }],
            ..request
        };
        assert_eq!(full_permission_option(&rejected_only), None);
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
        assert!(json.find("bear-map").unwrap() < json.find("analyse this").unwrap());
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
    async fn config_options_survive_restart_without_crossing_session_or_profile() {
        let tmp = std::env::temp_dir().join(format!("wisp_acp_configs_{}.sqlite", Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let options = serde_json::json!([{ "id": "model", "name": "Model", "type": "select", "currentValue": "fast", "options": [{"value":"fast","name":"Fast"}, {"value":"smart","name":"Smart"}] }]);
        let mut fresh = wisp_acp::AcpSessionState {
            modes: None,
            config_options: Some(serde_json::from_value(options.clone()).unwrap()),
        };
        restore_config_options(&store, "a", "profile-a", &mut fresh)
            .await
            .unwrap();
        let mut updated = options.clone();
        updated[0]["currentValue"] = serde_json::json!("smart");
        cache_config_options(&store, "a", "profile-a", &updated)
            .await
            .unwrap();
        cache_config_options(&store, "b", "profile-b", &options)
            .await
            .unwrap();
        drop(store);
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let mut resumed = wisp_acp::AcpSessionState {
            modes: None,
            config_options: None,
        };
        restore_config_options(&store, "a", "profile-a", &mut resumed)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(resumed.config_options.unwrap()).unwrap(),
            updated
        );
        assert_eq!(
            cached_config_options(&store, "b", "profile-b")
                .await
                .unwrap(),
            Some(options)
        );
        assert!(cached_config_options(&store, "a", "profile-b")
            .await
            .unwrap()
            .is_none());
        let mut cleared = wisp_acp::AcpSessionState {
            modes: None,
            config_options: Some(vec![]),
        };
        restore_config_options(&store, "a", "profile-a", &mut cleared)
            .await
            .unwrap();
        cache_config_options(&store, "a", "profile-a", &serde_json::Value::Null)
            .await
            .unwrap();
        let mut loaded = wisp_acp::AcpSessionState {
            modes: None,
            config_options: None,
        };
        restore_config_options(&store, "a", "profile-a", &mut loaded)
            .await
            .unwrap();
        assert_eq!(loaded.config_options, Some(vec![]));
        drop(store);
        let _ = std::fs::remove_file(tmp);
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
