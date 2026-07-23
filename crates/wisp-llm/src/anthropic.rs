//! Anthropic Messages API provider (`/v1/messages`).
//!
//! Converts the shared Message model to/from Anthropic's content-block format:
//! - system messages collapse into the top-level `system` field
//! - tool results (our `Role::Tool`) become `user` messages with
//!   `tool_result` content blocks
//! - assistant tool calls become `tool_use` content blocks

use crate::message::{Content, Message, Role, ToolCall, ToolSchema};
use crate::provider::{LlmError, Provider, Result, StreamSink, Utf8Stream};
use crate::{Completion, FunctionCall, Usage};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

pub struct AnthropicProvider {
    cfg: crate::provider::ProviderConfig,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(cfg: crate::provider::ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("wisp-science")
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client");
        Self { cfg, client }
    }

    fn endpoint(&self) -> String {
        let base = self.cfg.base_url.trim_end_matches('/');
        if base.ends_with("/v1/messages") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{base}/messages")
        } else {
            format!("{base}/v1/messages")
        }
    }

    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&self.cfg.api_key) {
            h.insert("x-api-key", v);
        }
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&self.cfg.anthropic_version) {
            h.insert("anthropic-version", v);
        }
        h
    }

    fn build_body(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        stream: bool,
    ) -> (String, Vec<Value>, Value) {
        // system: concatenate all system messages.
        let system: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_text())
            .collect::<Vec<_>>()
            .join("\n\n");

        let mut out: Vec<Value> = vec![];
        let mut pending_tool_results: Vec<Value> = vec![];

        let flush_tool_results = |pending: &mut Vec<Value>, out: &mut Vec<Value>| {
            if !pending.is_empty() {
                out.push(json!({ "role": "user", "content": std::mem::take(pending) }));
            }
        };

        for m in messages {
            match m.role {
                Role::System => {}
                Role::Tool => {
                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                        "content": m.content.as_text(),
                    }));
                }
                Role::User => {
                    flush_tool_results(&mut pending_tool_results, &mut out);
                    out.push(json!({ "role": "user", "content": user_content(&m.content) }));
                }
                Role::Assistant => {
                    flush_tool_results(&mut pending_tool_results, &mut out);
                    let mut blocks: Vec<Value> = vec![];
                    let text = m.content.as_text();
                    if !text.is_empty() {
                        blocks.push(json!({ "type": "text", "text": text }));
                    }
                    for tc in &m.tool_calls {
                        let input: Value = if tc.function.arguments.trim().is_empty() {
                            json!({})
                        } else {
                            serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}))
                        };
                        blocks.push(json!({ "type": "tool_use", "id": tc.id, "name": tc.function.name, "input": input }));
                    }
                    if blocks.is_empty() {
                        blocks.push(json!({ "type": "text", "text": " " }));
                    }
                    out.push(json!({ "role": "assistant", "content": blocks }));
                }
            }
        }
        flush_tool_results(&mut pending_tool_results, &mut out);

        let mut body = json!({
            "model": self.cfg.model,
            "max_tokens": self.cfg.max_tokens,
            "messages": out,
            "stream": stream,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| json!({ "name": t.function.name, "description": t.function.description, "input_schema": t.function.parameters }))
            .collect();
        if !tools_json.is_empty() {
            body["tools"] = json!(tools_json);
        }
        (system, out, body)
    }

    async fn request(&self, body: Value) -> Result<Value> {
        let resp = self
            .client
            .post(self.endpoint())
            .headers(self.headers())
            .json(&body)
            .send()
            .await?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        if status >= 400 {
            return Err(LlmError::Api { status, body: text });
        }
        let val: Value = serde_json::from_str(&text)?;
        Ok(val)
    }
}

fn user_content(c: &Content) -> Value {
    match c {
        Content::Text(s) => json!(s),
        Content::Parts(parts) => {
            let arr: Vec<Value> = parts
                .iter()
                .map(|p| match p {
                    crate::message::Part::Text { text, .. } => json!({ "type": "text", "text": text }),
                    crate::message::Part::Image { image_url, .. } => {
                        // data: URI -> {type:image, source:{type:base64, media_type, data}}
                        if let Some((media, data)) = image_url.url.strip_prefix("data:").and_then(|s| s.split_once(",")) {
                            let media = media.split(";").next().unwrap_or("image/png");
                            json!({ "type": "image", "source": { "type": "base64", "media_type": media, "data": data } })
                        } else {
                            json!({ "type": "text", "text": image_url.url })
                        }
                    }
                })
                .collect();
            json!(arr)
        }
    }
}

fn parse_completion(val: &Value) -> Completion {
    let mut content = String::new();
    let mut tool_calls = vec![];
    if let Some(blocks) = val.get("content").and_then(|v| v.as_array()) {
        for b in blocks {
            match b.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                        content.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let id = b
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = b
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input = b.get("input").cloned().unwrap_or(json!({}));
                    tool_calls.push(ToolCall {
                        id,
                        kind: "function".into(),
                        function: FunctionCall {
                            name,
                            arguments: input.to_string(),
                        },
                    });
                }
                _ => {}
            }
        }
    }
    let finish_reason = val
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(|r| match r {
            "tool_use" => "tool_calls".to_string(),
            "end_turn" | "stop_sequence" => "stop".to_string(),
            other => other.to_string(),
        });
    let usage = parse_usage(val.get("usage"));
    Completion {
        content,
        reasoning: None,
        tool_calls,
        finish_reason,
        usage,
    }
}

fn parse_usage(u: Option<&Value>) -> Usage {
    let field = |k: &str| {
        u.and_then(|u| u.get(k))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    };
    // Anthropic's `input_tokens` excludes cache read/creation; add them so the
    // figure means the same cache-inclusive total as the OpenAI providers.
    let cache_read = field("cache_read_input_tokens");
    Usage {
        input_tokens: field("input_tokens")
            .saturating_add(cache_read)
            .saturating_add(field("cache_creation_input_tokens")),
        output_tokens: field("output_tokens"),
        // Anthropic counts thinking inside output_tokens; no separate figure.
        reasoning_tokens: 0,
        cached_input_tokens: cache_read,
    }
}

fn merge_usage(current: &mut Usage, update: Usage) {
    // Streaming-compatible providers do not agree on which event carries the
    // final counters. Keep the greatest cumulative value seen for each field.
    current.input_tokens = current.input_tokens.max(update.input_tokens);
    current.output_tokens = current.output_tokens.max(update.output_tokens);
    current.reasoning_tokens = current.reasoning_tokens.max(update.reasoning_tokens);
    current.cached_input_tokens = current.cached_input_tokens.max(update.cached_input_tokens);
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }

    async fn complete(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<Completion> {
        let (_, _, body) = self.build_body(messages, tools, false);
        let val = self.request(body).await?;
        Ok(parse_completion(&val))
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> Result<Completion> {
        let (_, _, body) = self.build_body(messages, tools, true);
        let resp = self
            .client
            .post(self.endpoint())
            .headers(self.headers())
            .json(&body)
            .send()
            .await?;
        let status = resp.status().as_u16();
        if status >= 400 {
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, body: text });
        }
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut utf8 = Utf8Stream::default();
        // index -> (type, id, name, input_json_accumulator, text_accumulator)
        let mut blocks: std::collections::BTreeMap<usize, BlockAcc> =
            std::collections::BTreeMap::new();
        let mut content = String::new();
        let mut finish_reason: Option<String> = None;
        let mut usage = Usage::default();
        let mut saw_stop = false;

        while let Some(chunk) = stream.next().await {
            // Stop mid-generation: drop the stream and return the partial result
            // so the agent loop can bail (#58 — Stop was dead during streaming).
            if sink.is_cancelled() {
                break;
            }
            let bytes = chunk?;
            buf.push_str(&utf8.push(&bytes));
            while let Some(idx) = buf.find("\n\n") {
                let event = buf[..idx].to_string();
                buf.drain(..idx + 2);
                let (etype, data) = parse_sse_event(&event);
                if data.is_empty() {
                    continue;
                }
                let Ok(val) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                match etype.as_str() {
                    "message_start" => {
                        if let Some(u) = val.pointer("/message/usage").or_else(|| val.get("usage"))
                        {
                            merge_usage(&mut usage, parse_usage(Some(u)));
                        }
                    }
                    "content_block_start" => {
                        let i = val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let blk = val.get("content_block").cloned().unwrap_or(Value::Null);
                        let kind = blk
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("text")
                            .to_string();
                        let id = blk
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = blk
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        blocks.insert(
                            i,
                            BlockAcc {
                                kind,
                                id,
                                name,
                                input: String::new(),
                                text: String::new(),
                            },
                        );
                    }
                    "content_block_delta" => {
                        let i = val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let Some(delta) = val.get("delta") else {
                            continue;
                        };
                        let Some(b) = blocks.get_mut(&i) else {
                            continue;
                        };
                        match delta.get("type").and_then(|v| v.as_str()) {
                            Some("text_delta") => {
                                if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                                    b.text.push_str(t);
                                    content.push_str(t);
                                    sink.on_text(t);
                                }
                            }
                            Some("input_json_delta") => {
                                if let Some(p) = delta.get("partial_json").and_then(|v| v.as_str())
                                {
                                    b.input.push_str(p);
                                    sink.on_tool_call(i, &b.name, &b.input);
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(t) = delta.get("thinking").and_then(|v| v.as_str()) {
                                    sink.on_reasoning(t);
                                }
                            }
                            _ => {}
                        }
                    }
                    "message_delta" => {
                        if let Some(fr) = val.pointer("/delta/stop_reason").and_then(|v| v.as_str())
                        {
                            finish_reason = Some(match fr {
                                "tool_use" => "tool_calls".to_string(),
                                "end_turn" | "stop_sequence" => "stop".to_string(),
                                o => o.to_string(),
                            });
                        }
                        if let Some(u) = val.get("usage") {
                            merge_usage(&mut usage, parse_usage(Some(u)));
                        }
                    }
                    "message_stop" => {
                        saw_stop = true;
                    }
                    _ => {}
                }
            }
        }
        sink.on_usage(usage.clone());

        let tool_calls: Vec<ToolCall> = blocks
            .into_iter()
            .filter(|(_, b)| b.kind == "tool_use")
            .map(|(_, b)| ToolCall {
                id: b.id,
                kind: "function".into(),
                function: FunctionCall {
                    name: b.name,
                    arguments: b.input,
                },
            })
            .collect();

        if content.is_empty() && tool_calls.is_empty() && finish_reason.is_none() {
            return Err(LlmError::Incomplete);
        }
        if crate::provider::stream_was_cut(finish_reason.is_some() || saw_stop, sink.is_cancelled())
        {
            return Err(LlmError::Incomplete);
        }
        Ok(Completion {
            content,
            reasoning: None,
            tool_calls,
            finish_reason,
            usage,
        })
    }
}

struct BlockAcc {
    kind: String,
    id: String,
    name: String,
    input: String,
    text: String,
}

fn parse_sse_event(event: &str) -> (String, String) {
    let mut etype = String::new();
    let mut data = String::new();
    for line in event.lines() {
        if let Some(t) = line.strip_prefix("event:") {
            etype = t.trim().to_string();
        } else if let Some(d) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(d.trim());
        }
    }
    (etype, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_tokens_are_cache_inclusive() {
        // Anthropic reports fresh input, cache read, and cache creation as three
        // separate buckets; the normalized `input_tokens` is their sum, and the
        // cache-hit portion is surfaced on `cached_input_tokens`.
        let resp = json!({
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 200,
                "cache_read_input_tokens": 5000,
                "cache_creation_input_tokens": 300,
                "output_tokens": 42
            }
        });
        let comp = parse_completion(&resp);
        assert_eq!(comp.usage.input_tokens, 5500);
        assert_eq!(comp.usage.cached_input_tokens, 5000);
        assert_eq!(comp.usage.output_tokens, 42);
    }

    #[test]
    fn stream_usage_accepts_input_tokens_from_final_delta() {
        let mut usage = Usage::default();
        merge_usage(
            &mut usage,
            parse_usage(Some(&json!({"input_tokens": 0, "output_tokens": 0}))),
        );
        merge_usage(
            &mut usage,
            parse_usage(Some(&json!({"input_tokens": 136_286, "output_tokens": 81}))),
        );

        assert_eq!(usage.input_tokens, 136_286);
        assert_eq!(usage.output_tokens, 81);
    }

    #[test]
    fn sparse_final_delta_keeps_start_usage() {
        let mut usage = parse_usage(Some(&json!({
            "input_tokens": 200,
            "cache_read_input_tokens": 5000,
            "cache_creation_input_tokens": 300,
            "output_tokens": 1
        })));
        merge_usage(&mut usage, parse_usage(Some(&json!({"output_tokens": 42}))));

        assert_eq!(usage.input_tokens, 5500);
        assert_eq!(usage.cached_input_tokens, 5000);
        assert_eq!(usage.output_tokens, 42);
    }
}
