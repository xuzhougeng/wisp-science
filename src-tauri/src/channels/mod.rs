//! External IM channels (Feishu bot, WeChat iLink bot).
//!
//! Inbound text from a channel drives a normal agent session — the turn runs
//! through the same `send_message` path the UI uses, so history, tools,
//! approvals, and persistence all behave identically and the conversation is
//! visible in the desktop app. The final assistant message is sent back to
//! the IM chat when the turn completes.
//!
//! Desktop, Feishu, and WeChat share one durable last-message route. An ordinary
//! IM message always continues the session that most recently accepted a user
//! message on any of those surfaces; `/project`, `/session`, and `/new` can
//! explicitly move that shared target. Non-secret config lives in SQLite
//! settings; the Feishu app secret and WeChat bot token live in the keyring.

pub mod feishu;
pub mod feishu_card;
pub mod feishu_registration;
pub mod pbbp2;
pub mod weixin;

use crate::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use tauri::{AppHandle, Manager, State};
use tokio::sync::watch;
use wisp_store::secrets::Secret;
use wisp_store::Store;

const FEISHU_SECRET: &str = "feishu_app_secret";
const WEIXIN_TOKEN_SECRET: &str = "weixin_bot_token";
/// ponytail: single reply cap for both channels; per-channel limits if an API
/// ever rejects shorter messages.
const REPLY_MAX_CHARS: usize = 8000;

// ------------------------------------------------------------------- manager

#[derive(Clone, Default)]
pub struct ChannelStatus {
    /// "stopped" | "connecting" | "running" | "error"
    pub state: String,
    pub detail: String,
}

pub(crate) fn set_status(status: &Arc<StdMutex<ChannelStatus>>, state: &str, detail: &str) {
    let mut guard = status.lock().unwrap();
    guard.state = state.to_string();
    guard.detail = detail.to_string();
}

fn status_snapshot(status: &Arc<StdMutex<ChannelStatus>>) -> ChannelStatus {
    status.lock().unwrap().clone()
}

#[derive(Default)]
pub struct ChannelManager {
    feishu: StdMutex<Option<watch::Sender<bool>>>,
    weixin: StdMutex<Option<watch::Sender<bool>>>,
    feishu_status: Arc<StdMutex<ChannelStatus>>,
    weixin_status: Arc<StdMutex<ChannelStatus>>,
    feishu_registrations:
        tokio::sync::Mutex<HashMap<String, feishu_registration::RegistrationFlow>>,
}

impl ChannelManager {
    pub fn new() -> Self {
        let mgr = Self::default();
        set_status(&mgr.feishu_status, "stopped", "");
        set_status(&mgr.weixin_status, "stopped", "");
        mgr
    }

    pub fn stop_feishu(&self) {
        if let Some(tx) = self.feishu.lock().unwrap().take() {
            let _ = tx.send(true);
        }
        set_status(&self.feishu_status, "stopped", "");
    }

    pub fn stop_weixin(&self) {
        if let Some(tx) = self.weixin.lock().unwrap().take() {
            let _ = tx.send(true);
        }
        set_status(&self.weixin_status, "stopped", "");
    }

    pub async fn start_feishu(&self, app: &AppHandle) {
        self.stop_feishu();
        let state = app.state::<AppState>();
        let app_id = get_setting(&state.store, "feishu_app_id").await;
        let secret = load_secret(FEISHU_SECRET).await;
        let international = get_setting(&state.store, "feishu_international").await == "true";
        if app_id.is_empty() || secret.is_empty() {
            set_status(
                &self.feishu_status,
                "error",
                "请先填写 App ID 与 App Secret",
            );
            return;
        }
        let (tx, rx) = watch::channel(false);
        *self.feishu.lock().unwrap() = Some(tx);
        let status = self.feishu_status.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            feishu::run(app, app_id, secret, international, status, rx).await;
        });
    }

    pub async fn start_weixin(&self, app: &AppHandle) {
        self.stop_weixin();
        let state = app.state::<AppState>();
        let binding = load_weixin_binding(&state.store).await;
        let token = load_secret(WEIXIN_TOKEN_SECRET).await;
        let Some(binding) = binding else {
            set_status(&self.weixin_status, "error", "请先扫码绑定微信");
            return;
        };
        if token.is_empty() {
            set_status(&self.weixin_status, "error", "登录凭证缺失,请重新扫码绑定");
            return;
        }
        let (tx, rx) = watch::channel(false);
        *self.weixin.lock().unwrap() = Some(tx);
        let status = self.weixin_status.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            weixin::run(app, binding, token, status, rx).await;
        });
    }
}

/// Start whichever channels the user left enabled. Called once at app launch.
pub async fn autostart(app: AppHandle) {
    let state = app.state::<AppState>();
    let mgr = app.state::<ChannelManager>();
    if get_setting(&state.store, "feishu_enabled").await == "true" {
        mgr.start_feishu(&app).await;
    }
    if get_setting(&state.store, "weixin_enabled").await == "true" {
        mgr.start_weixin(&app).await;
    }
}

// ------------------------------------------------------------------- helpers

pub(crate) async fn get_setting(store: &Store, key: &str) -> String {
    store
        .get_setting(key)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

async fn load_secret(name: &'static str) -> String {
    tokio::task::spawn_blocking(move || Secret::get(name).unwrap_or_default())
        .await
        .unwrap_or_default()
}

async fn load_weixin_binding(store: &Store) -> Option<weixin::Binding> {
    serde_json::from_str(&get_setting(store, "weixin_binding").await).ok()
}

// --------------------------------------------- live agent progress observers

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProgressEvent {
    AssistantDelta(String),
    Activity,
    ToolStarted(String),
    ToolFinished {
        name: String,
        ok: bool,
        duration_ms: u64,
    },
}

type ProgressSubscribers =
    HashMap<String, Vec<(u64, tokio::sync::mpsc::UnboundedSender<ProgressEvent>)>>;

static PROGRESS_SUBSCRIBERS: OnceLock<StdMutex<ProgressSubscribers>> = OnceLock::new();
static PENDING_PROGRESS: OnceLock<
    StdMutex<HashMap<u64, tokio::sync::mpsc::UnboundedSender<ProgressEvent>>>,
> = OnceLock::new();
static NEXT_PROGRESS_SUBSCRIBER: AtomicU64 = AtomicU64::new(1);

pub(crate) struct ProgressSubscription {
    frame_id: String,
    id: u64,
}

struct PendingProgress {
    id: u64,
}

impl PendingProgress {
    fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for PendingProgress {
    fn drop(&mut self) {
        if let Some(pending) = PENDING_PROGRESS.get() {
            pending.lock().unwrap().remove(&self.id);
        }
    }
}

fn prepare_progress_observer(
    sender: tokio::sync::mpsc::UnboundedSender<ProgressEvent>,
) -> PendingProgress {
    let id = NEXT_PROGRESS_SUBSCRIBER.fetch_add(1, Ordering::Relaxed);
    PENDING_PROGRESS
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(id, sender);
    PendingProgress { id }
}

/// Activate an observer only after `send_message` owns the target session's
/// runtime lock. This prevents a queued IM turn from receiving progress events
/// produced by an earlier desktop or IM turn on the same session.
pub(crate) fn activate_progress_observer(id: u64, frame_id: &str) -> Option<ProgressSubscription> {
    let sender = PENDING_PROGRESS
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .remove(&id)?;
    Some(subscribe_agent_events(frame_id, sender))
}

impl Drop for ProgressSubscription {
    fn drop(&mut self) {
        let Some(registry) = PROGRESS_SUBSCRIBERS.get() else {
            return;
        };
        let mut registry = registry.lock().unwrap();
        if let Some(entries) = registry.get_mut(&self.frame_id) {
            entries.retain(|(id, _)| *id != self.id);
            if entries.is_empty() {
                registry.remove(&self.frame_id);
            }
        }
    }
}

fn subscribe_agent_events(
    frame_id: &str,
    sender: tokio::sync::mpsc::UnboundedSender<ProgressEvent>,
) -> ProgressSubscription {
    let id = NEXT_PROGRESS_SUBSCRIBER.fetch_add(1, Ordering::Relaxed);
    PROGRESS_SUBSCRIBERS
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .entry(frame_id.to_string())
        .or_default()
        .push((id, sender));
    ProgressSubscription {
        frame_id: frame_id.to_string(),
        id,
    }
}

/// Forward only safe-to-project progress events. The Feishu renderer never
/// receives raw reasoning text or tool output; it maps these events to coarse
/// activity labels in `feishu_card`.
pub(crate) fn publish_agent_event(event: &crate::AgentEvent) {
    let (frame_id, progress) = match event {
        crate::AgentEvent::Text { frame_id, delta } => {
            (frame_id, ProgressEvent::AssistantDelta(delta.clone()))
        }
        crate::AgentEvent::Reasoning { frame_id, .. }
        | crate::AgentEvent::Stdout { frame_id, .. } => (frame_id, ProgressEvent::Activity),
        crate::AgentEvent::ToolCall { frame_id, name, .. } => {
            (frame_id, ProgressEvent::ToolStarted(name.clone()))
        }
        crate::AgentEvent::ToolResult {
            frame_id,
            name,
            ok,
            duration_ms,
            ..
        } => (
            frame_id,
            ProgressEvent::ToolFinished {
                name: name.clone(),
                ok: *ok,
                duration_ms: *duration_ms,
            },
        ),
        _ => return,
    };
    let Some(registry) = PROGRESS_SUBSCRIBERS.get() else {
        return;
    };
    let mut registry = registry.lock().unwrap();
    if let Some(entries) = registry.get_mut(frame_id) {
        entries.retain(|(_, sender)| sender.send(progress.clone()).is_ok());
        if entries.is_empty() {
            registry.remove(frame_id);
        }
    }
}

/// Char-boundary-safe cap so an oversized agent answer cannot blow up the
/// IM send API.
fn truncate_reply(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    format!("{cut}\n……(内容过长已截断,完整内容请在桌面端查看)")
}

// ------------------------------------------------ shared last-message route

const LAST_MESSAGE_ROUTE_KEY: &str = "channel_last_message_route";
/// Serializes route validation, explicit switches, first-session creation, and
/// accepted-send updates across the desktop, Feishu, and WeChat.
static ROUTE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The shared routing target. `session_id=None` is an explicit `/project` or
/// `/new` selection: the next ordinary message creates a session there.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct SharedRoute {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProjectChoice {
    id: String,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionChoice {
    id: String,
    title: String,
}

async fn route_get_unlocked(store: &Store) -> SharedRoute {
    serde_json::from_str(&get_setting(store, LAST_MESSAGE_ROUTE_KEY).await).unwrap_or_default()
}

async fn route_set_unlocked(store: &Store, route: &SharedRoute) -> Result<(), String> {
    let json = serde_json::to_string(route).map_err(|error| error.to_string())?;
    store
        .set_setting(LAST_MESSAGE_ROUTE_KEY, &json)
        .await
        .map_err(|error| error.to_string())
}

/// Validate the persisted target. On first run after upgrading, recover the
/// most recently inserted user message from the transcript store. This is only
/// a cold-start fallback; every subsequent accepted send writes the exact route.
async fn validated_route_unlocked(store: &Store) -> Result<SharedRoute, String> {
    let original = route_get_unlocked(store).await;
    let mut route = original.clone();
    if let Some(session_id) = route.session_id.as_deref() {
        match store
            .frame_project_id(session_id)
            .await
            .map_err(|error| error.to_string())?
        {
            Some(owner) => route.project_id = Some(owner),
            None => {
                route.project_id = None;
                route.session_id = None;
            }
        }
    }
    if route.session_id.is_none() {
        if let Some(project_id) = route.project_id.as_deref() {
            if store
                .get_project(project_id)
                .await
                .map_err(|error| error.to_string())?
                .is_none()
            {
                route.project_id = None;
            }
        }
    }
    if route.project_id.is_none() && route.session_id.is_none() {
        if let Some((session_id, project_id)) = store
            .last_user_message_session()
            .await
            .map_err(|error| error.to_string())?
        {
            route = SharedRoute {
                project_id: Some(project_id),
                session_id: Some(session_id),
            };
        }
    }
    if route != original {
        route_set_unlocked(store, &route).await?;
    }
    Ok(route)
}

async fn validated_route(store: &Store) -> Result<SharedRoute, String> {
    let _guard = ROUTE_LOCK.lock().await;
    validated_route_unlocked(store).await
}

async fn set_route(store: &Store, route: &SharedRoute) -> Result<(), String> {
    let _guard = ROUTE_LOCK.lock().await;
    route_set_unlocked(store, route).await
}

/// Called by the shared desktop/channel `send_message` path after validation,
/// but before waiting for the destination session's turn lock. A queued user
/// message therefore changes the route immediately, matching send order rather
/// than eventual execution order.
pub(crate) async fn record_last_message_session(
    store: &Store,
    frame_id: &str,
) -> Result<(), String> {
    let _guard = ROUTE_LOCK.lock().await;
    let project_id = store
        .frame_project_id(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Session '{frame_id}' no longer exists."))?;
    route_set_unlocked(
        store,
        &SharedRoute {
            project_id: Some(project_id),
            session_id: Some(frame_id.to_string()),
        },
    )
    .await
}

/// Resolve an ordinary IM message to the shared target. Holding `ROUTE_LOCK`
/// across validation and creation makes the no-history case linearizable: a
/// Feishu message and a WeChat message arriving together reuse one new frame.
async fn resolve_message_session(
    store: &Store,
    default_project_id: &str,
) -> Result<String, String> {
    let _route_guard = ROUTE_LOCK.lock().await;
    let mut route = validated_route_unlocked(store).await?;
    if route.project_id.is_none() {
        route.project_id = Some(default_project_id.to_string());
    }
    if route.session_id.is_none() {
        let project_id = route.project_id.as_deref().unwrap_or_default();
        let frame_id = crate::create_session_frame(store, project_id)
            .await
            .map_err(|error| format!("创建会话失败: {error}"))?;
        route.session_id = Some(frame_id);
        // Persist before running the turn so a provider error does not make a
        // retry fan out into another empty session.
        route_set_unlocked(store, &route)
            .await
            .map_err(|error| format!("保存共享路由失败: {error}"))?;
    }
    Ok(route.session_id.unwrap_or_default())
}

/// Serialize turns per IM conversation so two near-simultaneous first messages
/// cannot each create their own session. Different chats still run in parallel.
fn chat_turn_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    static CHAT_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    CHAT_LOCKS
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

async fn project_choices(store: &Store) -> Result<Vec<ProjectChoice>, String> {
    store
        .list_projects()
        .await
        .map_err(|error| error.to_string())
        .map(|rows| {
            rows.into_iter()
                .map(|(id, name, ..)| ProjectChoice {
                    id,
                    name: if name.trim().is_empty() {
                        "未命名项目".into()
                    } else {
                        name
                    },
                })
                .collect()
        })
}

async fn session_choices(store: &Store, project_id: &str) -> Result<Vec<SessionChoice>, String> {
    store
        .list_sessions(project_id)
        .await
        .map_err(|error| error.to_string())
        .map(|rows| {
            rows.into_iter()
                .map(|(id, title, ..)| SessionChoice { id, title })
                .collect()
        })
}

fn select_choice<'a, T>(
    choices: &'a [T],
    selector: &str,
    id: impl Fn(&T) -> &str,
    label: impl Fn(&T) -> &str,
    kind: &str,
) -> Result<&'a T, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err(format!("请提供{kind}序号、名称或 ID。"));
    }
    if let Ok(number) = selector.parse::<usize>() {
        return number
            .checked_sub(1)
            .and_then(|index| choices.get(index))
            .ok_or_else(|| format!("没有序号为 {number} 的{kind}。"));
    }
    if let Some(choice) = choices
        .iter()
        .find(|choice| id(choice).eq_ignore_ascii_case(selector))
    {
        return Ok(choice);
    }
    let label_matches: Vec<&T> = choices
        .iter()
        .filter(|choice| label(choice).eq_ignore_ascii_case(selector))
        .collect();
    match label_matches.as_slice() {
        [choice] => return Ok(*choice),
        [_, _, ..] => return Err(format!("找到多个同名{kind}，请使用序号或 ID。")),
        _ => {}
    }
    let selector_lower = selector.to_ascii_lowercase();
    let id_matches: Vec<&T> = choices
        .iter()
        .filter(|choice| id(choice).to_ascii_lowercase().starts_with(&selector_lower))
        .collect();
    match id_matches.as_slice() {
        [choice] => Ok(*choice),
        [_, _, ..] => Err(format!("ID 前缀不唯一，请输入更长的{kind} ID。")),
        _ => Err(format!("未找到{kind}“{selector}”。")),
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn format_project_list(choices: &[ProjectChoice], current: Option<&str>) -> String {
    if choices.is_empty() {
        return "还没有可用项目，请先在桌面端创建项目。".into();
    }
    const LIMIT: usize = 20;
    let mut lines = vec!["项目列表:".to_string()];
    for (index, choice) in choices.iter().take(LIMIT).enumerate() {
        let marker = if current == Some(choice.id.as_str()) {
            " ← 当前"
        } else {
            ""
        };
        lines.push(format!(
            "{}. {} · {}{}",
            index + 1,
            choice.name,
            short_id(&choice.id),
            marker
        ));
    }
    if choices.len() > LIMIT {
        lines.push(format!("…另有 {} 个项目未显示", choices.len() - LIMIT));
    }
    lines.push("发送 /project <序号|名称|ID> 切换项目。".into());
    lines.join("\n")
}

fn format_session_list(choices: &[SessionChoice], current: Option<&str>) -> String {
    if choices.is_empty() {
        return "当前项目还没有已有会话。发送普通消息即可创建，或发送 /new。".into();
    }
    const LIMIT: usize = 10;
    let mut lines = vec!["最近会话:".to_string()];
    for (index, choice) in choices.iter().take(LIMIT).enumerate() {
        let marker = if current == Some(choice.id.as_str()) {
            " ← 当前"
        } else {
            ""
        };
        lines.push(format!(
            "{}. {} · {}{}",
            index + 1,
            choice.title,
            short_id(&choice.id),
            marker
        ));
    }
    if choices.len() > LIMIT {
        lines.push(format!("…另有 {} 个会话未显示", choices.len() - LIMIT));
    }
    lines.push("发送 /session <序号|标题|ID> 切换，/new 开启新会话。".into());
    lines.join("\n")
}

async fn project_name(store: &Store, project_id: &str) -> String {
    store
        .get_project(project_id)
        .await
        .ok()
        .flatten()
        .map(|(name, _)| name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| short_id(project_id))
}

async fn route_status_text(store: &Store, route: &SharedRoute) -> String {
    let Some(project_id) = route.project_id.as_deref() else {
        return "还没有最近发言会话。下一条普通消息会使用桌面端当前项目，也可以先发送 /project 选择。".into();
    };
    let project = project_name(store, project_id).await;
    let session = match route.session_id.as_deref() {
        Some(session_id) => {
            let title = store
                .get_session_reference(session_id)
                .await
                .ok()
                .flatten()
                .map(|item| item.title)
                .unwrap_or_else(|| "新会话".into());
            format!("{title} · {}", short_id(session_id))
        }
        None => "新会话（下一条普通消息创建）".into(),
    };
    format!("共享目标项目: {project}\n最近发言会话: {session}")
}

/// The turn's answer for IM delivery. Agent turns normally finish via the
/// `attempt_completion` tool, so the real answer is that tool result — the
/// desktop promotes it into the assistant bubble the same way. Fall back to
/// the last non-empty assistant text for turns that end in plain text.
async fn last_assistant_text(store: &Store, frame_id: &str) -> Option<String> {
    let msgs = store.load_messages(frame_id).await.ok()?;
    msgs.iter()
        .rev()
        .filter(|m| {
            m.role == wisp_llm::Role::Assistant
                || (m.role == wisp_llm::Role::Tool
                    && m.tool_name.as_deref() == Some("attempt_completion"))
        })
        .map(|m| m.content.as_text())
        .find(|t| !t.trim().is_empty())
}

const HELP_TEXT: &str = "可用命令:\n/status — 查看共享的最近发言会话\n/project — 列出项目\n/project <序号|名称|ID> — 切换项目\n/session — 列出当前项目的最近会话\n/session <序号|标题|ID> — 切换会话\n/new — 在当前项目开启新会话\n/stop — 停止当前任务\n/help — 显示本帮助\n\n桌面端、微信和飞书共用同一个路由目标：普通消息始终继续最近一次实际发送过用户消息的 session。/project、/session 和 /new 会显式切换这个共享目标。";

/// Route one inbound IM text: chat commands are handled locally, everything
/// else drives an agent turn. Returns the reply to send back (may be empty).
pub(crate) async fn handle_inbound(
    app: &AppHandle,
    channel: &str,
    chat_key: &str,
    text: &str,
) -> String {
    handle_inbound_observed(app, channel, chat_key, text, None).await
}

/// Feishu uses the optional observer to project the same live events shown in
/// the desktop transcript into a rate-limited CardKit progress card. WeChat
/// continues to call the simpler wrapper above.
pub(crate) async fn handle_inbound_observed(
    app: &AppHandle,
    channel: &str,
    chat_key: &str,
    text: &str,
    progress: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let state = app.state::<AppState>();
    let chat_lock_key = format!("{channel}:{chat_key}");
    let turn_lock = chat_turn_lock(&chat_lock_key);
    let _turn_guard = turn_lock.lock().await;
    let mut parts = text.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default().to_ascii_lowercase();
    let argument = parts.next().unwrap_or_default().trim();
    match command.as_str() {
        "/help" => return HELP_TEXT.to_string(),
        "/status" => {
            return match validated_route(&state.store).await {
                Ok(route) => route_status_text(&state.store, &route).await,
                Err(error) => format!("读取共享路由失败: {error}"),
            };
        }
        "/project" | "/projects" => {
            let mut route = match validated_route(&state.store).await {
                Ok(route) => route,
                Err(error) => return format!("读取共享路由失败: {error}"),
            };
            let choices = match project_choices(&state.store).await {
                Ok(choices) => choices,
                Err(error) => return format!("读取项目失败: {error}"),
            };
            if argument.is_empty() {
                return format_project_list(&choices, route.project_id.as_deref());
            }
            let selected = match select_choice(
                &choices,
                argument,
                |choice| choice.id.as_str(),
                |choice| choice.name.as_str(),
                "项目",
            ) {
                Ok(selected) => selected,
                Err(error) => return error,
            };
            route.project_id = Some(selected.id.clone());
            route.session_id = None;
            if let Err(error) = set_route(&state.store, &route).await {
                return format!("切换共享路由失败: {error}");
            }
            return format!(
                "共享目标已切换到项目“{}”。下一条微信或飞书普通消息会在这里创建新会话；也可发送 /session 选择已有会话。",
                selected.name
            );
        }
        "/session" | "/sessions" => {
            let mut route = match validated_route(&state.store).await {
                Ok(route) => route,
                Err(error) => return format!("读取共享路由失败: {error}"),
            };
            if route.project_id.is_none() {
                route.project_id = Some(state.active("main").id);
            }
            if argument.eq_ignore_ascii_case("new") {
                route.session_id = None;
                let project = project_name(
                    &state.store,
                    route.project_id.as_deref().unwrap_or_default(),
                )
                .await;
                if let Err(error) = set_route(&state.store, &route).await {
                    return format!("切换共享路由失败: {error}");
                }
                return format!(
                    "已准备在项目“{project}”中开启共享的新会话。请从微信或飞书发送下一条普通消息。"
                );
            }
            let project_id = route.project_id.as_deref().unwrap_or_default();
            let choices = match session_choices(&state.store, project_id).await {
                Ok(choices) => choices,
                Err(error) => return format!("读取会话失败: {error}"),
            };
            if argument.is_empty() {
                let project = project_name(&state.store, project_id).await;
                return format!(
                    "项目: {project}\n{}",
                    format_session_list(&choices, route.session_id.as_deref())
                );
            }
            let selected = match select_choice(
                &choices,
                argument,
                |choice| choice.id.as_str(),
                |choice| choice.title.as_str(),
                "会话",
            ) {
                Ok(selected) => selected,
                Err(error) => return error,
            };
            route.session_id = Some(selected.id.clone());
            if let Err(error) = set_route(&state.store, &route).await {
                return format!("切换共享路由失败: {error}");
            }
            return format!(
                "共享目标已切换到会话“{}” · {}。后续微信和飞书消息都会继续这个会话。",
                selected.title,
                short_id(&selected.id)
            );
        }
        "/new" => {
            let mut route = match validated_route(&state.store).await {
                Ok(route) => route,
                Err(error) => return format!("读取共享路由失败: {error}"),
            };
            if route.project_id.is_none() {
                route.project_id = Some(state.active("main").id);
            }
            route.session_id = None;
            let project = project_name(
                &state.store,
                route.project_id.as_deref().unwrap_or_default(),
            )
            .await;
            if let Err(error) = set_route(&state.store, &route).await {
                return format!("切换共享路由失败: {error}");
            }
            return format!(
                "已准备在项目“{project}”中开启共享的新会话。请从微信或飞书发送下一条普通消息。"
            );
        }
        "/stop" => {
            let route = match validated_route(&state.store).await {
                Ok(route) => route,
                Err(error) => return format!("读取共享路由失败: {error}"),
            };
            let Some(frame_id) = route.session_id else {
                return "当前没有最近发言会话。".to_string();
            };
            return match crate::stop_agent(app.state(), Some(frame_id)).await {
                Ok(()) => "已请求停止当前任务。".to_string(),
                Err(e) => format!("停止失败:{e}"),
            };
        }
        _ => {}
    }

    if text.starts_with('/') {
        return format!("未知命令“{command}”。发送 /help 查看可用命令。");
    }

    let Some(window) = app.get_webview_window("main") else {
        return "桌面端主窗口不可用,无法处理消息。".to_string();
    };
    // Resolve and, when needed, create the shared target atomically.
    let session_id = match resolve_message_session(&state.store, &state.active("main").id).await {
        Ok(session_id) => session_id,
        Err(error) => return format!("路由消息失败: {error}"),
    };
    let progress = progress.map(prepare_progress_observer);
    // The routing decision is now durable. Release the short critical section
    // before the long agent turn so a later `/stop` can interrupt it. The
    // destination runtime serializes subsequent turns in this same session.
    drop(_turn_guard);
    let result = crate::send_message(
        app.state(),
        app.clone(),
        window,
        Some(session_id.clone()),
        text.to_string(),
        None,
        None,
        None,
        None,
        progress.as_ref().map(PendingProgress::id),
        None,
        None,
    )
    .await;
    match result {
        Ok(frame_id) => match last_assistant_text(&state.store, &frame_id).await {
            Some(text) => truncate_reply(&text, REPLY_MAX_CHARS),
            None => "(本轮完成,但没有文本回复)".to_string(),
        },
        Err(e) => format!("处理失败:{e}"),
    }
}

// ------------------------------------------------------------------ commands

/// Everything the settings pane needs, mirrored in `ui/src/dto.rs`
/// (snake_case, same style as `Settings`).
#[derive(Serialize)]
pub struct ChannelsStatus {
    pub feishu_enabled: bool,
    pub feishu_bound: bool,
    pub feishu_international: bool,
    pub feishu_app_id: String,
    pub feishu_has_secret: bool,
    pub feishu_state: String,
    pub feishu_detail: String,
    pub weixin_enabled: bool,
    pub weixin_bound: bool,
    pub weixin_state: String,
    pub weixin_detail: String,
}

#[tauri::command]
pub(crate) async fn channels_status(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
) -> Result<ChannelsStatus, String> {
    let feishu = status_snapshot(&mgr.feishu_status);
    let weixin = status_snapshot(&mgr.weixin_status);
    let feishu_app_id = get_setting(&state.store, "feishu_app_id").await;
    let feishu_has_secret = !load_secret(FEISHU_SECRET).await.is_empty();
    Ok(ChannelsStatus {
        feishu_enabled: get_setting(&state.store, "feishu_enabled").await == "true",
        feishu_bound: !feishu_app_id.is_empty() && feishu_has_secret,
        feishu_international: get_setting(&state.store, "feishu_international").await == "true",
        feishu_app_id,
        feishu_has_secret,
        feishu_state: feishu.state,
        feishu_detail: feishu.detail,
        weixin_enabled: get_setting(&state.store, "weixin_enabled").await == "true",
        weixin_bound: load_weixin_binding(&state.store).await.is_some(),
        weixin_state: weixin.state,
        weixin_detail: weixin.detail,
    })
}

#[tauri::command]
pub(crate) async fn set_feishu_channel(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
    app: AppHandle,
    enabled: bool,
    international: bool,
    app_id: String,
    app_secret: String,
) -> Result<(), String> {
    let app_id = app_id.trim().to_string();
    state
        .store
        .set_setting("feishu_app_id", &app_id)
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting(
            "feishu_international",
            if international { "true" } else { "false" },
        )
        .await
        .map_err(|e| e.to_string())?;
    let secret_input = app_secret.trim().to_string();
    if !secret_input.is_empty() {
        // Empty input means "keep the stored secret", mirroring set_settings.
        tokio::task::spawn_blocking(move || Secret::set(FEISHU_SECRET, &secret_input))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    }
    if enabled && (app_id.is_empty() || load_secret(FEISHU_SECRET).await.is_empty()) {
        return Err("启用前请先填写 App ID 与 App Secret。".into());
    }
    state
        .store
        .set_setting("feishu_enabled", if enabled { "true" } else { "false" })
        .await
        .map_err(|e| e.to_string())?;
    if enabled {
        mgr.start_feishu(&app).await;
    } else {
        mgr.stop_feishu();
    }
    Ok(())
}

#[derive(Serialize)]
pub struct FeishuBindStart {
    /// Opaque backend flow id. Device codes never cross into the webview.
    pub flow_id: String,
    /// data: URL of the registration verification QR image.
    pub qr_image: String,
    pub expires_in_seconds: u64,
}

#[derive(Serialize)]
pub struct FeishuBindPoll {
    /// "pending" | "confirmed" | "denied" | "expired"
    pub state: String,
    pub retry_after_ms: u64,
    pub app_id: String,
}

#[tauri::command]
pub(crate) async fn feishu_bind_start(
    mgr: State<'_, ChannelManager>,
    international: bool,
) -> Result<FeishuBindStart, String> {
    let started = feishu_registration::RegistrationFlow::begin(international)
        .await
        .map_err(|error| format!("{error:#}"))?;
    let flow_id = uuid::Uuid::new_v4().to_string();
    let qr_image = qr_svg_data_url(&started.verification_uri)?;
    let mut flows = mgr.feishu_registrations.lock().await;
    flows.retain(|_, flow| !flow.expired());
    flows.insert(flow_id.clone(), started.flow);
    Ok(FeishuBindStart {
        flow_id,
        qr_image,
        expires_in_seconds: started.expires_in_seconds,
    })
}

#[tauri::command]
pub(crate) async fn feishu_bind_poll(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
    app: AppHandle,
    flow_id: String,
) -> Result<FeishuBindPoll, String> {
    let result = {
        let mut flows = mgr.feishu_registrations.lock().await;
        let flow = flows
            .get_mut(&flow_id)
            .ok_or_else(|| "飞书扫码流程不存在或已结束,请重新扫码。".to_string())?;
        let result = flow.poll().await.map_err(|error| format!("{error:#}"))?;
        if !matches!(
            &result,
            feishu_registration::RegistrationPoll::Pending { .. }
        ) {
            flows.remove(&flow_id);
        }
        result
    };

    match result {
        feishu_registration::RegistrationPoll::Pending { retry_after } => Ok(FeishuBindPoll {
            state: "pending".into(),
            retry_after_ms: retry_after.as_millis().min(u64::MAX as u128) as u64,
            app_id: String::new(),
        }),
        feishu_registration::RegistrationPoll::Denied => Ok(FeishuBindPoll {
            state: "denied".into(),
            retry_after_ms: 0,
            app_id: String::new(),
        }),
        feishu_registration::RegistrationPoll::Expired => Ok(FeishuBindPoll {
            state: "expired".into(),
            retry_after_ms: 0,
            app_id: String::new(),
        }),
        feishu_registration::RegistrationPoll::Success {
            app_id,
            app_secret,
            international,
        } => {
            let stored_secret = app_secret;
            tokio::task::spawn_blocking(move || Secret::set(FEISHU_SECRET, &stored_secret))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            state
                .store
                .set_setting("feishu_app_id", &app_id)
                .await
                .map_err(|e| e.to_string())?;
            state
                .store
                .set_setting(
                    "feishu_international",
                    if international { "true" } else { "false" },
                )
                .await
                .map_err(|e| e.to_string())?;
            if get_setting(&state.store, "feishu_enabled").await == "true" {
                mgr.start_feishu(&app).await;
            }
            Ok(FeishuBindPoll {
                state: "confirmed".into(),
                retry_after_ms: 0,
                app_id,
            })
        }
    }
}

#[tauri::command]
pub(crate) async fn feishu_bind_cancel(
    mgr: State<'_, ChannelManager>,
    flow_id: String,
) -> Result<(), String> {
    mgr.feishu_registrations.lock().await.remove(&flow_id);
    Ok(())
}

#[tauri::command]
pub(crate) async fn feishu_unbind(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
) -> Result<(), String> {
    mgr.stop_feishu();
    mgr.feishu_registrations.lock().await.clear();
    let _ = tokio::task::spawn_blocking(|| Secret::delete(FEISHU_SECRET)).await;
    state
        .store
        .set_setting("feishu_app_id", "")
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("feishu_enabled", "false")
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn set_weixin_channel(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    if enabled && load_weixin_binding(&state.store).await.is_none() {
        return Err("启用前请先扫码绑定微信。".into());
    }
    state
        .store
        .set_setting("weixin_enabled", if enabled { "true" } else { "false" })
        .await
        .map_err(|e| e.to_string())?;
    if enabled {
        mgr.start_weixin(&app).await;
    } else {
        mgr.stop_weixin();
    }
    Ok(())
}

#[derive(Serialize)]
pub struct WeixinBindStart {
    /// Opaque id to poll `weixin_bind_poll` with.
    pub qrcode: String,
    /// data: URL of the QR image to render.
    pub qr_image: String,
}

pub(crate) fn qr_svg_data_url(content: &str) -> Result<String, String> {
    use base64::Engine;
    let code = qrcode::QrCode::new(content.as_bytes()).map_err(|e| e.to_string())?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    ))
}

#[tauri::command]
pub(crate) async fn weixin_bind_start() -> Result<WeixinBindStart, String> {
    let client = weixin::IlinkClient::new("", "").map_err(|e| e.to_string())?;
    let qr = client.get_qrcode().await.map_err(|e| e.to_string())?;
    Ok(WeixinBindStart {
        qr_image: qr_svg_data_url(&qr.qrcode_img_content)?,
        qrcode: qr.qrcode,
    })
}

/// Poll the scan status; on "confirmed" the binding is persisted (token in
/// the keyring) and the channel restarts if it was enabled.
#[tauri::command]
pub(crate) async fn weixin_bind_poll(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
    app: AppHandle,
    qrcode: String,
) -> Result<String, String> {
    let client = weixin::IlinkClient::new("", "").map_err(|e| e.to_string())?;
    let st = client
        .qrcode_status(&qrcode)
        .await
        .map_err(|e| e.to_string())?;
    if st.ret != 0 {
        return Err(format!("查询扫码状态失败:ret={}", st.ret));
    }
    if st.status != "confirmed" {
        return Ok(st.status);
    }
    if st.bot_token.is_empty() {
        return Err("扫码确认成功,但服务端未返回登录凭证。".into());
    }
    let token = st.bot_token.clone();
    tokio::task::spawn_blocking(move || Secret::set(WEIXIN_TOKEN_SECRET, &token))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let binding = weixin::Binding {
        user_id: st.ilink_user_id,
        account_id: st.ilink_bot_id,
        base_url: st.baseurl,
        bound_at: chrono::Utc::now().to_rfc3339(),
    };
    state
        .store
        .set_setting(
            "weixin_binding",
            &serde_json::to_string(&binding).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| e.to_string())?;
    // Fresh binding → fresh cursor: never replay another login's backlog.
    state
        .store
        .set_setting("weixin_sync_buf", "")
        .await
        .map_err(|e| e.to_string())?;
    if get_setting(&state.store, "weixin_enabled").await == "true" {
        mgr.start_weixin(&app).await;
    }
    Ok("confirmed".into())
}

#[tauri::command]
pub(crate) async fn weixin_unbind(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
) -> Result<(), String> {
    mgr.stop_weixin();
    let _ = tokio::task::spawn_blocking(|| Secret::delete(WEIXIN_TOKEN_SECRET)).await;
    state
        .store
        .set_setting("weixin_binding", "")
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("weixin_sync_buf", "")
        .await
        .map_err(|e| e.to_string())?;
    state
        .store
        .set_setting("weixin_enabled", "false")
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_route_serializes_project_and_session() {
        let route = SharedRoute {
            project_id: Some("project-1".into()),
            session_id: Some("session-1".into()),
        };
        let json = serde_json::to_string(&route).unwrap();
        assert_eq!(serde_json::from_str::<SharedRoute>(&json).unwrap(), route);
    }

    #[test]
    fn selectors_accept_number_name_and_unique_id_prefix() {
        let projects = vec![
            ProjectChoice {
                id: "aaa11111-full".into(),
                name: "Alpha".into(),
            },
            ProjectChoice {
                id: "bbb22222-full".into(),
                name: "Beta".into(),
            },
        ];
        let select = |value| {
            select_choice(
                &projects,
                value,
                |choice| choice.id.as_str(),
                |choice| choice.name.as_str(),
                "项目",
            )
            .map(|choice| choice.id.as_str())
        };
        assert_eq!(select("2"), Ok("bbb22222-full"));
        assert_eq!(select("alpha"), Ok("aaa11111-full"));
        assert_eq!(select("bbb2"), Ok("bbb22222-full"));
        assert!(select("3").unwrap_err().contains("没有序号"));
        assert!(select("missing").unwrap_err().contains("未找到"));
    }

    #[test]
    fn help_and_lists_expose_project_session_routing() {
        assert!(HELP_TEXT.contains("/status"));
        assert!(HELP_TEXT.contains("/project"));
        assert!(HELP_TEXT.contains("/session"));
        assert!(HELP_TEXT.contains("微信和飞书共用同一个路由目标"));
        let projects = vec![ProjectChoice {
            id: "project-123456".into(),
            name: "Alpha".into(),
        }];
        let sessions = vec![SessionChoice {
            id: "session-123456".into(),
            title: "Analysis".into(),
        }];
        assert!(format_project_list(&projects, Some("project-123456")).contains("← 当前"));
        assert!(format_session_list(&sessions, Some("session-123456")).contains("← 当前"));
    }

    #[tokio::test]
    async fn shared_route_follows_the_last_sent_session_and_recovers_after_delete() {
        let path = std::env::temp_dir().join(format!(
            "wisp_channels_last_route_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("project-1", "Alpha", "/workspace/alpha")
            .await
            .unwrap();
        store
            .create_frame("session-1", "project-1", "OPERON", "wisp")
            .await
            .unwrap();
        store
            .create_frame("session-2", "project-1", "OPERON", "wisp")
            .await
            .unwrap();
        store
            .append_message("session-1", 1, &wisp_llm::Message::user("first"))
            .await
            .unwrap();
        store
            .append_message("session-2", 1, &wisp_llm::Message::user("second"))
            .await
            .unwrap();

        // Cold-start migration discovers the latest actual user message.
        let route = validated_route(&store).await.unwrap();
        assert_eq!(route.session_id.as_deref(), Some("session-2"));

        // The same writer is called by desktop, Feishu, and WeChat turn starts;
        // there is intentionally no channel/chat key in the persisted value.
        record_last_message_session(&store, "session-1")
            .await
            .unwrap();
        let route = validated_route(&store).await.unwrap();
        assert_eq!(route.project_id.as_deref(), Some("project-1"));
        assert_eq!(route.session_id.as_deref(), Some("session-1"));

        store
            .delete_session("session-1", "project-1")
            .await
            .unwrap();
        let route = validated_route(&store).await.unwrap();
        assert_eq!(route.session_id.as_deref(), Some("session-2"));
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn explicit_new_route_is_not_replaced_by_transcript_fallback() {
        let path = std::env::temp_dir().join(format!(
            "wisp_channels_pending_route_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("project-1", "Alpha", "/workspace/alpha")
            .await
            .unwrap();
        store
            .create_frame("old-session", "project-1", "OPERON", "wisp")
            .await
            .unwrap();
        store
            .append_message("old-session", 1, &wisp_llm::Message::user("old"))
            .await
            .unwrap();
        set_route(
            &store,
            &SharedRoute {
                project_id: Some("project-1".into()),
                session_id: None,
            },
        )
        .await
        .unwrap();

        let route = validated_route(&store).await.unwrap();
        assert_eq!(route.project_id.as_deref(), Some("project-1"));
        assert_eq!(route.session_id, None);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn simultaneous_feishu_and_wechat_first_messages_share_one_session() {
        let path = std::env::temp_dir().join(format!(
            "wisp_channels_concurrent_first_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("project-1", "Alpha", "/workspace/alpha")
            .await
            .unwrap();

        let (feishu, wechat) = tokio::join!(
            resolve_message_session(&store, "project-1"),
            resolve_message_session(&store, "project-1")
        );
        let feishu = feishu.unwrap();
        let wechat = wechat.unwrap();
        assert_eq!(feishu, wechat);
        assert_eq!(store.list_root_frames("project-1").await.unwrap().len(), 1);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn reply_promotes_attempt_completion_result_like_the_desktop() {
        let path = std::env::temp_dir().join(format!(
            "wisp_channels_reply_promotion_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("project-1", "Alpha", "/workspace/alpha")
            .await
            .unwrap();
        store
            .create_frame("session-1", "project-1", "OPERON", "wisp")
            .await
            .unwrap();
        store
            .append_message("session-1", 1, &wisp_llm::Message::user("分析一下"))
            .await
            .unwrap();
        // Tool-driven turn: the assistant stub next to the tool call is tiny,
        // the real answer is the attempt_completion result.
        store
            .append_message(
                "session-1",
                2,
                &wisp_llm::Message::assistant("我来分析一下"),
            )
            .await
            .unwrap();
        store
            .append_message(
                "session-1",
                3,
                &wisp_llm::Message::tool("tc-1", "attempt_completion", "完整的分析结论"),
            )
            .await
            .unwrap();
        assert_eq!(
            last_assistant_text(&store, "session-1").await.as_deref(),
            Some("完整的分析结论")
        );
        // Other tool results are never promoted.
        store
            .append_message(
                "session-1",
                4,
                &wisp_llm::Message::tool("tc-2", "shell", "raw tool output"),
            )
            .await
            .unwrap();
        assert_eq!(
            last_assistant_text(&store, "session-1").await.as_deref(),
            Some("完整的分析结论")
        );
        // A later plain-text turn wins over the earlier completion.
        store
            .append_message(
                "session-1",
                5,
                &wisp_llm::Message::assistant("后续普通回复"),
            )
            .await
            .unwrap();
        assert_eq!(
            last_assistant_text(&store, "session-1").await.as_deref(),
            Some("后续普通回复")
        );
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn truncate_reply_is_char_boundary_safe() {
        assert_eq!(truncate_reply("短文本", 8000), "短文本");
        let long = "好".repeat(9000);
        let cut = truncate_reply(&long, 8000);
        assert!(cut.starts_with(&"好".repeat(10)));
        assert!(cut.contains("已截断"));
        assert!(cut.chars().count() < 8100);
    }

    #[test]
    fn progress_observer_redacts_reasoning_and_tool_output() {
        let frame_id = format!("progress-{}", uuid::Uuid::new_v4());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let pending = prepare_progress_observer(sender);

        publish_agent_event(&crate::AgentEvent::Text {
            frame_id: frame_id.clone(),
            delta: "earlier queued turn".into(),
        });
        assert!(receiver.try_recv().is_err());

        let subscription = activate_progress_observer(pending.id(), &frame_id).unwrap();

        publish_agent_event(&crate::AgentEvent::Reasoning {
            frame_id: frame_id.clone(),
            delta: "private chain of thought".into(),
        });
        assert_eq!(receiver.try_recv(), Ok(ProgressEvent::Activity));

        publish_agent_event(&crate::AgentEvent::ToolResult {
            frame_id: frame_id.clone(),
            name: "shell".into(),
            ok: true,
            content: "SECRET=do-not-forward".into(),
            duration_ms: 42,
        });
        assert_eq!(
            receiver.try_recv(),
            Ok(ProgressEvent::ToolFinished {
                name: "shell".into(),
                ok: true,
                duration_ms: 42,
            })
        );

        drop(subscription);
        drop(pending);
        publish_agent_event(&crate::AgentEvent::Text {
            frame_id,
            delta: "ignored".into(),
        });
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn qr_svg_data_url_produces_svg() {
        use base64::Engine;
        let url = qr_svg_data_url("https://example.com/bind").unwrap();
        let b64 = url.strip_prefix("data:image/svg+xml;base64,").unwrap();
        let svg = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap(),
        )
        .unwrap();
        assert!(svg.contains("<svg"));
    }
}
