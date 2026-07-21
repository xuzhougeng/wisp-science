//! Shared message + tool schema model.
//!
//! Mirrors the OpenAI chat format (role/content/tool_calls/tool_call_id) the
//! upstream agent stores, plus an optional `reasoning` channel for
//! DeepSeek/Qwen/MiniMax-style thinking fields. Providers convert this into
//! their own wire format.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text {
        #[serde(rename = "type")]
        kind: String,
        text: String,
    },
    Image {
        #[serde(rename = "type")]
        kind: String,
        image_url: ImageUrl,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
}

/// Message content: a plain string or a multipart array (text + images).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<Part>),
}

impl Default for Content {
    fn default() -> Self {
        Content::Text(String::new())
    }
}

impl Content {
    pub fn text(s: impl Into<String>) -> Self {
        Content::Text(s.into())
    }
    pub fn as_text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    Part::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.as_text().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    /// Always "function" for OpenAI-style tool calls.
    #[serde(default, rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// Raw JSON string on the wire (OpenAI); parsed into a Value on ingest.
    #[serde(default)]
    pub arguments: String,
}

impl ToolCall {
    pub fn args_value(&self) -> Value {
        if self.function.arguments.trim().is_empty() {
            return serde_json::json!({});
        }
        serde_json::from_str(&self.function.arguments).unwrap_or_else(|_| serde_json::json!({}))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(default)]
    pub content: Content,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// DeepSeek `reasoning_content` / Qwen `reasoning` / MiniMax
    /// `reasoning_details` — normalized to plain text on ingest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Unix seconds; used by the context compactor's age-based rules.
    #[serde(default)]
    pub ts: i64,
    /// Display alias of the model that produced this turn (assistant messages only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: Content::text(text),
            tool_calls: vec![],
            tool_call_id: None,
            tool_name: None,
            reasoning: None,
            ts: 0,
            model_name: None,
        }
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Content::text(text),
            tool_calls: vec![],
            tool_call_id: None,
            tool_name: None,
            reasoning: None,
            ts: now(),
            model_name: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: Content::text(content),
            tool_calls: vec![],
            tool_call_id: None,
            tool_name: None,
            reasoning: None,
            ts: now(),
            model_name: None,
        }
    }
    pub fn tool(
        id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: Content::text(content),
            tool_calls: vec![],
            tool_call_id: Some(id.into()),
            tool_name: Some(name.into()),
            reasoning: None,
            ts: now(),
            model_name: None,
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// JSON-schema description of a tool, in OpenAI `tools` format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSchema {
    pub fn new(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            kind: "function".into(),
            function: ToolFunction {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// Result of a completion, provider-normalized.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    /// "stop" | "tool_calls" | "length" | provider-specific
    pub finish_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Reasoning/thinking tokens, when the provider reports them separately
    /// (OpenAI `*_tokens_details.reasoning_tokens`). 0 = not reported.
    pub reasoning_tokens: u64,
}
