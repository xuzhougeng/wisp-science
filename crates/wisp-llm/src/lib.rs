//! Provider-agnostic LLM client for Wisp.
//!
//! - `OpenAiCompatible` covers OpenAI, DeepSeek, Qwen, MiniMax, Ollava, LM
//!   Studio, and any `/chat/completions` endpoint.
//! - `Anthropic` covers the Messages API (`/v1/messages`).
//!
//! Both implement non-blocking [`Provider::complete`] and SSE
//! [`Provider::stream`]. Reasoning channels (`reasoning_content` /
//! `reasoning` / `reasoning_details`, Anthropic `thinking_delta`) are
//! normalized to a single `reasoning` string.

pub mod anthropic;
pub mod message;
pub mod openai;
pub mod provider;
pub mod responses;
pub mod routed;

pub use message::{
    Completion, Content, FunctionCall, ImageUrl, Message, Part, Role, ToolCall, ToolSchema, Usage,
};
pub use provider::{build, is_retriable, NullSink, Provider, ProviderConfig, ProviderKind, StreamSink};
pub use provider::{LlmError, Result};
pub use routed::RoutedProvider;
