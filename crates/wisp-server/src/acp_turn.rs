//! A conversation run by an ACP agent instead of Wisp's own model: the
//! agent is started in the project folder, keeps its own session, and each
//! message is a prompt to it. Its text, thoughts, tool calls, plan and
//! permission questions come through the same output as a Wisp turn, so
//! the conversation streams, asks and stops the same way.
//!
//! The binding and what is saved are the WebView desktop's own
//! (`acp_sessions`, `frame_acp_agent_selection`, `acp:<title>` tool rows,
//! `wisp:plan` rows), ported from `src-tauri/src/acp.rs` without its MCP
//! bridge, modes, configuration options or sign-in.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use wisp_acp::{
    acp::schema::v1::{
        ContentBlock, McpServer, McpServerHttp, McpServerStdio, SessionId, TextContent,
    },
    AcpError, AcpPermissionKind, AcpPermissionRequest, AcpSessionEvent, AcpSessionHandle,
    AcpUpdateKind,
};
use wisp_core::Output;
use wisp_llm::Message;
use wisp_store::Store;

use crate::{
    acp_agents::{self, AcpAgent},
    output::{tool_question, ServerOutput, Session},
};

/// How long the agent has to start and introduce itself.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);
/// Tool name of a saved plan; outside `acp:` so it is never a tool row.
pub(crate) const PLAN_TOOL_NAME: &str = "wisp:plan";
const TOOL_PREFIX: &str = "acp:";

/// Wisp's tools for the agent this turn, and the bridge that serves them.
pub(crate) struct Offered {
    pub bridge: Arc<crate::acp_bridge::Bridge>,
    pub tools: Vec<Box<dyn wisp_tools::Tool>>,
}

/// How the agent reaches the bridge: over HTTP when it can, through the
/// desktop's relay when it only starts programs, or not at all.
fn bridge_server(handle: &AcpSessionHandle, url: &str) -> Vec<McpServer> {
    let http = handle
        .info()
        .capabilities
        .pointer("/mcpCapabilities/http")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if http {
        return vec![McpServer::Http(McpServerHttp::new(
            crate::acp_bridge::NAME,
            url,
        ))];
    }
    match crate::acp_bridge::relay() {
        Some(relay) => vec![McpServer::Stdio(
            McpServerStdio::new(crate::acp_bridge::NAME, relay.clone())
                .args(vec![crate::acp_bridge::RELAY_FLAG.into(), url.into()]),
        )],
        None => vec![],
    }
}

/// The agent process a conversation is talking to, kept between turns,
/// with the modes and options the agent last reported.
pub(crate) struct AcpLive {
    /// Where the bridge offers Wisp's tools to this agent.
    token: String,
    bridge: Arc<crate::acp_bridge::Bridge>,
    agent: String,
    fingerprint: String,
    cwd: PathBuf,
    session: SessionId,
    handle: Arc<AcpSessionHandle>,
    modes: Option<serde_json::Value>,
    options: serde_json::Value,
    commands: serde_json::Value,
}

/// A command the agent accepts at the start of a message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcpCommand {
    /// Without the slash.
    pub name: String,
    pub description: String,
    /// What to write after it, when it takes something.
    pub hint: String,
}

/// One choice of a mode or an option.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcpChoice {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpValue {
    /// One of a list.
    Choice {
        current: String,
        choices: Vec<AcpChoice>,
    },
    /// On or off.
    Flag(bool),
}

/// A mode or an option the agent offers for the conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpSetting {
    pub id: String,
    pub name: String,
    pub value: AcpValue,
}

/// What the agent running a conversation lets the user set: its mode
/// (planning, asking before edits, and the like) and its options (often
/// the model). Known once the agent has started.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcpControls {
    pub mode: Option<AcpSetting>,
    pub options: Vec<AcpSetting>,
    /// The commands it accepts, as it last listed them.
    pub commands: Vec<AcpCommand>,
}

/// A value to give an option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpPick {
    Choice(String),
    Flag(bool),
}

fn choices(list: &serde_json::Value, id: &str) -> Vec<AcpChoice> {
    let text = |item: &serde_json::Value, key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut found = Vec::new();
    for item in list.as_array().into_iter().flatten() {
        // A group carries its own list.
        if let Some(inner) = item.get("options") {
            found.extend(choices(inner, id));
            continue;
        }
        found.push(AcpChoice {
            id: text(item, id),
            name: text(item, "name"),
            description: text(item, "description"),
        });
    }
    found
}

/// The modes and options as the agent reported them, in the wire shapes.
fn controls_of(live: &AcpLive) -> AcpControls {
    let mut controls = settings_of(live.modes.as_ref(), &live.options);
    let text = |item: &serde_json::Value, key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    controls.commands = live
        .commands
        .as_array()
        .into_iter()
        .flatten()
        .map(|command| AcpCommand {
            name: text(command, "name").trim_start_matches('/').to_owned(),
            description: text(command, "description"),
            hint: command
                .get("input")
                .map(|input| text(input, "hint"))
                .unwrap_or_default(),
        })
        .filter(|command| !command.name.is_empty())
        .collect();
    controls
}

fn settings_of(modes: Option<&serde_json::Value>, options: &serde_json::Value) -> AcpControls {
    let mode = modes.and_then(|modes| {
        let available = choices(modes.get("availableModes")?, "id");
        (!available.is_empty()).then(|| AcpSetting {
            id: "mode".into(),
            name: "Mode".into(),
            value: AcpValue::Choice {
                current: modes
                    .get("currentModeId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                choices: available,
            },
        })
    });
    let options = options
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let id = option.get("id")?.as_str()?.to_owned();
            let name = option
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&id)
                .to_owned();
            let current = option.get("currentValue")?;
            let value = match current {
                serde_json::Value::Bool(on) => AcpValue::Flag(*on),
                current => AcpValue::Choice {
                    current: current.as_str()?.to_owned(),
                    choices: choices(option.get("options")?, "value"),
                },
            };
            Some(AcpSetting { id, name, value })
        })
        .collect();
    AcpControls {
        mode,
        options,
        commands: vec![],
    }
}

/// What the running agent offers to set, when an agent runs the
/// conversation and has started.
pub(crate) async fn controls(session: &Session) -> Option<AcpControls> {
    let slot = session.acp.lock().await;
    let live = slot.as_ref().filter(|live| live.handle.is_alive())?;
    Some(controls_of(live))
}

/// Keeps what the agent reports about its mode and options.
async fn remember(session: &Session, kind: AcpUpdateKind, payload: &serde_json::Value) {
    let mut slot = session.acp.lock().await;
    let Some(live) = slot.as_mut() else {
        return;
    };
    match kind {
        AcpUpdateKind::CurrentMode => {
            if let (Some(modes), Some(current)) =
                (live.modes.as_mut(), payload.get("currentModeId").cloned())
            {
                modes["currentModeId"] = current;
            }
        }
        AcpUpdateKind::ConfigOptions => {
            if let Some(options) = payload.get("configOptions") {
                live.options = options.clone();
            }
        }
        AcpUpdateKind::AvailableCommands => {
            if let Some(commands) = payload.get("availableCommands") {
                live.commands = commands.clone();
            }
        }
        _ => {}
    }
}

/// Puts the running agent in `mode`.
pub(crate) async fn set_mode(session: &Session, mode: &str) -> Result<AcpControls> {
    let mut slot = session.acp.lock().await;
    let live = slot
        .as_mut()
        .filter(|live| live.handle.is_alive())
        .context("The agent is not running; send a message first")?;
    live.handle
        .set_mode(live.session.clone(), mode)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    if let Some(modes) = live.modes.as_mut() {
        modes["currentModeId"] = mode.into();
    }
    Ok(controls_of(live))
}

/// Sets one of the running agent's options.
pub(crate) async fn set_option(
    session: &Session,
    option: &str,
    pick: AcpPick,
) -> Result<AcpControls> {
    let mut slot = session.acp.lock().await;
    let live = slot
        .as_mut()
        .filter(|live| live.handle.is_alive())
        .context("The agent is not running; send a message first")?;
    let value: wisp_acp::acp::schema::v1::SessionConfigOptionValue =
        serde_json::from_value(match pick {
            AcpPick::Choice(value) => serde_json::json!({ "value": value }),
            AcpPick::Flag(on) => serde_json::json!({ "type": "boolean", "value": on }),
        })?;
    let options = live
        .handle
        .set_config(live.session.clone(), option, value)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    live.options = serde_json::to_value(options)?;
    Ok(controls_of(live))
}

/// The same fingerprint the WebView keeps on a binding: a hash of the
/// program and its arguments, so a changed command is noticed.
pub(crate) fn fingerprint(agent: &AcpAgent) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in serde_json::to_vec(&(agent.command.trim(), &agent.args)).unwrap_or_default() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

/// The agent a conversation was started with or set to start with.
pub(crate) async fn chosen(store: &Store, frame: &str) -> Result<Option<String>> {
    if let Some(binding) = store.get_acp_session(frame).await? {
        return Ok(Some(binding.agent_profile_id));
    }
    store.frame_acp_agent_selection(frame).await
}

/// The agent that runs `frame`, when one does. An agent that was removed
/// from the list stops the conversation rather than handing it to Wisp.
pub(crate) async fn agent_of(store: &Store, frame: &str) -> Result<Option<AcpAgent>> {
    let Some(id) = chosen(store, frame).await? else {
        return Ok(None);
    };
    acp_agents::list(store)
        .await
        .into_iter()
        .find(|agent| agent.id == id)
        .map(Some)
        .context("This conversation's ACP agent was removed from Settings; continue in a new conversation")
}

/// Gives a conversation that has no message yet to an ACP agent. The
/// choice is for good: another agent, or Wisp, needs a new conversation.
pub(crate) async fn choose(store: &Store, project: &str, frame: &str, agent: &str) -> Result<()> {
    let current = chosen(store, frame).await?.unwrap_or_default();
    if current == agent {
        return Ok(());
    }
    ensure!(
        current.is_empty(),
        "This conversation keeps its ACP agent; start a new conversation for another"
    );
    ensure!(
        store.max_message_seq(frame).await? == 0,
        "An ACP agent is chosen before the first message"
    );
    ensure!(
        acp_agents::list(store)
            .await
            .iter()
            .any(|kept| kept.id == agent),
        "That agent is gone"
    );
    store
        .set_frame_acp_agent_selection(frame, project, agent)
        .await
}

/// The live session with the agent, starting the program (and resuming
/// its saved session) when there is none.
async fn connect(
    store: &Store,
    session: &Session,
    frame: &str,
    agent: &AcpAgent,
    cwd: &Path,
    offered: Offered,
) -> Result<(Arc<AcpSessionHandle>, SessionId)> {
    let fingerprint = fingerprint(agent);
    let mut slot = session.acp.lock().await;
    if let Some(live) = slot.as_ref() {
        if live.handle.is_alive()
            && live.agent == agent.id
            && live.fingerprint == fingerprint
            && live.cwd == cwd
        {
            // The same address, with the tools as they are now.
            live.bridge.offer(&live.token, cwd, offered.tools);
            return Ok((live.handle.clone(), live.session.clone()));
        }
    }
    // A dead process or a changed agent: start again from the binding.
    if let Some(live) = slot.take() {
        live.bridge.withdraw(&live.token);
        live.handle.shutdown(Duration::from_secs(2)).await;
    }
    let binding = store.get_acp_session(frame).await?;
    match &binding {
        Some(binding) => ensure!(
            binding.agent_profile_id == agent.id
                && binding.profile_fingerprint == fingerprint
                && Path::new(&binding.cwd) == cwd,
            "This ACP agent's program or the project folder changed; continue in a new conversation"
        ),
        None => ensure!(
            store.max_message_seq(frame).await? == 0,
            "An ACP agent can only be given an empty conversation"
        ),
    }
    ensure!(
        acp_agents::found(&agent.command),
        "The agent's program was not found on this computer"
    );
    let handle = tokio::time::timeout(
        LAUNCH_TIMEOUT,
        AcpSessionHandle::launch(acp_agents::profile(agent)),
    )
    .await
    .context("The agent did not start within a minute")?
    .map_err(|error| anyhow::anyhow!("The agent could not be started: {error}"))?;
    let handle = Arc::new(handle);
    let token = uuid::Uuid::new_v4().simple().to_string();
    offered.bridge.offer(&token, cwd, offered.tools);
    let servers = bridge_server(&handle, &offered.bridge.url(&token));
    let (id, state) = match &binding {
        Some(binding) => {
            let id = SessionId::new(binding.agent_session_id.clone());
            match handle
                .resume_session(id.clone(), cwd, servers.clone())
                .await
            {
                Ok(state) => (id, state),
                Err(AcpError::Unsupported(_)) => {
                    match handle.load_session(id.clone(), cwd, servers).await {
                        Ok(state) => (id, state),
                        Err(AcpError::Unsupported(_)) => {
                            bail!("This ACP agent cannot pick up its earlier session")
                        }
                        Err(error) => bail!("{error}"),
                    }
                }
                Err(error) => bail!("{error}"),
            }
        }
        None => {
            let started = handle
                .new_session(cwd, servers)
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            let info = handle.info();
            let now = chrono::Utc::now().timestamp();
            store
                .save_acp_session(&wisp_store::AcpSessionBinding {
                    frame_id: frame.into(),
                    agent_profile_id: agent.id.clone(),
                    profile_fingerprint: fingerprint.clone(),
                    agent_session_id: started.session_id.to_string(),
                    cwd: cwd.to_string_lossy().into_owned(),
                    protocol_version: i64::from(info.protocol_version),
                    agent_info_json: serde_json::to_string(&info.implementation.as_ref().map(
                        |found| {
                            serde_json::json!({
                                "name": found.name,
                                "title": found.title,
                                "version": found.version,
                            })
                        },
                    ))
                    .unwrap_or_default(),
                    capabilities_json: info.capabilities.to_string(),
                    created_at: now,
                    updated_at: now,
                })
                .await?;
            (started.session_id, started.state)
        }
    };
    *slot = Some(AcpLive {
        token,
        bridge: offered.bridge,
        agent: agent.id.clone(),
        fingerprint,
        cwd: cwd.to_path_buf(),
        session: id.clone(),
        handle: handle.clone(),
        modes: state.modes,
        options: state
            .config_options
            .and_then(|options| serde_json::to_value(options).ok())
            .unwrap_or_default(),
        commands: serde_json::Value::Null,
    });
    Ok((handle, id))
}

/// A tool call as it is saved: the WebView's `AcpToolEnvelope`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolEnvelope {
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
    #[serde(default)]
    pub raw_input: String,
    #[serde(default)]
    pub raw_output: String,
}

impl ToolEnvelope {
    fn message(&self) -> Message {
        Message::tool(
            &self.call_id,
            format!("{TOOL_PREFIX}{}", self.title),
            serde_json::to_string(self).unwrap_or_else(|_| "{}".into()),
        )
    }

    /// A saved row's envelope, when it is one.
    pub(crate) fn of(name: Option<&str>, body: &str) -> Option<Self> {
        name?.strip_prefix(TOOL_PREFIX)?;
        let envelope: Self = serde_json::from_str(body).ok()?;
        (envelope.v == 1 && !envelope.call_id.is_empty()).then_some(envelope)
    }
}

fn value_text(value: Option<&serde_json::Value>) -> String {
    match value {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(value) => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

/// Folds a tool call or its update into the turn's list.
fn upsert(tools: &mut Vec<ToolEnvelope>, payload: &serde_json::Value) {
    let Some(id) = payload
        .get("toolCallId")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let at = match tools.iter().position(|tool| tool.call_id == id) {
        Some(at) => at,
        None => {
            tools.push(ToolEnvelope {
                v: 1,
                call_id: id.into(),
                title: "ACP tool".into(),
                status: "pending".into(),
                ..Default::default()
            });
            tools.len() - 1
        }
    };
    let tool = &mut tools[at];
    let text = |key: &str| payload.get(key).and_then(serde_json::Value::as_str);
    if let Some(title) = text("title").filter(|title| !title.trim().is_empty()) {
        tool.title = title.into();
    }
    if let Some(kind) = text("kind") {
        tool.kind = kind.into();
    }
    if let Some(status) = text("status") {
        tool.status = status.into();
    }
    for (key, field) in [
        ("content", &mut tool.content),
        ("locations", &mut tool.locations),
        ("rawInput", &mut tool.raw_input),
        ("rawOutput", &mut tool.raw_output),
    ] {
        if payload.get(key).is_some() {
            *field = value_text(payload.get(key));
        }
    }
}

fn text_of(payload: &serde_json::Value) -> Option<&str> {
    payload
        .get("content")
        .and_then(|content| content.get("text"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| payload.get("text").and_then(serde_json::Value::as_str))
}

/// The option that answers a permission question: allow once (or always
/// when that is all there is), or reject once.
fn option_for(request: &AcpPermissionRequest, allow: bool) -> Option<String> {
    let wanted: &[AcpPermissionKind] = if allow {
        &[AcpPermissionKind::AllowOnce, AcpPermissionKind::AllowAlways]
    } else {
        &[
            AcpPermissionKind::RejectOnce,
            AcpPermissionKind::RejectAlways,
        ]
    };
    wanted.iter().find_map(|kind| {
        request
            .options
            .iter()
            .find(|option| option.kind == *kind)
            .map(|option| option.id.clone())
    })
}

/// What the agent did during one prompt.
#[derive(Default)]
struct Turn {
    text: String,
    thought: String,
    tools: Vec<ToolEnvelope>,
    plan: Option<serde_json::Value>,
}

impl Turn {
    fn apply(&mut self, out: &ServerOutput, kind: AcpUpdateKind, payload: &serde_json::Value) {
        match kind {
            AcpUpdateKind::AgentMessage => {
                if let Some(text) = text_of(payload) {
                    self.text.push_str(text);
                    out.assistant_text(text);
                }
            }
            AcpUpdateKind::AgentThought => {
                if let Some(text) = text_of(payload) {
                    self.thought.push_str(text);
                    out.reasoning(text);
                }
            }
            AcpUpdateKind::ToolCall | AcpUpdateKind::ToolCallUpdate => {
                upsert(&mut self.tools, payload);
                if let Some(tool) = self.tools.iter().find(|tool| {
                    payload
                        .get("toolCallId")
                        .and_then(serde_json::Value::as_str)
                        == Some(tool.call_id.as_str())
                }) {
                    let mut live = out.session.live.lock().unwrap();
                    live.tool = tool.title.clone();
                    live.preview = tool.raw_input.clone();
                }
            }
            AcpUpdateKind::Plan => self.plan = Some(payload.clone()),
            _ => {}
        }
    }
}

/// Answers a permission question: by itself with full access, otherwise
/// by asking the user the way a Wisp tool asks.
async fn answer(
    out: &ServerOutput,
    handle: &AcpSessionHandle,
    request: AcpPermissionRequest,
) -> Result<()> {
    let option = if out.full_access {
        option_for(&request, true)
    } else {
        let title = request
            .tool_call
            .get("title")
            .and_then(serde_json::Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("ACP tool")
            .to_owned();
        {
            let mut live = out.session.live.lock().unwrap();
            live.tool = title.clone();
            live.preview = value_text(request.tool_call.get("rawInput"));
        }
        let decision = out.confirm_decision_async(&tool_question(&title)).await;
        option_for(&request, decision.approved())
    };
    handle
        .respond_permission(request.request_id, option)
        .map_err(|error| anyhow::anyhow!("{error}"))
}

/// One message to the agent, start to end. Everything the agent did is
/// saved when it answers; a stop is passed on to the agent.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    store: Store,
    session: Arc<Session>,
    project: String,
    request: wisp_dto::native_conversations::SendRequest,
    full_access: bool,
    root: PathBuf,
    agent: AcpAgent,
    offered: Offered,
) -> Result<()> {
    let frame = request.session_id.clone();
    let (handle, id) = connect(&store, &session, &frame, &agent, &root, offered).await?;
    let always = crate::runtime::project_grants(&store, &project)
        .await
        .into_iter()
        .collect();
    let (persist, saved) = crate::runtime::persisting(&store, &frame, &session).await?;
    let out = ServerOutput {
        session: session.clone(),
        frame: frame.clone(),
        project,
        turn: request.request_id.clone(),
        persist,
        full_access,
        plan_mode: false,
        always,
    };
    out.on_message(&Message::user(&request.message));
    let prompt = handle.prompt(
        id.clone(),
        vec![ContentBlock::Text(TextContent::new(
            request.message.clone(),
        ))],
    );
    tokio::pin!(prompt);
    let mut turn = Turn::default();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut stop_sent = false;
    let outcome = loop {
        tokio::select! {
            result = &mut prompt => break result.map_err(|error| anyhow::anyhow!("{error}")),
            _ = tick.tick() => {
                if session.cancel.load(Ordering::Relaxed) && !stop_sent {
                    stop_sent = true;
                    let _ = handle.cancel(id.clone());
                }
            }
            event = handle.next_event() => match event {
                Some(AcpSessionEvent::Update { kind, payload, .. }) => {
                    remember(&session, kind, &payload).await;
                    turn.apply(&out, kind, &payload)
                }
                Some(AcpSessionEvent::Permission(request)) => {
                    if let Err(error) = answer(&out, &handle, request).await {
                        break Err(error);
                    }
                }
                Some(AcpSessionEvent::Exited { error }) => {
                    break Err(anyhow::anyhow!(error.unwrap_or_else(|| "The ACP agent exited".into())))
                }
                None => break Err(anyhow::anyhow!("The ACP agent stopped answering")),
            }
        }
    };
    // Updates may race the prompt's answer: take what is already there.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while outcome.is_ok() && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(75), handle.next_event()).await {
            Ok(Some(AcpSessionEvent::Update { kind, payload, .. })) => {
                remember(&session, kind, &payload).await;
                turn.apply(&out, kind, &payload)
            }
            Ok(Some(AcpSessionEvent::Permission(request))) => {
                let _ = handle.respond_permission(request.request_id, None);
            }
            _ => break,
        }
    }
    for tool in &turn.tools {
        out.on_message(&tool.message());
    }
    if let Some(entries) = turn
        .plan
        .as_ref()
        .and_then(|plan| plan.get("entries"))
        .filter(|entries| entries.is_array())
    {
        out.on_message(&Message::tool(
            format!("plan-{}", uuid::Uuid::new_v4()),
            PLAN_TOOL_NAME,
            serde_json::json!({ "v": 1, "source": "acp", "entries": entries }).to_string(),
        ));
    }
    // A turn that answered closes with the agent's message; one that was
    // stopped keeps only what the agent had said.
    let stopped = session.cancel.load(Ordering::Relaxed);
    if !turn.text.is_empty() || !turn.thought.is_empty() || (outcome.is_ok() && !stopped) {
        let mut answer = Message::assistant(turn.text);
        answer.reasoning = (!turn.thought.is_empty()).then_some(turn.thought);
        answer.model_name = Some(agent.label.clone());
        out.on_message(&answer);
    }
    drop(out);
    wisp_store::join_or_abort_persist(saved, Duration::from_secs(10)).await??;
    outcome?;
    ensure!(
        !session.cancel.load(Ordering::Relaxed),
        "Turn stopped by user"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_updates_fold_into_one_saved_row_per_call() {
        let mut tools = Vec::new();
        upsert(
            &mut tools,
            &serde_json::json!({"toolCallId": "t1", "title": "Read data.csv", "kind": "read",
                "status": "in_progress", "rawInput": {"path": "data.csv"}}),
        );
        upsert(
            &mut tools,
            &serde_json::json!({"toolCallId": "t1", "status": "completed", "rawOutput": "3 rows"}),
        );
        upsert(&mut tools, &serde_json::json!({"title": "no id"}));
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool.title, "Read data.csv");
        assert_eq!(tool.status, "completed");
        assert!(tool.raw_input.contains("data.csv"));
        assert_eq!(tool.raw_output, "3 rows");
        let message = tool.message();
        assert_eq!(message.tool_name.as_deref(), Some("acp:Read data.csv"));
        let back = ToolEnvelope::of(message.tool_name.as_deref(), &message.content.as_text());
        assert_eq!(back.as_ref(), Some(tool));
        assert!(ToolEnvelope::of(Some(PLAN_TOOL_NAME), "{}").is_none());
    }

    #[test]
    fn the_fingerprint_follows_the_program_and_its_arguments() {
        let agent = AcpAgent {
            id: "a".into(),
            label: "A".into(),
            command: "agent".into(),
            args: vec!["--acp".into()],
        };
        let renamed = AcpAgent {
            label: "B".into(),
            ..agent.clone()
        };
        assert_eq!(fingerprint(&agent), fingerprint(&renamed));
        let changed = AcpAgent {
            args: vec![],
            ..agent.clone()
        };
        assert_ne!(fingerprint(&agent), fingerprint(&changed));
        assert!(fingerprint(&agent).starts_with("fnv1a64:"));
    }
}
