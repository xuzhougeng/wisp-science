//! Feishu (Lark) channel: a self-built app bot over the official long
//! connection, so a desktop app needs no public callback URL.
//!
//! Flow: endpoint discovery (HTTP) → WSS → pbbp2 frames → ACK within 3s →
//! `im.message.receive_v1` events drive an agent session; replies go back over
//! REST (`tenant_access_token` cached, refreshed when <30 min remain).
//! Protocol facts follow phantty's tested implementation and the official Go
//! SDK (`larksuite/oapi-sdk-go`); payloads are plaintext JSON.

use super::{feishu_card, pbbp2, set_status, ChannelStatus};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const FEISHU_BASE: &str = "https://open.feishu.cn";
const LARK_BASE: &str = "https://open.larksuite.com";
const DEDUPE_WINDOW: usize = 128;

fn api_base(international: bool) -> &'static str {
    if international {
        LARK_BASE
    } else {
        FEISHU_BASE
    }
}

// ---------------------------------------------------------------- REST client

pub struct FeishuRest {
    http: reqwest::Client,
    app_id: String,
    app_secret: String,
    base: &'static str,
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
    pub fn new(app_id: &str, app_secret: &str, international: bool) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("wisp-science")
                .timeout(Duration::from_secs(30))
                .build()?,
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            base: api_base(international),
            token: tokio::sync::Mutex::new(None),
        })
    }

    /// Cached tenant token; Feishu only rotates it when <30 min remain, so we
    /// refresh on the same boundary.
    async fn tenant_token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        if let Some((token, expires_at)) = guard.as_ref() {
            if *expires_at > Instant::now() + Duration::from_secs(30 * 60) {
                return Ok(token.clone());
            }
        }
        let resp: TokenResp = self
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
        if resp.code != 0 {
            bail!(
                "tenant_access_token failed: code={} {}",
                resp.code,
                resp.msg
            );
        }
        let token = resp.tenant_access_token;
        *guard = Some((
            token.clone(),
            Instant::now() + Duration::from_secs(resp.expire),
        ));
        Ok(token)
    }

    pub async fn send_text(&self, chat_id: &str, text: &str) -> Result<()> {
        self.send_text_to("chat_id", chat_id, text).await
    }

    /// Message a user by `open_id`, outside any conversation they started.
    pub async fn send_text_to_user(&self, open_id: &str, text: &str) -> Result<()> {
        self.send_text_to("open_id", open_id, text).await
    }

    async fn send_text_to(&self, id_type: &str, id: &str, text: &str) -> Result<()> {
        let token = self.tenant_token().await?;
        let content = serde_json::to_string(&json!({ "text": text }))?;
        let resp: serde_json::Value = self
            .http
            .post(format!(
                "{}/open-apis/im/v1/messages?receive_id_type={id_type}",
                self.base
            ))
            .bearer_auth(token)
            .json(&json!({
                "receive_id": id,
                "msg_type": "text",
                "content": content,
            }))
            .send()
            .await?
            .json()
            .await?;
        let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            bail!(
                "send message failed: code={code} {}",
                resp.get("msg").and_then(|m| m.as_str()).unwrap_or("")
            );
        }
        Ok(())
    }

    pub async fn create_streaming_card(&self, initial_markdown: &str) -> Result<String> {
        let token = self.tenant_token().await?;
        let card = feishu_card::build_streaming_card(initial_markdown);
        let resp = checked_json(
            self.http
                .post(format!("{}/open-apis/cardkit/v1/cards", self.base))
                .bearer_auth(token)
                .json(&json!({ "type": "card_json", "data": card }))
                .send()
                .await?,
            "create streaming card",
        )
        .await?;
        resp.get("data")
            .and_then(|data| data.get("card_id"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("create streaming card response missing data.card_id"))
    }

    pub async fn send_card(&self, chat_id: &str, card_id: &str) -> Result<()> {
        let token = self.tenant_token().await?;
        let content = serde_json::to_string(&json!({
            "type": "card",
            "data": { "card_id": card_id },
        }))?;
        let resp = checked_json(
            self.http
                .post(format!(
                    "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
                    self.base
                ))
                .bearer_auth(token)
                .json(&json!({
                    "receive_id": chat_id,
                    "msg_type": "interactive",
                    "content": content,
                }))
                .send()
                .await?,
            "send streaming card",
        )
        .await?;
        if resp
            .get("data")
            .and_then(|data| data.get("message_id"))
            .and_then(|value| value.as_str())
            .is_none()
        {
            bail!("send streaming card response missing data.message_id");
        }
        Ok(())
    }

    pub async fn stream_card_content(
        &self,
        card_id: &str,
        content: &str,
        sequence: i64,
    ) -> Result<()> {
        let token = self.tenant_token().await?;
        checked_json(
            self.http
                .put(format!(
                    "{}/open-apis/cardkit/v1/cards/{}/elements/{}/content",
                    self.base,
                    card_id,
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

    pub async fn close_streaming_card(&self, card_id: &str, sequence: i64) -> Result<()> {
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

    /// The bot's own open_id, needed to detect "@ me" in group chats.
    pub async fn bot_open_id(&self) -> Result<String> {
        let token = self.tenant_token().await?;
        let resp: serde_json::Value = self
            .http
            .get(format!("{}/open-apis/bot/v3/info", self.base))
            .bearer_auth(token)
            .send()
            .await?
            .json()
            .await?;
        resp.get("bot")
            .and_then(|b| b.get("open_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("bot info response missing bot.open_id"))
    }
}

async fn checked_json(response: reqwest::Response, operation: &str) -> Result<serde_json::Value> {
    let status = response.status();
    let value: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("{operation}: invalid JSON response (HTTP {status})"))?;
    let code = value
        .get("code")
        .and_then(|code| code.as_i64())
        .unwrap_or(-1);
    if !status.is_success() || code != 0 {
        bail!(
            "{operation} failed: HTTP {status}, code={code} {}",
            value.get("msg").and_then(|msg| msg.as_str()).unwrap_or("")
        );
    }
    Ok(value)
}

// ------------------------------------------------------- endpoint discovery

struct Endpoint {
    url: String,
    ping_interval: Duration,
    reconnect_interval: Duration,
}

async fn discover_endpoint(
    http: &reqwest::Client,
    base: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<Endpoint> {
    // Key casing matters: lowercase keys get a 514 AuthFailed.
    let resp: serde_json::Value = http
        .post(format!("{base}/callback/ws/endpoint"))
        .json(&json!({"AppID": app_id, "AppSecret": app_secret}))
        .send()
        .await
        .context("endpoint discovery request failed")?
        .json()
        .await?;
    let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    if code != 0 {
        let msg = match code {
            514 => "AppID/AppSecret 校验失败".to_string(),
            1000040350 => "连接数超限(每应用最多 50 条)".to_string(),
            _ => resp
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown")
                .to_string(),
        };
        bail!("endpoint discovery failed: code={code} {msg}");
    }
    let data = resp
        .get("data")
        .ok_or_else(|| anyhow!("endpoint discovery: missing data"))?;
    let url = data
        .get("URL")
        .and_then(|u| u.as_str())
        .filter(|u| u.starts_with("wss://"))
        .ok_or_else(|| anyhow!("endpoint discovery: missing wss URL"))?
        .to_string();
    let cfg = data.get("ClientConfig").cloned().unwrap_or_default();
    let secs = |key: &str, default: u64| -> u64 {
        cfg.get(key)
            .and_then(|v| v.as_u64())
            .filter(|v| *v > 0)
            .unwrap_or(default)
    };
    Ok(Endpoint {
        url,
        ping_interval: Duration::from_secs(secs("PingInterval", 120)),
        reconnect_interval: Duration::from_secs(secs("ReconnectInterval", 30).min(120)),
    })
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

// ------------------------------------------------- event parsing (pure, tested)

#[derive(Debug, PartialEq)]
pub struct InboundMessage {
    pub event_id: String,
    pub chat_id: String,
    pub sender_open_id: String,
    /// None when the message is not plain text (image, file, sticker, …).
    pub text: Option<String>,
}

/// Persisted owner identity. The first inbound sender is never written here.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct OwnerBinding {
    pub open_id: String,
    #[serde(default)]
    pub bound_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SenderDecision {
    Allow,
    Reject,
    /// No owner is bound yet. Record a pending pairing request; do not accept.
    Unbound,
}

pub const NON_OWNER_REPLY: &str = "此机器人只响应已绑定的所有者。";
pub const UNBOUND_REPLY: &str =
    "尚未绑定所有者。请在桌面端「设置 → 远程接入 → 飞书」确认配对后，再发送消息。";

/// Compare an inbound sender with the desktop-confirmed owner.
///
/// An empty owner is never treated as "anyone": the first messenger stays
/// pending until the desktop confirms or an open_id is entered explicitly.
pub fn sender_decision(sender_open_id: &str, owner_open_id: Option<&str>) -> SenderDecision {
    if sender_open_id.is_empty() {
        return SenderDecision::Reject;
    }
    match owner_open_id.map(str::trim).filter(|id| !id.is_empty()) {
        None => SenderDecision::Unbound,
        Some(owner) if owner == sender_open_id => SenderDecision::Allow,
        Some(_) => SenderDecision::Reject,
    }
}

/// Normalize an `im.message.receive_v1` event payload. Returns None for other
/// event types, non-user senders, and group messages that do not @ the bot.
pub fn parse_message_event(payload: &[u8], bot_open_id: &str) -> Option<InboundMessage> {
    let v: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let header = v.get("header")?;
    if header.get("event_type")?.as_str()? != "im.message.receive_v1" {
        return None;
    }
    let event_id = header.get("event_id")?.as_str()?.to_string();
    let event = v.get("event")?;
    let message = event.get("message")?;
    let chat_id = message.get("chat_id")?.as_str()?.to_string();
    let sender_open_id = event
        .get("sender")
        .and_then(|s| s.get("sender_id"))
        .and_then(|s| s.get("open_id"))
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    if sender_open_id.is_empty() || sender_open_id == bot_open_id {
        return None;
    }
    let mentions = message
        .get("mentions")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    if message.get("chat_type").and_then(|c| c.as_str()) == Some("group") {
        // Group messages count only when the bot itself is mentioned; there is
        // no boolean for this — compare mention open_ids against our own.
        let at_me = mentions.iter().any(|m| {
            m.get("id")
                .and_then(|id| id.get("open_id"))
                .and_then(|id| id.as_str())
                == Some(bot_open_id)
        });
        if !at_me {
            return None;
        }
    }
    let text = if message.get("message_type").and_then(|t| t.as_str()) == Some("text") {
        // content is a JSON *string* that needs a second parse.
        let content = message.get("content")?.as_str()?;
        let inner: serde_json::Value = serde_json::from_str(content).ok()?;
        let mut text = inner.get("text")?.as_str()?.to_string();
        for m in &mentions {
            if let Some(key) = m.get("key").and_then(|k| k.as_str()) {
                text = text.replace(key, "");
            }
        }
        Some(text.trim().to_string())
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

// --------------------------------------------------------------- channel loop

pub async fn run(
    app: AppHandle,
    app_id: String,
    app_secret: String,
    international: bool,
    status: Arc<StdMutex<ChannelStatus>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let rest = match FeishuRest::new(&app_id, &app_secret, international) {
        Ok(rest) => Arc::new(rest),
        Err(e) => {
            set_status(&status, "error", &format!("HTTP 客户端初始化失败:{e}"));
            return;
        }
    };
    loop {
        set_status(&status, "connecting", "正在连接飞书…");
        // connect_once watches `shutdown` itself; stop latency during the
        // connection setup awaits is bounded by their HTTP timeouts.
        let result = connect_once(&app, &rest, &app_id, &app_secret, &status, &mut shutdown).await;
        if *shutdown.borrow() {
            break;
        }
        let (detail, wait) = match result {
            Ok(wait) => ("连接断开,准备重连…".to_string(), wait),
            Err(e) => (format!("{e:#}"), Duration::from_secs(30)),
        };
        set_status(&status, "error", &detail);
        tracing::warn!(target: "wisp", channel = "feishu", detail, "channel disconnected");
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = shutdown.changed() => break,
        }
    }
    set_status(&status, "stopped", "");
}

/// One connection lifetime. Returns the reconnect delay on orderly loss.
async fn connect_once(
    app: &AppHandle,
    rest: &Arc<FeishuRest>,
    app_id: &str,
    app_secret: &str,
    status: &Arc<StdMutex<ChannelStatus>>,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Duration> {
    let bot_open_id = rest
        .bot_open_id()
        .await
        .context("获取机器人信息失败(请检查凭证与「获取机器人信息」权限)")?;
    let ep = discover_endpoint(&rest.http, rest.base, app_id, app_secret).await?;
    let service_id = query_param(&ep.url, "service_id").unwrap_or_default();

    let (ws, _) = tokio_tungstenite::connect_async(&ep.url)
        .await
        .context("WSS 连接失败")?;
    let (mut sink, mut stream) = ws.split();

    // Replies and turn-driving run on a worker so the read loop can keep
    // ACKing within Feishu's 3-second deadline during long agent turns.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<InboundMessage>(64);
    let worker = {
        let app = app.clone();
        let rest = rest.clone();
        tokio::spawn(async move {
            while let Some(msg) = event_rx.recv().await {
                if let Some(reply) = super::authorize_feishu_sender(&app, &msg.sender_open_id).await
                {
                    if let Err(e) = rest.send_text(&msg.chat_id, &reply).await {
                        tracing::warn!(target: "wisp", channel = "feishu", error = %e, "send owner-auth reply failed");
                    }
                    continue;
                }
                let Some(text) = msg.text.as_deref().filter(|text| !text.is_empty()) else {
                    if let Err(e) = rest
                        .send_text(&msg.chat_id, "暂不支持该消息类型,请发送文本消息。")
                        .await
                    {
                        tracing::warn!(target: "wisp", channel = "feishu", error = %e, "send unsupported-message reply failed");
                    }
                    continue;
                };

                // Slash commands are immediate control-plane replies. Ordinary
                // agent turns get one CardKit card that evolves from progress
                // to the final answer, matching the desktop information flow.
                if !text.trim_start().starts_with('/') {
                    let initial = feishu_card::ProgressState::default().render();
                    match rest.create_streaming_card(&initial).await {
                        Ok(card_id) => match rest.send_card(&msg.chat_id, &card_id).await {
                            Ok(()) => {
                                run_streamed_turn(&app, rest.clone(), &msg.chat_id, text, card_id)
                                    .await;
                                continue;
                            }
                            Err(error) => {
                                tracing::warn!(target: "wisp", channel = "feishu", %error, "send progress card failed; falling back to text");
                            }
                        },
                        Err(error) => {
                            tracing::warn!(target: "wisp", channel = "feishu", %error, "create progress card failed; falling back to text");
                        }
                    }
                }

                let reply = super::handle_inbound(&app, "feishu", &msg.chat_id, text).await;
                if reply.is_empty() {
                    continue;
                }
                if let Err(e) = rest.send_text(&msg.chat_id, &reply).await {
                    tracing::warn!(target: "wisp", channel = "feishu", error = %e, "send reply failed");
                }
            }
        })
    };

    set_status(status, "running", "已连接,等待消息");
    let mut ping = tokio::time::interval(ep.ping_interval);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.reset(); // fire after one interval, not immediately
    let mut seen = SeenEvents::new(DEDUPE_WINDOW);

    let result: Result<Duration> = loop {
        tokio::select! {
            _ = shutdown.changed() => break Ok(Duration::ZERO),
            _ = ping.tick() => {
                if let Err(e) = sink.send(WsMessage::Binary(pbbp2::build_ping(&service_id).into())).await {
                    break Err(anyhow!("ping 发送失败:{e}"));
                }
            }
            frame = stream.next() => {
                let Some(frame) = frame else { break Ok(ep.reconnect_interval) };
                let data = match frame {
                    Ok(WsMessage::Binary(data)) => data,
                    Ok(WsMessage::Close(_)) => break Ok(ep.reconnect_interval),
                    Ok(_) => continue,
                    Err(e) => break Err(anyhow!("读取失败:{e}")),
                };
                let Ok(frame) = pbbp2::decode(&data) else {
                    tracing::warn!(target: "wisp", channel = "feishu", "frame decode error ({} bytes)", data.len());
                    continue;
                };
                if frame.method != 1 {
                    continue; // control frame (pong etc.)
                }
                // ACK before handling, to meet the 3s deadline no matter what.
                if let Err(e) = sink.send(WsMessage::Binary(pbbp2::build_ack(&frame).into())).await {
                    break Err(anyhow!("ACK 发送失败:{e}"));
                }
                if frame.header("type") != Some("event") {
                    continue;
                }
                if let Some(msg) = parse_message_event(&frame.payload, &bot_open_id) {
                    if seen.insert(&msg.event_id) {
                        // Queue full → drop rather than stall the read loop.
                        let _ = event_tx.try_send(msg);
                    }
                }
            }
        }
    };
    drop(event_tx);
    worker.abort();
    result
}

async fn run_streamed_turn(
    app: &AppHandle,
    rest: Arc<FeishuRest>,
    chat_id: &str,
    text: &str,
    card_id: String,
) {
    let sequence = Arc::new(AtomicI64::new(1));
    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress_worker = tokio::spawn(stream_progress_events(
        rest.clone(),
        card_id.clone(),
        sequence.clone(),
        progress_rx,
    ));

    let reply =
        super::handle_inbound_observed(app, "feishu", chat_id, text, Some(progress_tx)).await;
    // The observer is removed when handle_inbound_observed returns, closing
    // the receiver after every queued delta has been consumed.
    let _ = progress_worker.await;

    let final_text = if reply.trim().is_empty() {
        "(本轮完成,但没有文本回复)"
    } else {
        reply.trim()
    };
    let update = rest
        .stream_card_content(
            &card_id,
            final_text,
            sequence.fetch_add(1, Ordering::SeqCst),
        )
        .await;
    if let Err(error) = update {
        tracing::warn!(target: "wisp", channel = "feishu", %error, "final progress-card update failed; sending text fallback");
        let _ = rest.send_text(chat_id, final_text).await;
    }
    if let Err(error) = rest
        .close_streaming_card(&card_id, sequence.fetch_add(1, Ordering::SeqCst))
        .await
    {
        tracing::warn!(target: "wisp", channel = "feishu", %error, "close progress card failed");
    }
}

async fn stream_progress_events(
    rest: Arc<FeishuRest>,
    card_id: String,
    sequence: Arc<AtomicI64>,
    mut events: tokio::sync::mpsc::UnboundedReceiver<super::ProgressEvent>,
) {
    let mut state = feishu_card::ProgressState::default();
    let mut dirty = false;
    let mut ticker = tokio::time::interval(Duration::from_millis(900));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else {
                    if dirty {
                        let _ = rest.stream_card_content(
                            &card_id,
                            &state.render(),
                            sequence.fetch_add(1, Ordering::SeqCst),
                        ).await;
                    }
                    break;
                };
                match event {
                    super::ProgressEvent::AssistantDelta(delta) => state.assistant_delta(&delta),
                    super::ProgressEvent::Activity => state.reasoning_activity(),
                    super::ProgressEvent::ToolStarted { name, .. } => state.tool_started(&name),
                    super::ProgressEvent::ToolFinished { name, ok, duration_ms } => {
                        state.tool_finished(&name, ok, duration_ms);
                    }
                    // Feishu interactive approvals remain a separate follow-up.
                    // Its current worker cannot receive a reply while a turn is
                    // blocked, so do not project a misleading non-actionable card.
                    super::ProgressEvent::ApprovalRequested(_) | super::ProgressEvent::TurnFinished { .. } | super::ProgressEvent::TurnAnswer(_) => continue,
                }
                dirty = true;
            }
            _ = ticker.tick(), if dirty => {
                let rendered = state.render();
                if let Err(error) = rest.stream_card_content(
                    &card_id,
                    &rendered,
                    sequence.fetch_add(1, Ordering::SeqCst),
                ).await {
                    tracing::warn!(target: "wisp", channel = "feishu", %error, "progress-card update failed");
                }
                dirty = false;
            }
        }
    }
}

/// Fixed-size recent-event-id window for at-least-once delivery dedupe.
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

    /// Returns true when the id is new.
    fn insert(&mut self, id: &str) -> bool {
        if self.set.contains(id) {
            return false;
        }
        self.set.insert(id.to_string());
        self.order.push_back(id.to_string());
        if self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_json(
        chat_type: &str,
        message_type: &str,
        content: &str,
        mentions: serde_json::Value,
    ) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema": "2.0",
            "header": {"event_id": "ev-1", "event_type": "im.message.receive_v1"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_user"}},
                "message": {
                    "message_id": "om_1",
                    "chat_id": "oc_1",
                    "chat_type": chat_type,
                    "message_type": message_type,
                    "content": content,
                    "mentions": mentions,
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn parses_p2p_text() {
        let payload = event_json("p2p", "text", "{\"text\":\"hello\"}", json!([]));
        let msg = parse_message_event(&payload, "ou_bot").unwrap();
        assert_eq!(msg.event_id, "ev-1");
        assert_eq!(msg.chat_id, "oc_1");
        assert_eq!(msg.sender_open_id, "ou_user");
        assert_eq!(msg.text.as_deref(), Some("hello"));
    }

    #[test]
    fn group_requires_bot_mention_and_strips_placeholder() {
        let mentions = json!([{"key": "@_user_1", "id": {"open_id": "ou_bot"}}]);
        let payload = event_json(
            "group",
            "text",
            "{\"text\":\"@_user_1 run tests\"}",
            mentions,
        );
        let msg = parse_message_event(&payload, "ou_bot").unwrap();
        assert_eq!(msg.text.as_deref(), Some("run tests"));

        let other = json!([{"key": "@_user_1", "id": {"open_id": "ou_someone"}}]);
        let payload = event_json("group", "text", "{\"text\":\"@_user_1 hi\"}", other);
        assert!(parse_message_event(&payload, "ou_bot").is_none());
    }

    #[test]
    fn non_text_message_yields_none_text() {
        let payload = event_json("p2p", "image", "{\"image_key\":\"k\"}", json!([]));
        let msg = parse_message_event(&payload, "ou_bot").unwrap();
        assert_eq!(msg.text, None);
    }

    #[test]
    fn ignores_other_event_types_and_self_echo() {
        let payload = serde_json::to_vec(&json!({
            "header": {"event_id": "ev-2", "event_type": "card.action.trigger"},
            "event": {}
        }))
        .unwrap();
        assert!(parse_message_event(&payload, "ou_bot").is_none());

        let echo = serde_json::to_vec(&json!({
            "header": {"event_id": "ev-3", "event_type": "im.message.receive_v1"},
            "event": {
                "sender": {"sender_id": {"open_id": "ou_bot"}},
                "message": {"chat_id": "oc_1", "chat_type": "p2p",
                             "message_type": "text", "content": "{\"text\":\"x\"}"}
            }
        }))
        .unwrap();
        assert!(parse_message_event(&echo, "ou_bot").is_none());
    }

    #[test]
    fn sender_decision_never_auto_binds_the_first_messenger() {
        assert_eq!(sender_decision("ou_user", None), SenderDecision::Unbound);
        assert_eq!(
            sender_decision("ou_user", Some("")),
            SenderDecision::Unbound
        );
        assert_eq!(
            sender_decision("ou_user", Some("  ")),
            SenderDecision::Unbound
        );
    }

    #[test]
    fn sender_decision_allows_only_the_bound_owner() {
        assert_eq!(
            sender_decision("ou_owner", Some("ou_owner")),
            SenderDecision::Allow
        );
        assert_eq!(
            sender_decision("ou_stranger", Some("ou_owner")),
            SenderDecision::Reject
        );
        assert_eq!(
            sender_decision("", Some("ou_owner")),
            SenderDecision::Reject
        );
    }

    #[test]
    fn parse_keeps_non_owner_payloads_for_the_auth_gate() {
        // Auth is a separate step so group @-bot from a stranger still
        // produces an InboundMessage, then sender_decision rejects it.
        let mentions = json!([{"key": "@_user_1", "id": {"open_id": "ou_bot"}}]);
        let payload = event_json("group", "text", "{\"text\":\"@_user_1 hi\"}", mentions);
        let msg = parse_message_event(&payload, "ou_bot").unwrap();
        assert_eq!(msg.sender_open_id, "ou_user");
        assert_eq!(
            sender_decision(&msg.sender_open_id, Some("ou_owner")),
            SenderDecision::Reject
        );
        assert_eq!(
            sender_decision(&msg.sender_open_id, Some("ou_user")),
            SenderDecision::Allow
        );
    }

    #[test]
    fn query_param_extracts_service_id() {
        assert_eq!(
            query_param(
                "wss://x.feishu.cn/ws/v2?a=1&service_id=42&b=2",
                "service_id"
            )
            .as_deref(),
            Some("42")
        );
        assert_eq!(query_param("wss://x.feishu.cn/ws/v2", "service_id"), None);
    }

    #[test]
    fn seen_events_dedupes_within_window() {
        let mut seen = SeenEvents::new(2);
        assert!(seen.insert("a"));
        assert!(!seen.insert("a"));
        assert!(seen.insert("b"));
        assert!(seen.insert("c")); // evicts "a"
        assert!(seen.insert("a"));
    }
}
