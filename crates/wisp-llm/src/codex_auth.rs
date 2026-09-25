//! ChatGPT Plus/Pro (Codex) subscription login.
//!
//! The flow matches the public Codex CLI client used by pi and OpenCode:
//! PKCE against `auth.openai.com`, either a localhost callback or a device
//! code, then Bearer calls to `chatgpt.com/backend-api/codex/responses`.
//! Tokens stay with the caller (the OS keyring). This module never writes them.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTH_BASE: &str = "https://auth.openai.com";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub const DEVICE_VERIFICATION_URI: &str = "https://auth.openai.com/codex/device";
pub const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
pub const SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub const JWT_AUTH_CLAIM: &str = "https://api.openai.com/auth";
pub const ORIGINATOR: &str = "wisp";
pub const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";
pub const DEFAULT_MODEL: &str = "gpt-5.5";
pub const SUBSCRIPTION_SECRET: &str = "codex_subscription";
const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;
const DEVICE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
    pub account_id: String,
}

impl CodexCredentials {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(raw: &str) -> Option<Self> {
        let creds: Self = serde_json::from_str(raw).ok()?;
        if creds.access_token.is_empty()
            || creds.refresh_token.is_empty()
            || creds.account_id.is_empty()
        {
            return None;
        }
        Some(creds)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    pub device_auth_id: String,
    pub user_code: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    Pending,
    SlowDown,
    Complete {
        authorization_code: String,
        code_verifier: String,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserCallback {
    Code(String),
    Cancelled,
    BindFailed(String),
    TimedOut,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn needs_refresh(expires_at_ms: i64, now_ms: i64) -> bool {
    expires_at_ms <= 0 || now_ms + REFRESH_SKEW_MS >= expires_at_ms
}

pub fn generate_pkce() -> Pkce {
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    Pkce {
        verifier,
        challenge,
    }
}

pub fn random_state() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn authorize_url(pkce: &Pkce, state: &str) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("authorize url");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", ORIGINATOR);
    url.to_string()
}

/// Accept a redirect URL, `code#state`, a query string, or a bare code.
pub fn parse_authorization_input(input: &str) -> (Option<String>, Option<String>) {
    let value = input.trim();
    if value.is_empty() {
        return (None, None);
    }
    if let Ok(url) = url::Url::parse(value) {
        return (
            nonempty(
                url.query_pairs()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.into_owned()),
            ),
            nonempty(
                url.query_pairs()
                    .find(|(k, _)| k == "state")
                    .map(|(_, v)| v.into_owned()),
            ),
        );
    }
    if let Some((code, state)) = value.split_once('#') {
        return (
            nonempty(Some(code.to_string())),
            nonempty(Some(state.to_string())),
        );
    }
    if value.contains("code=") {
        let params = url::form_urlencoded::parse(value.as_bytes());
        let mut code = None;
        let mut state = None;
        for (key, item) in params {
            if key == "code" {
                code = nonempty(Some(item.into_owned()));
            } else if key == "state" {
                state = nonempty(Some(item.into_owned()));
            }
        }
        return (code, state);
    }
    (Some(value.to_string()), None)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|item| !item.is_empty())
}

pub fn account_id_from_access_token(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = decode_b64url(payload)?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get(JWT_AUTH_CLAIM)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn decode_b64url(input: &str) -> Option<Vec<u8>> {
    let mut padded = input.replace('-', "+").replace('_', "/");
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(padded)
        .ok()
}

pub fn credentials_from_token_body(
    body: &str,
    previous_refresh: Option<&str>,
    now_ms: i64,
) -> Result<CodexCredentials, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| format!("Codex token response was not JSON: {body}"))?;
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| format!("Codex token response missing access_token: {body}"))?
        .to_string();
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .or_else(|| previous_refresh.map(str::to_string))
        .ok_or_else(|| "Codex token response missing refresh_token".to_string())?;
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|secs| *secs > 0)
        .ok_or_else(|| format!("Codex token response missing expires_in: {body}"))?;
    let account_id = account_id_from_access_token(&access)
        .ok_or_else(|| "Codex access token has no ChatGPT account id".to_string())?;
    Ok(CodexCredentials {
        access_token: access,
        refresh_token: refresh,
        expires_at_ms: now_ms.saturating_add(expires_in.saturating_mul(1000)),
        account_id,
    })
}

pub fn parse_device_code(body: &str) -> Result<DeviceCode, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| format!("Codex device code response was not JSON: {body}"))?;
    let device_auth_id = value
        .get("device_auth_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| format!("Codex device code response missing device_auth_id: {body}"))?
        .to_string();
    let user_code = value
        .get("user_code")
        .and_then(Value::as_str)
        .filter(|code| !code.is_empty())
        .ok_or_else(|| format!("Codex device code response missing user_code: {body}"))?
        .to_string();
    let interval = match value.get("interval") {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(5),
        Some(Value::String(text)) => text.trim().parse::<u64>().unwrap_or(5),
        _ => 5,
    };
    Ok(DeviceCode {
        device_auth_id,
        user_code,
        interval_seconds: interval.max(1),
    })
}

pub fn interpret_device_poll(status: u16, body: &str) -> DevicePoll {
    if (200..300).contains(&status) {
        let Ok(value) = serde_json::from_str::<Value>(body) else {
            return DevicePoll::Failed(format!("Codex device poll was not JSON: {body}"));
        };
        let code = value
            .get("authorization_code")
            .and_then(Value::as_str)
            .unwrap_or("");
        let verifier = value
            .get("code_verifier")
            .and_then(Value::as_str)
            .unwrap_or("");
        if code.is_empty() || verifier.is_empty() {
            return DevicePoll::Failed(format!(
                "Codex device poll missing authorization_code: {body}"
            ));
        }
        return DevicePoll::Complete {
            authorization_code: code.to_string(),
            code_verifier: verifier.to_string(),
        };
    }
    if status == 403 || status == 404 {
        return DevicePoll::Pending;
    }
    let code = serde_json::from_str::<Value>(body).ok().and_then(|value| {
        let error = value.get("error")?;
        if let Some(text) = error.as_str() {
            return Some(text.to_string());
        }
        error
            .get("code")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    match code.as_deref() {
        Some("deviceauth_authorization_pending") => DevicePoll::Pending,
        Some("slow_down") => DevicePoll::SlowDown,
        _ => DevicePoll::Failed(format!("Codex device poll failed ({status}): {body}")),
    }
}

pub fn codex_responses_url(base: &str) -> String {
    let normalized = base.trim().trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

pub fn http_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60));
    match proxy.map(str::trim) {
        None | Some("") => {}
        Some("none") => builder = builder.no_proxy(),
        Some(url) => {
            if let Ok(proxy) = reqwest::Proxy::all(url) {
                builder = builder.proxy(proxy);
            }
        }
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

pub async fn exchange_authorization_code(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<CodexCredentials, String> {
    let response = client
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_pairs(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ]))
        .send()
        .await
        .map_err(|error| format!("Codex token exchange failed: {error}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(format!("Codex token exchange failed ({status}): {body}"));
    }
    credentials_from_token_body(&body, None, now_ms())
}

pub async fn refresh_credentials(
    client: &reqwest::Client,
    creds: CodexCredentials,
) -> Result<CodexCredentials, String> {
    let response = client
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_pairs(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &creds.refresh_token),
            ("client_id", CLIENT_ID),
        ]))
        .send()
        .await
        .map_err(|error| format!("Codex token refresh failed: {error}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(format!("Codex token refresh failed ({status}): {body}"));
    }
    credentials_from_token_body(&body, Some(&creds.refresh_token), now_ms())
}

pub async fn refresh_if_due(
    client: &reqwest::Client,
    creds: CodexCredentials,
    now_ms: i64,
) -> Result<CodexCredentials, String> {
    if !needs_refresh(creds.expires_at_ms, now_ms) && !creds.access_token.is_empty() {
        return Ok(creds);
    }
    refresh_credentials(client, creds).await
}

pub async fn start_device_login(client: &reqwest::Client) -> Result<DeviceCode, String> {
    let response = client
        .post(DEVICE_USER_CODE_URL)
        .json(&json!({ "client_id": CLIENT_ID }))
        .send()
        .await
        .map_err(|error| format!("Codex device code request failed: {error}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status == 404 {
        return Err("Codex device-code login is not enabled. Use browser sign-in instead.".into());
    }
    if status >= 400 {
        return Err(format!(
            "Codex device code request failed ({status}): {body}"
        ));
    }
    parse_device_code(&body)
}

pub async fn poll_device_once(
    client: &reqwest::Client,
    device: &DeviceCode,
) -> Result<DevicePoll, String> {
    let response = client
        .post(DEVICE_TOKEN_URL)
        .json(&json!({
            "device_auth_id": device.device_auth_id,
            "user_code": device.user_code,
        }))
        .send()
        .await
        .map_err(|error| format!("Codex device poll failed: {error}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    Ok(interpret_device_poll(status, &body))
}

pub async fn poll_device_until_complete(
    client: &reqwest::Client,
    device: DeviceCode,
    cancel: &AtomicBool,
) -> Result<CodexCredentials, String> {
    let started = std::time::Instant::now();
    let mut interval = Duration::from_secs(device.interval_seconds);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Codex sign-in cancelled".into());
        }
        if started.elapsed() > DEVICE_TIMEOUT {
            return Err("Codex device sign-in timed out".into());
        }
        match poll_device_once(client, &device).await? {
            DevicePoll::Pending => sleep_or_cancel(interval, cancel).await?,
            DevicePoll::SlowDown => {
                interval = interval.saturating_add(Duration::from_secs(5));
                sleep_or_cancel(interval, cancel).await?;
            }
            DevicePoll::Complete {
                authorization_code,
                code_verifier,
            } => {
                return exchange_authorization_code(
                    client,
                    &authorization_code,
                    &code_verifier,
                    DEVICE_REDIRECT_URI,
                )
                .await;
            }
            DevicePoll::Failed(message) => return Err(message),
        }
    }
}

async fn sleep_or_cancel(duration: Duration, cancel: &AtomicBool) -> Result<(), String> {
    let step = Duration::from_millis(200);
    let mut left = duration;
    while left > Duration::ZERO {
        if cancel.load(Ordering::Relaxed) {
            return Err("Codex sign-in cancelled".into());
        }
        let slice = left.min(step);
        tokio::time::sleep(slice).await;
        left = left.saturating_sub(slice);
    }
    Ok(())
}

fn form_pairs(pairs: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

/// Read one browser redirect on `127.0.0.1:1455`. The redirect URI is fixed by
/// the Codex client registration, so the port cannot move.
pub fn wait_for_browser_callback(expected_state: &str, cancel: &AtomicBool) -> BrowserCallback {
    let listener = match std::net::TcpListener::bind("127.0.0.1:1455") {
        Ok(listener) => listener,
        Err(error) => {
            return BrowserCallback::BindFailed(format!(
                "Could not listen on 127.0.0.1:1455 ({error}). Paste the redirect URL instead."
            ));
        }
    };
    if listener.set_nonblocking(true).is_err() {
        return BrowserCallback::BindFailed(
            "Could not wait for the ChatGPT redirect. Paste the redirect URL instead.".into(),
        );
    }
    let deadline = std::time::Instant::now() + DEVICE_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return BrowserCallback::Cancelled;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 8192];
                let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let outcome = authorization_code_from_request(&request, expected_state);
                let (status, page) = match &outcome {
                    Ok(_) => (200, "ChatGPT sign-in completed. You can close this window."),
                    Err("state") => (
                        400,
                        "Sign-in state did not match. Return to Wisp and try again.",
                    ),
                    Err(_) => (400, "The redirect did not include an authorization code."),
                };
                let body = format!(
                    "<!doctype html><meta charset=\"utf-8\"><title>Wisp</title><p>{page}</p>"
                );
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
                if let Ok(code) = outcome {
                    return BrowserCallback::Code(code);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    BrowserCallback::TimedOut
}

pub fn authorization_code_from_request(
    request: &str,
    expected_state: &str,
) -> Result<String, &'static str> {
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or("request")?;
    let url = url::Url::parse(&format!("http://localhost{target}")).map_err(|_| "request")?;
    if url.path() != "/auth/callback" {
        return Err("path");
    }
    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .filter(|code| !code.is_empty())
        .ok_or("code")?;
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    if state != expected_state {
        return Err("state");
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_jwt(account: &str) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
            r#"{{"https://api.openai.com/auth":{{"chatgpt_account_id":"{account}"}}}}"#
        ));
        format!("{header}.{payload}.sig")
    }

    #[test]
    fn pkce_challenge_is_the_s256_digest_of_the_verifier() {
        let pkce = generate_pkce();
        assert!(pkce.verifier.len() >= 43);
        let digest = Sha256::digest(pkce.verifier.as_bytes());
        assert_eq!(
            pkce.challenge,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
        );
    }

    #[test]
    fn authorize_url_uses_the_codex_public_client() {
        let pkce = Pkce {
            verifier: "verifier".into(),
            challenge: "challenge".into(),
        };
        let url = url::Url::parse(&authorize_url(&pkce, "state-1")).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.path(), "/oauth/authorize");
        assert_eq!(query.get("client_id").map(String::as_str), Some(CLIENT_ID));
        assert_eq!(
            query.get("code_challenge").map(String::as_str),
            Some("challenge")
        );
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(
            query.get("redirect_uri").map(String::as_str),
            Some(REDIRECT_URI)
        );
        assert_eq!(query.get("state").map(String::as_str), Some("state-1"));
        assert_eq!(
            query.get("codex_cli_simplified_flow").map(String::as_str),
            Some("true")
        );
        assert!(query
            .get("scope")
            .is_some_and(|scope| scope.contains("api.connectors.read")));
        assert_eq!(query.get("originator").map(String::as_str), Some("wisp"));
    }

    #[test]
    fn authorization_input_accepts_redirect_url_hash_and_bare_code() {
        assert_eq!(
            parse_authorization_input("http://localhost:1455/auth/callback?code=abc&state=xyz"),
            (Some("abc".into()), Some("xyz".into()))
        );
        assert_eq!(
            parse_authorization_input("abc#xyz"),
            (Some("abc".into()), Some("xyz".into()))
        );
        assert_eq!(
            parse_authorization_input("code=abc&state=xyz"),
            (Some("abc".into()), Some("xyz".into()))
        );
        assert_eq!(
            parse_authorization_input("  bare-code  "),
            (Some("bare-code".into()), None)
        );
    }

    #[test]
    fn account_id_comes_from_the_chatgpt_auth_claim() {
        let token = sample_jwt("acct-1");
        assert_eq!(
            account_id_from_access_token(&token).as_deref(),
            Some("acct-1")
        );
        assert!(account_id_from_access_token("not-a-jwt").is_none());
    }

    #[test]
    fn token_body_keeps_the_previous_refresh_token_when_rotation_omits_it() {
        let token = sample_jwt("acct-9");
        let body = format!(r#"{{"access_token":"{token}","expires_in":3600}}"#);
        let creds = credentials_from_token_body(&body, Some("refresh-old"), 1_000).unwrap();
        assert_eq!(creds.account_id, "acct-9");
        assert_eq!(creds.refresh_token, "refresh-old");
        assert_eq!(creds.expires_at_ms, 1_000 + 3_600_000);
        assert!(credentials_from_token_body(&body, None, 0).is_err());
    }

    #[test]
    fn device_poll_maps_pending_slowdown_and_success() {
        assert_eq!(interpret_device_poll(404, ""), DevicePoll::Pending);
        assert_eq!(
            interpret_device_poll(400, r#"{"error":"deviceauth_authorization_pending"}"#),
            DevicePoll::Pending
        );
        assert_eq!(
            interpret_device_poll(400, r#"{"error":{"code":"slow_down"}}"#),
            DevicePoll::SlowDown
        );
        assert_eq!(
            interpret_device_poll(
                200,
                r#"{"authorization_code":"code","code_verifier":"ver"}"#
            ),
            DevicePoll::Complete {
                authorization_code: "code".into(),
                code_verifier: "ver".into(),
            }
        );
    }

    #[test]
    fn device_code_parses_string_intervals() {
        let device =
            parse_device_code(r#"{"device_auth_id":"dev","user_code":"ABCD-1234","interval":"5"}"#)
                .unwrap();
        assert_eq!(device.user_code, "ABCD-1234");
        assert_eq!(device.interval_seconds, 5);
    }

    #[test]
    fn responses_url_appends_the_codex_path_once() {
        assert_eq!(
            codex_responses_url("https://chatgpt.com/backend-api/"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            codex_responses_url("https://chatgpt.com/backend-api/codex"),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(
            codex_responses_url("https://relay.example/codex/responses"),
            "https://relay.example/codex/responses"
        );
    }

    #[test]
    fn callback_request_checks_path_and_state() {
        let request = "GET /auth/callback?code=abc&state=ok HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(
            authorization_code_from_request(request, "ok").unwrap(),
            "abc"
        );
        assert_eq!(
            authorization_code_from_request(request, "other"),
            Err("state")
        );
        assert_eq!(
            authorization_code_from_request("GET /other?code=abc&state=ok HTTP/1.1\r\n\r\n", "ok"),
            Err("path")
        );
    }

    #[test]
    fn refresh_is_due_inside_the_five_minute_window() {
        assert!(!needs_refresh(10_000_000, 0));
        assert!(needs_refresh(REFRESH_SKEW_MS, 0));
        assert!(needs_refresh(0, 0));
    }

    #[test]
    fn credential_json_round_trips_and_rejects_a_partial_document() {
        let creds = CodexCredentials {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_at_ms: 42,
            account_id: "acct".into(),
        };
        let parsed = CodexCredentials::from_json(&creds.to_json()).unwrap();
        assert_eq!(parsed, creds);
        assert!(CodexCredentials::from_json(r#"{"access_token":"only"}"#).is_none());
    }
}
