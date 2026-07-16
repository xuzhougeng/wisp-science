//! External IM channels (Feishu bot, WeChat iLink bot).
//!
//! Inbound text from a channel drives a normal agent session — the turn runs
//! through the same `send_message` path the UI uses, so history, tools,
//! approvals, and persistence all behave identically and the conversation is
//! visible in the desktop app. The final assistant message is sent back to
//! the IM chat when the turn completes.
//!
//! Each IM chat has a durable project/session binding (JSON map in the
//! `channel_sessions` setting). The first ordinary message snapshots the
//! desktop's active project and creates a session there; `/project` and
//! `/session` make that routing explicit and switchable. Non-secret config
//! lives in SQLite settings; the Feishu app secret and WeChat bot token live
//! in the keyring.

pub mod feishu;
pub mod pbbp2;
pub mod weixin;

use crate::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
            feishu::run(app, app_id, secret, status, rx).await;
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

/// Char-boundary-safe cap so an oversized agent answer cannot blow up the
/// IM send API.
fn truncate_reply(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    format!("{cut}\n……(内容过长已截断,完整内容请在桌面端查看)")
}

// ----------------------------------------------- chat ↔ project/session map

const SESSION_MAP_KEY: &str = "channel_sessions";
/// Serializes read-modify-write on the session map across channel workers.
static MAP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One IM conversation's routing target. `session_id=None` means that the next
/// ordinary message should create a fresh session inside `project_id`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct ChatBinding {
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

async fn session_map(store: &Store) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str(&get_setting(store, SESSION_MAP_KEY).await).unwrap_or_default()
}

/// v1 stored each map value as a bare session-id string. Accept it forever and
/// upgrade it to the structured binding on the next write.
fn binding_from_value(value: &serde_json::Value) -> Option<ChatBinding> {
    if let Some(session_id) = value.as_str() {
        return (!session_id.is_empty()).then(|| ChatBinding {
            project_id: None,
            session_id: Some(session_id.to_string()),
        });
    }
    serde_json::from_value(value.clone()).ok()
}

async fn binding_get(store: &Store, key: &str) -> ChatBinding {
    session_map(store)
        .await
        .get(key)
        .and_then(binding_from_value)
        .unwrap_or_default()
}

async fn binding_set(store: &Store, key: &str, binding: &ChatBinding) {
    let _guard = MAP_LOCK.lock().await;
    let mut map = session_map(store).await;
    if binding.project_id.is_none() && binding.session_id.is_none() {
        map.remove(key);
    } else if let Ok(value) = serde_json::to_value(binding) {
        map.insert(key.to_string(), value);
    }
    if let Ok(json) = serde_json::to_string(&map) {
        let _ = store.set_setting(SESSION_MAP_KEY, &json).await;
    }
}

/// Drop deleted targets and make the session owner authoritative. This also
/// upgrades legacy string-only bindings without needing a schema migration.
async fn validated_binding(store: &Store, key: &str) -> Result<ChatBinding, String> {
    let original = binding_get(store, key).await;
    let mut binding = original.clone();
    if let Some(session_id) = binding.session_id.as_deref() {
        match store
            .frame_project_id(session_id)
            .await
            .map_err(|error| error.to_string())?
        {
            Some(owner) => binding.project_id = Some(owner),
            None => binding.session_id = None,
        }
    }
    if binding.session_id.is_none() {
        if let Some(project_id) = binding.project_id.as_deref() {
            if store
                .get_project(project_id)
                .await
                .map_err(|error| error.to_string())?
                .is_none()
            {
                binding.project_id = None;
            }
        }
    }
    if binding != original {
        binding_set(store, key, &binding).await;
    }
    Ok(binding)
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

async fn binding_status_text(store: &Store, binding: &ChatBinding) -> String {
    let Some(project_id) = binding.project_id.as_deref() else {
        return "当前对话尚未绑定项目。首条普通消息会使用桌面端当前项目，也可以先发送 /project 选择。".into();
    };
    let project = project_name(store, project_id).await;
    let session = match binding.session_id.as_deref() {
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
    format!("当前项目: {project}\n当前会话: {session}")
}

async fn last_assistant_text(store: &Store, frame_id: &str) -> Option<String> {
    let msgs = store.load_messages(frame_id).await.ok()?;
    msgs.iter()
        .rev()
        .find(|m| m.role == wisp_llm::Role::Assistant)
        .map(|m| m.content.as_text())
        .filter(|t| !t.trim().is_empty())
}

const HELP_TEXT: &str = "可用命令:\n/status — 查看当前项目和会话\n/project — 列出项目\n/project <序号|名称|ID> — 切换项目\n/session — 列出当前项目的最近会话\n/session <序号|标题|ID> — 切换会话\n/new — 在当前项目开启新会话\n/stop — 停止当前任务\n/help — 显示本帮助\n\n首次发送普通消息时，会在桌面端当前项目创建会话；此后该 IM 对话会固定连接到它，直到使用上述命令切换。";

/// Route one inbound IM text: chat commands are handled locally, everything
/// else drives an agent turn. Returns the reply to send back (may be empty).
pub(crate) async fn handle_inbound(
    app: &AppHandle,
    channel: &str,
    chat_key: &str,
    text: &str,
) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let state = app.state::<AppState>();
    let map_key = format!("{channel}:{chat_key}");
    let turn_lock = chat_turn_lock(&map_key);
    let _turn_guard = turn_lock.lock().await;
    let mut parts = text.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default().to_ascii_lowercase();
    let argument = parts.next().unwrap_or_default().trim();
    match command.as_str() {
        "/help" => return HELP_TEXT.to_string(),
        "/status" => {
            return match validated_binding(&state.store, &map_key).await {
                Ok(binding) => binding_status_text(&state.store, &binding).await,
                Err(error) => format!("读取当前绑定失败: {error}"),
            };
        }
        "/project" | "/projects" => {
            let mut binding = match validated_binding(&state.store, &map_key).await {
                Ok(binding) => binding,
                Err(error) => return format!("读取当前绑定失败: {error}"),
            };
            let choices = match project_choices(&state.store).await {
                Ok(choices) => choices,
                Err(error) => return format!("读取项目失败: {error}"),
            };
            if argument.is_empty() {
                return format_project_list(&choices, binding.project_id.as_deref());
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
            binding.project_id = Some(selected.id.clone());
            binding.session_id = None;
            binding_set(&state.store, &map_key, &binding).await;
            return format!(
                "已切换到项目“{}”。下一条普通消息会在这里创建新会话；也可发送 /session 选择已有会话。",
                selected.name
            );
        }
        "/session" | "/sessions" => {
            let mut binding = match validated_binding(&state.store, &map_key).await {
                Ok(binding) => binding,
                Err(error) => return format!("读取当前绑定失败: {error}"),
            };
            if binding.project_id.is_none() {
                binding.project_id = Some(state.active("main").id);
                binding_set(&state.store, &map_key, &binding).await;
            }
            if argument.eq_ignore_ascii_case("new") {
                binding.session_id = None;
                let project = project_name(
                    &state.store,
                    binding.project_id.as_deref().unwrap_or_default(),
                )
                .await;
                binding_set(&state.store, &map_key, &binding).await;
                return format!("已准备在项目“{project}”中开启新会话。请发送下一条普通消息。");
            }
            let project_id = binding.project_id.as_deref().unwrap_or_default();
            let choices = match session_choices(&state.store, project_id).await {
                Ok(choices) => choices,
                Err(error) => return format!("读取会话失败: {error}"),
            };
            if argument.is_empty() {
                let project = project_name(&state.store, project_id).await;
                return format!(
                    "项目: {project}\n{}",
                    format_session_list(&choices, binding.session_id.as_deref())
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
            binding.session_id = Some(selected.id.clone());
            binding_set(&state.store, &map_key, &binding).await;
            return format!(
                "已切换到会话“{}” · {}。后续消息会继续这个会话。",
                selected.title,
                short_id(&selected.id)
            );
        }
        "/new" => {
            let mut binding = match validated_binding(&state.store, &map_key).await {
                Ok(binding) => binding,
                Err(error) => return format!("读取当前绑定失败: {error}"),
            };
            if binding.project_id.is_none() {
                binding.project_id = Some(state.active("main").id);
            }
            binding.session_id = None;
            let project = project_name(
                &state.store,
                binding.project_id.as_deref().unwrap_or_default(),
            )
            .await;
            binding_set(&state.store, &map_key, &binding).await;
            return format!("已准备在项目“{project}”中开启新会话。请发送下一条普通消息。");
        }
        "/stop" => {
            let binding = match validated_binding(&state.store, &map_key).await {
                Ok(binding) => binding,
                Err(error) => return format!("读取当前绑定失败: {error}"),
            };
            let Some(frame_id) = binding.session_id else {
                return "当前没有关联的会话。".to_string();
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
    // The first ordinary message snapshots the desktop's active project. From
    // that point onward the binding is independent of desktop navigation.
    let mut binding = match validated_binding(&state.store, &map_key).await {
        Ok(binding) => binding,
        Err(error) => return format!("读取当前绑定失败: {error}"),
    };
    if binding.project_id.is_none() {
        binding.project_id = Some(state.active("main").id);
    }
    if binding.session_id.is_none() {
        let project_id = binding.project_id.as_deref().unwrap_or_default();
        let frame_id = match crate::create_session_frame(&state.store, project_id).await {
            Ok(frame_id) => frame_id,
            Err(error) => return format!("创建会话失败: {error}"),
        };
        binding.session_id = Some(frame_id);
        // Persist before running the turn so a provider error does not make a
        // retry fan out into another empty session.
        binding_set(&state.store, &map_key, &binding).await;
    }
    let session_id = binding.session_id.clone().unwrap_or_default();
    // The routing decision is now durable. Release the short critical section
    // before the long agent turn so a later `/stop` can interrupt it; a second
    // ordinary message will reuse this session and serialize on its runtime.
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
    Ok(ChannelsStatus {
        feishu_enabled: get_setting(&state.store, "feishu_enabled").await == "true",
        feishu_app_id: get_setting(&state.store, "feishu_app_id").await,
        feishu_has_secret: !load_secret(FEISHU_SECRET).await.is_empty(),
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
    app_id: String,
    app_secret: String,
) -> Result<(), String> {
    let app_id = app_id.trim().to_string();
    state
        .store
        .set_setting("feishu_app_id", &app_id)
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

fn qr_svg_data_url(content: &str) -> Result<String, String> {
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
    fn legacy_session_value_becomes_structured_binding() {
        let binding = binding_from_value(&serde_json::json!("session-1")).unwrap();
        assert_eq!(
            binding,
            ChatBinding {
                project_id: None,
                session_id: Some("session-1".into()),
            }
        );
        assert_eq!(
            binding_from_value(&serde_json::json!({
                "project_id": "project-1",
                "session_id": "session-2"
            })),
            Some(ChatBinding {
                project_id: Some("project-1".into()),
                session_id: Some("session-2".into()),
            })
        );
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
    async fn validates_and_upgrades_legacy_binding_owner() {
        let path = std::env::temp_dir().join(format!(
            "wisp_channels_binding_{}.sqlite",
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
            .set_setting(SESSION_MAP_KEY, r#"{"feishu:chat-1":"session-1"}"#)
            .await
            .unwrap();

        let binding = validated_binding(&store, "feishu:chat-1").await.unwrap();
        assert_eq!(binding.project_id.as_deref(), Some("project-1"));
        assert_eq!(binding.session_id.as_deref(), Some("session-1"));
        let persisted: serde_json::Value =
            serde_json::from_str(&store.get_setting(SESSION_MAP_KEY).await.unwrap().unwrap())
                .unwrap();
        assert_eq!(persisted["feishu:chat-1"]["project_id"], "project-1");

        store
            .delete_session("session-1", "project-1")
            .await
            .unwrap();
        let binding = validated_binding(&store, "feishu:chat-1").await.unwrap();
        assert_eq!(binding.project_id.as_deref(), Some("project-1"));
        assert_eq!(binding.session_id, None);
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
