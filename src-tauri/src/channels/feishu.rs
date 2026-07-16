//! Feishu (Lark) channel: a self-built app bot over the official long
//! connection, so a desktop app needs no public callback URL.
//!
//! Flow: endpoint discovery (HTTP) → WSS → pbbp2 frames → ACK within 3s →
//! `im.message.receive_v1` events drive an agent session; replies go back over
//! REST (`tenant_access_token` cached, refreshed when <30 min remain).
//! Protocol facts follow phantty's tested implementation and the official Go
//! SDK (`larksuite/oapi-sdk-go`); payloads are plaintext JSON.

use super::{pbbp2, set_status, ChannelStatus};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const BASE: &str = "https://open.feishu.cn";
/// ponytail: Feishu-CN only; Lark International needs larksuite.com domains.
const DEDUPE_WINDOW: usize = 128;

// ---------------------------------------------------------------- REST client

pub struct FeishuRest {
    http: reqwest::Client,
    app_id: String,
    app_secret: String,
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
    pub fn new(app_id: &str, app_secret: &str) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("wisp-science")
                .timeout(Duration::from_secs(30))
                .build()?,
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
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
                "{BASE}/open-apis/auth/v3/tenant_access_token/internal"
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
        let token = self.tenant_token().await?;
        let content = serde_json::to_string(&json!({ "text": text }))?;
        let resp: serde_json::Value = self
            .http
            .post(format!(
                "{BASE}/open-apis/im/v1/messages?receive_id_type=chat_id"
            ))
            .bearer_auth(token)
            .json(&json!({
                "receive_id": chat_id,
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

    /// The bot's own open_id, needed to detect "@ me" in group chats.
    pub async fn bot_open_id(&self) -> Result<String> {
        let token = self.tenant_token().await?;
        let resp: serde_json::Value = self
            .http
            .get(format!("{BASE}/open-apis/bot/v3/info"))
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

// ------------------------------------------------------- endpoint discovery

struct Endpoint {
    url: String,
    ping_interval: Duration,
    reconnect_interval: Duration,
}

async fn discover_endpoint(
    http: &reqwest::Client,
    app_id: &str,
    app_secret: &str,
) -> Result<Endpoint> {
    // Key casing matters: lowercase keys get a 514 AuthFailed.
    let resp: serde_json::Value = http
        .post(format!("{BASE}/callback/ws/endpoint"))
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
    status: Arc<StdMutex<ChannelStatus>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let rest = match FeishuRest::new(&app_id, &app_secret) {
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
    let ep = discover_endpoint(&rest.http, app_id, app_secret).await?;
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
                let reply = match &msg.text {
                    Some(text) if !text.is_empty() => {
                        super::handle_inbound(&app, "feishu", &msg.chat_id, text).await
                    }
                    _ => "暂不支持该消息类型,请发送文本消息。".to_string(),
                };
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
