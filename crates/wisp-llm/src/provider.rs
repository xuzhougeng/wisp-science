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

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("http: {}", error_chain(.0))]
    Http(#[from] reqwest::Error),
    #[error("decode: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("api: {status} {body}")]
    Api { status: u16, body: String },
    #[error("config: {0}")]
    Config(String),
    #[error("stream ended without completion")]
    Incomplete,
}

pub type Result<T> = std::result::Result<T, LlmError>;

/// True for transient provider failures worth retrying (rate limits, overload, 5xx).
pub fn is_retriable(err: &LlmError) -> bool {
    match err {
        LlmError::Api { status, body } => {
            matches!(*status, 408 | 429 | 500 | 502 | 503 | 529)
                || body.contains("overloaded")
                || body.contains("rate_limit")
                || body.contains("1305")
                || body.contains("too many requests")
                || body.contains("访问量过大")
        }
        LlmError::Http(e) => e.is_timeout() || e.is_connect() || e.is_request(),
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
    /// OpenAI reasoning effort (`reasoning.effort` / `reasoning_effort`). None = provider default.
    pub reasoning_effort: Option<String>,
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
}
