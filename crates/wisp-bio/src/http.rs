//! Bounded HTTP for the independently authored data clients. Credentials belong
//! to provider request builders; transport errors never print URLs or bodies.
use anyhow::{bail, Context, Result};
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
    location: Option<String>,
    content_range: Option<String>,
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
        self.execute(source, method.clone(), |outgoing| {
            let request = self.0.request(outgoing, url);
            if method == Method::GET {
                request.query(params)
            } else {
                request.form(params)
            }
        })
        .await
    }

    pub async fn send_json(&self, source: Source, url: &str, body: &Value) -> Result<Response> {
        self.execute(source, Method::POST, |_| self.0.post(url).json(body))
            .await
    }

    pub async fn get_accept(&self, source: Source, url: &str, accept: &str) -> Result<Response> {
        self.execute(source, Method::GET, |_| {
            self.0.get(url).header(reqwest::header::ACCEPT, accept)
        })
        .await
    }

    pub async fn poll_json(&self, source: Source, url: &str) -> Result<Response> {
        self.execute(source, Method::GET, |_| {
            self.0
                .get(url)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::CACHE_CONTROL, "no-cache")
        })
        .await
    }

    /// Small text-file submissions such as the Rfam batch sequence endpoint.
    pub async fn post_text_file(
        &self,
        source: Source,
        url: &str,
        field: &str,
        text: &str,
    ) -> Result<Response> {
        self.execute(source, Method::POST, |_| {
            let file = reqwest::multipart::Part::text(text.to_owned()).file_name("sequence.fa");
            self.0
                .post(url)
                .header(reqwest::header::ACCEPT, "application/json")
                .multipart(reqwest::multipart::Form::new().part(field.to_owned(), file))
        })
        .await
    }

    /// BioStudies file downloads may move between EMBL-EBI storage hosts.
    /// Each hop is a fresh GET: no authorization or query credentials are forwarded.
    pub async fn ebi_download(&self, source: Source, url: &str) -> Result<Response> {
        let original = reqwest::Url::parse(url).context("invalid download URL")?;
        let mut current = original.clone();
        for _ in 0..4 {
            let response = self
                .send(source, Method::GET, current.as_str(), &[])
                .await?;
            if !response.status.is_redirection() {
                return Ok(response);
            }
            let location = response
                .location
                .context("download redirect omitted its location")?;
            let next = current
                .join(&location)
                .context("invalid download redirect")?;
            let ebi = next.scheme() == "https"
                && next
                    .host_str()
                    .is_some_and(|host| host == "ebi.ac.uk" || host.ends_with(".ebi.ac.uk"));
            if !next.username().is_empty()
                || next.password().is_some()
                || (next.origin() != original.origin() && !ebi)
            {
                bail!("download redirected outside the trusted EMBL-EBI hosts");
            }
            current = next;
        }
        bail!("download exceeded its redirect limit")
    }

    /// A bounded, verified byte range; never silently accept a whole remote data file.
    pub async fn range(&self, source: Source, url: &str, start: u64, end: u64) -> Result<Vec<u8>> {
        if end < start || end - start >= MAX_RESPONSE as u64 {
            bail!("invalid or oversized byte range");
        }
        let response = self
            .execute(source, Method::GET, |_| {
                self.0
                    .get(url)
                    .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
            })
            .await?;
        response.check()?;
        if response.status != StatusCode::PARTIAL_CONTENT {
            bail!("{} did not honor the byte range", source.0);
        }
        let range = response
            .content_range
            .as_deref()
            .and_then(|s| s.strip_prefix("bytes "))
            .and_then(|s| s.split_once('/'))
            .and_then(|(range, _)| range.split_once('-'))
            .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)));
        if !range.is_some_and(|(a, b)| {
            a == start && b >= a && b <= end && b - a + 1 == response.body.len() as u64
        }) {
            bail!("{} returned an inconsistent byte range", source.0);
        }
        Ok(response.body)
    }

    /// JSON body with query-string parameters. Used by APIs that reject form posts
    /// (cBioPortal gene-filtered mutation and discrete CNA fetch).
    pub async fn send_json_query(
        &self,
        source: Source,
        method: Method,
        url: &str,
        params: &[(String, String)],
        body: &Value,
    ) -> Result<Response> {
        self.execute(source, method.clone(), |outgoing| {
            self.0.request(outgoing, url).query(params).json(body)
        })
        .await
    }

    async fn execute(
        &self,
        source: Source,
        method: Method,
        build: impl Fn(Method) -> reqwest::RequestBuilder,
    ) -> Result<Response> {
        let pacer = PACERS
            .lock()
            .unwrap()
            .entry(source.0)
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone();
        'attempts: for attempt in 0..2 {
            {
                let mut last = pacer.lock().await;
                if let Some(previous) = *last {
                    tokio::time::sleep_until(previous + source.1).await;
                }
                *last = Some(Instant::now());
            }
            let mut response = match build(method.clone()).send().await {
                Ok(response) => response,
                Err(_) if method == Method::GET && attempt == 0 => {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                Err(_) => bail!("{} connection failed or timed out", source.0),
            };
            let status = response.status();
            let total_count = total_count_header(response.headers());
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let content_range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
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
                    location,
                    content_range,
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
            loop {
                let chunk = match response.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(_) if method == Method::GET && attempt == 0 => {
                        drop(response);
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        continue 'attempts;
                    }
                    Err(_) => bail!("{} response could not be read", source.0),
                };
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
                location,
                content_range,
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::get, Router};

    #[tokio::test]
    async fn downloads_follow_local_hops_but_reject_foreign_redirects_and_bad_ranges() {
        let app = Router::new()
            .route(
                "/redirect",
                get(|| async { (StatusCode::FOUND, [("location", "/data")]) }),
            )
            .route(
                "/foreign",
                get(|| async {
                    (
                        StatusCode::FOUND,
                        [("location", "https://example.test/file")],
                    )
                }),
            )
            .route("/data", get(|| async { "sample\tvalue\n" }))
            .route(
                "/range",
                get(|headers: axum::http::HeaderMap| async move {
                    assert_eq!(headers["range"], "bytes=5-7");
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [("content-range", "bytes 5-7/20")],
                        "abc",
                    )
                }),
            )
            .route(
                "/bad-range",
                get(|| async {
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [("content-range", "bytes 6-8/20")],
                        "abc",
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = Http(
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        );
        let source = Source("download fixture", Duration::ZERO);
        assert_eq!(
            http.ebi_download(source, &format!("{base}/redirect"))
                .await
                .unwrap()
                .text()
                .unwrap(),
            "sample\tvalue\n"
        );
        assert!(http
            .ebi_download(source, &format!("{base}/foreign"))
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("trusted"));
        assert_eq!(
            http.range(source, &format!("{base}/range"), 5, 7)
                .await
                .unwrap(),
            b"abc"
        );
        assert!(http
            .range(source, &format!("{base}/bad-range"), 5, 7)
            .await
            .is_err());
        assert!(http
            .range(source, &format!("{base}/data"), 5, 7)
            .await
            .is_err());
        server.abort();
    }
}
