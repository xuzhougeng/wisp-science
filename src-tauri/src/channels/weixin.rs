//! WeChat channel over the official iLink bot API (`ilinkai.weixin.qq.com`).
//!
//! Shape mirrors phantty's tested implementation: QR-scan binding yields a
//! `bot_token`; `getupdates` long-polls with an opaque cursor
//! (`get_updates_buf`); `sendmessage` replies as text. The scanning user is
//! the owner — only their 1:1 messages are handled; group messages are
//! dropped. `errcode == -14` means the session expired and the user must
//! re-scan. Replies must go out within ~30 min of the inbound message
//! (`context_token` window).

use super::{set_status, ChannelStatus};
use anyhow::{bail, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::watch;

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

pub async fn run(
    app: AppHandle,
    binding: Binding,
    token: String,
    status: Arc<StdMutex<ChannelStatus>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = match IlinkClient::new(&binding.base_url, &token) {
        Ok(c) => c,
        Err(e) => {
            set_status(&status, "error", &format!("HTTP 客户端初始化失败:{e}"));
            return;
        }
    };
    let state = app.state::<crate::AppState>();
    let mut cursor = super::get_setting(&state.store, "weixin_sync_buf").await;
    set_status(&status, "running", "已连接,等待消息");

    loop {
        let updates = tokio::select! {
            r = client.get_updates(&cursor) => r,
            _ = shutdown.changed() => break,
        };
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
            let _ = state.store.set_setting("weixin_enabled", "false").await;
            set_status(&status, "error", "微信登录已过期,请重新扫码绑定");
            tracing::warn!(target: "wisp", channel = "weixin", "session expired (-14); channel disabled");
            return;
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
            let Some(text) = extract_text(msg) else {
                let _ = client
                    .send_text(
                        &msg.from_user_id,
                        "暂不支持该消息类型,请发送文本消息。",
                        &msg.context_token,
                    )
                    .await;
                continue;
            };
            let reply = super::handle_inbound(&app, "weixin", &msg.from_user_id, &text).await;
            if reply.is_empty() {
                continue;
            }
            if let Err(e) = client
                .send_text(&msg.from_user_id, &reply, &msg.context_token)
                .await
            {
                tracing::warn!(target: "wisp", channel = "weixin", error = %e, "send reply failed");
            }
        }
        if !updates.get_updates_buf.is_empty() && updates.get_updates_buf != cursor {
            cursor = updates.get_updates_buf.clone();
            let _ = state.store.set_setting("weixin_sync_buf", &cursor).await;
        }
        let pause = Duration::from_millis(updates.longpolling_timeout_ms.max(1000) as u64);
        tokio::select! {
            _ = tokio::time::sleep(pause) => {}
            _ = shutdown.changed() => break,
        }
    }
    set_status(&status, "stopped", "");
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
