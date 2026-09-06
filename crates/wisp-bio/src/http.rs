//! Bounded HTTP for the independently authored data clients. Credentials belong
//! to provider request builders; transport errors never print URLs or bodies.
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};

pub(crate) const MAX_RESPONSE: usize = 4 * 1024 * 1024;
type Pace = Arc<Mutex<Option<Instant>>>;
static PACERS: LazyLock<std::sync::Mutex<HashMap<&'static str, Pace>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

#[derive(Clone, Copy)]
pub(crate) struct Source(pub &'static str, pub Duration);
pub(crate) const NCBI: Source = Source("NCBI", Duration::from_millis(350));
pub(crate) const PMC_IDCONV: Source = Source("PMC ID Converter", Duration::from_millis(350));
pub(crate) const EUROPE_PMC: Source = Source("Europe PMC", Duration::from_millis(500));

#[derive(Clone)]
pub(crate) struct Http(pub reqwest::Client);

pub(crate) struct Response {
    pub status: StatusCode,
    pub body: Vec<u8>,
    pub total_count: Option<u64>,
    source: &'static str,
}

impl Response {
    pub fn check(&self) -> Result<()> {
        if !self.status.is_success() {
            bail!("{} returned HTTP {}", self.source, self.status.as_u16());
        }
        Ok(())
    }
    pub fn json(self) -> Result<Value> {
        self.check()?;
        serde_json::from_slice(&self.body).context("upstream returned invalid JSON")
    }
    pub fn text(self) -> Result<String> {
        self.check()?;
        String::from_utf8(self.body).context("upstream returned invalid UTF-8")
    }
}

impl Http {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::Client::builder()
                .user_agent(concat!("wisp-science/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        ))
    }

    pub async fn send(
        &self,
        source: Source,
        method: Method,
        url: &str,
        params: &[(String, String)],
    ) -> Result<Response> {
        self.exchange(source, method, url, params, None).await
    }

    /// JSON body with query-string parameters. Used by APIs that reject form posts
    /// (cBioPortal gene-filtered mutation and discrete CNA fetch).
    pub async fn send_json(
        &self,
        source: Source,
        method: Method,
        url: &str,
        params: &[(String, String)],
        body: &Value,
    ) -> Result<Response> {
        self.exchange(source, method, url, params, Some(body)).await
    }

    async fn exchange(
        &self,
        source: Source,
        method: Method,
        url: &str,
        params: &[(String, String)],
        json_body: Option<&Value>,
    ) -> Result<Response> {
        let pacer = PACERS
            .lock()
            .unwrap()
            .entry(source.0)
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone();
        for attempt in 0..2 {
            {
                let mut last = pacer.lock().await;
                if let Some(previous) = *last {
                    tokio::time::sleep_until(previous + source.1).await;
                }
                *last = Some(Instant::now());
            }
            let request = self.0.request(method.clone(), url);
            let request = if let Some(body) = json_body {
                request.query(params).json(body)
            } else if method == Method::GET {
                request.query(params)
            } else {
                request.form(params)
            };
            let mut response = request
                .send()
                .await
                .map_err(|_| anyhow!("{} connection failed or timed out", source.0))?;
            let status = response.status();
            let total_count = total_count_header(response.headers());
            if attempt == 0 && (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
            {
                let delay = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .map(|header| header.to_str().ok().and_then(retry_delay))
                    .unwrap_or(Some(2));
                if let Some(delay) = delay.filter(|seconds| *seconds <= 5) {
                    drop(response);
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
            }
            // Error bodies can echo credentials. Callers classify by status.
            if !status.is_success() {
                return Ok(Response {
                    status,
                    body: Vec::new(),
                    total_count: None,
                    source: source.0,
                });
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_RESPONSE as u64)
            {
                bail!(
                    "{} response exceeded 4 MiB; request fewer records",
                    source.0
                );
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| anyhow!("{} response could not be read", source.0))?
            {
                if body.len() + chunk.len() > MAX_RESPONSE {
                    bail!(
                        "{} response exceeded 4 MiB; request fewer records",
                        source.0
                    );
                }
                body.extend_from_slice(&chunk);
            }
            return Ok(Response {
                status,
                body,
                total_count,
                source: source.0,
            });
        }
        unreachable!("second attempt returns a response")
    }
}

fn total_count_header(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    for name in ["total-count", "x-total-count"] {
        if let Some(value) = headers.get(name).and_then(|header| header.to_str().ok()) {
            if let Ok(count) = value.parse::<u64>() {
                return Some(count);
            }
        }
    }
    None
}

fn retry_delay(value: &str) -> Option<u64> {
    value.parse().ok().or_else(|| {
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .map(|date| (date.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64)
    })
}
