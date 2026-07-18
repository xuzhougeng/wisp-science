//! Process and protocol boundary for local ACP v1 agents.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::{Path, PathBuf},
    pin::pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use agent_client_protocol::{
    schema::{
        v1::{
            AuthMethod, AuthenticateRequest, CancelNotification, CloseSessionRequest, ContentBlock,
            Implementation, InitializeRequest, LoadSessionRequest, McpServer, NewSessionRequest,
            PermissionOptionId, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
            RequestPermissionResponse, ResumeSessionRequest, SelectedPermissionOutcome,
            SessionConfigOption, SessionConfigOptionValue, SessionId, SessionModeState,
            SessionNotification, SessionUpdate, SetSessionConfigOptionRequest,
            SetSessionModeRequest, StopReason, TextContent,
        },
        ProtocolVersion,
    },
    Agent, Client, ConnectTo, ConnectionTo, Handled, JsonRpcMessage, Lines, UntypedMessage,
};
use futures::{io::BufReader, AsyncBufReadExt, AsyncWriteExt, StreamExt};
use tokio::sync::{mpsc, oneshot};

pub use agent_client_protocol as acp;

const DEFAULT_STDERR_LIMIT: usize = 64 * 1024;
static PERMISSION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Non-secret command configuration for one installed ACP agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpAgentProfile {
    pub id: String,
    pub label: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl AcpAgentProfile {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        command: impl Into<PathBuf>,
        args: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            command: command.into(),
            args,
            env: BTreeMap::new(),
        }
    }

    /// Adds one explicit environment override for the ACP child process.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    fn validate(&self) -> Result<(), AcpError> {
        if self.id.trim().is_empty() {
            return Err(AcpError::InvalidProfile("agent id is empty"));
        }
        if self.label.trim().is_empty() {
            return Err(AcpError::InvalidProfile("agent label is empty"));
        }
        if self.command.as_os_str().is_empty() {
            return Err(AcpError::InvalidProfile("agent command is empty"));
        }
        if self.env.keys().any(|key| key.is_empty()) {
            return Err(AcpError::InvalidProfile("environment key is empty"));
        }
        Ok(())
    }
}

/// Applies the Codex ACP policy used for controlled project execution.
///
/// Codex's `agent` mode is deliberately approval-aware: each turn is confined
/// to workspace-write with network disabled, and requests outside that sandbox
/// are returned to the ACP client. Callers must reject escalations at their
/// permission boundary instead of treating this profile as approval-free.
pub fn codex_project_sandbox_profile(mut profile: AcpAgentProfile) -> AcpAgentProfile {
    let config = serde_json::json!({
        "approval_policy": "on-request",
        "sandbox_mode": "workspace-write",
        "sandbox_workspace_write": {
            "network_access": false,
        },
        "web_search": "disabled",
        "mcp_servers": {},
    });
    profile.env.insert(
        "CODEX_CONFIG".into(),
        serde_json::to_string(&config).expect("static Codex config serializes"),
    );
    profile
        .env
        .insert("INITIAL_AGENT_MODE".into(), "agent".into());
    for value in [
        r#"approval_policy="on-request""#,
        r#"sandbox_mode="workspace-write""#,
        "sandbox_workspace_write.network_access=false",
        r#"web_search="disabled""#,
        "mcp_servers={}",
    ] {
        profile.args.push("-c".into());
        profile.args.push(value.into());
    }
    profile
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpImplementation {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpAuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// Information negotiated during ACP initialization.
#[derive(Debug, Clone, PartialEq)]
pub struct AcpAgentInfo {
    pub protocol_version: u16,
    pub implementation: Option<AcpImplementation>,
    pub capabilities: serde_json::Value,
    pub auth_methods: Vec<AcpAuthMethod>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpUpdateKind {
    UserMessage,
    AgentMessage,
    AgentThought,
    ToolCall,
    ToolCallUpdate,
    Plan,
    AvailableCommands,
    CurrentMode,
    ConfigOptions,
    SessionInfo,
    Usage,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpPermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpPermissionOption {
    pub id: String,
    pub name: String,
    pub kind: AcpPermissionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcpPermissionRequest {
    pub request_id: String,
    pub session_id: String,
    pub tool_call: serde_json::Value,
    pub options: Vec<AcpPermissionOption>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcpUsageCost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcpUsageUpdate {
    pub used: u64,
    pub size: u64,
    pub cost: Option<AcpUsageCost>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AcpSessionEvent {
    Update {
        session_id: String,
        kind: AcpUpdateKind,
        payload: serde_json::Value,
        usage: Option<AcpUsageUpdate>,
    },
    Permission(AcpPermissionRequest),
    Exited {
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpStopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcpPromptOutcome {
    pub stop_reason: AcpStopReason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcpSessionState {
    pub modes: Option<serde_json::Value>,
    pub config_options: Option<Vec<SessionConfigOption>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AcpSessionStart {
    pub session_id: SessionId,
    pub state: AcpSessionState,
}

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("invalid ACP agent profile: {0}")]
    InvalidProfile(&'static str),
    #[error("ACP agent selected unsupported protocol version {actual}; expected 1")]
    ProtocolMismatch { actual: u16 },
    #[error("ACP capability is not advertised: {0}")]
    Unsupported(&'static str),
    #[error("ACP agent process is closed")]
    Closed,
    #[error("ACP agent error: {0}")]
    Agent(String),
}

type Reply<T> = oneshot::Sender<Result<T, AcpError>>;

enum Command {
    Authenticate(String, Reply<()>),
    NewSession(PathBuf, Vec<McpServer>, Reply<AcpSessionStart>),
    Load(SessionId, PathBuf, Vec<McpServer>, Reply<AcpSessionState>),
    Resume(SessionId, PathBuf, Vec<McpServer>, Reply<AcpSessionState>),
    Prompt(SessionId, Vec<ContentBlock>, Reply<AcpPromptOutcome>),
    Cancel(SessionId),
    RespondPermission(String, Option<String>),
    SetConfig(
        SessionId,
        String,
        SessionConfigOptionValue,
        Reply<Vec<SessionConfigOption>>,
    ),
    SetMode(SessionId, String, Reply<()>),
    Close(SessionId, Reply<()>),
    Shutdown,
}

struct PendingPermission {
    session_id: SessionId,
    options: Vec<PermissionOptionId>,
    reply: oneshot::Sender<Option<PermissionOptionId>>,
}

type PendingPermissions = Arc<Mutex<HashMap<String, PendingPermission>>>;

/// Handle to one local ACP process. Dropping it force-stops the child.
pub struct AcpSessionHandle {
    info: AcpAgentInfo,
    command_tx: mpsc::UnboundedSender<Command>,
    event_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<AcpSessionEvent>>,
    stderr: BoundedStderr,
    actor: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl AcpSessionHandle {
    pub async fn launch(profile: AcpAgentProfile) -> Result<Self, AcpError> {
        Self::launch_with_stderr_limit(profile, DEFAULT_STDERR_LIMIT).await
    }

    pub async fn launch_with_stderr_limit(
        profile: AcpAgentProfile,
        stderr_limit: usize,
    ) -> Result<Self, AcpError> {
        profile.validate()?;
        let stderr = BoundedStderr::new(stderr_limit);
        let process = ProcessTransport {
            profile,
            stderr: stderr.clone(),
        };
        launch_transport(process, stderr).await
    }

    pub fn info(&self) -> &AcpAgentInfo {
        &self.info
    }

    /// False once the agent actor has stopped — child exit, connection
    /// failure, or shutdown. Every request on a dead handle fails, so callers
    /// caching handles should relaunch instead of retrying.
    pub fn is_alive(&self) -> bool {
        !self.command_tx.is_closed()
    }

    pub fn stderr(&self) -> String {
        self.stderr.snapshot()
    }

    pub async fn next_event(&self) -> Option<AcpSessionEvent> {
        self.event_rx.lock().await.recv().await
    }

    pub async fn authenticate(&self, method_id: impl Into<String>) -> Result<(), AcpError> {
        self.request(|reply| Command::Authenticate(method_id.into(), reply))
            .await
    }

    pub async fn new_session(
        &self,
        cwd: impl AsRef<Path>,
        mcp_servers: Vec<McpServer>,
    ) -> Result<AcpSessionStart, AcpError> {
        self.request(|reply| Command::NewSession(cwd.as_ref().to_path_buf(), mcp_servers, reply))
            .await
    }

    pub async fn load_session(
        &self,
        session_id: SessionId,
        cwd: impl AsRef<Path>,
        mcp_servers: Vec<McpServer>,
    ) -> Result<AcpSessionState, AcpError> {
        self.request(|reply| {
            Command::Load(session_id, cwd.as_ref().to_path_buf(), mcp_servers, reply)
        })
        .await
    }

    pub async fn resume_session(
        &self,
        session_id: SessionId,
        cwd: impl AsRef<Path>,
        mcp_servers: Vec<McpServer>,
    ) -> Result<AcpSessionState, AcpError> {
        self.request(|reply| {
            Command::Resume(session_id, cwd.as_ref().to_path_buf(), mcp_servers, reply)
        })
        .await
    }

    pub async fn prompt_text(
        &self,
        session_id: SessionId,
        prompt: impl Into<String>,
    ) -> Result<AcpPromptOutcome, AcpError> {
        self.prompt(
            session_id,
            vec![ContentBlock::Text(TextContent::new(prompt.into()))],
        )
        .await
    }

    pub async fn prompt(
        &self,
        session_id: SessionId,
        content: Vec<ContentBlock>,
    ) -> Result<AcpPromptOutcome, AcpError> {
        self.request(|reply| Command::Prompt(session_id, content, reply))
            .await
    }

    pub fn cancel(&self, session_id: SessionId) -> Result<(), AcpError> {
        self.command_tx
            .send(Command::Cancel(session_id))
            .map_err(|_| AcpError::Closed)
    }

    pub fn respond_permission(
        &self,
        request_id: impl Into<String>,
        option_id: Option<String>,
    ) -> Result<(), AcpError> {
        self.command_tx
            .send(Command::RespondPermission(request_id.into(), option_id))
            .map_err(|_| AcpError::Closed)
    }

    pub async fn set_config(
        &self,
        session_id: SessionId,
        config_id: impl Into<String>,
        value: SessionConfigOptionValue,
    ) -> Result<Vec<SessionConfigOption>, AcpError> {
        self.request(|reply| Command::SetConfig(session_id, config_id.into(), value, reply))
            .await
    }

    pub async fn set_mode(
        &self,
        session_id: SessionId,
        mode_id: impl Into<String>,
    ) -> Result<(), AcpError> {
        self.request(|reply| Command::SetMode(session_id, mode_id.into(), reply))
            .await
    }

    pub async fn close_session(&self, session_id: SessionId) -> Result<(), AcpError> {
        self.request(|reply| Command::Close(session_id, reply))
            .await
    }

    pub async fn shutdown(&self, grace: Duration) {
        let _ = self.command_tx.send(Command::Shutdown);
        let actor = self.actor.lock().expect("ACP actor mutex poisoned").take();
        if let Some(mut actor) = actor {
            if tokio::time::timeout(grace, &mut actor).await.is_err() {
                actor.abort();
                let _ = actor.await;
            }
        }
    }

    async fn request<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, AcpError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.command_tx
            .send(command(reply_tx))
            .map_err(|_| AcpError::Closed)?;
        reply_rx.await.map_err(|_| AcpError::Closed)?
    }
}

async fn launch_transport(
    process: impl ConnectTo<Client>,
    stderr: BoundedStderr,
) -> Result<AcpSessionHandle, AcpError> {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let actor = tokio::spawn(async move {
        let result = run_actor(process, command_rx, event_tx.clone(), ready_tx).await;
        let _ = event_tx.send(AcpSessionEvent::Exited {
            error: result.err().map(|error| error.to_string()),
        });
    });

    let info = match ready_rx.await {
        Ok(result) => result?,
        Err(_) => {
            // The process waiter and stderr reader finish independently, so a
            // just-exited child may not have surfaced its diagnostics yet. The
            // child is dead (its stderr pipe hits EOF), so poll briefly until
            // the reader flushes rather than betting on one fixed delay — a
            // fixed 25ms raced the reader under slow CI and flaked (#179).
            let message = wait_for_stderr(&stderr).await;
            actor.abort();
            return Err(if message.is_empty() {
                AcpError::Closed
            } else {
                AcpError::Agent(message)
            });
        }
    };

    Ok(AcpSessionHandle {
        info,
        command_tx,
        event_rx: tokio::sync::Mutex::new(event_rx),
        stderr,
        actor: Mutex::new(Some(actor)),
    })
}

/// Wait for a just-exited child's stderr reader to flush, returning as soon as
/// anything is captured. Bounded so a genuinely silent exit still returns.
async fn wait_for_stderr(stderr: &BoundedStderr) -> String {
    // ponytail: 500ms ceiling (50×10ms). Only the fully-elapsed, no-stderr case
    // waits the whole budget, and it's already an error path. Raise if a slower
    // agent ever needs longer to flush.
    for _ in 0..50 {
        let snapshot = stderr.snapshot();
        if !snapshot.is_empty() {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    stderr.snapshot()
}

impl Drop for AcpSessionHandle {
    fn drop(&mut self) {
        if let Some(actor) = self.actor.lock().expect("ACP actor mutex poisoned").take() {
            actor.abort();
        }
    }
}

async fn run_actor(
    process: impl ConnectTo<Client>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<AcpSessionEvent>,
    ready: oneshot::Sender<Result<AcpAgentInfo, AcpError>>,
) -> Result<(), agent_client_protocol::Error> {
    let pending: PendingPermissions = Arc::new(Mutex::new(HashMap::new()));
    Client
        .builder()
        .name("wisp-acp")
        .on_receive_notification(
            {
                let events = events.clone();
                async move |message: UntypedMessage, cx| {
                    if !SessionNotification::matches_method(&message.method) {
                        return Ok(Handled::No {
                            message: (message, cx),
                            retry: false,
                        });
                    }
                    if let Ok(notification) =
                        SessionNotification::parse_message(&message.method, &message.params)
                    {
                        let kind = update_kind(&notification.update);
                        let usage = typed_usage_update(&notification.update);
                        let payload = serde_json::to_value(notification.update).unwrap_or_else(
                            |error| serde_json::json!({ "serializationError": error.to_string() }),
                        );
                        let _ = events.send(AcpSessionEvent::Update {
                            session_id: notification.session_id.to_string(),
                            kind,
                            payload,
                            usage,
                        });
                    }
                    // Ignore future update variants until Wisp learns how to display them.
                    Ok(Handled::Yes)
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let events = events.clone();
                let pending = pending.clone();
                async move |request: RequestPermissionRequest,
                            responder,
                            cx: ConnectionTo<Agent>| {
                    let request_id = format!(
                        "permission-{}",
                        PERMISSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                    );
                    let (reply_tx, reply_rx) = oneshot::channel();
                    let options = request
                        .options
                        .iter()
                        .map(|option| option.option_id.clone())
                        .collect();
                    pending
                        .lock()
                        .expect("pending permissions poisoned")
                        .insert(
                            request_id.clone(),
                            PendingPermission {
                                session_id: request.session_id.clone(),
                                options,
                                reply: reply_tx,
                            },
                        );
                    let event = AcpPermissionRequest {
                        request_id: request_id.clone(),
                        session_id: request.session_id.to_string(),
                        tool_call: serde_json::to_value(request.tool_call).unwrap_or_default(),
                        options: request.options.into_iter().map(permission_option).collect(),
                    };
                    if events.send(AcpSessionEvent::Permission(event)).is_err() {
                        pending
                            .lock()
                            .expect("pending permissions poisoned")
                            .remove(&request_id);
                        responder.respond(RequestPermissionResponse::new(
                            RequestPermissionOutcome::Cancelled,
                        ))?;
                        return Ok(());
                    }
                    cx.spawn(async move {
                        let outcome = match reply_rx.await.ok().flatten() {
                            Some(option_id) => RequestPermissionOutcome::Selected(
                                SelectedPermissionOutcome::new(option_id),
                            ),
                            None => RequestPermissionOutcome::Cancelled,
                        };
                        responder.respond(RequestPermissionResponse::new(outcome))
                    })?;
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(process, async move |connection| {
            let initialized = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1).client_info(
                    Implementation::new("wisp-science", env!("CARGO_PKG_VERSION")),
                ))
                .block_task()
                .await?;
            if initialized.protocol_version != ProtocolVersion::V1 {
                let _ = ready.send(Err(AcpError::ProtocolMismatch {
                    actual: initialized.protocol_version.as_u16(),
                }));
                return Err(agent_client_protocol::util::internal_error(
                    "agent selected unsupported ACP protocol version",
                ));
            }
            let capabilities = initialized.agent_capabilities.clone();
            let _ = ready.send(Ok(agent_info(initialized)));

            while let Some(command) = commands.recv().await {
                match command {
                    Command::Authenticate(method_id, reply) => answer(
                        reply,
                        connection
                            .send_request(AuthenticateRequest::new(method_id))
                            .block_task()
                            .await
                            .map(|_| ()),
                    ),
                    Command::NewSession(cwd, mcp_servers, reply) => {
                        let request = NewSessionRequest::new(cwd).mcp_servers(mcp_servers);
                        answer(
                            reply,
                            connection
                                .send_request(request)
                                .block_task()
                                .await
                                .map(|response| AcpSessionStart {
                                    session_id: response.session_id,
                                    state: session_state(response.modes, response.config_options),
                                }),
                        );
                    }
                    Command::Load(session_id, cwd, mcp_servers, reply) => {
                        if !capabilities.load_session {
                            let _ = reply.send(Err(AcpError::Unsupported("session/load")));
                        } else {
                            let request =
                                LoadSessionRequest::new(session_id, cwd).mcp_servers(mcp_servers);
                            answer(
                                reply,
                                connection.send_request(request).block_task().await.map(
                                    |response| {
                                        session_state(response.modes, response.config_options)
                                    },
                                ),
                            );
                        }
                    }
                    Command::Resume(session_id, cwd, mcp_servers, reply) => {
                        if capabilities.session_capabilities.resume.is_none() {
                            let _ = reply.send(Err(AcpError::Unsupported("session/resume")));
                        } else {
                            let request =
                                ResumeSessionRequest::new(session_id, cwd).mcp_servers(mcp_servers);
                            answer(
                                reply,
                                connection.send_request(request).block_task().await.map(
                                    |response| {
                                        session_state(response.modes, response.config_options)
                                    },
                                ),
                            );
                        }
                    }
                    Command::Prompt(session_id, content, reply) => {
                        let request = PromptRequest::new(session_id, content);
                        let request_connection = connection.clone();
                        connection.spawn(async move {
                            let result = request_connection
                                .send_request(request)
                                .block_task()
                                .await
                                .map(|response| AcpPromptOutcome {
                                    stop_reason: stop_reason(response.stop_reason),
                                });
                            answer(reply, result);
                            Ok(())
                        })?;
                    }
                    Command::Cancel(session_id) => {
                        cancel_permissions(&pending, &session_id);
                        connection.send_notification(CancelNotification::new(session_id))?;
                    }
                    Command::RespondPermission(request_id, selected) => {
                        let permission = pending
                            .lock()
                            .expect("pending permissions poisoned")
                            .remove(&request_id);
                        if let Some(permission) = permission {
                            let selected = selected
                                .map(PermissionOptionId::new)
                                .filter(|option| permission.options.contains(option));
                            let _ = permission.reply.send(selected);
                        }
                    }
                    Command::SetConfig(session_id, config_id, value, reply) => {
                        let request =
                            SetSessionConfigOptionRequest::new(session_id, config_id, value);
                        answer(
                            reply,
                            connection
                                .send_request(request)
                                .block_task()
                                .await
                                .map(|response| response.config_options),
                        );
                    }
                    Command::SetMode(session_id, mode_id, reply) => {
                        let request = SetSessionModeRequest::new(session_id, mode_id);
                        answer(
                            reply,
                            connection
                                .send_request(request)
                                .block_task()
                                .await
                                .map(|_| ()),
                        );
                    }
                    Command::Close(session_id, reply) => {
                        if capabilities.session_capabilities.close.is_none() {
                            let _ = reply.send(Err(AcpError::Unsupported("session/close")));
                        } else {
                            cancel_permissions(&pending, &session_id);
                            answer(
                                reply,
                                connection
                                    .send_request(CloseSessionRequest::new(session_id))
                                    .block_task()
                                    .await
                                    .map(|_| ()),
                            );
                        }
                    }
                    Command::Shutdown => {
                        cancel_all_permissions(&pending);
                        return Ok(());
                    }
                }
            }
            cancel_all_permissions(&pending);
            Ok(())
        })
        .await
}

fn answer<T>(reply: Reply<T>, result: Result<T, agent_client_protocol::Error>) {
    let _ = reply.send(result.map_err(|error| AcpError::Agent(error.to_string())));
}

fn session_state(
    modes: Option<SessionModeState>,
    config_options: Option<Vec<SessionConfigOption>>,
) -> AcpSessionState {
    AcpSessionState {
        modes: modes.and_then(|modes| serde_json::to_value(modes).ok()),
        config_options,
    }
}

fn cancel_permissions(pending: &PendingPermissions, session_id: &SessionId) {
    let mut pending = pending.lock().expect("pending permissions poisoned");
    let request_ids = pending
        .iter()
        .filter(|(_, permission)| &permission.session_id == session_id)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for request_id in request_ids {
        if let Some(permission) = pending.remove(&request_id) {
            let _ = permission.reply.send(None);
        }
    }
}

fn cancel_all_permissions(pending: &PendingPermissions) {
    for (_, permission) in pending
        .lock()
        .expect("pending permissions poisoned")
        .drain()
    {
        let _ = permission.reply.send(None);
    }
}

fn agent_info(response: agent_client_protocol::schema::v1::InitializeResponse) -> AcpAgentInfo {
    AcpAgentInfo {
        protocol_version: response.protocol_version.as_u16(),
        implementation: response.agent_info.map(|info| AcpImplementation {
            name: info.name,
            title: info.title,
            version: info.version,
        }),
        capabilities: serde_json::to_value(response.agent_capabilities).unwrap_or_default(),
        auth_methods: response.auth_methods.into_iter().map(auth_method).collect(),
    }
}

fn auth_method(method: AuthMethod) -> AcpAuthMethod {
    AcpAuthMethod {
        id: method.id().to_string(),
        name: method.name().to_string(),
        description: method.description().map(str::to_string),
    }
}

fn permission_option(
    option: agent_client_protocol::schema::v1::PermissionOption,
) -> AcpPermissionOption {
    use agent_client_protocol::schema::v1::PermissionOptionKind;
    let kind = match option.kind {
        PermissionOptionKind::AllowOnce => AcpPermissionKind::AllowOnce,
        PermissionOptionKind::AllowAlways => AcpPermissionKind::AllowAlways,
        PermissionOptionKind::RejectOnce => AcpPermissionKind::RejectOnce,
        PermissionOptionKind::RejectAlways => AcpPermissionKind::RejectAlways,
        _ => AcpPermissionKind::Unknown,
    };
    AcpPermissionOption {
        id: option.option_id.to_string(),
        name: option.name,
        kind,
    }
}

fn update_kind(update: &SessionUpdate) -> AcpUpdateKind {
    match update {
        SessionUpdate::UserMessageChunk(_) => AcpUpdateKind::UserMessage,
        SessionUpdate::AgentMessageChunk(_) => AcpUpdateKind::AgentMessage,
        SessionUpdate::AgentThoughtChunk(_) => AcpUpdateKind::AgentThought,
        SessionUpdate::ToolCall(_) => AcpUpdateKind::ToolCall,
        SessionUpdate::ToolCallUpdate(_) => AcpUpdateKind::ToolCallUpdate,
        SessionUpdate::Plan(_) => AcpUpdateKind::Plan,
        SessionUpdate::AvailableCommandsUpdate(_) => AcpUpdateKind::AvailableCommands,
        SessionUpdate::CurrentModeUpdate(_) => AcpUpdateKind::CurrentMode,
        SessionUpdate::ConfigOptionUpdate(_) => AcpUpdateKind::ConfigOptions,
        SessionUpdate::SessionInfoUpdate(_) => AcpUpdateKind::SessionInfo,
        SessionUpdate::UsageUpdate(_) => AcpUpdateKind::Usage,
        _ => AcpUpdateKind::Unknown,
    }
}

fn typed_usage_update(update: &SessionUpdate) -> Option<AcpUsageUpdate> {
    let SessionUpdate::UsageUpdate(update) = update else {
        return None;
    };
    Some(AcpUsageUpdate {
        used: update.used,
        size: update.size,
        cost: update.cost.as_ref().map(|cost| AcpUsageCost {
            amount: cost.amount,
            currency: cost.currency.clone(),
        }),
    })
}

fn stop_reason(reason: StopReason) -> AcpStopReason {
    match reason {
        StopReason::EndTurn => AcpStopReason::EndTurn,
        StopReason::MaxTokens => AcpStopReason::MaxTokens,
        StopReason::MaxTurnRequests => AcpStopReason::MaxTurnRequests,
        StopReason::Refusal => AcpStopReason::Refusal,
        StopReason::Cancelled => AcpStopReason::Cancelled,
        _ => AcpStopReason::Unknown,
    }
}

#[derive(Clone)]
struct BoundedStderr {
    bytes: Arc<Mutex<VecDeque<u8>>>,
    limit: usize,
}

impl BoundedStderr {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Arc::new(Mutex::new(VecDeque::with_capacity(limit))),
            limit,
        }
    }

    fn push(&self, line: &[u8]) {
        if self.limit == 0 {
            return;
        }
        let mut bytes = self.bytes.lock().expect("stderr buffer poisoned");
        for byte in line.iter().copied().chain([b'\n']) {
            if bytes.len() == self.limit {
                bytes.pop_front();
            }
            bytes.push_back(byte);
        }
    }

    fn snapshot(&self) -> String {
        let bytes = self.bytes.lock().expect("stderr buffer poisoned");
        let bytes = bytes.iter().copied().collect::<Vec<_>>();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }
}

struct ProcessTransport {
    profile: AcpAgentProfile,
    stderr: BoundedStderr,
}

impl ConnectTo<Client> for ProcessTransport {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> agent_client_protocol::Result<()> {
        let mut command = async_process::Command::new(&self.profile.command);
        command.args(&self.profile.args).envs(&self.profile.env);
        #[cfg(windows)]
        {
            use async_process::windows::CommandExt as _;
            command.creation_flags(0x0800_0000);
        }
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(agent_client_protocol::Error::into_internal_error)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| agent_client_protocol::util::internal_error("missing agent stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| agent_client_protocol::util::internal_error("missing agent stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| agent_client_protocol::util::internal_error("missing agent stderr"))?;

        let captured = self.stderr.clone();
        let child_stderr = self.stderr.clone();
        // Drain stderr on an independent task. Racing the reader inside the
        // transport select dropped it as soon as the protocol or the child
        // ended — sometimes before a just-exited child's diagnostics were
        // read, losing them for good (#179). The pipe hits EOF once the child
        // dies, so the task always terminates.
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Some(line) = lines.next().await {
                if let Ok(line) = line {
                    captured.push(line.as_bytes());
                }
            }
        });
        let incoming = Box::pin(BufReader::new(stdout).lines());
        let outgoing = Box::pin(futures::sink::unfold(
            stdin,
            async move |mut stdin, line: String| {
                stdin.write_all(line.as_bytes()).await?;
                stdin.write_all(b"\n").await?;
                Ok::<_, std::io::Error>(stdin)
            },
        ));
        let protocol = ConnectTo::<Client>::connect_to(Lines::new(outgoing, incoming), client);
        let child_monitor = async move {
            let mut guard = ChildGuard(child);
            let status = guard
                .wait()
                .await
                .map_err(agent_client_protocol::Error::into_internal_error)?;
            if status.success() {
                Ok(())
            } else {
                let stderr = child_stderr.snapshot();
                let detail = if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                };
                Err(agent_client_protocol::util::internal_error(format!(
                    "agent process exited with {status}{detail}"
                )))
            }
        };

        let protocol = pin!(protocol);
        let child_monitor = pin!(child_monitor);
        match futures::future::select(protocol, child_monitor).await {
            futures::future::Either::Left((result, _))
            | futures::future::Either::Right((result, _)) => result,
        }
    }
}

struct ChildGuard(async_process::Child);

impl ChildGuard {
    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.0.status().await
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        drop(self.0.kill());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{AgentCapabilities, InitializeResponse};
    use agent_client_protocol::Channel;

    #[test]
    fn standard_usage_update_is_exposed_as_typed_data() {
        use agent_client_protocol::schema::v1::{Cost, UsageUpdate};

        let update = SessionUpdate::UsageUpdate(
            UsageUpdate::new(53_000, 200_000).cost(Cost::new(0.045, "USD")),
        );
        assert_eq!(
            typed_usage_update(&update),
            Some(AcpUsageUpdate {
                used: 53_000,
                size: 200_000,
                cost: Some(AcpUsageCost {
                    amount: 0.045,
                    currency: "USD".into(),
                }),
            })
        );
    }

    #[tokio::test]
    async fn official_sdk_in_memory_v1_handshake() {
        let (client_transport, agent_transport) = Channel::duplex();
        let agent = tokio::spawn(async move {
            Agent
                .builder()
                .on_receive_request(
                    async move |request: InitializeRequest, responder, _cx| {
                        responder.respond(
                            InitializeResponse::new(request.protocol_version)
                                .agent_capabilities(AgentCapabilities::new()),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(agent_transport)
                .await
        });

        Client
            .builder()
            .connect_with(client_transport, async |connection| {
                let response = connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                assert_eq!(response.protocol_version, ProtocolVersion::V1);
                Ok(())
            })
            .await
            .unwrap();
        agent.abort();
    }
}
