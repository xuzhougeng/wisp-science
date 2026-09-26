//! SuperGrok / X Premium+ subscription login for xAI.
//!
//! RFC 8628 device code against `auth.x.ai` with the public Grok CLI client
//! (the same one Hermes Agent uses). The token endpoint comes from OIDC
//! discovery and must stay on `*.x.ai`, because it later receives the refresh
//! token. The access token is a Bearer key for the ordinary
//! `api.x.ai/v1/chat/completions` endpoint. Tokens stay with the caller
//! (the OS keyring). This module never writes them.

use crate::codex_auth::{decode_b64url, form_pairs, needs_refresh, now_ms, sleep_or_cancel};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
pub const DEVICE_CODE_URL: &str = "https://auth.x.ai/oauth2/device/code";
pub const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
pub const DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
pub const DEFAULT_MODEL: &str = "grok-4.6";
pub const SUBSCRIPTION_SECRET: &str = "xai_subscription";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEVICE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XaiCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
    pub token_endpoint: String,
    #[serde(default)]
    pub account: String,
}

impl XaiCredentials {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(raw: &str) -> Option<Self> {
        let creds: Self = serde_json::from_str(raw).ok()?;
        if creds.access_token.is_empty()
            || creds.refresh_token.is_empty()
            || validate_xai_url(&creds.token_endpoint).is_err()
        {
            return None;
        }
        Some(creds)
    }

    /// Account shown in Settings; the token response may carry no email.
    pub fn display_account(&self) -> String {
        if self.account.is_empty() {
            "xAI".into()
        } else {
            self.account.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Verification page with the code prefilled; falls back to the bare page.
    pub verification_uri_complete: String,
    pub interval_seconds: u64,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    Pending,
    SlowDown,
    Complete(String),
    Failed(String),
}

/// The bearer and refresh tokens must only ever go to HTTPS on `x.ai` or a
/// subdomain; a substituted discovery document would otherwise capture them.
pub fn validate_xai_url(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw.trim()).map_err(|_| format!("Invalid xAI URL: {raw}"))?;
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if url.scheme() != "https" {
        return Err(format!("xAI URL must use HTTPS: {raw}"));
    }
    if host != "x.ai" && !host.ends_with(".x.ai") {
        return Err(format!("xAI URL is not on x.ai: {raw}"));
    }
    Ok(())
}

pub fn token_endpoint_from_discovery(body: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| format!("xAI OIDC discovery was not JSON: {body}"))?;
    let endpoint = value
        .get("token_endpoint")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "xAI OIDC discovery has no token_endpoint".to_string())?;
    validate_xai_url(endpoint)?;
    Ok(endpoint.to_string())
}

fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&decode_b64url(payload)?).ok()
}

pub fn credentials_from_token_body(
    body: &str,
    previous_refresh: Option<&str>,
    token_endpoint: &str,
    now_ms: i64,
) -> Result<XaiCredentials, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| format!("xAI token response was not JSON: {body}"))?;
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| format!("xAI token response missing access_token: {body}"))?
        .to_string();
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .or_else(|| previous_refresh.map(str::to_string))
        .ok_or_else(|| "xAI token response missing refresh_token".to_string())?;
    let access_claims = jwt_claims(&access);
    // ponytail: no expiry at all means refresh before every turn, not a failed login.
    let expires_at_ms = match value.get("expires_in").and_then(Value::as_i64) {
        Some(secs) if secs > 0 => now_ms.saturating_add(secs.saturating_mul(1000)),
        _ => access_claims
            .as_ref()
            .and_then(|claims| claims.get("exp"))
            .and_then(Value::as_i64)
            .map(|exp| exp.saturating_mul(1000))
            .unwrap_or(0),
    };
    let account = value
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(jwt_claims)
        .or(access_claims)
        .and_then(|claims| {
            ["email", "preferred_username", "sub"]
                .into_iter()
                .find_map(|key| claims.get(key).and_then(Value::as_str).map(str::to_string))
        })
        .unwrap_or_default();
    Ok(XaiCredentials {
        access_token: access,
        refresh_token: refresh,
        expires_at_ms,
        token_endpoint: token_endpoint.to_string(),
        account,
    })
}

pub fn parse_device_code(body: &str) -> Result<DeviceCode, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| format!("xAI device code response was not JSON: {body}"))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
    };
    let number = |key: &str, default: u64| match value.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(default),
        _ => default,
    };
    let missing = |key: &str| format!("xAI device code response missing {key}: {body}");
    let verification_uri = text("verification_uri").ok_or_else(|| missing("verification_uri"))?;
    validate_xai_url(&verification_uri)?;
    let verification_uri_complete = text("verification_uri_complete")
        .filter(|url| validate_xai_url(url).is_ok())
        .unwrap_or_else(|| verification_uri.clone());
    Ok(DeviceCode {
        device_code: text("device_code").ok_or_else(|| missing("device_code"))?,
        user_code: text("user_code").ok_or_else(|| missing("user_code"))?,
        verification_uri,
        verification_uri_complete,
        interval_seconds: number("interval", 5).max(1),
        expires_in_seconds: number("expires_in", DEVICE_TIMEOUT.as_secs()),
    })
}

pub fn interpret_device_poll(status: u16, body: &str) -> DevicePoll {
    if (200..300).contains(&status) {
        return DevicePoll::Complete(body.to_string());
    }
    let error = serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    match error.as_deref() {
        Some("authorization_pending") => DevicePoll::Pending,
        Some("slow_down") => DevicePoll::SlowDown,
        Some("expired_token") => {
            DevicePoll::Failed("The xAI sign-in code expired. Start again.".into())
        }
        Some("access_denied") => DevicePoll::Failed("xAI sign-in was denied.".into()),
        _ => DevicePoll::Failed(format!("xAI device poll failed ({status}): {body}")),
    }
}

async fn post_form(
    client: &reqwest::Client,
    url: &str,
    pairs: &[(&str, &str)],
    what: &str,
) -> Result<(u16, String), String> {
    let response = client
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(form_pairs(pairs))
        .send()
        .await
        .map_err(|error| format!("xAI {what} failed: {error}"))?;
    let status = response.status().as_u16();
    Ok((status, response.text().await.unwrap_or_default()))
}

pub async fn discover_token_endpoint(client: &reqwest::Client) -> Result<String, String> {
    let response = client
        .get(DISCOVERY_URL)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|error| format!("xAI OIDC discovery failed: {error}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(format!("xAI OIDC discovery failed ({status}): {body}"));
    }
    token_endpoint_from_discovery(&body)
}

pub async fn start_device_login(client: &reqwest::Client) -> Result<DeviceCode, String> {
    let (status, body) = post_form(
        client,
        DEVICE_CODE_URL,
        &[("client_id", CLIENT_ID), ("scope", SCOPE)],
        "device code request",
    )
    .await?;
    if status >= 400 {
        return Err(format!("xAI device code request failed ({status}): {body}"));
    }
    parse_device_code(&body)
}

pub async fn poll_device_until_complete(
    client: &reqwest::Client,
    device: DeviceCode,
    token_endpoint: &str,
    cancel: &AtomicBool,
) -> Result<XaiCredentials, String> {
    validate_xai_url(token_endpoint)?;
    let started = std::time::Instant::now();
    let deadline = Duration::from_secs(device.expires_in_seconds).min(DEVICE_TIMEOUT);
    let mut interval = Duration::from_secs(device.interval_seconds);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled".into());
        }
        if started.elapsed() > deadline {
            return Err("xAI device sign-in timed out".into());
        }
        let (status, body) = post_form(
            client,
            token_endpoint,
            &[
                ("grant_type", DEVICE_CODE_GRANT),
                ("client_id", CLIENT_ID),
                ("device_code", &device.device_code),
            ],
            "device poll",
        )
        .await?;
        match interpret_device_poll(status, &body) {
            DevicePoll::Pending => sleep_or_cancel(interval, cancel).await?,
            DevicePoll::SlowDown => {
                interval = interval.saturating_add(Duration::from_secs(5));
                sleep_or_cancel(interval, cancel).await?;
            }
            DevicePoll::Complete(body) => {
                return credentials_from_token_body(&body, None, token_endpoint, now_ms());
            }
            DevicePoll::Failed(message) => return Err(message),
        }
    }
}

pub async fn refresh_credentials(
    client: &reqwest::Client,
    creds: XaiCredentials,
) -> Result<XaiCredentials, String> {
    validate_xai_url(&creds.token_endpoint)?;
    let (status, body) = post_form(
        client,
        &creds.token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", &creds.refresh_token),
        ],
        "token refresh",
    )
    .await?;
    if status >= 400 {
        return Err(format!("xAI token refresh failed ({status}): {body}"));
    }
    let mut next = credentials_from_token_body(
        &body,
        Some(&creds.refresh_token),
        &creds.token_endpoint,
        now_ms(),
    )?;
    if next.account.is_empty() {
        next.account = creds.account;
    }
    Ok(next)
}

pub async fn refresh_if_due(
    client: &reqwest::Client,
    creds: XaiCredentials,
    now_ms: i64,
) -> Result<XaiCredentials, String> {
    if !needs_refresh(creds.expires_at_ms, now_ms) && !creds.access_token.is_empty() {
        return Ok(creds);
    }
    refresh_credentials(client, creds).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";

    fn jwt(claims: &str) -> String {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!("{}.{}.sig", b64(r#"{"alg":"none"}"#), b64(claims))
    }

    #[test]
    fn only_https_x_ai_urls_may_receive_tokens() {
        assert!(validate_xai_url("https://auth.x.ai/oauth2/token").is_ok());
        assert!(validate_xai_url("https://x.ai/token").is_ok());
        assert!(validate_xai_url("http://auth.x.ai/oauth2/token").is_err());
        assert!(validate_xai_url("https://auth.x.ai.evil.example/token").is_err());
        assert!(validate_xai_url("https://notx.ai/token").is_err());
        assert!(token_endpoint_from_discovery(
            r#"{"token_endpoint":"https://evil.example/token"}"#
        )
        .is_err());
        assert_eq!(
            token_endpoint_from_discovery(&format!(r#"{{"token_endpoint":"{TOKEN_URL}"}}"#))
                .unwrap(),
            TOKEN_URL
        );
    }

    #[test]
    fn token_body_reads_expiry_account_and_keeps_old_refresh() {
        let id_token = jwt(r#"{"email":"grok@example.com"}"#);
        let body = format!(
            r#"{{"access_token":"a","refresh_token":"r","expires_in":3600,"id_token":"{id_token}"}}"#
        );
        let creds = credentials_from_token_body(&body, None, TOKEN_URL, 1_000).unwrap();
        assert_eq!(creds.expires_at_ms, 1_000 + 3_600_000);
        assert_eq!(creds.account, "grok@example.com");
        assert_eq!(creds.token_endpoint, TOKEN_URL);

        let access = jwt(r#"{"exp":2000,"sub":"user-1"}"#);
        let rotated = format!(r#"{{"access_token":"{access}"}}"#);
        let creds = credentials_from_token_body(&rotated, Some("old"), TOKEN_URL, 0).unwrap();
        assert_eq!(creds.refresh_token, "old");
        assert_eq!(creds.expires_at_ms, 2_000_000);
        assert_eq!(creds.account, "user-1");
        assert!(credentials_from_token_body(&rotated, None, TOKEN_URL, 0).is_err());
    }

    #[test]
    fn device_code_requires_an_x_ai_verification_page() {
        let device = parse_device_code(
            r#"{"device_code":"d","user_code":"ABCD-EFGH","verification_uri":"https://accounts.x.ai/device",
                "verification_uri_complete":"https://accounts.x.ai/device?code=ABCD-EFGH","interval":"3","expires_in":600}"#,
        )
        .unwrap();
        assert_eq!(device.user_code, "ABCD-EFGH");
        assert_eq!(device.interval_seconds, 3);
        assert_eq!(device.expires_in_seconds, 600);
        assert!(device.verification_uri_complete.ends_with("code=ABCD-EFGH"));
        assert!(parse_device_code(
            r#"{"device_code":"d","user_code":"U","verification_uri":"https://evil.example/device"}"#
        )
        .is_err());
    }

    #[test]
    fn device_poll_follows_rfc_8628_errors() {
        assert_eq!(
            interpret_device_poll(400, r#"{"error":"authorization_pending"}"#),
            DevicePoll::Pending
        );
        assert_eq!(
            interpret_device_poll(400, r#"{"error":"slow_down"}"#),
            DevicePoll::SlowDown
        );
        assert!(matches!(
            interpret_device_poll(400, r#"{"error":"expired_token"}"#),
            DevicePoll::Failed(_)
        ));
        assert_eq!(
            interpret_device_poll(200, "{}"),
            DevicePoll::Complete("{}".into())
        );
    }

    #[test]
    fn stored_credentials_reject_a_foreign_token_endpoint() {
        let creds = XaiCredentials {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at_ms: 1,
            token_endpoint: TOKEN_URL.into(),
            account: String::new(),
        };
        assert_eq!(
            XaiCredentials::from_json(&creds.to_json()),
            Some(creds.clone())
        );
        assert_eq!(creds.display_account(), "xAI");
        let forged = XaiCredentials {
            token_endpoint: "https://evil.example/token".into(),
            ..creds
        };
        assert!(XaiCredentials::from_json(&forged.to_json()).is_none());
    }
}
