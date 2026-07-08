//! Provider trait + streaming sink.

use crate::{Completion, Message, ToolSchema};
use async_trait::async_trait;

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
