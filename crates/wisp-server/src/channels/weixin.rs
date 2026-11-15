//! WeChat through Tencent's iLink bot service, a port of the WebView's
//! `src-tauri/src/channels/weixin.rs`. Binding shows a QR code the owner
//! scans in WeChat; the bot then long-polls for the owner's messages and
//! answers in the same chat. Only the owner who scanned is answered, and
//! only one to one. The bot token is kept in the keychain under the
//! WebView's name, the binding and the poll cursor in the store, so both
//! desktops share one binding. The research assistant's own WeChat (the
//! WebView's second destination) is not ported.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{mpsc, watch, RwLock};
use tokio::task::JoinSet;

use super::{get_setting, set_status, Channel, ChannelState, ChatEvent};
use crate::Server;

pub const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const CHANNEL_VERSION: &str = "1.0.2";
const BOT_TYPE: &str = "3";
const SESSION_EXPIRED_ERRCODE: i64 = -14;

/// The WebView's names, so either desktop's binding serves both.
pub(crate) const ENABLED: &str = "weixin_enabled";
const BINDING: &str = "weixin_binding";
const CURSOR: &str = "weixin_sync_buf";
const TOKEN: &str = "weixin_bot_token";
/// The research assistant's WeChat in the WebView; one bot serves one.
const ASSISTANT_BINDING: &str = "assistant_weixin_binding";

#[derive(Deserialize, Default)]
struct QrCode {
    #[serde(default)]
    ret: i64,
    /// What the scan is polled by.
    #[serde(default)]
    qrcode: String,
    /// The text the QR code shows.
    #[serde(default)]
    qrcode_img_content: String,
}

#[derive(Deserialize, Default)]
struct QrStatus {
    #[serde(default)]
    ret: i64,
    /// "wait" | "scaned" | "confirmed" | "expired"
    #[serde(default)]
    status: String,
    #[serde(default)]
    bot_token: String,
    #[serde(default)]
    baseurl: String,
    #[serde(default)]
    ilink_bot_id: String,
    #[serde(default)]
    ilink_user_id: String,
}

#[derive(Deserialize, Default)]
struct Updates {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    longpolling_timeout_ms: i64,
    #[serde(default)]
    get_updates_buf: String,
    #[serde(default)]
    msgs: Vec<Msg>,
}

#[derive(Deserialize, Default)]
pub(crate) struct Msg {
    #[serde(default)]
    from_user_id: String,
    #[serde(default)]
    to_user_id: String,
    #[serde(default)]
    context_token: String,
    #[serde(default)]
    group_id: String,
    #[serde(default)]
    item_list: Vec<Item>,
}

#[derive(Deserialize, Default)]
struct Item {
    #[serde(rename = "type", default)]
    kind: i64,
    #[serde(default)]
    text_item: Option<TextPayload>,
    #[serde(default)]
    voice_item: Option<TextPayload>,
}

#[derive(Deserialize, Default)]
struct TextPayload {
    #[serde(default)]
    text: String,
}

/// Text worth handling: text items (type 1) and voice transcripts
/// (type 3), joined.
fn extract_text(msg: &Msg) -> Option<String> {
    let parts: Vec<&str> = msg
        .item_list
        .iter()
        .filter_map(|item| match item.kind {
            1 => item.text_item.as_ref(),
            3 => item.voice_item.as_ref(),
            _ => None,
        })
        .map(|payload| payload.text.trim())
        .filter(|text| !text.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// The binding as the store keeps it; the token is in the keychain.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    /// The owner who scanned: the only sender answered.
    pub user_id: String,
    /// The bot's own account, to skip its echoes.
    pub account_id: String,
    pub base_url: String,
    pub bound_at: String,
}

fn should_handle(msg: &Msg, binding: &Binding) -> bool {
    if !msg.group_id.is_empty() || msg.from_user_id.is_empty() {
        return false;
    }
    // The bot's own echo, someone other than the owner, another bot.
    msg.from_user_id != binding.account_id
        && (binding.user_id.is_empty() || msg.from_user_id == binding.user_id)
        && (binding.account_id.is_empty()
            || msg.to_user_id.is_empty()
            || msg.to_user_id == binding.account_id)
}

struct IlinkClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl IlinkClient {
    fn new(base_url: &str, token: &str) -> Result<Self> {
        Ok(Self {
            // getupdates holds the request about 35 s on the service's side.
            http: reqwest::Client::builder()
                .user_agent("wisp-science")
                .timeout(Duration::from_secs(75))
                .build()?,
            base_url: match base_url.trim_end_matches('/') {
                "" => DEFAULT_BASE_URL.into(),
                base => base.into(),
            },
            token: token.into(),
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        // The service expects a random number's base64 as a fingerprint.
        let uin = base64::engine::general_purpose::STANDARD
            .encode((uuid::Uuid::new_v4().as_u128() as u32).to_string());
        let request = self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .header("AuthorizationType", "ilink_bot_token")
            .header("X-WECHAT-UIN", uin);
        if self.token.is_empty() {
            request
        } else {
            request.bearer_auth(&self.token)
        }
    }

    async fn qrcode(&self) -> Result<QrCode> {
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
        ensure!(
            qr.ret == 0 && !qr.qrcode.is_empty() && !qr.qrcode_img_content.is_empty(),
            "Could not get a WeChat QR code: ret={}",
            qr.ret
        );
        Ok(qr)
    }

    async fn qrcode_status(&self, qrcode: &str) -> Result<QrStatus> {
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

    async fn updates(&self, cursor: &str) -> Result<Updates> {
        Ok(self
            .request(reqwest::Method::POST, "/ilink/bot/getupdates")
            .json(&json!({
                "get_updates_buf": cursor,
                "base_info": {"channel_version": CHANNEL_VERSION},
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn send_text(&self, to_user_id: &str, text: &str, context_token: &str) -> Result<()> {
        let reply: serde_json::Value = self
            .request(reqwest::Method::POST, "/ilink/bot/sendmessage")
            .json(&json!({
                "msg": {
                    "to_user_id": to_user_id,
                    "client_id": format!("wisp-weixin-{}", uuid::Uuid::new_v4().simple()),
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
        let ret = reply.get("ret").and_then(|ret| ret.as_i64()).unwrap_or(0);
        ensure!(
            ret == 0,
            "sendmessage failed: ret={ret} errcode={}",
            reply
                .get("errcode")
                .and_then(|code| code.as_i64())
                .unwrap_or(0)
        );
        Ok(())
    }
}

/// A QR code as dark and light modules, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrMatrix {
    pub width: usize,
    pub dark: Vec<bool>,
}

impl QrMatrix {
    pub(crate) fn of(content: &str) -> Result<Self> {
        let code = qrcode::QrCode::new(content.as_bytes()).context("Could not make a QR code")?;
        Ok(Self {
            width: code.width(),
            dark: code
                .to_colors()
                .into_iter()
                .map(|color| color == qrcode::Color::Dark)
                .collect(),
        })
    }
}

/// WeChat as Settings shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeixinStatus {
    pub enabled: bool,
    pub bound: bool,
    pub state: ChannelState,
    /// What the connection is doing, or why it is not up.
    pub detail: String,
}

/// A QR code to scan in WeChat, and what its scan is polled by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeixinQr {
    pub qrcode: String,
    pub matrix: QrMatrix,
}

/// How far the owner has got with a QR code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeixinScan {
    Waiting,
    Scanned,
    /// Bound; the bot runs when WeChat is on.
    Confirmed,
    Expired,
}

struct Inbound {
    from: String,
    text: String,
}

async fn send_latest(client: &IlinkClient, context: &RwLock<String>, to: &str, text: &str) {
    let context = context.read().await.clone();
    if let Err(error) = client.send_text(to, text, &context).await {
        tracing::warn!(target: "wisp", channel = "weixin", %error, "send reply failed");
    }
}

/// One message that runs a turn: approvals it asks for are sent as they
/// come, then its answer.
async fn agent_turn(
    server: &Server,
    client: &IlinkClient,
    context: &RwLock<String>,
    message: Inbound,
) {
    let (events, mut heard) = mpsc::unbounded_channel();
    let turn = server.handle_inbound(Channel::Weixin, &message.from, &message.text, Some(events));
    tokio::pin!(turn);
    // WeChat shows no progress; only approvals are sent while it runs.
    let reply = loop {
        tokio::select! {
            reply = &mut turn => break reply,
            Some(event) = heard.recv() => {
                if let ChatEvent::Approval(text) = event {
                    send_latest(client, context, &message.from, &text).await;
                }
            }
        }
    };
    while let Ok(event) = heard.try_recv() {
        if let ChatEvent::Approval(text) = event {
            send_latest(client, context, &message.from, &text).await;
        }
    }
    if !reply.is_empty() {
        send_latest(client, context, &message.from, &reply).await;
    }
}

async fn run(
    server: Server,
    binding: Binding,
    token: String,
    status: Arc<std::sync::Mutex<(ChannelState, String)>>,
    mut stop: watch::Receiver<bool>,
) {
    let client = match IlinkClient::new(&binding.base_url, &token) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            set_status(
                &status,
                ChannelState::Error,
                &format!("Could not start the HTTP client: {error}"),
            );
            return;
        }
    };
    let store = server.0.store.clone();
    let mut cursor = get_setting(&store, CURSOR).await;
    let context = Arc::new(RwLock::new(String::new()));
    // Turns run one after another; commands (an approval, /stop) at once.
    let (turns, mut queued) = mpsc::channel::<Inbound>(32);
    let worker = {
        let server = server.clone();
        let client = client.clone();
        let context = context.clone();
        let mut stop = stop.clone();
        tokio::spawn(async move {
            loop {
                let message = tokio::select! {
                    biased;
                    _ = stop.changed() => None,
                    message = queued.recv() => message,
                };
                let Some(message) = message else { break };
                agent_turn(&server, &client, &context, message).await;
            }
        })
    };
    let mut commands = JoinSet::new();
    let mut expired = false;
    set_status(
        &status,
        ChannelState::Running,
        "Connected; waiting for messages",
    );
    loop {
        let updates = tokio::select! {
            updates = client.updates(&cursor) => updates,
            _ = stop.changed() => break,
        };
        if *stop.borrow() {
            break;
        }
        let updates = match updates {
            Ok(updates) => updates,
            Err(error) => {
                set_status(
                    &status,
                    ChannelState::Error,
                    &format!("Could not fetch messages: {error}"),
                );
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                    _ = stop.changed() => break,
                }
            }
        };
        if updates.errcode == SESSION_EXPIRED_ERRCODE {
            // The token comes from a scan only; it cannot be refreshed.
            let _ = store.set_setting(ENABLED, "false").await;
            set_status(
                &status,
                ChannelState::Error,
                "The WeChat sign-in expired; bind again with a new QR code",
            );
            expired = true;
            break;
        }
        if updates.ret != 0 {
            set_status(
                &status,
                ChannelState::Error,
                &format!(
                    "Could not fetch messages: ret={} errcode={}",
                    updates.ret, updates.errcode
                ),
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                _ = stop.changed() => break,
            }
        }
        set_status(
            &status,
            ChannelState::Running,
            "Connected; waiting for messages",
        );
        for msg in &updates.msgs {
            if !should_handle(msg, &binding) {
                continue;
            }
            // Replies go out in the window of the owner's latest message.
            *context.write().await = msg.context_token.clone();
            let from = msg.from_user_id.clone();
            let Some(text) = extract_text(msg) else {
                let (client, context) = (client.clone(), context.clone());
                commands.spawn(async move {
                    send_latest(
                        &client,
                        &context,
                        &from,
                        "暂不支持该消息类型,请发送文本消息。",
                    )
                    .await;
                });
                continue;
            };
            let message = Inbound { from, text };
            if message.text.trim_start().starts_with('/') {
                let (server, client, context) = (server.clone(), client.clone(), context.clone());
                commands.spawn(async move {
                    let reply = server
                        .handle_inbound(Channel::Weixin, &message.from, &message.text, None)
                        .await;
                    if !reply.is_empty() {
                        send_latest(&client, &context, &message.from, &reply).await;
                    }
                });
            } else if let Err(full) = turns.try_send(message) {
                let message = full.into_inner();
                let (client, context) = (client.clone(), context.clone());
                commands.spawn(async move {
                    send_latest(
                        &client,
                        &context,
                        &message.from,
                        "微信任务队列已满，请稍后重试。",
                    )
                    .await;
                });
            }
        }
        if !updates.get_updates_buf.is_empty() && updates.get_updates_buf != cursor {
            cursor = updates.get_updates_buf.clone();
            let _ = store.set_setting(CURSOR, &cursor).await;
        }
        while commands.try_join_next().is_some() {}
        let pause = Duration::from_millis(updates.longpolling_timeout_ms.max(1000) as u64);
        tokio::select! {
            _ = tokio::time::sleep(pause) => {}
            _ = stop.changed() => break,
        }
    }
    drop(turns);
    // A turn under way finishes in its conversation; it is not cut off at
    // an arbitrary point because WeChat was turned off.
    drop(worker);
    commands.detach_all();
    if !expired && !*stop.borrow() {
        set_status(&status, ChannelState::Stopped, "");
    }
}

impl Server {
    async fn weixin_binding(&self, key: &str) -> Option<Binding> {
        serde_json::from_str(&get_setting(&self.0.store, key).await).ok()
    }

    fn ilink_client(&self) -> Result<IlinkClient> {
        IlinkClient::new(&self.0.channels.ilink_base.lock().unwrap(), "")
    }

    /// WeChat as it stands.
    pub async fn weixin_status(&self) -> WeixinStatus {
        let (state, detail) = self.0.channels.weixin.snapshot();
        WeixinStatus {
            enabled: get_setting(&self.0.store, ENABLED).await == "true",
            bound: self.weixin_binding(BINDING).await.is_some(),
            state,
            detail,
        }
    }

    /// Asks iLink for a QR code to bind a bot with.
    pub async fn start_weixin_bind(&self) -> Result<WeixinQr> {
        let qr = self.ilink_client()?.qrcode().await?;
        Ok(WeixinQr {
            matrix: QrMatrix::of(&qr.qrcode_img_content)?,
            qrcode: qr.qrcode,
        })
    }

    /// Where the scan of `qrcode` stands. Once confirmed the binding is
    /// kept (the token in the keychain) and the bot restarts when on.
    pub async fn poll_weixin_bind(&self, qrcode: &str) -> Result<WeixinScan> {
        let status = self.ilink_client()?.qrcode_status(qrcode).await?;
        ensure!(
            status.ret == 0,
            "Could not check the scan: ret={}",
            status.ret
        );
        match status.status.as_str() {
            "confirmed" => {}
            "scaned" => return Ok(WeixinScan::Scanned),
            "expired" => return Ok(WeixinScan::Expired),
            _ => return Ok(WeixinScan::Waiting),
        }
        ensure!(
            !status.bot_token.is_empty(),
            "The scan was confirmed, but no token came back"
        );
        let _config = self.0.channels.weixin_config.lock().await;
        if status.ilink_user_id.trim().is_empty() || status.ilink_bot_id.trim().is_empty() {
            bail!("The scan did not say who the owner or the bot is; bind again");
        }
        if self
            .weixin_binding(ASSISTANT_BINDING)
            .await
            .is_some_and(|binding| binding.account_id == status.ilink_bot_id)
        {
            bail!("This WeChat bot is bound to the research assistant already; use another bot, or unbind it there first");
        }
        self.0.secrets.set(TOKEN, &status.bot_token)?;
        let binding = Binding {
            user_id: status.ilink_user_id,
            account_id: status.ilink_bot_id,
            base_url: status.baseurl,
            bound_at: chrono::Utc::now().to_rfc3339(),
        };
        let store = &self.0.store;
        store
            .set_setting(BINDING, &serde_json::to_string(&binding)?)
            .await?;
        // A new binding starts from now, never another sign-in's backlog.
        store.set_setting(CURSOR, "").await?;
        if get_setting(store, ENABLED).await == "true" {
            self.start_weixin().await;
        }
        Ok(WeixinScan::Confirmed)
    }

    /// Turns the bot on or off; on needs a binding.
    pub async fn set_weixin_enabled(&self, enabled: bool) -> Result<WeixinStatus> {
        {
            let _config = self.0.channels.weixin_config.lock().await;
            ensure!(
                !enabled || self.weixin_binding(BINDING).await.is_some(),
                "Bind WeChat with a QR code before turning it on"
            );
            self.0
                .store
                .set_setting(ENABLED, if enabled { "true" } else { "false" })
                .await?;
            if enabled {
                self.start_weixin().await;
            } else {
                self.0.channels.weixin.stop();
            }
        }
        Ok(self.weixin_status().await)
    }

    /// Forgets the binding and its token, and stops the bot.
    pub async fn unbind_weixin(&self) -> Result<WeixinStatus> {
        {
            let _config = self.0.channels.weixin_config.lock().await;
            self.0.channels.weixin.stop();
            // The keychain has no delete here; empty reads as none.
            self.0.secrets.set(TOKEN, "")?;
            let store = &self.0.store;
            store.set_setting(BINDING, "").await?;
            store.set_setting(CURSOR, "").await?;
            store.set_setting(ENABLED, "false").await?;
        }
        Ok(self.weixin_status().await)
    }

    /// Starts the bot in place of any running one. Called with the
    /// WeChat lock held.
    pub(crate) async fn start_weixin(&self) {
        let worker = &self.0.channels.weixin;
        let stop = worker.begin();
        let status = worker.status.clone();
        let Some(binding) = self.weixin_binding(BINDING).await else {
            set_status(
                &status,
                ChannelState::Error,
                "Bind WeChat with a QR code first",
            );
            return;
        };
        let token = self.0.secrets.get(TOKEN).unwrap_or_default();
        if token.is_empty() {
            set_status(
                &status,
                ChannelState::Error,
                "The bot's token is missing; bind again",
            );
            return;
        }
        tokio::spawn(run(self.clone(), binding, token, status, stop));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(from: &str, to: &str, group: &str, items: serde_json::Value) -> Msg {
        serde_json::from_value(json!({
            "from_user_id": from,
            "to_user_id": to,
            "group_id": group,
            "item_list": items,
        }))
        .unwrap()
    }

    #[test]
    fn text_and_voice_transcripts_are_read_and_the_rest_is_not() {
        let both = msg(
            "u",
            "",
            "",
            json!([
                {"type": 1, "text_item": {"text": " hello "}},
                {"type": 2},
                {"type": 3, "voice_item": {"text": "spoken"}},
            ]),
        );
        assert_eq!(extract_text(&both).as_deref(), Some("hello\nspoken"));
        assert_eq!(extract_text(&msg("u", "", "", json!([{"type": 2}]))), None);
    }

    #[test]
    fn only_the_owner_is_answered_one_to_one() {
        let binding = Binding {
            user_id: "owner".into(),
            account_id: "bot".into(),
            ..Default::default()
        };
        let text = json!([{"type": 1, "text_item": {"text": "hi"}}]);
        assert!(should_handle(
            &msg("owner", "bot", "", text.clone()),
            &binding
        ));
        assert!(should_handle(&msg("owner", "", "", text.clone()), &binding));
        assert!(!should_handle(
            &msg("stranger", "bot", "", text.clone()),
            &binding
        ));
        assert!(!should_handle(
            &msg("owner", "bot", "group", text.clone()),
            &binding
        ));
        assert!(!should_handle(
            &msg("bot", "owner", "", text.clone()),
            &binding
        ));
        assert!(!should_handle(
            &msg("owner", "other-bot", "", text),
            &binding
        ));
    }

    #[test]
    fn a_qr_code_is_a_square_of_modules() {
        let matrix = QrMatrix::of("https://example.test/qr").unwrap();
        assert!(matrix.width >= 21);
        assert_eq!(matrix.dark.len(), matrix.width * matrix.width);
        // The finder pattern's corner is dark.
        assert!(matrix.dark[0]);
    }
}
