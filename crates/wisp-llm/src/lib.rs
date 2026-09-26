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
pub mod codex_auth;
pub mod message;
pub mod openai;
pub mod provider;
pub mod responses;
pub mod routed;
pub mod scripted;
mod system_proxy;
pub mod xai_auth;

pub use message::{
    tool_call_pairing, Completion, Content, FunctionCall, ImageUrl, Message, Part, Role, ToolCall,
    ToolSchema, Usage,
};
pub use provider::{
    ambient_proxy_env, annotate_transport_error, build, is_fail_fast_transport,
    is_model_transport_failure, is_retriable, leftover_proxy_note, NullSink, Provider,
    ProviderConfig, ProviderKind, StreamSink,
};
pub use provider::{LlmError, Result};
pub use routed::RoutedProvider;
pub use scripted::{
    ScriptedApiError, ScriptedCompletion, ScriptedProvider, ScriptedProviderSnapshot,
    ScriptedRequest, ScriptedToolCall,
};
pub use system_proxy::ambient_system_proxy;
