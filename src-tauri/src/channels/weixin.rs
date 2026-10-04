//! WeChat channel over the official iLink bot API (`ilinkai.weixin.qq.com`).
//!
//! Shape mirrors phantty's tested implementation: QR-scan binding yields a
//! `bot_token`; `getupdates` long-polls with an opaque cursor
//! (`get_updates_buf`); `sendmessage` replies as text. The scanning user is
//! the owner — only their 1:1 messages are handled; group messages are
//! dropped. `errcode == -14` means the session expired and the user must
//! re-scan. Replies must go out within ~30 min of the inbound message
//! (`context_token` window).

use super::{set_status, weixin_keys, ChannelStatus, WeixinDestination};
use anyhow::{bail, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, watch, RwLock};
use tokio::task::JoinSet;

#[derive(Clone)]
struct AssistantNotification {
    binding: Binding,
    project_id: String,
    text: String,
    approval: Option<crate::ConfirmRequest>,
}

fn assistant_notifications() -> &'static tokio::sync::broadcast::Sender<AssistantNotification> {
    static NOTIFICATIONS: std::sync::OnceLock<
        tokio::sync::broadcast::Sender<AssistantNotification>,
    > = std::sync::OnceLock::new();
    NOTIFICATIONS.get_or_init(|| tokio::sync::broadcast::channel(64).0)
}

pub(crate) fn notify_assistant(binding: Binding, project_id: String, text: String) {
    let _ = assistant_notifications().send(AssistantNotification {
        binding,
        project_id,
        text,
        approval: None,
    });
}

pub(crate) fn notify_assistant_approval(
    binding: Binding,
    project_id: String,
    request: crate::ConfirmRequest,
) {
    let text = super::assistant::approval_message(&request);
    let _ = assistant_notifications().send(AssistantNotification {
        binding,
        project_id,
        text,
        approval: Some(request),
    });
}

fn notification_matches(notification: &AssistantNotification, binding: &Binding) -> bool {
    !binding.user_id.is_empty()
        && notification.binding.user_id == binding.user_id
        && notification.binding.account_id == binding.account_id
}

fn queue_notification(
    outbox: &mut std::collections::VecDeque<AssistantNotification>,
    binding: &Binding,
    notification: AssistantNotification,
) {
    if notification_matches(&notification, binding) {
        if notification.approval.is_some() {
            outbox.push_front(notification);
        } else {
            outbox.push_back(notification);
        }
    }
}

pub const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const CHANNEL_VERSION: &str = "1.0.2";
const BOT_TYPE: &str = "3";
const SESSION_EXPIRED_ERRCODE: i64 = -14;

// ------------------------------------------------------------------- wire types

#[derive(Deserialize, Default)]
pub struct QrCode {
    #[serde(default)]
    pub ret: i64,
    /// Opaque QR session id — poll `get_qrcode_status` with it.
    #[serde(default)]
    pub qrcode: String,
    /// The string to render as a QR image (not an image itself).
    #[serde(default)]
    pub qrcode_img_content: String,
}

#[derive(Deserialize, Default)]
pub struct QrStatus {
    #[serde(default)]
    pub ret: i64,
    /// "wait" | "scaned" | "confirmed" | "expired"
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub baseurl: String,
    #[serde(default)]
    pub ilink_bot_id: String,
    #[serde(default)]
    pub ilink_user_id: String,
}

#[derive(Deserialize, Default)]
pub struct Updates {
    #[serde(default)]
    pub ret: i64,
    #[serde(default)]
    pub errcode: i64,
    #[serde(default)]
    pub longpolling_timeout_ms: i64,
    #[serde(default)]
    pub get_updates_buf: String,
    #[serde(default)]
    pub msgs: Vec<Msg>,
}

#[derive(Deserialize, Default)]
pub struct Msg {
    #[serde(default)]
    pub from_user_id: String,
    #[serde(default)]
    pub to_user_id: String,
    #[serde(default)]
    pub context_token: String,
    #[serde(default)]
    pub group_id: String,
    #[serde(default)]
    pub item_list: Vec<Item>,
}

#[derive(Deserialize, Default)]
pub struct Item {
    #[serde(rename = "type", default)]
    pub kind: i64,
    #[serde(default)]
    pub text_item: Option<TextPayload>,
    #[serde(default)]
    pub voice_item: Option<TextPayload>,
}

#[derive(Deserialize, Default)]
pub struct TextPayload {
    #[serde(default)]
    pub text: String,
}

/// Text worth handling: plain text items (type 1) plus voice transcripts
/// (type 3), concatenated.
pub fn extract_text(msg: &Msg) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for item in &msg.item_list {
        let payload = match item.kind {
            1 => item.text_item.as_ref(),
            3 => item.voice_item.as_ref(),
            _ => None,
        };
        if let Some(p) = payload {
            if !p.text.trim().is_empty() {
                parts.push(p.text.trim());
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// Persisted (non-secret) binding metadata; the bot token lives in the keyring.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Binding {
    /// The user who scanned — the only accepted sender.
    pub user_id: String,
    /// The bot's own account id (echo filter + recipient check).
    pub account_id: String,
    pub base_url: String,
    pub bound_at: String,
}

pub fn should_handle(msg: &Msg, binding: &Binding) -> bool {
    if !msg.group_id.is_empty() || msg.from_user_id.is_empty() {
        return false;
    }
    if msg.from_user_id == binding.account_id {
        return false; // our own echo
    }
    if !binding.user_id.is_empty() && msg.from_user_id != binding.user_id {
        return false; // not the owner
    }
    if !binding.account_id.is_empty()
        && !msg.to_user_id.is_empty()
        && msg.to_user_id != binding.account_id
    {
        return false; // addressed to another bot
    }
    true
}

// ------------------------------------------------------------------ HTTP client

pub struct IlinkClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl IlinkClient {
    pub fn new(base_url: &str, token: &str) -> Result<Self> {
        Ok(Self {
            // getupdates long-polls ~35s server-side; leave headroom.
            http: reqwest::Client::builder()
                .user_agent("wisp-science")
                .timeout(Duration::from_secs(75))
                .build()?,
            base_url: if base_url.is_empty() {
                DEFAULT_BASE_URL.to_string()
            } else {
                base_url.to_string()
            },
            token: token.to_string(),
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        // X-WECHAT-UIN: base64 of a random uint decimal string (request
        // fingerprint the servers expect; mirrors the reference bridges).
        let uin = base64::engine::general_purpose::STANDARD
            .encode((uuid::Uuid::new_v4().as_u128() as u32).to_string());
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .header("AuthorizationType", "ilink_bot_token")
            .header("X-WECHAT-UIN", uin);
        if !self.token.is_empty() {
            req = req.bearer_auth(&self.token);
        }
        req
    }

    pub async fn get_qrcode(&self) -> Result<QrCode> {
        let qr: QrCode = self
            .request(
                reqwest::Method::GET,
                &format!("/ilink/bot/get_bot_qrcode?bot_type={BOT_TYPE}"),
            )
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if qr.ret != 0 || qr.qrcode.is_empty() || qr.qrcode_img_content.is_empty() {
            bail!("get_bot_qrcode failed: ret={}", qr.ret);
        }
        Ok(qr)
    }

    pub async fn qrcode_status(&self, qrcode: &str) -> Result<QrStatus> {
        let encoded: String = url::form_urlencoded::byte_serialize(qrcode.as_bytes()).collect();
        Ok(self
            .request(
                reqwest::Method::GET,
                &format!("/ilink/bot/get_qrcode_status?qrcode={encoded}"),
            )
            .header("iLink-App-ClientVersion", "1")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    pub async fn get_updates(&self, buf: &str) -> Result<Updates> {
        Ok(self
            .request(reqwest::Method::POST, "/ilink/bot/getupdates")
            .json(&json!({
                "get_updates_buf": buf,
                "base_info": {"channel_version": CHANNEL_VERSION},
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    pub async fn send_text(&self, to_user_id: &str, text: &str, context_token: &str) -> Result<()> {
        let client_id = format!("wisp-weixin-{}", uuid::Uuid::new_v4().simple());
        let resp: serde_json::Value = self
            .request(reqwest::Method::POST, "/ilink/bot/sendmessage")
            .json(&json!({
                "msg": {
                    "to_user_id": to_user_id,
                    "client_id": client_id,
                    "message_type": 2,
                    "message_state": 2,
                    "context_token": context_token,
                    "item_list": [{"type": 1, "text_item": {"text": text}}],
                },
                "base_info": {"channel_version": CHANNEL_VERSION},
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let ret = resp.get("ret").and_then(|r| r.as_i64()).unwrap_or(0);
        if ret != 0 {
            bail!(
                "sendmessage failed: ret={ret} errcode={}",
                resp.get("errcode").and_then(|e| e.as_i64()).unwrap_or(0)
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- channel loop

#[derive(Debug)]
struct InboundText {
    from_user_id: String,
    text: String,
}

fn is_control_text(text: &str, destination: WeixinDestination) -> bool {
    match destination {
        WeixinDestination::Projects => text.trim_start().starts_with('/'),
        // Every assistant slash command is a control text except `/resume`,
        // which runs a turn.
        WeixinDestination::Assistant => super::assistant::is_control_text(text),
    }
}

async fn send_with_latest_context(
    client: &IlinkClient,
    latest_context: &RwLock<String>,
    to_user_id: &str,
    text: &str,
) -> Result<()> {
    let context_token = latest_context.read().await.clone();
    client.send_text(to_user_id, text, &context_token).await
}

async fn handle_control_text(
    app: &AppHandle,
    client: &IlinkClient,
    latest_context: &RwLock<String>,
    message: InboundText,
    destination: WeixinDestination,
) {
    let reply = super::handle_inbound(
        app,
        weixin_keys(destination).channel,
        &message.from_user_id,
        &message.text,
    )
    .await;
    if reply.is_empty() {
        return;
    }
    if let Err(error) =
        send_with_latest_context(client, latest_context, &message.from_user_id, &reply).await
    {
        tracing::warn!(target: "wisp", channel = "weixin", %error, "send control reply failed");
    }
}

async fn handle_agent_turn(
    app: &AppHandle,
    client: &IlinkClient,
    latest_context: &RwLock<String>,
    message: InboundText,
    destination: WeixinDestination,
) {
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let turn = super::handle_inbound_observed(
        app,
        weixin_keys(destination).channel,
        &message.from_user_id,
        &message.text,
        Some(progress_tx),
    );
    tokio::pin!(turn);

    let mut answer = None;
    let reply = loop {
        tokio::select! {
            reply = &mut turn => break reply,
            Some(event) = progress_rx.recv() => {
                if let super::ProgressEvent::TurnAnswer(text) = &event {
                    answer = Some(text.clone());
                }
                if let super::ProgressEvent::ApprovalRequested(request) = event {
                    let text = approval_message(destination, &request);
                    if let Err(error) = send_with_latest_context(
                        client,
                        latest_context,
                        &message.from_user_id,
                        &text,
                    ).await {
                        tracing::warn!(target: "wisp", channel = "weixin", %error, "send approval request failed");
                    }
                }
            }
        }
    };

    while let Ok(event) = progress_rx.try_recv() {
        if let super::ProgressEvent::TurnAnswer(text) = event {
            answer = Some(text);
        }
    }
    // A fast background report can be persisted just after this turn ends.
    // Send this turn's snapshot rather than reusing that later report as its
    // startup acknowledgement.
    let reply = answer
        .filter(|text| !text.trim().is_empty())
        .map(|text| super::truncate_reply(&text, super::REPLY_MAX_CHARS))
        .unwrap_or(reply);

    if reply.is_empty() {
        return;
    }
    if let Err(error) =
        send_with_latest_context(client, latest_context, &message.from_user_id, &reply).await
    {
        tracing::warn!(target: "wisp", channel = "weixin", %error, "send turn reply failed");
    }
}

fn approval_message(destination: WeixinDestination, request: &crate::ConfirmRequest) -> String {
    match destination {
        WeixinDestination::Projects => super::render_approval_request(request),
        WeixinDestination::Assistant => super::assistant::approval_message(request),
    }
}

pub async fn run(
    app: AppHandle,
    binding: Binding,
    token: String,
    status: Arc<StdMutex<ChannelStatus>>,
    mut shutdown: watch::Receiver<bool>,
    destination: WeixinDestination,
) {
    let keys = weixin_keys(destination);
    let client = match IlinkClient::new(&binding.base_url, &token) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            set_status(&status, "error", &format!("HTTP 客户端初始化失败:{e}"));
            return;
        }
    };
    let state = app.state::<crate::AppState>();
    let mut cursor = super::get_setting(&state.store, keys.cursor).await;
    let latest_context = Arc::new(RwLock::new(String::new()));
    let mut notifications = assistant_notifications().subscribe();
    let mut outbox = std::collections::VecDeque::<AssistantNotification>::new();
    let turn_busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let turn_idle = Arc::new(tokio::sync::Notify::new());
    let (turn_tx, mut turn_rx) = mpsc::channel::<InboundText>(32);
    let (turn_stop_tx, mut turn_stop_rx) = watch::channel(false);
    let turn_worker = {
        let app = app.clone();
        let client = client.clone();
        let latest_context = latest_context.clone();
        let turn_busy = turn_busy.clone();
        let turn_idle = turn_idle.clone();
        tokio::spawn(async move {
            loop {
                if *turn_stop_rx.borrow() {
                    break;
                }
                let message = tokio::select! {
                    biased;
                    _ = turn_stop_rx.changed() => None,
                    message = turn_rx.recv() => message,
                };
                let Some(message) = message else {
                    break;
                };
                turn_busy.store(true, std::sync::atomic::Ordering::SeqCst);
                handle_agent_turn(&app, &client, &latest_context, message, destination).await;
                turn_busy.store(false, std::sync::atomic::Ordering::SeqCst);
                turn_idle.notify_one();
            }
        })
    };
    let mut control_tasks = JoinSet::new();
    let mut session_expired = false;
    set_status(&status, "running", "已连接,等待消息");

    loop {
        if destination == WeixinDestination::Assistant && !latest_context.read().await.is_empty() {
            while let Some(notification) = outbox.front() {
                if let Some(request) = &notification.approval {
                    let pending = state
                        .confirms
                        .lock()
                        .unwrap()
                        .get(&request.frame_id)
                        .is_some_and(|p| p.request.approval_id == request.approval_id);
                    if !pending {
                        outbox.pop_front();
                        continue;
                    }
                } else if turn_busy.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                let visible = crate::research_assistant::visible_projects(&state.store)
                    .await
                    .is_ok_and(|projects| projects.iter().any(|p| p.0 == notification.project_id));
                if !visible {
                    outbox.pop_front();
                    continue;
                }
                if let Err(error) = send_with_latest_context(
                    &client,
                    &latest_context,
                    &binding.user_id,
                    &super::truncate_reply(&notification.text, super::REPLY_MAX_CHARS),
                )
                .await
                {
                    // Keep the result until another owner message refreshes
                    // iLink's reply window; it is already saved on the desktop.
                    tracing::warn!(%error, "assistant notification waiting for a usable reply window");
                    break;
                }
                outbox.pop_front();
            }
        }
        let updates = tokio::select! {
            r = client.get_updates(&cursor) => r,
            _ = turn_idle.notified(), if !outbox.is_empty() => continue,
            notification = notifications.recv(), if destination == WeixinDestination::Assistant => {
                if let Ok(notification) = notification {
                    queue_notification(&mut outbox, &binding, notification);
                }
                continue;
            }
            _ = shutdown.changed() => break,
        };
        if *shutdown.borrow() {
            break;
        }
        let updates = match updates {
            Ok(u) => u,
            Err(e) => {
                set_status(&status, "error", &format!("拉取消息失败:{e}"));
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                    _ = shutdown.changed() => break,
                }
            }
        };
        if updates.errcode == SESSION_EXPIRED_ERRCODE {
            // Token is scan-only; it cannot be refreshed programmatically.
            let _ = state.store.set_setting(keys.enabled, "false").await;
            set_status(&status, "error", "微信登录已过期,请重新扫码绑定");
            tracing::warn!(target: "wisp", channel = "weixin", "session expired (-14); channel disabled");
            session_expired = true;
            break;
        }
        if updates.ret != 0 {
            set_status(
                &status,
                "error",
                &format!(
                    "拉取消息失败:ret={} errcode={}",
                    updates.ret, updates.errcode
                ),
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                _ = shutdown.changed() => break,
            }
        }
        set_status(&status, "running", "已连接,等待消息");
        for msg in &updates.msgs {
            if !should_handle(msg, &binding) {
                continue;
            }
            *latest_context.write().await = msg.context_token.clone();
            let Some(text) = extract_text(msg) else {
                let client = client.clone();
                let latest_context = latest_context.clone();
                let to_user_id = msg.from_user_id.clone();
                control_tasks.spawn(async move {
                    let _ = send_with_latest_context(
                        &client,
                        &latest_context,
                        &to_user_id,
                        "暂不支持该消息类型,请发送文本消息。",
                    )
                    .await;
                });
                continue;
            };
            let message = InboundText {
                from_user_id: msg.from_user_id.clone(),
                text,
            };
            if is_control_text(&message.text, destination) {
                let app = app.clone();
                let client = client.clone();
                let latest_context = latest_context.clone();
                control_tasks.spawn(async move {
                    handle_control_text(&app, &client, &latest_context, message, destination).await;
                });
            } else if let Err(error) = turn_tx.try_send(message) {
                let message = error.into_inner();
                let client = client.clone();
                let latest_context = latest_context.clone();
                control_tasks.spawn(async move {
                    let _ = send_with_latest_context(
                        &client,
                        &latest_context,
                        &message.from_user_id,
                        "微信任务队列已满，请稍后重试。",
                    )
                    .await;
                });
            }
        }
        if !updates.get_updates_buf.is_empty() && updates.get_updates_buf != cursor {
            cursor = updates.get_updates_buf.clone();
            let _ = state.store.set_setting(keys.cursor, &cursor).await;
        }
        while control_tasks.try_join_next().is_some() {}
        let pause = Duration::from_millis(updates.longpolling_timeout_ms.max(1000) as u64);
        tokio::select! {
            _ = tokio::time::sleep(pause) => {}
            _ = turn_idle.notified(), if !outbox.is_empty() => {}
            notification = notifications.recv(), if destination == WeixinDestination::Assistant => {
                if let Ok(notification) = notification {
                    queue_notification(&mut outbox, &binding, notification);
                }
            }
            _ = shutdown.changed() => break,
        }
    }
    let _ = turn_stop_tx.send(true);
    drop(turn_tx);
    // Dropping the JoinHandle detaches an already-running turn so disabling a
    // channel cannot cancel the shared desktop session at an arbitrary await.
    drop(turn_worker);
    control_tasks.detach_all();
    if !session_expired && !*shutdown.borrow() {
        set_status(&status, "stopped", "");
    }
    super::emit_channels_updated(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_approval_notice_offers_replies_and_explains_full_permission_scope() {
        let request = crate::ConfirmRequest::new(
            "research-assistant",
            "Save a plan".into(),
            "research_plan",
            "add".into(),
        );
        let message = approval_message(WeixinDestination::Assistant, &request);
        for instruction in [
            "yes",
            "no",
            "full",
            "full off",
            "仅限助理会话",
            "重启 Wisp 后失效",
        ] {
            assert!(message.contains(instruction), "{instruction}");
        }
        assert!(!message.contains("请在桌面科研助理"));
        assert!(!message.contains("/approve"));
        assert!(approval_message(WeixinDestination::Projects, &request).contains("/approve"));
    }

    #[test]
    fn supervised_project_notice_names_the_operation_and_requests_a_one_shot_answer() {
        let request = crate::ConfirmRequest::new(
            "research-assistant",
            "Confirm project operation".into(),
            "project_approval",
            "Project RNA, CPU2: remove old output; deletion was not requested".into(),
        );
        let text = approval_message(WeixinDestination::Assistant, &request);
        assert!(text.contains("CPU2"));
        assert!(text.contains("yes"));
        assert!(text.contains("no"));
        assert!(!text.contains("full"));
    }

    fn binding() -> Binding {
        Binding {
            user_id: "owner".into(),
            account_id: "bot".into(),
            base_url: String::new(),
            bound_at: String::new(),
        }
    }

    fn msg(from: &str, to: &str, group: &str) -> Msg {
        Msg {
            from_user_id: from.into(),
            to_user_id: to.into(),
            context_token: "ctx".into(),
            group_id: group.into(),
            item_list: vec![],
        }
    }

    #[test]
    fn filters_group_echo_stranger_and_wrong_recipient() {
        assert!(should_handle(&msg("owner", "bot", ""), &binding()));
        assert!(!should_handle(&msg("owner", "bot", "g1"), &binding()));
        assert!(!should_handle(&msg("bot", "owner", ""), &binding()));
        assert!(!should_handle(&msg("stranger", "bot", ""), &binding()));
        assert!(!should_handle(&msg("owner", "other-bot", ""), &binding()));
        assert!(!should_handle(&msg("", "bot", ""), &binding()));
    }

    #[test]
    fn completion_notifications_never_follow_a_different_owner_or_bot() {
        let notice = AssistantNotification {
            binding: binding(),
            project_id: "project".into(),
            text: "result".into(),
            approval: None,
        };
        assert!(notification_matches(&notice, &binding()));
        let mut other = binding();
        other.user_id = "stranger".into();
        assert!(!notification_matches(&notice, &other));
        other = binding();
        other.account_id = "other-bot".into();
        assert!(!notification_matches(&notice, &other));
    }

    #[test]
    fn approval_notices_take_priority_over_reports_waiting_for_the_active_turn() {
        let mut outbox = std::collections::VecDeque::new();
        queue_notification(
            &mut outbox,
            &binding(),
            AssistantNotification {
                binding: binding(),
                project_id: "p".into(),
                text: "report".into(),
                approval: None,
            },
        );
        let request = crate::ConfirmRequest::new(
            "research-assistant",
            "confirm".into(),
            "project_approval",
            "operation".into(),
        );
        queue_notification(
            &mut outbox,
            &binding(),
            AssistantNotification {
                binding: binding(),
                project_id: "p".into(),
                text: "approval".into(),
                approval: Some(request),
            },
        );
        assert_eq!(outbox.pop_front().unwrap().text, "approval");
        assert_eq!(outbox.pop_front().unwrap().text, "report");
    }

    #[test]
    fn extracts_text_and_voice_transcripts() {
        let parsed: Updates = serde_json::from_str(
            r#"{"ret":0,"get_updates_buf":"NEXT","msgs":[
                {"from_user_id":"u1","context_token":"ctx","item_list":[
                    {"type":1,"text_item":{"text":"hi"}},
                    {"type":3,"voice_item":{"text":"transcribed"}},
                    {"type":2,"image_item":{}}
                ]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(parsed.get_updates_buf, "NEXT");
        assert_eq!(
            extract_text(&parsed.msgs[0]).as_deref(),
            Some("hi\ntranscribed")
        );
    }

    #[test]
    fn media_only_message_has_no_text() {
        let m = Msg {
            item_list: vec![Item {
                kind: 2,
                ..Item::default()
            }],
            ..msg("owner", "bot", "")
        };
        assert_eq!(extract_text(&m), None);
    }

    #[test]
    fn slash_commands_use_the_non_blocking_control_lane() {
        for destination in [WeixinDestination::Projects, WeixinDestination::Assistant] {
            assert!(is_control_text("/approve ABCDEF12", destination));
            assert!(is_control_text("  /reject ABCDEF12 not safe", destination));
            assert!(is_control_text("/stop", destination));
            assert!(!is_control_text("please run /status later", destination));
            assert!(!is_control_text("analyze the dataset", destination));
        }
        // The assistant's `/resume` reruns a turn, so it waits its turn.
        assert!(is_control_text("/unknown", WeixinDestination::Assistant));
        assert!(is_control_text("/model 2", WeixinDestination::Assistant));
        assert!(!is_control_text("/resume", WeixinDestination::Assistant));
    }

    #[test]
    fn assistant_approval_replies_bypass_the_blocked_turn_queue_only_for_assistant() {
        for text in ["yes", " NO ", "Full", "full off"] {
            assert!(
                is_control_text(text, WeixinDestination::Assistant),
                "{text}"
            );
            assert!(
                !is_control_text(text, WeixinDestination::Projects),
                "{text}"
            );
        }
    }
}
