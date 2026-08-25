//! Provider trait + streaming sink.

use crate::{Completion, Message, ToolSchema};
use async_trait::async_trait;

const OPENAI_PYTHON_TOOL_ALIAS: &str = "wisp_python";

/// Codex models reserve `python` for their hosted runtime. Keep Wisp's stable
/// internal tool name, but avoid the collision on OpenAI-compatible wires.
pub(crate) fn openai_wire_tool_name(name: &str) -> &str {
    match name {
        "python" => OPENAI_PYTHON_TOOL_ALIAS,
        _ => name,
    }
}

pub(crate) fn openai_internal_tool_name(name: &str) -> &str {
    match name {
        OPENAI_PYTHON_TOOL_ALIAS => "python",
        _ => name,
    }
}

/// OpenAI-compatible history must carry `tool_calls[].function.arguments` as
/// a *valid* JSON string: strict gateways re-parse it and 400 with Python
/// `json` errors like "Unterminated string" when the value is broken. Ours
/// can be: context compaction cuts oversized arguments mid-string
/// (`wisp-core` `bounded_latest_turn`), and a `finish_reason: "length"` turn
/// can persist a half-written call. Replace anything that doesn't parse with
/// an empty object, matching the Anthropic provider's stance.
pub(crate) fn valid_json_tool_arguments(arguments: &str) -> String {
    let trimmed = arguments.trim();
    if !trimmed.is_empty() && serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return arguments.to_string();
    }
    "{}".to_string()
}

/// reqwest's Display hides the useful part ("connection refused", "proxy
/// unreachable", dns errors) in `source()`; walk the chain so users see it (#77).
fn error_chain(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(cause) = src {
        s.push_str(": ");
        s.push_str(&cause.to_string());
        src = cause.source();
    }
    s
}

#[derive(Debug)]
pub enum LlmError {
    Http(reqwest::Error),
    Decode(serde_json::Error),
    Api {
        status: u16,
        body: String,
    },
    Config(String),
    Incomplete,
    /// A Responses-API turn that ended in a terminal status other than
    /// `completed` (HTTP 200, but `incomplete`/`failed`/`cancelled`). Carries
    /// the wire detail (`incomplete_details.reason` or `error.message`) so
    /// callers can tell an exhausted output budget from a genuine failure.
    NotCompleted {
        status: String,
        reason: String,
    },
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Provider envelopes bury the actionable sentence inside JSON
            // (`{"error":{"message":"..."}}`); show just that sentence on the
            // error card. The raw body stays on the struct untouched —
            // `is_retriable` / `is_context_overflow` pattern-match on it.
            LlmError::Api { status, body } => {
                write!(f, "api: {status} {}", api_error_body_summary(body))
            }
            LlmError::Http(error) => write!(f, "http: {}", error_chain(error)),
            LlmError::Decode(error) => write!(f, "decode: {error}"),
            LlmError::Config(message) => write!(f, "config: {message}"),
            LlmError::Incomplete => write!(f, "stream ended without completion"),
            LlmError::NotCompleted { status, reason } => {
                write!(f, "response ended with status '{status}' ({reason})")
            }
        }
    }
}

impl std::error::Error for LlmError {}

impl From<reqwest::Error> for LlmError {
    fn from(error: reqwest::Error) -> Self {
        LlmError::Http(error)
    }
}

impl From<serde_json::Error> for LlmError {
    fn from(error: serde_json::Error) -> Self {
        LlmError::Decode(error)
    }
}

/// Extract the one actionable sentence from a provider's JSON error envelope
/// (`error.message`, or a top-level `message`). Unparseable bodies pass
/// through unchanged.
fn api_error_body_summary(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return body.to_string();
    };
    value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("message").and_then(|v| v.as_str()))
        .map(str::to_string)
        .unwrap_or_else(|| body.to_string())
}

pub type Result<T> = std::result::Result<T, LlmError>;

impl LlmError {
    /// The model stopped because it exhausted the output token budget
    /// (`max_output_tokens`), so a retry with a larger budget can succeed.
    pub fn output_limit_hit(&self) -> bool {
        matches!(self, LlmError::NotCompleted { reason, .. } if reason == "max_output_tokens")
    }
    /// Provider rejected the request because the assembled prompt exceeds the
    /// model context window.
    pub fn is_context_overflow(&self) -> bool {
        match self {
            LlmError::Api { status, body } if matches!(*status, 400 | 413) => {
                let lower = body.to_ascii_lowercase();
                lower.contains("context length")
                    || lower.contains("maximum context")
                    || lower.contains("too many tokens")
                    || lower.contains("context window")
                    || lower.contains("prompt is too long")
                    || lower.contains("token limit")
                    || lower.contains("context_length_exceeded")
                    || lower.contains("max context")
                    || lower.contains("entity too large")
                    || lower.contains("request entity too large")
            }
            _ => false,
        }
    }
}

const PROXY_ENV_KEYS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Process environment proxy vars reqwest follows when Settings proxy is empty.
pub fn ambient_proxy_env() -> Vec<(String, String)> {
    PROXY_ENV_KEYS
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| ((*key).to_string(), value))
        })
        .collect()
}

/// How the current proxy setting / leftover env should be named in an error.
pub fn leftover_proxy_note(configured: Option<&str>, env: &[(String, String)]) -> Option<String> {
    match configured.map(str::trim) {
        Some("none") => None,
        Some(url) if !url.is_empty() => Some(format!("via Model API proxy {url}")),
        _ => env
            .iter()
            .find(|(_, value)| !value.trim().is_empty())
            .map(|(key, value)| format!("via leftover {key}={value}")),
    }
}

/// Connect failures (and leftover-proxy refusals) that look like `http: ...`.
pub fn is_model_transport_failure(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("http: ")
        && (m.contains("connect")
            || m.contains("timed out")
            || m.contains("deadline has elapsed")
            || m.contains("dns")
            || m.contains("refused")
            || m.contains("proxy")
            || m.contains("socks")
            || m.contains("tunnel")
            || m.contains("unreachable"))
}

/// Dead local proxies and connection-refused should fail the turn immediately.
/// Retrying them is how a leftover `HTTPS_PROXY` becomes minutes of silence.
pub fn is_fail_fast_transport(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("connection refused")
        || m.contains("actively refused")
        || m.contains("os error 10061")
        || m.contains("os error 111")
        || (m.contains("proxy")
            && (m.contains("refused") || m.contains("tunnel") || m.contains("unreachable")))
        || m.contains("via leftover")
}

/// Append the active / leftover proxy so the UI can point at Settings.
pub fn annotate_transport_error(
    message: &str,
    configured: Option<&str>,
    env: &[(String, String)],
) -> String {
    if !is_model_transport_failure(message) {
        return message.to_string();
    }
    let Some(note) = leftover_proxy_note(configured, env) else {
        return message.to_string();
    };
    if message.contains(&note) {
        return message.to_string();
    }
    format!("{message} ({note})")
}

/// True for transient provider failures worth retrying (rate limits, overload, 5xx).
pub fn is_retriable(err: &LlmError) -> bool {
    match err {
        LlmError::Api { status, body } => {
            let lower = body.to_ascii_lowercase();
            matches!(*status, 408 | 429)
                || (500..=599).contains(status)
                || lower.contains("overloaded")
                || lower.contains("rate_limit")
                || lower.contains("1305")
                || lower.contains("too many requests")
                || lower.contains("访问量过大")
        }
        LlmError::Http(e) => {
            // Connect failures include leftover local proxies. On Windows a
            // closed Clash/V2Ray port is often `deadline has elapsed`, not
            // ECONNREFUSED — retrying that looks like "sent, no reply".
            if e.is_connect() || is_fail_fast_transport(&error_chain(e)) {
                false
            } else {
                e.is_timeout() || e.is_request()
            }
        }
        _ => false,
    }
}

/// A healthy SSE stream always delivers a terminal marker before closing: a
/// `finish_reason`/`stop_reason` chunk, or at least OpenAI's `[DONE]` line. A
/// stream that closes with neither — and was not cancelled by the user — was
/// cut mid-response (network drop, proxy kill, per-key concurrency limit), so
/// the partial text must not be mistaken for a finished answer (#437).
pub fn stream_was_cut(saw_terminal: bool, cancelled: bool) -> bool {
    !saw_terminal && !cancelled
}

/// Which provider family to build.
#[derive(Debug, Clone)]
pub enum ProviderKind {
    /// OpenAI / DeepSeek / Qwen / MiniMax / local Ollama / LM Studio — any
    /// `/chat/completions` endpoint.
    OpenAiCompatible,
    /// OpenAI's first-party `/v1/responses` endpoint.
    OpenAiResponses,
    /// Anthropic Messages API (`/v1/messages`).
    Anthropic,
}

/// Provider configuration. `base_url` is the API root (no `/chat/completions`).
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Anthropic-only; ignored for OpenAI-compatible.
    pub anthropic_version: String,
    /// Cap on output tokens per turn.
    pub max_tokens: u64,
    /// Reasoning effort: `reasoning.effort` / `reasoning_effort` for OpenAI,
    /// `output_config.effort` for Anthropic. None = provider default.
    pub reasoning_effort: Option<String>,
    /// HTTP proxy override. `None`/empty = follow system/env proxy settings;
    /// `"none"` = force a direct connection; otherwise a proxy URL
    /// (`http://`, `https://`, `socks5://`).
    pub proxy: Option<String>,
}

/// Shared reqwest client for all providers, honoring `cfg.proxy`.
/// Process-wide connection pool for the no-proxy case. Review / follow-up /
/// memory side calls used to build a fresh `reqwest::Client` per call — every
/// one of them paid a new TLS handshake (~hundreds of ms) before this cache.
/// Proxy configs stay per-client (they vary by profile and are rare).
static SHARED_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

pub(crate) fn http_client(cfg: &ProviderConfig) -> reqwest::Client {
    let proxy = cfg.proxy.as_deref().map(str::trim);
    if matches!(proxy, None | Some("")) {
        return SHARED_CLIENT.get_or_init(|| build_http_client(cfg)).clone();
    }
    build_http_client(cfg)
}

fn build_http_client(cfg: &ProviderConfig) -> reqwest::Client {
    let mut b = reqwest::Client::builder()
        .user_agent("wisp-science")
        // A total request timeout also caps a healthy, actively streaming SSE
        // response. Long agent turns therefore used to die at five minutes
        // even while bytes were still arriving. Bound connection setup and
        // each individual read instead: active streams may run as long as
        // needed, while a silent/broken socket still becomes a retriable
        // reqwest timeout.
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(300));
    match cfg.proxy.as_deref().map(str::trim) {
        None | Some("") => {}
        Some("none") => b = b.no_proxy(),
        // Invalid URLs are rejected at settings save; if one sneaks in, fall
        // back to ambient proxy behavior instead of panicking mid-turn.
        Some(url) => {
            if let Ok(p) = reqwest::Proxy::all(url) {
                b = b.proxy(p);
            }
        }
    }
    b.build().expect("reqwest client")
}

impl ProviderConfig {
    pub fn openai(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            kind: ProviderKind::OpenAiCompatible,
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            anthropic_version: "2023-06-01".into(),
            max_tokens: 8192,
            reasoning_effort: None,
            proxy: None,
        }
    }
    pub fn openai_responses(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            kind: ProviderKind::OpenAiResponses,
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            anthropic_version: "2023-06-01".into(),
            max_tokens: 8192,
            reasoning_effort: None,
            proxy: None,
        }
    }
    pub fn anthropic(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            kind: ProviderKind::Anthropic,
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            anthropic_version: "2023-06-01".into(),
            max_tokens: 8192,
            reasoning_effort: None,
            proxy: None,
        }
    }
}

/// Callbacks the agent loop receives while a streamed completion is in flight.
pub trait StreamSink: Send {
    fn on_text(&mut self, delta: &str);
    fn on_reasoning(&mut self, delta: &str);
    /// A tool call accumulated so far (index, name, arguments-so-far). Called
    /// as argument fragments arrive so the UI can render an in-progress call.
    fn on_tool_call(&mut self, index: usize, name: &str, arguments_so_far: &str);
    fn on_usage(&mut self, usage: crate::Usage);
    /// Whether the user requested cancellation. Streaming loops poll this each
    /// chunk so a Stop interrupts token generation mid-stream, not only between
    /// whole model turns. Default `false` for sinks that don't support cancel.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// A no-op sink for callers that only want the final `Completion`.
pub struct NullSink;
impl StreamSink for NullSink {
    fn on_text(&mut self, _: &str) {}
    fn on_reasoning(&mut self, _: &str) {}
    fn on_tool_call(&mut self, _: usize, _: &str, _: &str) {}
    fn on_usage(&mut self, _: crate::Usage) {}
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn model(&self) -> &str;
    /// Non-streaming completion.
    async fn complete(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<Completion>;
    /// Streaming completion; deltas go to `sink`, the assembled result is returned.
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> Result<Completion>;
}

/// Construct the concrete provider for a config.
pub fn build(cfg: ProviderConfig) -> Box<dyn Provider> {
    match cfg.kind {
        ProviderKind::OpenAiCompatible => Box::new(crate::openai::OpenAiProvider::new(cfg)),
        ProviderKind::OpenAiResponses => {
            Box::new(crate::responses::OpenAiResponsesProvider::new(cfg))
        }
        ProviderKind::Anthropic => Box::new(crate::anthropic::AnthropicProvider::new(cfg)),
    }
}

/// Incremental UTF-8 decoder for a chunked byte stream.
///
/// Network/TLS framing splits multi-byte characters across chunks (pervasive
/// with CJK text). Decoding each chunk in isolation with
/// `from_utf8(&bytes).unwrap_or("")` drops the *entire* chunk whenever it ends
/// (or begins) mid-character, silently gutting streamed content — the cause of
/// truncated/garbled writes. This holds back the incomplete trailing bytes and
/// emits them once the rest of the character arrives.
#[derive(Default)]
pub struct Utf8Stream {
    tail: Vec<u8>,
}

impl Utf8Stream {
    /// Feed one chunk; return the text that is now complete. Any incomplete
    /// trailing multi-byte sequence is retained until the next `push`.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.tail.extend_from_slice(bytes);
        match std::str::from_utf8(&self.tail) {
            Ok(s) => {
                let out = s.to_string();
                self.tail.clear();
                out
            }
            Err(e) => {
                let valid = e.valid_up_to();
                // `valid_up_to()` bytes are guaranteed valid UTF-8.
                let out = String::from_utf8_lossy(&self.tail[..valid]).into_owned();
                match e.error_len() {
                    // A genuinely invalid sequence (not a boundary split): drop
                    // it so a malformed stream can never stall the buffer.
                    Some(bad) => self.tail.drain(..valid + bad),
                    // Incomplete trailing char: keep it for the next chunk.
                    None => self.tail.drain(..valid),
                };
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_stream_reassembles_char_split_across_chunks() {
        // "支气管" streamed with the byte boundaries falling *inside* each
        // 3-byte character — the exact case that the old per-chunk decode drops.
        let full = "支气管扩张 review body\n\n";
        let bytes = full.as_bytes();
        let mut s = Utf8Stream::default();
        let mut out = String::new();
        // 2-byte chunks guarantee splits mid-character for 3-byte CJK codepoints.
        for chunk in bytes.chunks(2) {
            out.push_str(&s.push(chunk));
        }
        assert_eq!(out, full, "content lost across chunk boundaries");
        assert!(s.tail.is_empty(), "no bytes left dangling at stream end");
    }

    #[test]
    fn utf8_stream_matches_whole_input_for_ascii() {
        let mut s = Utf8Stream::default();
        assert_eq!(s.push(b"data: {\"x\":1}\n\n"), "data: {\"x\":1}\n\n");
    }

    // Proxy setting variants must all yield a usable client — including an
    // invalid URL (falls back to ambient proxy) — never a panic mid-turn.
    #[test]
    fn http_client_accepts_all_proxy_variants() {
        for proxy in [
            None,
            Some(""),
            Some("none"),
            Some("http://127.0.0.1:7890"),
            Some("socks5://127.0.0.1:1080"),
            Some("::not a url::"),
        ] {
            let mut cfg = ProviderConfig::openai("https://api.example.com", "k", "m");
            cfg.proxy = proxy.map(Into::into);
            let _ = http_client(&cfg);
        }
    }

    // #437: a stream that closes without a terminal marker is a cut, EXCEPT
    // when the user hit Stop — that must keep returning the partial (#58).
    #[test]
    fn stream_cut_detection_spares_user_cancel() {
        assert!(stream_was_cut(false, false), "silent EOF is a cut");
        assert!(
            !stream_was_cut(true, false),
            "finish_reason/[DONE] is a clean end"
        );
        assert!(!stream_was_cut(false, true), "user Stop is not a cut");
        assert!(!stream_was_cut(true, true));
    }

    #[test]
    fn context_overflow_is_detected_from_provider_errors() {
        assert!(LlmError::Api {
            status: 400,
            body: "maximum context length exceeded".into()
        }
        .is_context_overflow());
        assert!(LlmError::Api {
            status: 413,
            body: "Request Entity Too Large".into()
        }
        .is_context_overflow());
        assert!(!LlmError::Api {
            status: 429,
            body: "rate_limit".into()
        }
        .is_context_overflow());
    }

    #[test]
    fn api_display_surfaces_the_envelope_message() {
        let error = LlmError::Api {
            status: 400,
            body: r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages: roles must alternate between \"user\" and \"assistant\""}}"#.into(),
        };
        assert_eq!(
            error.to_string(),
            "api: 400 messages: roles must alternate between \"user\" and \"assistant\""
        );
        // The raw body stays on the struct for pattern-based classifiers.
        let LlmError::Api { body, .. } = &error else {
            panic!()
        };
        assert!(body.contains("invalid_request_error"));
    }

    #[test]
    fn api_display_passes_through_non_json_bodies() {
        let error = LlmError::Api {
            status: 500,
            body: "upstream connection reset".into(),
        };
        assert_eq!(error.to_string(), "api: 500 upstream connection reset");
    }

    #[test]
    fn transient_gateway_statuses_are_retried() {
        for status in [500, 503, 504, 520, 522, 524, 529, 599] {
            assert!(
                is_retriable(&LlmError::Api {
                    status,
                    body: "gateway failure".into(),
                }),
                "status {status} should be retriable"
            );
        }
        assert!(is_retriable(&LlmError::Api {
            status: 400,
            body: "OVERLOADED upstream".into(),
        }));
        assert!(!is_retriable(&LlmError::Api {
            status: 400,
            body: "invalid request".into(),
        }));
    }

    #[test]
    fn leftover_proxy_note_names_env_or_setting() {
        assert_eq!(leftover_proxy_note(Some("none"), &[]), None);
        assert_eq!(
            leftover_proxy_note(Some("http://127.0.0.1:7890"), &[]).as_deref(),
            Some("via Model API proxy http://127.0.0.1:7890")
        );
        let env = vec![("HTTPS_PROXY".into(), "http://127.0.0.1:7890".into())];
        assert_eq!(
            leftover_proxy_note(None, &env).as_deref(),
            Some("via leftover HTTPS_PROXY=http://127.0.0.1:7890")
        );
        assert_eq!(leftover_proxy_note(Some("none"), &env), None);
    }

    #[test]
    fn connection_refused_fails_fast_and_is_annotated() {
        let raw =
            "http: error sending request: tcp connect error: Connection refused (os error 111)";
        assert!(is_model_transport_failure(raw));
        assert!(is_fail_fast_transport(raw));
        let env = vec![("HTTPS_PROXY".into(), "http://127.0.0.1:7890".into())];
        let annotated = annotate_transport_error(raw, None, &env);
        assert!(annotated.contains("via leftover HTTPS_PROXY=http://127.0.0.1:7890"));
        assert_eq!(
            annotate_transport_error("api: 401 invalid key", None, &env),
            "api: 401 invalid key"
        );
    }

    #[test]
    fn windows_proxy_refusal_fails_fast() {
        assert!(is_fail_fast_transport(
            "http: error sending request: No connection could be made because the target machine actively refused it. (os error 10061)"
        ));
        assert!(is_model_transport_failure(
            "http: error sending request for url (https://api.example.com/v1/chat/completions): client error (Connect): tcp connect error: deadline has elapsed"
        ));
    }

    #[tokio::test]
    async fn live_connection_refused_is_not_retriable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);
        let err = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("client")
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect_err("closed port should refuse");
        let wrapped = LlmError::Http(err);
        assert!(
            !is_retriable(&wrapped),
            "a dead listener must not enter the multi-minute retry window: {wrapped}"
        );
    }
}
