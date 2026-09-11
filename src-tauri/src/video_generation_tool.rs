//! `generate_video` — one configured video-model request that submits an
//! asynchronous generation job, polls until it completes, and writes the
//! downloaded MP4 into the current project. Supports xAI `grok-imagine-video`,
//! `grok-imagine-video-1.5`, and `grok-imagine-video-1.5-preview` through
//! OpenAI/xAI-compatible proxies.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolEvent, ToolResult};

const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_VIDEO_BYTES: usize = 500 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 1024 * 1024;
/// The proxy occasionally rejects a submission with `auth_unavailable` / 503;
/// a short retry usually succeeds.
const MAX_SUBMIT_RETRIES: u32 = 3;
const SUBMIT_RETRY_BACKOFF: Duration = Duration::from_secs(2);
/// Video generation typically takes 1–2 minutes; poll every 5 seconds and give
/// up after 10 minutes.
const POLL_INTERVAL: Duration = Duration::from_secs(5);
const POLL_TIMEOUT: Duration = Duration::from_secs(600);

pub struct GenerateVideoTool {
    api_url: String,
    api_key: String,
    model: String,
    proxy: Option<String>,
    session_id: String,
    options: crate::models::VideoGenerationOptions,
    poll_interval: Duration,
    poll_timeout: Duration,
}

impl GenerateVideoTool {
    pub fn new(api_url: String, api_key: String, model: String, proxy: Option<String>) -> Self {
        Self {
            api_url,
            api_key,
            model,
            proxy,
            session_id: uuid::Uuid::new_v4().to_string(),
            options: crate::models::VideoGenerationOptions::default(),
            poll_interval: POLL_INTERVAL,
            poll_timeout: POLL_TIMEOUT,
        }
    }

    pub fn with_options(mut self, options: crate::models::VideoGenerationOptions) -> Self {
        self.options = options;
        self
    }

    #[cfg(test)]
    fn with_poll_timing(mut self, interval: Duration, timeout: Duration) -> Self {
        self.poll_interval = interval;
        self.poll_timeout = timeout;
        self
    }

    pub fn with_session_id(mut self, session_id: &str) -> Self {
        self.session_id = session_id.to_string();
        self
    }

    fn request_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        wisp_llm::provider::apply_request_identity(
            request,
            &self.api_url,
            &self.options.user_agent,
            self.options.send_user_agent,
            &self.session_id,
            self.options.send_session_id,
            &self.options.session_header_name,
        )
    }

    fn api_root(&self) -> String {
        let base = self.api_url.trim().trim_end_matches('/');
        if let Some(root) = base.strip_suffix("/videos/generations") {
            root.to_string()
        } else if matches!(base, "https://api.x.ai") {
            format!("{base}/v1")
        } else {
            base.to_string()
        }
    }

    fn configured_model(&self) -> &str {
        self.model.trim()
    }

    fn request_body(
        &self,
        prompt: &str,
        duration: u32,
        aspect_ratio: &str,
        resolution: &str,
    ) -> Value {
        json!({
            "model": self.configured_model(),
            "prompt": prompt,
            "duration": duration,
            "aspect_ratio": aspect_ratio,
            "resolution": resolution,
        })
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/videos/generations",
            self.api_root().trim_end_matches('/')
        )
    }

    fn status_endpoint(&self, request_id: &str) -> String {
        format!(
            "{}/videos/{}",
            self.api_root().trim_end_matches('/'),
            request_id
        )
    }

    fn model_endpoint(&self) -> String {
        format!(
            "{}/models/{}",
            self.api_root().trim_end_matches('/'),
            self.model.trim()
        )
    }

    fn models_endpoint(&self) -> String {
        format!("{}/models", self.api_root().trim_end_matches('/'))
    }

    fn client(&self) -> Result<reqwest::Client, String> {
        let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(300));
        if self.options.send_user_agent {
            builder = builder.user_agent(wisp_llm::provider::effective_user_agent(
                &self.options.user_agent,
            ));
        }
        match self.proxy.as_deref().map(str::trim) {
            None | Some("") => {}
            Some("none") => builder = builder.no_proxy(),
            Some(proxy) => {
                builder = builder.proxy(
                    reqwest::Proxy::all(proxy)
                        .map_err(|error| format!("invalid video-generation proxy: {error}"))?,
                );
            }
        }
        builder
            .build()
            .map_err(|error| format!("video-generation HTTP client: {error}"))
    }

    async fn read_body(
        mut response: reqwest::Response,
        limit: usize,
        what: &str,
    ) -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("{what} response failed: {error}"))?
        {
            if body.len().saturating_add(chunk.len()) > limit {
                return Err(format!("{what} response is too large"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    fn error_message(status: reqwest::StatusCode, body: &[u8]) -> String {
        let message = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&body[..body.len().min(2_048)]).into());
        format!("video API returned {}: {message}", status.as_u16())
    }

    fn is_retryable_submit_error(status: reqwest::StatusCode, body: &[u8]) -> bool {
        status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            || String::from_utf8_lossy(body).contains("auth_unavailable")
    }

    /// Submit the generation job, retrying transient `auth_unavailable`/503
    /// failures. Returns the `request_id` to poll.
    async fn submit(
        &self,
        client: &reqwest::Client,
        prompt: &str,
        duration: u32,
        aspect_ratio: &str,
        resolution: &str,
    ) -> Result<String, String> {
        let body = self.request_body(prompt, duration, aspect_ratio, resolution);
        let mut last_error = String::new();
        for attempt in 0..=MAX_SUBMIT_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(SUBMIT_RETRY_BACKOFF * attempt).await;
            }
            let response = self
                .request_headers(client.post(self.endpoint()))
                .bearer_auth(self.api_key.trim())
                .json(&body)
                .send()
                .await
                .map_err(|error| format!("video-generation request failed: {error}"))?;
            let status = response.status();
            let body = Self::read_body(response, MAX_ERROR_BYTES, "video-generation").await?;
            if !status.is_success() {
                last_error = Self::error_message(status, &body);
                if attempt < MAX_SUBMIT_RETRIES && Self::is_retryable_submit_error(status, &body) {
                    continue;
                }
                return Err(last_error);
            }
            let value: Value = serde_json::from_slice(&body)
                .map_err(|error| format!("invalid video-generation response: {error}"))?;
            let request_id = value
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| "video-generation response has no request_id".to_string())?;
            return Ok(request_id.to_string());
        }
        Err(last_error)
    }

    /// Poll `GET /videos/{request_id}` until the job finishes. Returns the
    /// temporary video URL on `done`; `failed`/`expired` are tool errors.
    async fn poll_until_done(
        &self,
        client: &reqwest::Client,
        request_id: &str,
    ) -> Result<String, String> {
        let started = std::time::Instant::now();
        loop {
            if started.elapsed() > self.poll_timeout {
                return Err(format!(
                    "video generation did not finish within {} seconds",
                    self.poll_timeout.as_secs()
                ));
            }
            let response = self
                .request_headers(client.get(self.status_endpoint(request_id)))
                .bearer_auth(self.api_key.trim())
                .send()
                .await
                .map_err(|error| format!("video-generation status request failed: {error}"))?;
            let status = response.status();
            let body =
                Self::read_body(response, MAX_ERROR_BYTES, "video-generation status").await?;
            if !status.is_success() {
                return Err(Self::error_message(status, &body));
            }
            let value: Value = serde_json::from_slice(&body)
                .map_err(|error| format!("invalid video-generation status response: {error}"))?;
            match value.get("status").and_then(Value::as_str).unwrap_or("") {
                "done" => {
                    let url = value
                        .pointer("/video/url")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .ok_or_else(|| {
                            "video-generation status response has no video.url".to_string()
                        })?;
                    return Ok(url.to_string());
                }
                "failed" => {
                    let message = value
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error");
                    return Err(format!("video generation failed: {message}"));
                }
                "expired" => return Err("video generation request expired".into()),
                // "pending" (or anything unrecognized) keeps polling.
                _ => tokio::time::sleep(self.poll_interval).await,
            }
        }
    }

    async fn download_video(&self, client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
        let url = url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("video-generation response URL is not http(s)".into());
        }
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("video-generation download failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "video-generation download returned {}",
                response.status().as_u16()
            ));
        }
        Self::read_body(response, MAX_VIDEO_BYTES, "video-generation download").await
    }

    async fn generate(
        &self,
        prompt: &str,
        duration: u32,
        aspect_ratio: &str,
        resolution: &str,
    ) -> Result<Vec<u8>, String> {
        if !crate::models::is_video_generation_model(self.configured_model()) {
            return Err(crate::models::VIDEO_GENERATION_UNSUPPORTED.into());
        }
        if self.api_key.trim().is_empty() {
            return Err("the assigned video-generation model has no API key".into());
        }
        let client = self.client()?;
        let request_id = self
            .submit(&client, prompt, duration, aspect_ratio, resolution)
            .await?;
        let url = self.poll_until_done(&client, &request_id).await?;
        // The URL is temporary; download it immediately.
        self.download_video(&client, &url).await
    }

    /// Validate credentials and access without creating a billable video.
    ///
    /// Video-only models cannot be sent to Responses or Chat Completions. The
    /// provider model metadata route provides a lightweight authenticated probe.
    pub async fn validate_model_access(&self) -> Result<(), String> {
        if !crate::models::is_video_generation_model(self.configured_model()) {
            return Err(crate::models::VIDEO_GENERATION_UNSUPPORTED.into());
        }
        if self.api_key.trim().is_empty() {
            return Err("the assigned video-generation model has no API key".into());
        }
        let expected = self.configured_model();
        let client = self.client()?;
        for list_fallback in [false, true] {
            let endpoint = if list_fallback {
                self.models_endpoint()
            } else {
                self.model_endpoint()
            };
            let response = self
                .request_headers(client.get(endpoint))
                .bearer_auth(self.api_key.trim())
                .send()
                .await
                .map_err(|error| format!("video-generation validation failed: {error}"))?;
            let status = response.status();
            let body =
                Self::read_body(response, MAX_ERROR_BYTES, "video-generation validation").await?;
            if !status.is_success() {
                if !list_fallback
                    && matches!(
                        status,
                        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
                    )
                {
                    continue;
                }
                let message = serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|value| {
                        value
                            .pointer("/error/message")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| {
                        String::from_utf8_lossy(&body[..body.len().min(2_048)]).into()
                    });
                return Err(format!(
                    "video model API returned {}: {message}",
                    status.as_u16()
                ));
            }
            let value: Value = serde_json::from_slice(&body)
                .map_err(|error| format!("invalid video model response: {error}"))?;
            if list_fallback {
                let found = value
                    .get("data")
                    .and_then(Value::as_array)
                    .is_some_and(|models| {
                        models.iter().any(|model| {
                            model.get("id").and_then(Value::as_str).is_some_and(|id| {
                                crate::models::model_id_tail(id)
                                    .eq_ignore_ascii_case(crate::models::model_id_tail(expected))
                            })
                        })
                    });
                if !found {
                    return Err(format!("model list does not include {expected}"));
                }
            } else {
                let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
                if !crate::models::model_id_tail(id)
                    .eq_ignore_ascii_case(crate::models::model_id_tail(expected))
                {
                    return Err(format!(
                        "provider returned model '{}' while validating {expected}",
                        if id.is_empty() { "(missing)" } else { id }
                    ));
                }
            }
            return Ok(());
        }
        unreachable!("model validation always returns from one of its two probes")
    }
}

#[async_trait]
impl Tool for GenerateVideoTool {
    fn name(&self) -> &str {
        "generate_video"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "generate_video",
            "Generate one MP4 video with the configured video model (xAI grok-imagine-video, grok-imagine-video-1.5, or grok-imagine-video-1.5-preview) and save it inside the project. Call it when the user asks for a video, clip, or animation generated by a video model. Generation is asynchronous and usually takes 1-2 minutes; the tool submits the job, polls until it finishes, and downloads the result. Use a project-relative path under media/. On success, reference the saved file path in the final answer.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Complete self-contained visual brief for the video, derived from the user's request and relevant project context"
                    },
                    "path": {
                        "type": "string",
                        "description": "Project-relative output path in the form media/<descriptive-name>.mp4"
                    },
                    "duration": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 15,
                        "description": "Video length in seconds (default from the model profile, usually 5)"
                    },
                    "aspect_ratio": {
                        "type": "string",
                        "enum": ["16:9", "9:16", "1:1", "4:3", "3:4"],
                        "description": "Output aspect ratio (default from the model profile, usually 16:9)"
                    },
                    "resolution": {
                        "type": "string",
                        "enum": ["480p", "720p", "1080p"],
                        "description": "Output resolution (default from the model profile, usually 720p)"
                    }
                },
                "required": ["prompt", "path"]
            }),
        )
    }

    fn preview(&self, args: &Value) -> String {
        args.get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if prompt.is_empty() {
            return ToolResult::fail("generate_video error: prompt is required");
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return ToolResult::fail(format!(
                "generate_video error: prompt exceeds {MAX_PROMPT_BYTES} bytes"
            ));
        }
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if !path
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("mp4"))
        {
            return ToolResult::fail("generate_video error: path must end in .mp4");
        }
        if let Err(error) = wisp_tools::safety::validate_relative_pattern(path) {
            return ToolResult::fail(format!("generate_video {path} error: {error}"));
        }
        if std::path::Path::new(path).parent() != Some(std::path::Path::new("media")) {
            return ToolResult::fail(
                "generate_video error: path must be a file directly under media/",
            );
        }
        if let Err(error) = std::fs::create_dir_all(env.project_root().join("media")) {
            return ToolResult::fail(format!(
                "generate_video {path} error: cannot create media directory: {error}"
            ));
        }
        let real = match wisp_tools::safety::validate_file_path(env.project_root(), path) {
            Ok(path) => path,
            Err(error) => {
                return ToolResult::fail(format!("generate_video {path} error: {error}"));
            }
        };
        let duration = match args.get("duration") {
            None | Some(Value::Null) => self.options.duration_secs,
            Some(value) => match value.as_u64() {
                Some(value) if (1..=15).contains(&value) => value as u32,
                _ => {
                    return ToolResult::fail(
                        "generate_video error: duration must be an integer between 1 and 15",
                    )
                }
            },
        };
        let aspect_ratio = match args.get("aspect_ratio").and_then(Value::as_str) {
            None => self.options.aspect_ratio.as_str(),
            Some(value) if crate::models::VIDEO_ASPECT_RATIOS.contains(&value) => value,
            Some(_) => return ToolResult::fail("generate_video error: unsupported aspect_ratio"),
        };
        let resolution = match args.get("resolution").and_then(Value::as_str) {
            None => self.options.resolution.as_str(),
            Some(value) if crate::models::VIDEO_RESOLUTIONS.contains(&value) => value,
            Some(_) => return ToolResult::fail("generate_video error: unsupported resolution"),
        };
        let video = match self
            .generate(prompt, duration, aspect_ratio, resolution)
            .await
        {
            Ok(video) => video,
            Err(error) => return ToolResult::fail(format!("generate_video error: {error}")),
        };
        if let Err(error) = std::fs::write(&real, &video) {
            return ToolResult::fail(format!("generate_video {path} error: {error}"));
        }
        env.emit(ToolEvent::FileChanged { path: path.into() }).await;
        ToolResult::ok(format!(
            "Generated {} byte MP4 at {path}. Reference this file path in the final answer.",
            video.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct RecordingEnv {
        root: PathBuf,
        events: Mutex<Vec<ToolEvent>>,
    }

    #[async_trait]
    impl ToolEnv for RecordingEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, event: ToolEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    /// Bind a throwaway TCP listener and report its API base URL
    /// (`http://addr/v1`) plus the bare origin (`http://addr`) so canned
    /// responses can embed download URLs before serving starts.
    async fn bind() -> (tokio::net::TcpListener, String, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        (
            listener,
            format!("http://{address}/v1"),
            format!("http://{address}"),
        )
    }

    /// Serve one canned `(status, body)` per accepted connection, in order, and
    /// collect the raw requests.
    fn serve_script(
        listener: tokio::net::TcpListener,
        script: Vec<(u16, String)>,
    ) -> tokio::task::JoinHandle<Vec<String>> {
        tokio::spawn(async move {
            let mut requests = Vec::with_capacity(script.len());
            for (status, body) in script {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let header_end = loop {
                    let mut chunk = [0u8; 4096];
                    let read = stream.read(&mut chunk).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&chunk[..read]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or_default();
                while request.len() < header_end + content_length {
                    let mut chunk = [0u8; 4096];
                    let read = stream.read(&mut chunk).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&chunk[..read]);
                }
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                requests.push(String::from_utf8(request).unwrap());
            }
            requests
        })
    }

    fn fast_tool(api_url: String) -> GenerateVideoTool {
        GenerateVideoTool::new(
            api_url,
            "sk-test".into(),
            "grok-imagine-video".into(),
            Some("none".into()),
        )
        .with_poll_timing(Duration::from_millis(5), Duration::from_secs(10))
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "wisp_generate_video_{tag}_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[tokio::test]
    async fn submits_polls_and_downloads_the_mp4() {
        let (listener, api_url, origin) = bind().await;
        let requests = serve_script(
            listener,
            vec![
                (200, json!({"request_id": "req-1"}).to_string()),
                (200, json!({"status": "pending", "progress": 40}).to_string()),
                (
                    200,
                    json!({"status": "done", "progress": 100, "video": {"url": format!("{origin}/media/tmp.mp4"), "duration": 5}})
                        .to_string(),
                ),
                (200, "fake-mp4-bytes".into()),
            ],
        );
        let root = temp_root("done");
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = fast_tool(api_url)
            .with_options(crate::models::VideoGenerationOptions {
                send_user_agent: false,
                send_session_id: Some(true),
                session_header_name: "x-custom-session".into(),
                ..Default::default()
            })
            .with_session_id("frame-video")
            .run(
                &json!({
                    "prompt": "A cell dividing under a microscope",
                    "path": "media/division.mp4",
                    "duration": 8,
                    "aspect_ratio": "9:16",
                    "resolution": "1080p"
                }),
                &env,
            )
            .await;

        assert!(result.success, "{}", result.content);
        assert_eq!(
            std::fs::read(root.join("media/division.mp4")).unwrap(),
            b"fake-mp4-bytes"
        );
        assert!(env.events.lock().unwrap().iter().any(
            |event| matches!(event, ToolEvent::FileChanged { path } if path == "media/division.mp4")
        ));
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests
            .iter()
            .all(|request| !request.to_ascii_lowercase().contains("user-agent:")));
        assert!(requests
            .iter()
            .take(3)
            .all(|request| request.contains("x-custom-session: frame-video")));
        assert!(!requests[3].contains("x-custom-session:"));
        assert!(requests[0].starts_with("POST /v1/videos/generations HTTP/1.1"));
        let body: Value =
            serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["model"], "grok-imagine-video");
        assert_eq!(body["duration"], 8);
        assert_eq!(body["aspect_ratio"], "9:16");
        assert_eq!(body["resolution"], "1080p");
        assert!(requests[1].starts_with("GET /v1/videos/req-1 HTTP/1.1"));
        assert!(requests[2].starts_with("GET /v1/videos/req-1 HTTP/1.1"));
        assert!(requests[3].starts_with("GET /media/tmp.mp4 HTTP/1.1"));
        assert!(requests.iter().take(3).all(|request| request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-test")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn failed_status_is_a_tool_error() {
        let (listener, api_url, _origin) = bind().await;
        let requests = serve_script(
            listener,
            vec![
                (200, json!({"request_id": "req-2"}).to_string()),
                (
                    200,
                    json!({"status": "failed", "error": {"message": "content rejected"}})
                        .to_string(),
                ),
            ],
        );
        let root = temp_root("failed");
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = fast_tool(api_url)
            .run(&json!({"prompt": "clip", "path": "media/clip.mp4"}), &env)
            .await;

        assert!(!result.success);
        assert!(
            result.content.contains("content rejected"),
            "{}",
            result.content
        );
        assert!(!root.join("media/clip.mp4").exists());
        assert_eq!(requests.await.unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn expired_status_is_a_tool_error() {
        let (listener, api_url, _origin) = bind().await;
        let _requests = serve_script(
            listener,
            vec![
                (200, json!({"request_id": "req-3"}).to_string()),
                (200, json!({"status": "expired"}).to_string()),
            ],
        );
        let root = temp_root("expired");
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = fast_tool(api_url)
            .run(&json!({"prompt": "clip", "path": "media/clip.mp4"}), &env)
            .await;

        assert!(!result.success);
        assert!(result.content.contains("expired"), "{}", result.content);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn auth_unavailable_submission_is_retried() {
        let (listener, api_url, _origin) = bind().await;
        let requests = serve_script(
            listener,
            vec![
                (
                    503,
                    json!({"error": {"message": "auth_unavailable: token refresh in progress"}})
                        .to_string(),
                ),
                (200, json!({"request_id": "req-4"}).to_string()),
                (
                    200,
                    json!({"status": "done", "video": {"url": "http://127.0.0.1:9/unused.mp4"}})
                        .to_string(),
                ),
            ],
        );
        // The done URL is unreachable on purpose: the submission retry is what
        // this test asserts, and the download failure proves polling reached
        // `done` after the retried submit.
        let root = temp_root("retry");
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = fast_tool(api_url)
            .run(&json!({"prompt": "clip", "path": "media/clip.mp4"}), &env)
            .await;

        // The submission retry succeeded; only the (unreachable) download fails.
        assert!(!result.success);
        assert!(result.content.contains("download"), "{}", result.content);
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].starts_with("POST /v1/videos/generations HTTP/1.1"));
        assert!(requests[1].starts_with("POST /v1/videos/generations HTTP/1.1"));
        assert!(requests[2].starts_with("GET /v1/videos/req-4 HTTP/1.1"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn non_retryable_submit_error_is_not_retried() {
        let (listener, api_url, _origin) = bind().await;
        let requests = serve_script(
            listener,
            vec![(400, json!({"error": {"message": "bad prompt"}}).to_string())],
        );
        let root = temp_root("badreq");
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = fast_tool(api_url)
            .run(&json!({"prompt": "clip", "path": "media/clip.mp4"}), &env)
            .await;

        assert!(!result.success);
        assert!(result.content.contains("bad prompt"), "{}", result.content);
        assert_eq!(requests.await.unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rejects_unsafe_paths_before_calling_the_api() {
        let root = temp_root("path");
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let tool = fast_tool("http://127.0.0.1:9/v1".into());

        for path in [
            "clip.mp4",
            "../media/clip.mp4",
            "media/sub/clip.mp4",
            "media/clip.png",
        ] {
            let result = tool
                .run(&json!({"prompt": "clip", "path": path}), &env)
                .await;
            assert!(!result.success, "{path} should be rejected");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn rejects_invalid_overrides_before_calling_the_api() {
        let root = temp_root("args");
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let tool = fast_tool("http://127.0.0.1:9/v1".into());

        for args in [
            json!({"prompt": "clip", "path": "media/clip.mp4", "duration": 0}),
            json!({"prompt": "clip", "path": "media/clip.mp4", "duration": 16}),
            json!({"prompt": "clip", "path": "media/clip.mp4", "aspect_ratio": "21:9"}),
            json!({"prompt": "clip", "path": "media/clip.mp4", "resolution": "4k"}),
        ] {
            let result = tool.run(&args, &env).await;
            assert!(!result.success, "{args} should be rejected");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn request_body_uses_profile_defaults() {
        let tool = GenerateVideoTool::new(
            "https://api.x.ai".into(),
            "xai-test".into(),
            "grok-imagine-video-1.5".into(),
            Some("none".into()),
        )
        .with_options(crate::models::VideoGenerationOptions {
            user_agent: String::new(),
            send_user_agent: true,
            send_session_id: None,
            session_header_name: String::new(),
            duration_secs: 10,
            aspect_ratio: "1:1".into(),
            resolution: "480p".into(),
        });
        assert_eq!(tool.api_root(), "https://api.x.ai/v1");
        let body = tool.request_body("clip", 10, "1:1", "480p");
        assert_eq!(body["model"], "grok-imagine-video-1.5");
        assert_eq!(body["prompt"], "clip");
        assert_eq!(body["duration"], 10);
        assert_eq!(body["aspect_ratio"], "1:1");
        assert_eq!(body["resolution"], "480p");
    }

    #[tokio::test]
    async fn validation_honors_identity_controls_and_custom_session_name() {
        let (listener, api_url, _) = bind().await;
        let captured = serve_script(
            listener,
            vec![(200, json!({"id": "grok-imagine-video"}).to_string())],
        );
        GenerateVideoTool::new(
            api_url,
            "fake-key".into(),
            "grok-imagine-video".into(),
            Some("none".into()),
        )
        .with_options(crate::models::VideoGenerationOptions {
            send_user_agent: false,
            send_session_id: Some(true),
            session_header_name: "x-custom-session".into(),
            ..Default::default()
        })
        .with_session_id("frame-media")
        .validate_model_access()
        .await
        .unwrap();
        let request = captured.await.unwrap().remove(0);
        assert!(!request.to_ascii_lowercase().contains("user-agent:"));
        assert!(request.contains("x-custom-session: frame-media"));
        assert!(!request.contains("x-opencode-session:"));
    }

    #[tokio::test]
    async fn validates_with_the_model_endpoint() {
        let (listener, api_url, _origin) = bind().await;
        let requests = serve_script(
            listener,
            vec![(
                200,
                json!({"id": "grok-imagine-video", "object": "model"}).to_string(),
            )],
        );

        GenerateVideoTool::new(
            api_url,
            "sk-test".into(),
            "grok-imagine-video".into(),
            Some("none".into()),
        )
        .with_options(crate::models::VideoGenerationOptions {
            user_agent: "research-client/1.0".into(),
            ..Default::default()
        })
        .validate_model_access()
        .await
        .unwrap();

        let requests = requests.await.unwrap();
        assert!(requests[0].starts_with("GET /v1/models/grok-imagine-video HTTP/1.1"));
        assert!(requests[0].contains("user-agent: research-client/1.0"));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-test"));
    }

    #[tokio::test]
    async fn validation_falls_back_to_the_model_list() {
        let (listener, api_url, _origin) = bind().await;
        let requests = serve_script(
            listener,
            vec![
                (404, "404 page not found".into()),
                (
                    200,
                    json!({"object": "list", "data": [{"id": "xai/grok-imagine-video-1.5-preview"}]})
                        .to_string(),
                ),
            ],
        );

        GenerateVideoTool::new(
            api_url,
            "sk-test".into(),
            "grok-imagine-video-1.5-preview".into(),
            Some("none".into()),
        )
        .validate_model_access()
        .await
        .unwrap();

        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("GET /v1/models HTTP/1.1"));
    }

    #[tokio::test]
    async fn rejects_unsupported_video_models_before_calling_the_api() {
        let result = GenerateVideoTool::new(
            "http://127.0.0.1:9/v1".into(),
            "sk-test".into(),
            "grok-imagine-video-2.0".into(),
            Some("none".into()),
        )
        .generate("clip", 5, "16:9", "720p")
        .await;
        assert!(result.unwrap_err().contains("grok-imagine-video"));
    }
}
