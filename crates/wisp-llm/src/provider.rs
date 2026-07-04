//! Provider trait + streaming sink.

use crate::{Completion, Message, ToolSchema};
use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("http: {0}")]
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
            max_tokens: 4096,
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
            max_tokens: 4096,
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
