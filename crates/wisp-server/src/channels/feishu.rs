//! Feishu (Lark): a self-built app's bot over Feishu's long connection, so
//! a desktop needs no public callback address. A port of the WebView's
//! `src-tauri/src/channels/feishu.rs`.
//!
//! Endpoint discovery over HTTP, then a WebSocket of pbbp2 frames; each
//! event is acknowledged within Feishu's three seconds, and a
//! `im.message.receive_v1` drives the chats' target conversation. Replies
//! go back over REST with a cached tenant token. An ordinary message gets
//! a CardKit card that shows the turn's progress and ends as the answer.
//! Only the owner the desktop confirmed is answered: the first stranger to
//! write is kept as a pending owner until Settings confirms them.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, ensure, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::feishu_registration::{Accounts, RegistrationFlow, RegistrationPoll};
use super::{
    feishu_card, get_setting, pbbp2, set_status, weixin::QrMatrix, Channel, ChannelState,
    ChatEvent, TurnProgress,
};
use crate::Server;

const FEISHU_BASE: &str = "https://open.feishu.cn";
const LARK_BASE: &str = "https://open.larksuite.com";
const DEDUPE_WINDOW: usize = 128;

/// The WebView's names, so either desktop's app serves both.
pub(crate) const ENABLED: &str = "feishu_enabled";
const APP_ID: &str = "feishu_app_id";
const INTERNATIONAL: &str = "feishu_international";
const OWNER: &str = "feishu_owner_binding";
const PENDING_OWNER: &str = "feishu_pending_owner";
const SECRET: &str = "feishu_app_secret";

const NON_OWNER_REPLY: &str = "此机器人只响应已绑定的所有者。";
const UNBOUND_REPLY: &str = "尚未绑定所有者。请在桌面端设置中确认飞书配对后，再发送消息。";
const UNSUPPORTED_REPLY: &str = "暂不支持该消息类型,请发送文本消息。";

/// Where Feishu's services are; tests point them at a fake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hosts {
    pub feishu: String,
    pub lark: String,
    pub accounts: Accounts,
}

impl Default for Hosts {
    fn default() -> Self {
        Self {
            feishu: FEISHU_BASE.into(),
            lark: LARK_BASE.into(),
            accounts: Accounts::default(),
        }
    }
}

impl Hosts {
    fn open(&self, international: bool) -> String {
        if international {
            self.lark.clone()
        } else {
            self.feishu.clone()
        }
    }
}

struct FeishuRest {
    http: reqwest::Client,
    app_id: String,
    app_secret: String,
    base: String,
    token: tokio::sync::Mutex<Option<(String, Instant)>>,
}

#[derive(Deserialize)]
struct TokenResp {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    tenant_access_token: String,
    #[serde(default)]
    expire: u64,
}

impl FeishuRest {
    fn new(app_id: &str, app_secret: &str, base: String) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("wisp-science")
                .timeout(Duration::from_secs(30))
                .build()?,
            app_id: app_id.into(),
            app_secret: app_secret.into(),
            base,
            token: Default::default(),
        })
    }

    /// The tenant token, kept until less than half an hour is left, the
    /// point at which Feishu starts handing out a new one.
    async fn tenant_token(&self) -> Result<String> {
        let mut kept = self.token.lock().await;
        if let Some((token, expires)) = kept.as_ref() {
            if *expires > Instant::now() + Duration::from_secs(30 * 60) {
                return Ok(token.clone());
            }
        }
        let reply: TokenResp = self
            .http
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                self.base
            ))
            .json(&json!({"app_id": self.app_id, "app_secret": self.app_secret}))
            .send()
            .await?
            .json()
            .await?;
        ensure!(
            reply.code == 0,
            "tenant_access_token failed: code={} {}",
            reply.code,
            reply.msg
        );
        let token = reply.tenant_access_token;
        *kept = Some((
            token.clone(),
            Instant::now() + Duration::from_secs(reply.expire),
        ));
        Ok(token)
    }

    async fn send_text(&self, chat_id: &str, text: &str) -> Result<()> {
        let token = self.tenant_token().await?;
        let content = serde_json::to_string(&json!({ "text": text }))?;
        checked_json(
            self.http
                .post(format!(
                    "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
                    self.base
                ))
                .bearer_auth(token)
                .json(&json!({"receive_id": chat_id, "msg_type": "text", "content": content}))
                .send()
                .await?,
            "send message",
        )
        .await?;
        Ok(())
    }

    async fn create_streaming_card(&self, initial: &str) -> Result<String> {
        let token = self.tenant_token().await?;
        let card = feishu_card::build_streaming_card(initial);
        let reply = checked_json(
            self.http
                .post(format!("{}/open-apis/cardkit/v1/cards", self.base))
                .bearer_auth(token)
                .json(&json!({ "type": "card_json", "data": card }))
                .send()
                .await?,
            "create streaming card",
        )
        .await?;
        reply["data"]["card_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("create streaming card response missing data.card_id"))
    }

    async fn send_card(&self, chat_id: &str, card_id: &str) -> Result<()> {
        let token = self.tenant_token().await?;
        let content =
            serde_json::to_string(&json!({"type": "card", "data": { "card_id": card_id }}))?;
        let reply = checked_json(
            self.http
                .post(format!(
                    "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
                    self.base
                ))
                .bearer_auth(token)
                .json(
                    &json!({"receive_id": chat_id, "msg_type": "interactive", "content": content}),
                )
                .send()
                .await?,
            "send streaming card",
        )
        .await?;
        ensure!(
            reply["data"]["message_id"].as_str().is_some(),
            "send streaming card response missing data.message_id"
        );
        Ok(())
    }

    async fn stream_card_content(&self, card_id: &str, content: &str, sequence: i64) -> Result<()> {
        let token = self.tenant_token().await?;
        checked_json(
            self.http
                .put(format!(
                    "{}/open-apis/cardkit/v1/cards/{card_id}/elements/{}/content",
                    self.base,
                    feishu_card::PROGRESS_ELEMENT_ID
                ))
                .bearer_auth(token)
                .json(&json!({ "content": content, "sequence": sequence }))
                .send()
                .await?,
            "update streaming card",
        )
        .await?;
        Ok(())
    }

    async fn close_streaming_card(&self, card_id: &str, sequence: i64) -> Result<()> {
        let token = self.tenant_token().await?;
        checked_json(
            self.http
                .patch(format!(
                    "{}/open-apis/cardkit/v1/cards/{card_id}/settings",
                    self.base
                ))
                .bearer_auth(token)
                .json(&json!({
                    "settings": "{\"config\":{\"streaming_mode\":false}}",
                    "sequence": sequence,
                }))
                .send()
                .await?,
            "close streaming card",
        )
        .await?;
        Ok(())
    }

    /// The bot's own open_id, to tell when a group message mentions it.
    async fn bot_open_id(&self) -> Result<String> {
        let token = self.tenant_token().await?;
        let reply: serde_json::Value = self
            .http
            .get(format!("{}/open-apis/bot/v3/info", self.base))
            .bearer_auth(token)
            .send()
            .await?
            .json()
            .await?;
        reply["bot"]["open_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("bot info response missing bot.open_id"))
    }
}

async fn checked_json(response: reqwest::Response, operation: &str) -> Result<serde_json::Value> {
    let status = response.status();
    let value: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("{operation}: invalid JSON response (HTTP {status})"))?;
    let code = value["code"].as_i64().unwrap_or(-1);
    ensure!(
        status.is_success() && code == 0,
        "{operation} failed: HTTP {status}, code={code} {}",
        value["msg"].as_str().unwrap_or("")
    );
    Ok(value)
}

struct Endpoint {
    url: String,
    ping_interval: Duration,
    reconnect_interval: Duration,
}

/// A WebSocket address Feishu may hand out: secure, or on this computer
/// (a test's fake).
fn acceptable_socket(url: &str) -> bool {
    url.starts_with("wss://")
        || url::Url::parse(url).is_ok_and(|url| {
            url.scheme() == "ws"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        })
}

async fn discover_endpoint(
    http: &reqwest::Client,
    base: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<Endpoint> {
    // The keys' case matters: lowercase keys get a 514.
    let reply: serde_json::Value = http
        .post(format!("{base}/callback/ws/endpoint"))
        .json(&json!({"AppID": app_id, "AppSecret": app_secret}))
        .send()
        .await
        .context("endpoint discovery request failed")?
        .json()
        .await?;
    let code = reply["code"].as_i64().unwrap_or(0);
    if code != 0 {
        let why = match code {
            514 => "The App ID or App Secret was refused".to_owned(),
            1000040350 => "Too many connections (at most 50 per app)".to_owned(),
            _ => reply["msg"].as_str().unwrap_or("unknown").to_owned(),
        };
        bail!("endpoint discovery failed: code={code} {why}");
    }
    let data = &reply["data"];
    let url = data["URL"]
        .as_str()
        .filter(|url| acceptable_socket(url))
        .ok_or_else(|| anyhow!("endpoint discovery: missing wss URL"))?
        .to_owned();
    let seconds = |key: &str, default: u64| {
        data["ClientConfig"][key]
            .as_u64()
            .filter(|value| *value > 0)
            .unwrap_or(default)
    };
    Ok(Endpoint {
        url,
        ping_interval: Duration::from_secs(seconds("PingInterval", 120)),
        reconnect_interval: Duration::from_secs(seconds("ReconnectInterval", 30).min(120)),
    })
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_owned())
    })
}

#[derive(Debug, PartialEq)]
struct InboundMessage {
    event_id: String,
    chat_id: String,
    sender_open_id: String,
    /// None when the message is not plain text.
    text: Option<String>,
}

/// The owner as the store keeps them. The first stranger is never written
/// here; Settings confirms them.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct OwnerBinding {
    open_id: String,
    #[serde(default)]
    bound_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SenderDecision {
    Allow,
    Reject,
    /// No owner yet: the sender is kept as pending, not let in.
    Unbound,
}

fn sender_decision(sender: &str, owner: Option<&str>) -> SenderDecision {
    if sender.is_empty() {
        return SenderDecision::Reject;
    }
    match owner.map(str::trim).filter(|id| !id.is_empty()) {
        None => SenderDecision::Unbound,
        Some(owner) if owner == sender => SenderDecision::Allow,
        Some(_) => SenderDecision::Reject,
    }
}

/// An `im.message.receive_v1` event as a message; none for other events,
/// the bot's own messages, and group messages that do not mention it.
fn parse_message_event(payload: &[u8], bot_open_id: &str) -> Option<InboundMessage> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let header = value.get("header")?;
    if header.get("event_type")?.as_str()? != "im.message.receive_v1" {
        return None;
    }
    let event_id = header.get("event_id")?.as_str()?.to_owned();
    let event = value.get("event")?;
    let message = event.get("message")?;
    let chat_id = message.get("chat_id")?.as_str()?.to_owned();
    let sender_open_id = event["sender"]["sender_id"]["open_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if sender_open_id.is_empty() || sender_open_id == bot_open_id {
        return None;
    }
    let mentions = message["mentions"].as_array().cloned().unwrap_or_default();
    if message["chat_type"].as_str() == Some("group")
        && !mentions
            .iter()
            .any(|mention| mention["id"]["open_id"].as_str() == Some(bot_open_id))
    {
        return None;
    }
    let text = if message["message_type"].as_str() == Some("text") {
        // The content is JSON inside a string.
        let inner: serde_json::Value =
            serde_json::from_str(message.get("content")?.as_str()?).ok()?;
        let mut text = inner.get("text")?.as_str()?.to_owned();
        for mention in &mentions {
            if let Some(key) = mention["key"].as_str() {
                text = text.replace(key, "");
            }
        }
        Some(text.trim().to_owned())
    } else {
        None
    };
    Some(InboundMessage {
        event_id,
        chat_id,
        sender_open_id,
        text,
    })
}

/// The latest event ids, as Feishu delivers at least once.
struct SeenEvents {
    set: HashSet<String>,
    order: VecDeque<String>,
    cap: usize,
}

impl SeenEvents {
    fn new(cap: usize) -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    /// Whether the id is new.
    fn insert(&mut self, id: &str) -> bool {
        if !self.set.insert(id.to_owned()) {
            return false;
        }
        self.order.push_back(id.to_owned());
        if self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }
}

async fn run(
    server: Server,
    app_id: String,
    app_secret: String,
    base: String,
    status: Arc<std::sync::Mutex<(ChannelState, String)>>,
    mut stop: watch::Receiver<bool>,
) {
    let rest = match FeishuRest::new(&app_id, &app_secret, base) {
        Ok(rest) => Arc::new(rest),
        Err(error) => {
            set_status(
                &status,
                ChannelState::Error,
                &format!("Could not start the HTTP client: {error}"),
            );
            return;
        }
    };
    // The queue, its worker and the window of seen events outlive one
    // connection: events are acknowledged before they are queued, so a
    // reconnect must not drop them or cut off a turn.
    let (events, queued) = mpsc::channel::<InboundMessage>(64);
    let _worker = tokio::spawn(event_worker(
        server.clone(),
        rest.clone(),
        queued,
        stop.clone(),
    ));
    let mut seen = SeenEvents::new(DEDUPE_WINDOW);
    loop {
        set_status(&status, ChannelState::Connecting, "Connecting to Feishu…");
        let result = connect_once(&rest, &status, &mut stop, &events, &mut seen).await;
        if *stop.borrow() {
            break;
        }
        let (detail, wait) = match result {
            Ok(wait) => ("Disconnected; connecting again…".to_owned(), wait),
            Err(error) => (format!("{error:#}"), Duration::from_secs(30)),
        };
        set_status(&status, ChannelState::Error, &detail);
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stop.changed() => break,
        }
    }
    set_status(&status, ChannelState::Stopped, "");
}

/// Replies and turns run here, so the connection keeps acknowledging
/// within three seconds while a turn takes minutes.
async fn event_worker(
    server: Server,
    rest: Arc<FeishuRest>,
    mut queued: mpsc::Receiver<InboundMessage>,
    stop: watch::Receiver<bool>,
) {
    while let Some(message) = queued.recv().await {
        if *stop.borrow() {
            break;
        }
        if let Some(reply) = server
            .authorize_feishu_sender(&message.sender_open_id)
            .await
        {
            let _ = rest.send_text(&message.chat_id, &reply).await;
            continue;
        }
        let Some(text) = message.text.as_deref().filter(|text| !text.is_empty()) else {
            let _ = rest.send_text(&message.chat_id, UNSUPPORTED_REPLY).await;
            continue;
        };
        // A command is answered at once; a turn gets a card that shows its
        // progress and ends as the answer.
        if !text.trim_start().starts_with('/') {
            let initial = feishu_card::render(&TurnProgress::default());
            if let Ok(card) = rest.create_streaming_card(&initial).await {
                if rest.send_card(&message.chat_id, &card).await.is_ok() {
                    streamed_turn(&server, rest.clone(), &message.chat_id, text, card).await;
                    continue;
                }
            }
            tracing::warn!(target: "wisp", channel = "feishu", "progress card failed; answering in text");
        }
        let reply = server
            .handle_inbound(Channel::Feishu, &message.chat_id, text, None)
            .await;
        if !reply.is_empty() {
            if let Err(error) = rest.send_text(&message.chat_id, &reply).await {
                tracing::warn!(target: "wisp", channel = "feishu", %error, "send reply failed");
            }
        }
    }
}

/// One connection, until it drops; returns how long to wait before the
/// next.
async fn connect_once(
    rest: &Arc<FeishuRest>,
    status: &std::sync::Mutex<(ChannelState, String)>,
    stop: &mut watch::Receiver<bool>,
    events: &mpsc::Sender<InboundMessage>,
    seen: &mut SeenEvents,
) -> Result<Duration> {
    let bot = rest.bot_open_id().await.context(
        "Could not read the bot's details (check the credentials and the bot info permission)",
    )?;
    let endpoint =
        discover_endpoint(&rest.http, &rest.base, &rest.app_id, &rest.app_secret).await?;
    let service = query_param(&endpoint.url, "service_id").unwrap_or_default();
    let (socket, _) = tokio_tungstenite::connect_async(&endpoint.url)
        .await
        .context("Could not open the WebSocket")?;
    let (mut sink, mut stream) = socket.split();
    set_status(
        status,
        ChannelState::Running,
        "Connected; waiting for messages",
    );
    let mut ping = tokio::time::interval(endpoint.ping_interval);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.reset();
    loop {
        tokio::select! {
            _ = stop.changed() => break Ok(Duration::ZERO),
            _ = ping.tick() => {
                if let Err(error) = sink.send(WsMessage::Binary(pbbp2::build_ping(&service).into())).await {
                    break Err(anyhow!("ping failed: {error}"));
                }
            }
            frame = stream.next() => {
                let Some(frame) = frame else { break Ok(endpoint.reconnect_interval) };
                let data = match frame {
                    Ok(WsMessage::Binary(data)) => data,
                    Ok(WsMessage::Close(_)) => break Ok(endpoint.reconnect_interval),
                    Ok(_) => continue,
                    Err(error) => break Err(anyhow!("read failed: {error}")),
                };
                let Ok(frame) = pbbp2::decode(&data) else { continue };
                if frame.method != 1 {
                    continue;
                }
                // Acknowledged first, to keep to the three seconds.
                if let Err(error) = sink.send(WsMessage::Binary(pbbp2::build_ack(&frame).into())).await {
                    break Err(anyhow!("ACK failed: {error}"));
                }
                if frame.header("type") != Some("event") {
                    continue;
                }
                if let Some(message) = parse_message_event(&frame.payload, &bot) {
                    if seen.insert(&message.event_id) {
                        // A full queue drops rather than stalling the reads.
                        let _ = events.try_send(message);
                    }
                }
            }
        }
    }
}

/// A turn shown on its card: progress at most about once a second, then
/// the answer, and the card closed.
async fn streamed_turn(
    server: &Server,
    rest: Arc<FeishuRest>,
    chat: &str,
    text: &str,
    card: String,
) {
    let sequence = Arc::new(AtomicI64::new(1));
    let (events, heard) = mpsc::unbounded_channel();
    let updater = tokio::spawn(show_progress(
        rest.clone(),
        card.clone(),
        sequence.clone(),
        heard,
    ));
    let reply = server
        .handle_inbound(Channel::Feishu, chat, text, Some(events))
        .await;
    let _ = updater.await;
    let answer = match reply.trim() {
        "" => "(本轮完成,但没有文本回复)",
        reply => reply,
    };
    let shown = rest
        .stream_card_content(&card, answer, sequence.fetch_add(1, Ordering::SeqCst))
        .await;
    if shown.is_err() {
        let _ = rest.send_text(chat, answer).await;
    }
    let _ = rest
        .close_streaming_card(&card, sequence.fetch_add(1, Ordering::SeqCst))
        .await;
}

async fn show_progress(
    rest: Arc<FeishuRest>,
    card: String,
    sequence: Arc<AtomicI64>,
    mut heard: mpsc::UnboundedReceiver<ChatEvent>,
) {
    let mut latest: Option<TurnProgress> = None;
    let mut ticker = tokio::time::interval(Duration::from_millis(900));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        tokio::select! {
            event = heard.recv() => match event {
                // Feishu cannot answer an approval by text; it waits for
                // the desktop, and the card says nothing misleading.
                Some(ChatEvent::Approval(_)) => {}
                Some(ChatEvent::Progress(progress)) => latest = Some(progress),
                None => break,
            },
            _ = ticker.tick(), if latest.is_some() => {
                if let Some(progress) = latest.take() {
                    let _ = rest
                        .stream_card_content(
                            &card,
                            &feishu_card::render(&progress),
                            sequence.fetch_add(1, Ordering::SeqCst),
                        )
                        .await;
                }
            }
        }
    }
}

/// Feishu as Settings shows it. The secret itself is never part of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeishuStatus {
    pub enabled: bool,
    /// An App ID and App Secret are kept.
    pub bound: bool,
    /// Lark, the international service, rather than Feishu.
    pub international: bool,
    pub app_id: String,
    pub has_secret: bool,
    pub state: ChannelState,
    pub detail: String,
    /// Who the bot answers; empty when nobody is confirmed.
    pub owner: String,
    /// Who wrote first while nobody was confirmed.
    pub pending_owner: String,
}

/// A QR code that creates a bot app in Feishu when scanned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeishuBind {
    pub flow: String,
    pub matrix: QrMatrix,
    pub expires_in_seconds: u64,
}

/// How far the scan that creates the app has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeishuBindPoll {
    Pending {
        retry_after_ms: u64,
    },
    /// The app was made and kept; the one who scanned is its owner.
    Confirmed {
        app_id: String,
    },
    Denied,
    Expired,
}

impl Server {
    fn feishu_hosts(&self) -> Hosts {
        self.0.channels.feishu_hosts.lock().unwrap().clone()
    }

    async fn feishu_owner(&self) -> Option<String> {
        let binding: OwnerBinding =
            serde_json::from_str(&get_setting(&self.0.store, OWNER).await).ok()?;
        let owner = binding.open_id.trim();
        (!owner.is_empty()).then(|| owner.to_owned())
    }

    async fn save_feishu_owner(&self, open_id: &str) -> Result<()> {
        let open_id = open_id.trim();
        ensure!(!open_id.is_empty(), "An owner's open_id cannot be empty");
        let binding = OwnerBinding {
            open_id: open_id.into(),
            bound_at: chrono::Utc::now().to_rfc3339(),
        };
        let store = &self.0.store;
        store
            .set_setting(OWNER, &serde_json::to_string(&binding)?)
            .await?;
        store.set_setting(PENDING_OWNER, "").await
    }

    /// Whether a sender may reach a turn; `Some` is the reply instead.
    async fn authorize_feishu_sender(&self, sender: &str) -> Option<String> {
        let owner = self.feishu_owner().await;
        match sender_decision(sender, owner.as_deref()) {
            SenderDecision::Allow => None,
            SenderDecision::Reject => Some(NON_OWNER_REPLY.into()),
            SenderDecision::Unbound => {
                let store = &self.0.store;
                if get_setting(store, PENDING_OWNER).await.trim() != sender {
                    let _ = store.set_setting(PENDING_OWNER, sender).await;
                }
                Some(UNBOUND_REPLY.into())
            }
        }
    }

    /// Feishu as it stands.
    pub async fn feishu_status(&self) -> FeishuStatus {
        let store = &self.0.store;
        let app_id = get_setting(store, APP_ID).await;
        let has_secret = !self.0.secrets.get(SECRET).unwrap_or_default().is_empty();
        let (state, detail) = self.0.channels.feishu.snapshot();
        FeishuStatus {
            enabled: get_setting(store, ENABLED).await == "true",
            bound: !app_id.is_empty() && has_secret,
            international: get_setting(store, INTERNATIONAL).await == "true",
            app_id,
            has_secret,
            state,
            detail,
            owner: self.feishu_owner().await.unwrap_or_default(),
            pending_owner: get_setting(store, PENDING_OWNER).await.trim().to_owned(),
        }
    }

    /// Keeps the app's ID, whether it is on Lark, a typed secret (none
    /// keeps the stored one), and turns the bot on or off.
    pub async fn set_feishu(
        &self,
        enabled: bool,
        international: bool,
        app_id: &str,
        app_secret: Option<String>,
    ) -> Result<FeishuStatus> {
        let app_id = app_id.trim();
        let typed = app_secret.as_deref().map(str::trim).unwrap_or_default();
        let stored = self.0.secrets.get(SECRET).unwrap_or_default();
        ensure!(
            !enabled || (!app_id.is_empty() && !(typed.is_empty() && stored.is_empty())),
            "Enter the App ID and App Secret before turning Feishu on"
        );
        if !typed.is_empty() {
            self.0.secrets.set(SECRET, typed)?;
        }
        let store = &self.0.store;
        store.set_setting(APP_ID, app_id).await?;
        store
            .set_setting(INTERNATIONAL, if international { "true" } else { "false" })
            .await?;
        store
            .set_setting(ENABLED, if enabled { "true" } else { "false" })
            .await?;
        if enabled {
            self.start_feishu().await;
        } else {
            self.0.channels.feishu.stop();
        }
        Ok(self.feishu_status().await)
    }

    /// Asks Feishu (or Lark) for a QR code that makes a bot app when the
    /// owner scans it.
    pub async fn start_feishu_bind(&self, international: bool) -> Result<FeishuBind> {
        let started = RegistrationFlow::begin(self.feishu_hosts().accounts, international).await?;
        let flow = uuid::Uuid::new_v4().to_string();
        let matrix = QrMatrix::of(&started.verification_uri)?;
        let mut flows = self.0.channels.feishu_flows.lock().await;
        flows.retain(|_, flow| !flow.expired());
        flows.insert(flow.clone(), started.flow);
        Ok(FeishuBind {
            flow,
            matrix,
            expires_in_seconds: started.expires_in_seconds,
        })
    }

    /// Where the scan stands. Once confirmed the app's secret is kept in
    /// the keychain, the one who scanned becomes the owner, and the bot
    /// restarts when it is on.
    pub async fn poll_feishu_bind(&self, flow: &str) -> Result<FeishuBindPoll> {
        let result = {
            let mut flows = self.0.channels.feishu_flows.lock().await;
            let registration = flows
                .get_mut(flow)
                .context("This Feishu scan is over; get a new QR code")?;
            let result = registration.poll().await?;
            if !matches!(result, RegistrationPoll::Pending { .. }) {
                flows.remove(flow);
            }
            result
        };
        Ok(match result {
            RegistrationPoll::Pending { retry_after } => FeishuBindPoll::Pending {
                retry_after_ms: retry_after.as_millis().min(u64::MAX as u128) as u64,
            },
            RegistrationPoll::Denied => FeishuBindPoll::Denied,
            RegistrationPoll::Expired => FeishuBindPoll::Expired,
            RegistrationPoll::Success {
                app_id,
                app_secret,
                international,
                owner_open_id,
            } => {
                self.0.secrets.set(SECRET, &app_secret)?;
                let store = &self.0.store;
                store.set_setting(APP_ID, &app_id).await?;
                store
                    .set_setting(INTERNATIONAL, if international { "true" } else { "false" })
                    .await?;
                if !owner_open_id.trim().is_empty() {
                    self.save_feishu_owner(&owner_open_id).await?;
                }
                if get_setting(store, ENABLED).await == "true" {
                    self.start_feishu().await;
                }
                FeishuBindPoll::Confirmed { app_id }
            }
        })
    }

    pub async fn cancel_feishu_bind(&self, flow: &str) {
        self.0.channels.feishu_flows.lock().await.remove(flow);
    }

    /// Forgets the app, its secret and its owner, and stops the bot.
    pub async fn unbind_feishu(&self) -> Result<FeishuStatus> {
        self.0.channels.feishu.stop();
        self.0.channels.feishu_flows.lock().await.clear();
        // The keychain has no delete here; empty reads as none.
        self.0.secrets.set(SECRET, "")?;
        let store = &self.0.store;
        store.set_setting(APP_ID, "").await?;
        store.set_setting(ENABLED, "false").await?;
        store.set_setting(OWNER, "").await?;
        store.set_setting(PENDING_OWNER, "").await?;
        Ok(self.feishu_status().await)
    }

    /// Names the owner by open_id; empty forgets the owner.
    pub async fn set_feishu_owner(&self, open_id: &str) -> Result<FeishuStatus> {
        if open_id.trim().is_empty() {
            let store = &self.0.store;
            store.set_setting(OWNER, "").await?;
            store.set_setting(PENDING_OWNER, "").await?;
        } else {
            self.save_feishu_owner(open_id).await?;
        }
        Ok(self.feishu_status().await)
    }

    /// Makes the one who wrote first the owner.
    pub async fn confirm_feishu_owner(&self) -> Result<FeishuStatus> {
        let pending = get_setting(&self.0.store, PENDING_OWNER).await;
        ensure!(
            !pending.trim().is_empty(),
            "Nobody is waiting to be confirmed"
        );
        self.save_feishu_owner(&pending).await?;
        Ok(self.feishu_status().await)
    }

    pub async fn reject_feishu_owner(&self) -> Result<FeishuStatus> {
        self.0.store.set_setting(PENDING_OWNER, "").await?;
        Ok(self.feishu_status().await)
    }

    /// Starts the bot in place of any running one.
    pub(crate) async fn start_feishu(&self) {
        let worker = &self.0.channels.feishu;
        let stop = worker.begin();
        let status = worker.status.clone();
        let store = &self.0.store;
        let app_id = get_setting(store, APP_ID).await;
        let secret = self.0.secrets.get(SECRET).unwrap_or_default();
        let international = get_setting(store, INTERNATIONAL).await == "true";
        if app_id.is_empty() || secret.is_empty() {
            set_status(
                &status,
                ChannelState::Error,
                "Enter the App ID and App Secret first",
            );
            return;
        }
        let base = self.feishu_hosts().open(international);
        tokio::spawn(run(self.clone(), app_id, secret, base, status, stop));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        chat_type: &str,
        message_type: &str,
        content: &str,
        mentions: serde_json::Value,
    ) -> Vec<u8> {
        json!({
            "header": {"event_type": "im.message.receive_v1", "event_id": "ev-1"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_user"}},
                "message": {
                    "chat_id": "oc_1",
                    "chat_type": chat_type,
                    "message_type": message_type,
                    "content": content,
                    "mentions": mentions,
                },
            },
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn text_messages_are_read_and_group_messages_need_a_mention() {
        let text = r#"{"text":"@_user_1 hello"}"#;
        let direct = parse_message_event(&event("p2p", "text", text, json!([])), "ou_bot").unwrap();
        assert_eq!(direct.chat_id, "oc_1");
        assert_eq!(direct.sender_open_id, "ou_user");
        assert_eq!(direct.text.as_deref(), Some("@_user_1 hello"));
        assert!(parse_message_event(&event("group", "text", text, json!([])), "ou_bot").is_none());
        let mentioned = json!([{"key": "@_user_1", "id": {"open_id": "ou_bot"}}]);
        let group =
            parse_message_event(&event("group", "text", text, mentioned), "ou_bot").unwrap();
        assert_eq!(group.text.as_deref(), Some("hello"));
        // A picture is a message without text.
        let image = parse_message_event(&event("p2p", "image", "{}", json!([])), "ou_bot").unwrap();
        assert_eq!(image.text, None);
        // The bot's own message is not one.
        assert!(parse_message_event(&event("p2p", "text", text, json!([])), "ou_user").is_none());
    }

    #[test]
    fn only_a_confirmed_owner_is_let_in() {
        assert_eq!(sender_decision("ou_a", Some("ou_a")), SenderDecision::Allow);
        assert_eq!(
            sender_decision("ou_b", Some("ou_a")),
            SenderDecision::Reject
        );
        assert_eq!(sender_decision("ou_b", None), SenderDecision::Unbound);
        assert_eq!(sender_decision("ou_b", Some("  ")), SenderDecision::Unbound);
        assert_eq!(sender_decision("", None), SenderDecision::Reject);
    }

    #[test]
    fn events_seen_lately_are_dropped() {
        let mut seen = SeenEvents::new(2);
        assert!(seen.insert("a"));
        assert!(!seen.insert("a"));
        assert!(seen.insert("b"));
        assert!(seen.insert("c"));
        // "a" left the window.
        assert!(seen.insert("a"));
    }

    #[test]
    fn only_secure_or_local_sockets_are_dialled() {
        assert!(acceptable_socket(
            "wss://msg-frontier.feishu.cn/ws?service_id=1"
        ));
        assert!(acceptable_socket("ws://127.0.0.1:4000/ws"));
        assert!(!acceptable_socket("ws://example.com/ws"));
        assert_eq!(
            query_param("wss://x/ws?a=1&service_id=42", "service_id").as_deref(),
            Some("42")
        );
    }
}
