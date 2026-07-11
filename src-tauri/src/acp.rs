use crate::{acp_bridge_launch, ActiveProject, AgentEvent, AppState};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use uuid::Uuid;
use wisp_acp::{
    acp::schema::v1::{
        ContentBlock, McpServer, McpServerStdio, ResourceLink, SessionId, TextContent,
    },
    AcpAgentProfile as LaunchProfile, AcpPermissionRequest, AcpSessionEvent, AcpSessionHandle,
    AcpStopReason, AcpUpdateKind,
};
use wisp_llm::Message;

const PROFILES_KEY: &str = "acp_agent_profiles";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AcpAgentProfile {
    #[serde(default)]
    pub id: String,
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct AcpAgentInfoDto {
    protocol_version: u16,
    implementation: Option<serde_json::Value>,
    capabilities: serde_json::Value,
    auth_methods: Vec<serde_json::Value>,
}

#[derive(Serialize, Clone)]
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

fn fingerprint(profile: &AcpAgentProfile) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in serde_json::to_vec(&(profile.command.trim(), &profile.args)).unwrap_or_default() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

async fn profiles(store: &wisp_store::Store) -> Vec<AcpAgentProfile> {
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

fn launch_profile(profile: &AcpAgentProfile) -> LaunchProfile {
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

fn mcp_server(
    state: &AppState,
    project: &ActiveProject,
    frame_id: &str,
) -> Result<McpServer, String> {
    let (command, args) = acp_bridge_launch(&state.app_data, project, frame_id)?;
    Ok(McpServer::Stdio(
        McpServerStdio::new("wisp-science", PathBuf::from(command)).args(args),
    ))
}

async fn runtime_for(
    state: &AppState,
    project: &ActiveProject,
    frame_id: &str,
    requested_profile_id: Option<&str>,
) -> Result<Arc<AcpRuntime>, String> {
    if let Some(runtime) = state.acp_sessions.lock().await.get(frame_id).cloned() {
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
    let bridge = vec![mcp_server(state, project, frame_id)?];
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

pub(crate) async fn run_acp_turn(
    state: &AppState,
    app: &AppHandle,
    project: &ActiveProject,
    frame_id: &str,
    profile_id: Option<&str>,
    message: &str,
    attachments: &[String],
) -> Result<String, String> {
    let runtime = runtime_for(state, project, frame_id, profile_id).await?;
    if let Some(session_state) = runtime.session_state.lock().await.take() {
        let _ = app.emit(
            "acp-session-state",
            serde_json::json!({
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
    let mut content = vec![ContentBlock::Text(TextContent::new(message.to_string()))];
    for attachment in attachments {
        let path = wisp_tools::safety::validate_file_path(project.root.as_path(), attachment)
            .map_err(|_| format!("Attachment '{attachment}' is outside the active project."))?;
        let uri = url::Url::from_file_path(&path).map_err(|_| {
            format!(
                "Attachment path '{}' cannot be represented as a file URI.",
                path.display()
            )
        })?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment");
        content.push(ContentBlock::ResourceLink(ResourceLink::new(
            name,
            uri.to_string(),
        )));
    }
    let seq = state
        .store
        .load_messages(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .len() as i64;
    state
        .store
        .append_message(frame_id, seq + 1, &Message::user(message))
        .await
        .map_err(|error| error.to_string())?;
    let _ = app.emit(
        "agent",
        AgentEvent::User {
            frame_id: frame_id.to_string(),
            text: message.to_string(),
        },
    );
    let prompt = runtime.handle.prompt(runtime.session_id.clone(), content);
    tokio::pin!(prompt);
    let mut assistant = String::new();
    let mut reasoning = String::new();
    let outcome = loop {
        tokio::select! {
            result = &mut prompt => break result.map_err(|error| error.to_string())?,
            event = runtime.handle.next_event() => match event {
                Some(AcpSessionEvent::Update { kind, payload, .. }) => {
                    if matches!(kind, AcpUpdateKind::AgentMessage | AcpUpdateKind::AgentThought) {
                        if let Some(text) = text_from_payload(&payload) {
                            let target = if kind == AcpUpdateKind::AgentMessage { &mut assistant } else { &mut reasoning };
                            target.push_str(text);
                            let event = if kind == AcpUpdateKind::AgentMessage {
                                AgentEvent::Text { frame_id: frame_id.to_string(), delta: text.to_string() }
                            } else {
                                AgentEvent::Reasoning { frame_id: frame_id.to_string(), delta: text.to_string() }
                            };
                            let _ = app.emit("agent", event);
                        }
                    } else {
                        let _ = app.emit("acp-session-update", serde_json::json!({
                            "frameId": frame_id,
                            "kind": format!("{kind:?}"),
                            "payload": payload,
                        }));
                    }
                }
                Some(AcpSessionEvent::Permission(request)) => {
                    state.acp_permissions.lock().await.insert(request.request_id.clone(), frame_id.to_string());
                    state.awaiting_confirm.lock().unwrap().insert(frame_id.to_string());
                    let _ = app.emit("permission-request", permission_event(frame_id, &request));
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
                        let _ = app.emit(
                            "agent",
                            AgentEvent::Text {
                                frame_id: frame_id.to_string(),
                                delta: text.to_string(),
                            },
                        );
                    } else if kind == AcpUpdateKind::AgentThought {
                        reasoning.push_str(text);
                        let _ = app.emit(
                            "agent",
                            AgentEvent::Reasoning {
                                frame_id: frame_id.to_string(),
                                delta: text.to_string(),
                            },
                        );
                    }
                }
                if !matches!(
                    kind,
                    AcpUpdateKind::AgentMessage | AcpUpdateKind::AgentThought
                ) {
                    let _ = app.emit(
                        "acp-session-update",
                        serde_json::json!({
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
    let mut persisted = Message::assistant(assistant);
    persisted.reasoning = (!reasoning.is_empty()).then_some(reasoning);
    persisted.model_name = profiles(&state.store)
        .await
        .into_iter()
        .find(|profile| profile.id == runtime.profile_id)
        .map(|profile| profile.label);
    state
        .store
        .append_message(frame_id, seq + 2, &persisted)
        .await
        .map_err(|error| error.to_string())?;
    cancel_pending_permissions(state, frame_id, &runtime).await;
    Ok(stop_reason(outcome.stop_reason).into())
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
    let frame_id = state
        .acp_permissions
        .lock()
        .await
        .remove(&request_id)
        .ok_or_else(|| "ACP permission request is no longer pending.".to_string())?;
    let runtime = state
        .acp_sessions
        .lock()
        .await
        .get(&frame_id)
        .cloned()
        .ok_or_else(|| "ACP session is no longer active.".to_string())?;
    let result = runtime
        .handle
        .respond_permission(request_id.clone(), option_id)
        .map_err(|error| error.to_string());
    if !state
        .acp_permissions
        .lock()
        .await
        .values()
        .any(|owner| owner == &frame_id)
    {
        state.awaiting_confirm.lock().unwrap().remove(&frame_id);
    }
    if result.is_ok() {
        let _ = app.emit(
            "permission-resolved",
            serde_json::json!({
                "frameId": frame_id,
                "requestId": request_id,
            }),
        );
    }
    result
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
        .retain(|_, owner| owner != frame_id);
    state.awaiting_confirm.lock().unwrap().remove(frame_id);
}

async fn cancel_pending_permissions(state: &AppState, frame_id: &str, runtime: &AcpRuntime) {
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
    state.awaiting_confirm.lock().unwrap().remove(frame_id);
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
    fn update_text_mapping_is_tolerant() {
        assert_eq!(
            text_from_payload(&serde_json::json!({"content":{"text":"a"}})),
            Some("a")
        );
        assert_eq!(text_from_payload(&serde_json::json!({"future":true})), None);
    }
}
