//! OAuth support for remote MCP servers.
//!
//! OAuth 2.0 authorization-code flow with PKCE, protected-resource metadata,
//! and dynamic client registration are handled without exposing credentials
//! to the UI. Connection metadata stays in normal MCP settings while every
//! credential remains in `wisp_store::secrets`.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::Instant,
};
use url::Url;

const CALLBACK_PATH: &str = "/callback";
const AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const CALLBACK_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone)]
pub struct PendingAuthorization {
    authorization_url: String,
    state: String,
    code_verifier: String,
    redirect_uri: String,
    client_id: String,
    client_secret: Option<String>,
    token_endpoint: String,
}

impl PendingAuthorization {
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }
}

#[derive(Deserialize)]
struct ProtectedResourceMetadata {
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Deserialize)]
struct AuthorizationServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct ClientRegistration {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Credential {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    token_endpoint: String,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: Option<i64>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

fn secret_name(connection_id: &str) -> String {
    format!("mcp_oauth:{connection_id}")
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build MCP OAuth HTTP client")
}

fn random_urlsafe(bytes: usize) -> Result<String> {
    let mut value = vec![0_u8; bytes];
    SystemRandom::new()
        .fill(&mut value)
        .map_err(|_| anyhow!("generate secure random OAuth value"))?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}

fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn callback_url(listener: &TcpListener) -> Result<String> {
    let address = listener
        .local_addr()
        .context("read MCP OAuth callback address")?;
    Ok(format!(
        "http://127.0.0.1:{}{CALLBACK_PATH}",
        address.port()
    ))
}

/// RFC 9728 places protected-resource metadata before the resource path. For
/// `https://mcp.notion.com/mcp`, the discovery URL is therefore
/// `https://mcp.notion.com/.well-known/oauth-protected-resource/mcp`.
fn protected_resource_metadata_url(resource: &str) -> Result<Url> {
    let resource = Url::parse(resource).context("parse MCP resource URL")?;
    let resource_path = resource.path().trim_end_matches('/');
    let mut metadata = resource.clone();
    metadata.set_path(&format!(
        "/.well-known/oauth-protected-resource{resource_path}"
    ));
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata)
}

/// RFC 8414 inserts the well-known suffix before an issuer path.
fn authorization_server_metadata_url(issuer: &str) -> Result<Url> {
    let issuer = Url::parse(issuer).context("parse MCP authorization server URL")?;
    let issuer_path = issuer.path().trim_end_matches('/');
    let mut metadata = issuer.clone();
    metadata.set_path(&format!(
        "/.well-known/oauth-authorization-server{issuer_path}"
    ));
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata)
}

/// OpenID Connect discovery appends its suffix to the issuer path.
fn openid_configuration_url(issuer: &str) -> Result<Url> {
    let issuer = Url::parse(issuer).context("parse MCP authorization server URL")?;
    let issuer_path = issuer.path().trim_end_matches('/');
    let mut metadata = issuer.clone();
    metadata.set_path(&format!("{issuer_path}/.well-known/openid-configuration"));
    metadata.set_query(None);
    metadata.set_fragment(None);
    Ok(metadata)
}

async fn json_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    operation: &str,
) -> Result<T> {
    let status = response.status();
    let text = response.text().await.context("read MCP OAuth response")?;
    if !status.is_success() {
        return Err(anyhow!(
            "{operation} failed with {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    serde_json::from_str(&text).with_context(|| format!("parse {operation} response"))
}

async fn discover_authorization_server(
    client: &reqwest::Client,
    issuer: &str,
) -> Result<AuthorizationServerMetadata> {
    let urls = [
        authorization_server_metadata_url(issuer)?,
        openid_configuration_url(issuer)?,
    ];
    let mut errors = Vec::new();
    for url in urls {
        let result = async {
            let response = client
                .get(url.clone())
                .send()
                .await
                .with_context(|| format!("request {url}"))?;
            json_response(response, "MCP authorization-server discovery").await
        }
        .await;
        match result {
            Ok(metadata) => return Ok(metadata),
            Err(error) => errors.push(format!("{url}: {error:#}")),
        }
    }
    Err(anyhow!(
        "MCP authorization-server discovery failed: {}",
        errors.join("; ")
    ))
}

/// Bind the loopback callback listener before registering a dynamic client, so
/// the exact redirect URI is known before the user opens a browser.
pub async fn begin_authorization(
    resource_url: &str,
) -> Result<(TcpListener, PendingAuthorization)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind local MCP OAuth callback")?;
    let redirect_uri = callback_url(&listener)?;
    let client = http_client()?;

    let protected_url = protected_resource_metadata_url(resource_url)?;
    let protected: ProtectedResourceMetadata = json_response(
        client
            .get(protected_url)
            .send()
            .await
            .context("discover MCP protected resource")?,
        "MCP OAuth discovery",
    )
    .await?;
    let auth_server = protected
        .authorization_servers
        .first()
        .ok_or_else(|| anyhow!("MCP OAuth discovery returned no authorization server"))?;
    let metadata = discover_authorization_server(&client, auth_server).await?;
    let registration_endpoint = metadata
        .registration_endpoint
        .as_deref()
        .ok_or_else(|| anyhow!("MCP OAuth server does not support dynamic client registration"))?;
    let registration: ClientRegistration = json_response(
        client
            .post(registration_endpoint)
            .header("accept", "application/json")
            .json(&json!({
                "client_name": "Wisp Science",
                "client_uri": "https://github.com/xuzhougeng/wisp-science",
                "redirect_uris": [redirect_uri],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none"
            }))
            .send()
            .await
            .context("register Wisp with MCP OAuth server")?,
        "MCP dynamic client registration",
    )
    .await?;

    let code_verifier = random_urlsafe(32)?;
    let state = random_urlsafe(32)?;
    let requested_scopes = protected.scopes_supported;
    let mut authorization_url =
        Url::parse(&metadata.authorization_endpoint).context("parse MCP authorization endpoint")?;
    let mut query = authorization_url.query_pairs_mut();
    query
        .append_pair("response_type", "code")
        .append_pair("client_id", &registration.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("state", &state)
        .append_pair("code_challenge", &code_challenge(&code_verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("resource", resource_url)
        .append_pair("prompt", "consent");
    if !requested_scopes.is_empty() {
        query.append_pair("scope", &requested_scopes.join(" "));
    }
    drop(query);
    Ok((
        listener,
        PendingAuthorization {
            authorization_url: authorization_url.into(),
            state,
            code_verifier,
            redirect_uri,
            client_id: registration.client_id,
            client_secret: registration.client_secret,
            token_endpoint: metadata.token_endpoint,
        },
    ))
}

fn callback_request_url(request: &str) -> Result<Url> {
    let target = request
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|line| line.split_whitespace().next())
        .ok_or_else(|| anyhow!("invalid local OAuth callback request"))?;
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .context("parse local OAuth callback URL")?;
    if url.path() != CALLBACK_PATH {
        return Err(anyhow!("unexpected local OAuth callback path"));
    }
    Ok(url)
}

fn callback_parameters(request: &str) -> Result<(Option<String>, String, Option<String>)> {
    let url = callback_request_url(request)?;
    let params = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    let error = params.get("error").map(|s| s.to_string());
    let state = params
        .get("state")
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("OAuth callback is missing state"))?;
    let code = params.get("code").map(|s| s.to_string());
    Ok((code, state, error))
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn reply_callback(stream: &mut TcpStream, ok: bool, message: &str) {
    let title = if ok {
        "MCP connected"
    } else {
        "MCP connection failed"
    };
    let message = html_escape(message);
    let body = format!("<!doctype html><meta charset=\"utf-8\"><title>{title}</title><h1>{title}</h1><p>{message}</p><p>You can close this tab and return to Wisp.</p>");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn read_callback_request(stream: &mut TcpStream, deadline: Instant) -> Option<String> {
    let mut request = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 2048];
    while request.len() < 16 * 1024 {
        let read_deadline = std::cmp::min(deadline, Instant::now() + CALLBACK_READ_TIMEOUT);
        let n = match tokio::time::timeout_at(read_deadline, stream.read(&mut buffer)).await {
            Ok(Ok(n)) => n,
            _ => return None,
        };
        if n == 0 {
            return None;
        }
        request.extend_from_slice(&buffer[..n]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&request).ok()?.to_string();
    callback_request_url(&request).is_ok().then_some(request)
}

async fn accept_callback_request(
    listener: &TcpListener,
    deadline: Instant,
) -> Result<(TcpStream, String)> {
    loop {
        let (mut stream, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| anyhow!("MCP authorization timed out after 10 minutes"))?
            .context("accept MCP OAuth callback")?;
        if let Some(request) = read_callback_request(&mut stream, deadline).await {
            return Ok((stream, request));
        }
        let _ = stream.shutdown().await;
    }
}

async fn exchange_code(pending: &PendingAuthorization, code: &str) -> Result<Credential> {
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("client_id", pending.client_id.clone()),
        ("redirect_uri", pending.redirect_uri.clone()),
        ("code_verifier", pending.code_verifier.clone()),
    ];
    if let Some(secret) = &pending.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    let tokens: TokenResponse = json_response(
        http_client()?
            .post(&pending.token_endpoint)
            .header("accept", "application/json")
            .form(&form)
            .send()
            .await
            .context("exchange MCP authorization code")?,
        "MCP token exchange",
    )
    .await?;
    Ok(Credential {
        client_id: pending.client_id.clone(),
        client_secret: pending.client_secret.clone(),
        token_endpoint: pending.token_endpoint.clone(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: tokens
            .expires_in
            .map(|seconds| Utc::now().timestamp() + seconds),
    })
}

/// Wait for the browser redirect, validate its CSRF state, then persist the
/// OAuth credential under the connection-specific keyring entry.
pub async fn finish_authorization(
    listener: TcpListener,
    pending: PendingAuthorization,
    connection_id: &str,
) -> Result<()> {
    let deadline = Instant::now() + AUTH_TIMEOUT;
    let (mut stream, request) = accept_callback_request(&listener, deadline).await?;
    let result = async {
        let (code, state, error) = callback_parameters(&request)?;
        if state != pending.state {
            return Err(anyhow!(
                "MCP OAuth state did not match; authorization was rejected"
            ));
        }
        if let Some(error) = error {
            return Err(anyhow!("MCP authorization was declined: {error}"));
        }
        let code = code.ok_or_else(|| anyhow!("MCP OAuth callback is missing code"))?;
        let credential = exchange_code(&pending, &code).await?;
        let secret =
            serde_json::to_string(&credential).context("serialize MCP OAuth credential")?;
        wisp_store::secrets::Secret::set(&secret_name(connection_id), &secret)
            .context("save MCP OAuth credential in OS keyring")?;
        Ok(())
    }
    .await;
    match &result {
        Ok(()) => reply_callback(&mut stream, true, "Authorization completed successfully.").await,
        Err(error) => reply_callback(&mut stream, false, &error.to_string()).await,
    }
    result
}

async fn refresh(connection_id: &str, credential: &mut Credential) -> Result<()> {
    let refresh_token = credential
        .refresh_token
        .clone()
        .ok_or_else(|| anyhow!("MCP access token expired; reconnect the service"))?;
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.clone()),
        ("client_id", credential.client_id.clone()),
    ];
    if let Some(secret) = &credential.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    let tokens: TokenResponse = json_response(
        http_client()?
            .post(&credential.token_endpoint)
            .header("accept", "application/json")
            .form(&form)
            .send()
            .await
            .context("refresh MCP access token")?,
        "MCP token refresh",
    )
    .await?;
    credential.access_token = tokens.access_token;
    if let Some(rotated) = tokens.refresh_token {
        credential.refresh_token = Some(rotated);
    }
    credential.expires_at = tokens
        .expires_in
        .map(|seconds| Utc::now().timestamp() + seconds);
    let secret =
        serde_json::to_string(credential).context("serialize refreshed MCP OAuth credential")?;
    wisp_store::secrets::Secret::set(&secret_name(connection_id), &secret)
        .context("save refreshed MCP OAuth credential in OS keyring")?;
    Ok(())
}

/// Connect the agent to an authorized remote MCP service, refreshing expiring
/// access tokens before the MCP handshake.
pub async fn connect(
    connection_id: &str,
    resource_url: &str,
    headers: &[(String, String)],
) -> Result<wisp_mcp::McpClient> {
    let raw = wisp_store::secrets::Secret::get(&secret_name(connection_id))
        .map_err(|_| anyhow!("OAuth authorization is not complete; reconnect the MCP service"))?;
    let mut credential: Credential =
        serde_json::from_str(&raw).context("parse saved MCP OAuth credential")?;
    if credential
        .expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now().timestamp() + 60)
    {
        refresh(connection_id, &mut credential).await?;
    }
    let mut authorized_headers = headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("authorization"))
        .cloned()
        .collect::<Vec<_>>();
    authorized_headers.push((
        "Authorization".to_string(),
        format!("Bearer {}", credential.access_token),
    ));
    wisp_mcp::McpClient::connect_http(resource_url, &authorized_headers).await
}

pub fn has_credential(connection_id: &str) -> bool {
    wisp_store::secrets::Secret::get(&secret_name(connection_id)).is_ok()
}

pub fn forget(connection_id: &str) {
    let _ = wisp_store::secrets::Secret::delete(&secret_name(connection_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc_7636_example() {
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn protected_resource_metadata_precedes_resource_path() {
        assert_eq!(
            protected_resource_metadata_url("https://mcp.notion.com/mcp")
                .unwrap()
                .as_str(),
            "https://mcp.notion.com/.well-known/oauth-protected-resource/mcp"
        );
    }

    #[test]
    fn authorization_server_metadata_preserves_issuer_path() {
        assert_eq!(
            authorization_server_metadata_url("https://login.example.com/tenant/oauth")
                .unwrap()
                .as_str(),
            "https://login.example.com/.well-known/oauth-authorization-server/tenant/oauth"
        );
        assert_eq!(
            openid_configuration_url("https://login.example.com/tenant/oauth")
                .unwrap()
                .as_str(),
            "https://login.example.com/tenant/oauth/.well-known/openid-configuration"
        );
    }

    #[test]
    fn callback_parser_decodes_code_and_state() {
        let (code, state, error) = callback_parameters(
            "GET /callback?code=abc%2B123&state=expected HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        )
        .unwrap();
        assert_eq!(code.as_deref(), Some("abc+123"));
        assert_eq!(state, "expected");
        assert!(error.is_none());
    }

    #[test]
    fn callback_parser_rejects_other_paths() {
        assert!(callback_parameters("GET /wrong?state=s HTTP/1.1\r\n\r\n").is_err());
    }

    #[test]
    fn callback_page_escapes_error_text() {
        assert_eq!(
            html_escape("<script>alert('x')</script>"),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;"
        );
    }

    #[tokio::test]
    async fn callback_listener_skips_empty_preconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let preconnect = TcpStream::connect(address).await.unwrap();
        drop(preconnect);

        let sender = tokio::spawn(async move {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream
                .write_all(b"GET /callback?code=abc&state=s HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .await
                .unwrap();
        });
        let (_, request) = accept_callback_request(
            &listener,
            Instant::now() + std::time::Duration::from_secs(2),
        )
        .await
        .unwrap();
        sender.await.unwrap();
        assert!(request.starts_with("GET /callback?"));
    }
}
