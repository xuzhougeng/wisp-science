//! Sign in with ChatGPT: OpenAI's official public-client flow for agents.
//!
//! Every login registers this installation as an agent: the authorize request
//! names the agent and a stable host id, the user approves it on the ChatGPT
//! consent page, and the callback returns the client id OpenAI issued for it.
//! Token exchange and refresh use that issued id. The access token is a Bearer
//! key for the ordinary `api.openai.com/v1/responses` endpoint and draws on the
//! user's ChatGPT plan. Mirrors pi's `openai-chatgpt` provider. Tokens stay
//! with the caller (the OS keyring). This module never writes them.

use crate::codex_auth::{
    account_email_from_access_token, form_pairs, http_error, needs_refresh, now_ms,
    transport_error, AuthorizationCallback, Pkce,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Placeholder that asks OpenAI to register a client for this login.
pub const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
/// Prefills the agent name on the consent page; the user can change it there.
pub const AGENT_NAME: &str = "Wisp Science";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const REDIRECT_URI: &str = "http://127.0.0.1:1455/auth/callback";
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub const DIRECT_TOKEN_SCOPE: &str = "chatgpt.tokens.use.direct";
pub const SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
pub const DEFAULT_BASE_URL: &str = RESOURCE;
pub const DEFAULT_MODEL: &str = "gpt-5.5";
pub const SUBSCRIPTION_SECRET: &str = "chatgpt_subscription";
/// Stable installation id. OpenAI lists each host as one connected agent, so a
/// new id per login would pile up agents in the user's ChatGPT settings.
pub const HOST_ID_SECRET: &str = "chatgpt_agent_host_id";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatGptCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
    /// Issued at sign-in; refresh must present it.
    pub client_id: String,
    /// Email from the ID token, for Settings only.
    #[serde(default)]
    pub account: String,
}

impl ChatGptCredentials {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(raw: &str) -> Option<Self> {
        let creds: Self = serde_json::from_str(raw).ok()?;
        if creds.access_token.is_empty()
            || creds.refresh_token.is_empty()
            || creds.client_id.is_empty()
        {
            return None;
        }
        Some(creds)
    }

    pub fn display_account(&self) -> String {
        if self.account.is_empty() {
            "ChatGPT".into()
        } else {
            self.account.clone()
        }
    }
}

pub fn authorize_url(pkce: &Pkce, state: &str, nonce: &str, host_id: uuid::Uuid) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("authorize url");
    url.query_pairs_mut()
        .append_pair("client_id", DYNAMIC_CLIENT_ID)
        .append_pair("agent_name_hint", AGENT_NAME)
        .append_pair("ext_agent_host_id", &format!("urn:uuid:{host_id}"))
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("resource", RESOURCE)
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("nonce", nonce);
    url.to_string()
}

/// A pasted redirect must be the full URL: it carries the issued client id.
pub fn callback_from_redirect(
    input: &str,
    expected_state: &str,
) -> Result<AuthorizationCallback, String> {
    let url = url::Url::parse(input.trim())
        .map_err(|_| "Paste the full redirect URL from the browser.".to_string())?;
    let param = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    if let Some(error) = param("error") {
        return Err(format!("ChatGPT authorization failed: {error}"));
    }
    let code = param("code").ok_or("The redirect URL has no authorization code.")?;
    if param("state").as_deref() != Some(expected_state) {
        return Err("That redirect belongs to a different sign-in attempt.".into());
    }
    let client_id = param("client_id")
        .ok_or("The redirect URL has no client_id. Paste the full URL from the address bar.")?;
    Ok(AuthorizationCallback {
        code,
        client_id: Some(client_id),
    })
}

pub fn credentials_from_token_body(
    body: &str,
    client_id: &str,
    previous: Option<&ChatGptCredentials>,
    now_ms: i64,
) -> Result<ChatGptCredentials, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| "ChatGPT token response was not JSON".to_string())?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let access_token = text("access_token").ok_or("ChatGPT token response missing access_token")?;
    let refresh_token = text("refresh_token")
        .or_else(|| previous.map(|creds| creds.refresh_token.clone()))
        .ok_or("ChatGPT token response missing refresh_token")?;
    // Without this scope the token cannot spend the ChatGPT plan.
    if !text("scope")
        .unwrap_or_default()
        .split_whitespace()
        .any(|scope| scope == DIRECT_TOKEN_SCOPE)
    {
        return Err(format!(
            "ChatGPT did not grant {DIRECT_TOKEN_SCOPE}. Sign in again and approve plan usage."
        ));
    }
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|secs| *secs > 0)
        .ok_or("ChatGPT token response has no expiry information")?;
    let account = text("id_token")
        .and_then(|token| account_email_from_access_token(&token))
        .or_else(|| previous.map(|creds| creds.account.clone()))
        .unwrap_or_default();
    Ok(ChatGptCredentials {
        access_token,
        refresh_token,
        expires_at_ms: now_ms.saturating_add(expires_in.saturating_mul(1000)),
        client_id: client_id.to_string(),
        account,
    })
}

async fn request_token(
    client: &reqwest::Client,
    stage: &str,
    pairs: &[(&str, &str)],
) -> Result<String, String> {
    let response = client
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(form_pairs(pairs))
        .send()
        .await
        .map_err(|error| transport_error(&format!("ChatGPT {stage}"), error))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(http_error(stage, status, &body));
    }
    Ok(body)
}

pub async fn exchange_authorization_code(
    client: &reqwest::Client,
    callback: &AuthorizationCallback,
    verifier: &str,
) -> Result<ChatGptCredentials, String> {
    let client_id = callback.client_id.as_deref().ok_or(
        "ChatGPT did not return a client id. Update Wisp or use the legacy Codex sign-in.",
    )?;
    let body = request_token(
        client,
        "sign-in",
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", &callback.code),
            ("code_verifier", verifier),
            ("redirect_uri", REDIRECT_URI),
            ("resource", RESOURCE),
        ],
    )
    .await?;
    credentials_from_token_body(&body, client_id, None, now_ms())
}

pub async fn refresh_if_due(
    client: &reqwest::Client,
    creds: ChatGptCredentials,
    now_ms: i64,
) -> Result<ChatGptCredentials, String> {
    if !needs_refresh(creds.expires_at_ms, now_ms) {
        return Ok(creds);
    }
    let body = request_token(
        client,
        "session refresh",
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &creds.client_id),
            ("refresh_token", &creds.refresh_token),
            ("resource", RESOURCE),
        ],
    )
    .await?;
    credentials_from_token_body(&body, &creds.client_id, Some(&creds), now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn jwt(payload: &str) -> String {
        let b64 = |text: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text);
        format!("{}.{}.sig", b64(r#"{"alg":"none"}"#), b64(payload))
    }

    #[test]
    fn authorize_url_registers_a_named_agent_for_this_host() {
        let pkce = Pkce {
            verifier: "verifier".into(),
            challenge: "challenge".into(),
        };
        let host = uuid::Uuid::parse_str("0B6F2F3C-8C1D-4B4E-9E2A-1C2D3E4F5A6B").unwrap();
        let url = url::Url::parse(&authorize_url(&pkce, "state-1", "nonce-1", host)).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.path(), "/api/accounts/authorize");
        let get = |key: &str| query.get(key).map(String::as_str);
        assert_eq!(get("client_id"), Some(DYNAMIC_CLIENT_ID));
        assert_eq!(get("agent_name_hint"), Some("Wisp Science"));
        assert_eq!(
            get("ext_agent_host_id"),
            Some("urn:uuid:0b6f2f3c-8c1d-4b4e-9e2a-1c2d3e4f5a6b")
        );
        assert_eq!(get("redirect_uri"), Some(REDIRECT_URI));
        assert_eq!(get("resource"), Some("https://api.openai.com/v1"));
        assert!(get("scope").is_some_and(|scope| scope.contains(DIRECT_TOKEN_SCOPE)));
        assert_eq!(get("code_challenge"), Some("challenge"));
        assert_eq!(get("code_challenge_method"), Some("S256"));
        assert_eq!(get("state"), Some("state-1"));
        assert_eq!(get("nonce"), Some("nonce-1"));
        assert!(get("originator").is_none());
    }

    #[test]
    fn pasted_redirect_needs_matching_state_and_the_issued_client_id() {
        let ok = callback_from_redirect(
            " http://127.0.0.1:1455/auth/callback?code=c1&state=s1&client_id=oaiapp_x ",
            "s1",
        )
        .unwrap();
        assert_eq!(ok.code, "c1");
        assert_eq!(ok.client_id.as_deref(), Some("oaiapp_x"));
        for (input, fragment) in [
            ("c1#s1", "full redirect URL"),
            (
                "http://127.0.0.1:1455/auth/callback?code=c1&state=other&client_id=a",
                "different sign-in",
            ),
            (
                "http://127.0.0.1:1455/auth/callback?code=c1&state=s1",
                "client_id",
            ),
            (
                "http://127.0.0.1:1455/auth/callback?error=access_denied&state=s1",
                "access_denied",
            ),
        ] {
            let error = callback_from_redirect(input, "s1").unwrap_err();
            assert!(error.contains(fragment), "{input}: {error}");
        }
    }

    #[test]
    fn token_body_requires_the_direct_scope_and_keeps_the_issued_client() {
        let id_token = jwt(r#"{"email":"researcher@example.org"}"#);
        let body = serde_json::json!({
            "access_token": "access-1",
            "refresh_token": "refresh-1",
            "id_token": id_token,
            "expires_in": 3600,
            "scope": "openid email offline_access resource.invoke chatgpt.tokens.use.direct",
        })
        .to_string();
        let creds = credentials_from_token_body(&body, "oaiapp_x", None, 1_000).unwrap();
        assert_eq!(creds.client_id, "oaiapp_x");
        assert_eq!(creds.expires_at_ms, 1_000 + 3_600_000);
        assert_eq!(creds.display_account(), "researcher@example.org");
        assert_eq!(
            ChatGptCredentials::from_json(&creds.to_json()).as_ref(),
            Some(&creds)
        );

        // A refresh may omit the rotated refresh token and the ID token.
        let refreshed = credentials_from_token_body(
            r#"{"access_token":"access-2","expires_in":60,"scope":"chatgpt.tokens.use.direct"}"#,
            "oaiapp_x",
            Some(&creds),
            2_000,
        )
        .unwrap();
        assert_eq!(refreshed.refresh_token, "refresh-1");
        assert_eq!(refreshed.account, "researcher@example.org");

        let error = credentials_from_token_body(
            r#"{"access_token":"a","refresh_token":"r","expires_in":60,"scope":"openid"}"#,
            "oaiapp_x",
            None,
            0,
        )
        .unwrap_err();
        assert!(error.contains(DIRECT_TOKEN_SCOPE));
        assert!(!error.contains("\"a\""));
    }

    #[test]
    fn stored_credentials_without_an_issued_client_are_unusable() {
        assert!(ChatGptCredentials::from_json(
            r#"{"access_token":"a","refresh_token":"r","expires_at_ms":1,"client_id":""}"#
        )
        .is_none());
        assert_eq!(
            ChatGptCredentials::from_json(
                r#"{"access_token":"a","refresh_token":"r","expires_at_ms":1,"client_id":"c"}"#
            )
            .unwrap()
            .display_account(),
            "ChatGPT"
        );
    }

    #[test]
    fn plan_sharing_limit_points_to_the_chatgpt_usage_page() {
        let message = http_error(
            "model request",
            429,
            r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"secret detail"}}"#,
        );
        assert!(message.contains("https://chatgpt.com/settings/usage"));
        assert!(!message.contains("secret detail"));
    }
}
