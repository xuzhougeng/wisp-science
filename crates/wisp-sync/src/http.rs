use crate::{CommitOutcome, CommitRequest, FileRelay, SyncHead, SyncRevision, SyncTransport};
use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use url::Url;

pub const MAX_RELAY_BODY_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone)]
pub struct RelayHttpState {
    relay: FileRelay,
    bearer_token: Arc<str>,
}

impl RelayHttpState {
    pub fn new(relay: FileRelay, bearer_token: impl Into<String>) -> Result<Self> {
        let bearer_token = bearer_token.into();
        if bearer_token.trim().is_empty() {
            anyhow::bail!("relay bearer token cannot be empty");
        }
        Ok(Self {
            relay,
            bearer_token: bearer_token.into(),
        })
    }
}

pub fn relay_router(state: RelayHttpState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/projects/{project_id}/head", get(get_head))
        .route(
            "/v1/projects/{project_id}/revisions/{revision_id}",
            get(get_revision),
        )
        .route(
            "/v1/blobs/{blob_id}",
            get(get_blob).head(head_blob).put(put_blob),
        )
        .route("/v1/projects/{project_id}/commit", post(commit))
        .layer(DefaultBodyLimit::max(MAX_RELAY_BODY_BYTES))
        .with_state(state)
}

fn authorized(headers: &HeaderMap, state: &RelayHttpState) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| {
            const MESSAGE: &[u8] = b"wisp-relay-bearer-token";
            let expected_key =
                ring::hmac::Key::new(ring::hmac::HMAC_SHA256, state.bearer_token.as_bytes());
            let expected = ring::hmac::sign(&expected_key, MESSAGE);
            let provided_key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, token.as_bytes());
            ring::hmac::verify(&provided_key, MESSAGE, expected.as_ref()).is_ok()
        })
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

fn internal(error: anyhow::Error) -> Response {
    tracing::warn!("relay request failed: {error:#}");
    (StatusCode::BAD_REQUEST, "invalid relay request").into_response()
}

async fn get_head(
    State(state): State<RelayHttpState>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    match state.relay.head(&project_id).await {
        Ok(Some(head)) => Json(head).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => internal(error),
    }
}

async fn get_revision(
    State(state): State<RelayHttpState>,
    Path((project_id, revision_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    match state.relay.revision(&project_id, &revision_id).await {
        Ok(revision) => Json(revision).into_response(),
        Err(error) if error.to_string().contains("not found") => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(error) => internal(error),
    }
}

async fn head_blob(
    State(state): State<RelayHttpState>,
    Path(blob_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    match state.relay.blob_exists(&blob_id).await {
        Ok(true) => StatusCode::OK.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => internal(error),
    }
}

async fn get_blob(
    State(state): State<RelayHttpState>,
    Path(blob_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    match state.relay.get_blob(&blob_id).await {
        Ok(bytes) => (StatusCode::OK, bytes).into_response(),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(error) => internal(error),
    }
}

async fn put_blob(
    State(state): State<RelayHttpState>,
    Path(blob_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    match state.relay.put_blob(&blob_id, body.to_vec()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal(error),
    }
}

async fn commit(
    State(state): State<RelayHttpState>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CommitRequest>,
) -> Response {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    match state.relay.commit(&project_id, request).await {
        Ok(CommitOutcome::Committed(head)) => Json(head).into_response(),
        Ok(CommitOutcome::Conflict(head)) => (StatusCode::CONFLICT, Json(head)).into_response(),
        Err(error) => internal(error),
    }
}

#[derive(Clone)]
pub struct HttpRelay {
    base_url: Url,
    bearer_token: Arc<str>,
    client: reqwest::Client,
}

impl HttpRelay {
    pub fn new(base_url: &str, bearer_token: impl Into<String>) -> Result<Self> {
        let mut base_url = Url::parse(base_url.trim()).context("invalid relay URL")?;
        let secure = base_url.scheme() == "https";
        let local_http = base_url.scheme() == "http"
            && base_url
                .host_str()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        if !secure && !local_http {
            anyhow::bail!("relay URL must use HTTPS (HTTP is allowed only for localhost)");
        }
        if base_url.cannot_be_a_base() {
            anyhow::bail!("invalid relay base URL");
        }
        base_url.set_query(None);
        base_url.set_fragment(None);
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let bearer_token = bearer_token.into();
        if bearer_token.trim().is_empty() {
            anyhow::bail!("relay token is not configured");
        }
        Ok(Self {
            base_url,
            bearer_token: bearer_token.into(),
            client: reqwest::Client::new(),
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base_url.join(path).context("invalid relay endpoint")
    }

    fn component(value: &str, name: &str) -> Result<()> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        {
            anyhow::bail!("invalid {name}");
        }
        Ok(())
    }

    fn blob_id(value: &str) -> Result<()> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            anyhow::bail!("invalid blob id");
        }
        Ok(())
    }

    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        Ok(self
            .client
            .request(method, self.endpoint(path)?)
            .bearer_auth(&*self.bearer_token))
    }

    async fn response_error(response: reqwest::Response) -> anyhow::Error {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        anyhow::anyhow!("relay request failed ({status}): {}", detail.trim())
    }
}

#[async_trait]
impl SyncTransport for HttpRelay {
    async fn head(&self, project_id: &str) -> Result<Option<SyncHead>> {
        Self::component(project_id, "project id")?;
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("v1/projects/{project_id}/head"),
            )?
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(Self::response_error(response).await);
        }
        Ok(Some(response.json().await?))
    }

    async fn revision(&self, project_id: &str, revision_id: &str) -> Result<SyncRevision> {
        Self::component(project_id, "project id")?;
        Self::component(revision_id, "revision id")?;
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("v1/projects/{project_id}/revisions/{revision_id}"),
            )?
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Self::response_error(response).await);
        }
        Ok(response.json().await?)
    }

    async fn blob_exists(&self, blob_id: &str) -> Result<bool> {
        Self::blob_id(blob_id)?;
        let response = self
            .request(reqwest::Method::HEAD, &format!("v1/blobs/{blob_id}"))?
            .send()
            .await?;
        match response.status() {
            reqwest::StatusCode::OK => Ok(true),
            reqwest::StatusCode::NOT_FOUND => Ok(false),
            _ => Err(Self::response_error(response).await),
        }
    }

    async fn get_blob(&self, blob_id: &str) -> Result<Vec<u8>> {
        Self::blob_id(blob_id)?;
        let response = self
            .request(reqwest::Method::GET, &format!("v1/blobs/{blob_id}"))?
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Self::response_error(response).await);
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RELAY_BODY_BYTES as u64)
        {
            anyhow::bail!("relay blob exceeds the client size limit");
        }
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_RELAY_BODY_BYTES {
            anyhow::bail!("relay blob exceeds the client size limit");
        }
        Ok(bytes.to_vec())
    }

    async fn put_blob(&self, blob_id: &str, bytes: Vec<u8>) -> Result<()> {
        Self::blob_id(blob_id)?;
        if bytes.len() > MAX_RELAY_BODY_BYTES {
            anyhow::bail!("sync blob exceeds the relay size limit");
        }
        let response = self
            .request(reqwest::Method::PUT, &format!("v1/blobs/{blob_id}"))?
            .body(bytes)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Self::response_error(response).await);
        }
        Ok(())
    }

    async fn commit(&self, project_id: &str, request: CommitRequest) -> Result<CommitOutcome> {
        Self::component(project_id, "project id")?;
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("v1/projects/{project_id}/commit"),
            )?
            .json(&request)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::CONFLICT {
            return Ok(CommitOutcome::Conflict(response.json().await?));
        }
        if !response.status().is_success() {
            return Err(Self::response_error(response).await);
        }
        Ok(CommitOutcome::Committed(response.json().await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[test]
    fn client_requires_tls_except_for_loopback() {
        assert!(HttpRelay::new("https://relay.example.test", "token").is_ok());
        assert!(HttpRelay::new("http://127.0.0.1:8787", "token").is_ok());
        assert!(HttpRelay::new("http://relay.example.test", "token").is_err());
        assert!(HttpRelay::new("https://relay.example.test", "").is_err());
    }

    #[tokio::test]
    async fn bearer_auth_is_required_for_relay_routes() {
        let root = std::env::temp_dir().join(format!("wisp-http-auth-{}", uuid::Uuid::new_v4()));
        let relay = FileRelay::open(&root).await.unwrap();
        let state = RelayHttpState::new(relay, "correct-token").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer wrong-token".parse().unwrap(),
        );
        assert!(!authorized(&headers, &state));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer correct-token".parse().unwrap(),
        );
        assert!(authorized(&headers, &state));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn router_accepts_authenticated_blob_and_cas_commit_without_network() {
        let root = std::env::temp_dir().join(format!("wisp-http-route-{}", uuid::Uuid::new_v4()));
        let relay = FileRelay::open(&root).await.unwrap();
        let app = relay_router(RelayHttpState::new(relay, "token").unwrap());

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/projects/project-1/head")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let blob = b"encrypted-placeholder";
        let blob_id = crate::sha256_hex(blob);
        let uploaded = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/v1/blobs/{blob_id}"))
                    .header("authorization", "Bearer token")
                    .body(Body::from(blob.as_slice()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(uploaded.status(), StatusCode::NO_CONTENT);

        let revision = SyncRevision {
            protocol_version: crate::SYNC_PROTOCOL_VERSION,
            project_id: "project-1".into(),
            revision_id: "revision-1".into(),
            parent_revision: None,
            device_id: "device-1".into(),
            created_at: 1,
            metadata_blob: blob_id.clone(),
            manifest_blob: blob_id,
            workspace_blobs: vec![],
            state_hash: crate::sha256_hex(b"state"),
            auth_tag: crate::sha256_hex(b"auth"),
        };
        let request = CommitRequest {
            base_revision: None,
            revision,
        };
        let committed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/projects/project-1/commit")
                    .header("authorization", "Bearer token")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(committed.status(), StatusCode::OK);

        let head = app
            .oneshot(
                Request::builder()
                    .uri("/v1/projects/project-1/head")
                    .header("authorization", "Bearer token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
