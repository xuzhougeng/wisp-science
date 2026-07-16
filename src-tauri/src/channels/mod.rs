//! External IM channels (Feishu bot, WeChat iLink bot).
//!
//! Inbound text from a channel drives a normal agent session — the turn runs
//! through the same `send_message` path the UI uses, so history, tools,
//! approvals, and persistence all behave identically and the conversation is
//! visible in the desktop app. The final assistant message is sent back to
//! the IM chat when the turn completes.
//!
//! Each IM chat maps to one session frame (JSON map in the `channel_sessions`
//! setting); `/new` resets the mapping. Non-secret config lives in SQLite
//! settings; the Feishu app secret and WeChat bot token live in the keyring.

pub mod feishu;
pub mod pbbp2;
pub mod weixin;

use crate::AppState;
use serde::Serialize;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
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

// -------------------------------------------------- chat ↔ session frame map

const SESSION_MAP_KEY: &str = "channel_sessions";
/// Serializes read-modify-write on the session map across channel workers.
static MAP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn session_map(store: &Store) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str(&get_setting(store, SESSION_MAP_KEY).await).unwrap_or_default()
}

async fn session_map_get(store: &Store, key: &str) -> Option<String> {
    session_map(store)
        .await
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

async fn session_map_set(store: &Store, key: &str, frame_id: Option<&str>) {
    let _guard = MAP_LOCK.lock().await;
    let mut map = session_map(store).await;
    match frame_id {
        Some(id) => {
            map.insert(key.to_string(), serde_json::Value::String(id.to_string()));
        }
        None => {
            map.remove(key);
        }
    }
    if let Ok(json) = serde_json::to_string(&map) {
        let _ = store.set_setting(SESSION_MAP_KEY, &json).await;
    }
}

async fn last_assistant_text(store: &Store, frame_id: &str) -> Option<String> {
    let msgs = store.load_messages(frame_id).await.ok()?;
    msgs.iter()
        .rev()
        .find(|m| m.role == wisp_llm::Role::Assistant)
        .map(|m| m.content.as_text())
        .filter(|t| !t.trim().is_empty())
}

const HELP_TEXT: &str = "可用命令:\n/new — 开启新会话\n/stop — 停止当前任务\n/help — 显示本帮助\n其余消息会直接交给 AI 助手处理。";

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
    match text {
        "/help" => return HELP_TEXT.to_string(),
        "/new" => {
            session_map_set(&state.store, &map_key, None).await;
            return "已重置,下一条消息将开启新会话。".to_string();
        }
        "/stop" => {
            let Some(frame_id) = session_map_get(&state.store, &map_key).await else {
                return "当前没有关联的会话。".to_string();
            };
            return match crate::stop_agent(app.state(), Some(frame_id)).await {
                Ok(()) => "已请求停止当前任务。".to_string(),
                Err(e) => format!("停止失败:{e}"),
            };
        }
        _ => {}
    }

    // Reuse the mapped frame while it still exists; otherwise start fresh.
    let mapped = session_map_get(&state.store, &map_key).await;
    let session_id = match &mapped {
        Some(id)
            if state
                .store
                .frame_project_id(id)
                .await
                .ok()
                .flatten()
                .is_some() =>
        {
            Some(id.clone())
        }
        _ => None,
    };
    let Some(window) = app.get_webview_window("main") else {
        return "桌面端主窗口不可用,无法处理消息。".to_string();
    };
    let result = crate::send_message(
        app.state(),
        app.clone(),
        window,
        session_id.clone(),
        text.to_string(),
        None,
        None,
        None,
        None,
    )
    .await;
    match result {
        Ok(frame_id) => {
            if session_id.is_none() {
                session_map_set(&state.store, &map_key, Some(&frame_id)).await;
            }
            match last_assistant_text(&state.store, &frame_id).await {
                Some(text) => truncate_reply(&text, REPLY_MAX_CHARS),
                None => "(本轮完成,但没有文本回复)".to_string(),
            }
        }
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
