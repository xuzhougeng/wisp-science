//! OpenAI-compatible `/chat/completions` provider (OpenAI, DeepSeek, Qwen,
//! MiniMax, Ollama, LM Studio, any OpenAI-compatible endpoint).
//!
//! Reasoning fields are normalized across vendors:
//! - DeepSeek: `reasoning_content` (string)
//! - Qwen / some OpenAI-compat: `reasoning` (string)
//! - MiniMax: `reasoning_details` (array of `{text}`)

use crate::message::{Content, Message, Part, Role, ToolCall, ToolSchema};
use crate::provider::{LlmError, Provider, Result, StreamSink, Utf8Stream};
use crate::{Completion, FunctionCall, Usage};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

pub struct OpenAiProvider {
    cfg: crate::provider::ProviderConfig,
    client: reqwest::Client,
}

impl OpenAiProvider {
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
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{base}/chat/completions")
        }
    }

    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        if !self.cfg.api_key.is_empty() {
            if let Ok(v) =
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.cfg.api_key))
            {
                h.insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        h
    }

    /// Convert our Message model into the OpenAI wire format, dropping fields
    /// the endpoint won't accept (`ts`, `tool_name`, image parts collapse to
    /// text for non-vision calls but are preserved as multipart when present).
    ///
    /// Also repairs orphaned tool-call pairings so strict endpoints (DeepSeek,
    /// OpenAI) don't 400 (#74): a turn interrupted after an assistant emitted
    /// `tool_calls` but before its `tool` results were persisted leaves a
    /// dangling `tool_calls` (or, symmetrically, an orphan `tool` message).
    /// GLM tolerates it; DeepSeek rejects it. We keep only `tool_calls` that
    /// have a matching `tool` reply, and drop `tool` messages with no matching
    /// call.
    fn sanitize(messages: &[Message]) -> Vec<Value> {
        // ids answered by a `tool` message, and ids requested by an assistant.
        let mut answered = std::collections::HashSet::new();
        let mut requested = std::collections::HashSet::new();
        for m in messages {
            match m.role {
                Role::Tool => {
                    if let Some(id) = &m.tool_call_id {
                        answered.insert(id.clone());
                    }
                }
                Role::Assistant => {
                    for tc in &m.tool_calls {
                        requested.insert(tc.id.clone());
                    }
                }
                _ => {}
            }
        }
        messages
            .iter()
            .filter_map(|m| match m.role {
                Role::System => Some(json!({ "role": "system", "content": m.content.as_text() })),
                Role::User => {
                    Some(json!({ "role": "user", "content": sanitize_user_content(&m.content) }))
                }
                Role::Assistant => {
                    let kept: Vec<&ToolCall> = m
                        .tool_calls
                        .iter()
                        .filter(|tc| answered.contains(&tc.id))
                        .collect();
                    let mut o = json!({ "role": "assistant", "content": m.content.as_text() });
                    if !kept.is_empty() {
                        o["tool_calls"] = serde_json::to_value(&kept).unwrap_or(Value::Null);
                    }
                    if let Some(r) = &m.reasoning {
                        o["reasoning_content"] = json!(r);
                    }
                    Some(o)
                }
                Role::Tool => {
                    let id = m.tool_call_id.clone().unwrap_or_default();
                    if !requested.contains(&id) {
                        return None;
                    }
                    Some(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": m.content.as_text(),
                    }))
                }
            })
            .collect()
    }

    fn build_body(&self, messages: &[Message], tools: &[ToolSchema], stream: bool) -> Value {
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
            .collect();
        let mut body = json!({
            "model": self.cfg.model,
            "messages": Self::sanitize(messages),
            "stream": stream,
            "max_tokens": self.cfg.max_tokens,
        });
        if stream {
            // Without this, OpenAI-compatible APIs (OpenAI/GLM/DeepSeek/Moonshot)
            // omit the token counts from the stream, leaving usage at 0.
            body["stream_options"] = json!({ "include_usage": true });
        }
        if !tools_json.is_empty() {
            body["tools"] = json!(tools_json);
        }
        if let Some(effort) = &self.cfg.reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }
        body
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

fn sanitize_user_content(c: &Content) -> Value {
    match c {
        Content::Text(s) => json!(s),
        Content::Parts(parts) => {
            let arr: Vec<Value> = parts
                .iter()
                .map(|p| match p {
                    Part::Text { text, .. } => json!({ "type": "text", "text": text }),
                    Part::Image { image_url, .. } => {
                        json!({ "type": "image_url", "image_url": { "url": image_url.url } })
                    }
                })
                .collect();
            json!(arr)
        }
    }
}

fn extract_reasoning(msg: &Value) -> Option<String> {
    if let Some(s) = msg.get("reasoning_content").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    if let Some(s) = msg.get("reasoning").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    if let Some(arr) = msg.get("reasoning_details").and_then(|v| v.as_array()) {
        let joined = arr
            .iter()
            .filter_map(|d| d.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

fn normalize_tool_calls(msg: &Value) -> Vec<ToolCall> {
    let mut out = vec![];
    let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) else {
        return out;
    };
    for tc in tcs {
        let id = tc
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let func = tc.get("function").cloned().unwrap_or(Value::Null);
        let name = func
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let args = func
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("{}")
            .to_string();
        out.push(ToolCall {
            id,
            kind: "function".into(),
            function: FunctionCall {
                name,
                arguments: args,
            },
        });
    }
    out
}

fn merge_stream_tool_call_delta(entry: &mut (String, String, String), tc: &Value) {
    if let Some(id) = tc
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.0 = id.to_string();
    }
    if let Some(f) = tc.get("function").and_then(|v| v.as_object()) {
        if let Some(n) = f
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            entry.1 = n.to_string();
        }
        if let Some(a) = f.get("arguments").and_then(|v| v.as_str()) {
            entry.2.push_str(a);
        }
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai-compatible"
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }

    async fn complete(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<Completion> {
        let body = self.build_body(messages, tools, false);
        let val = self.request(body).await?;
        let choice = val
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(Value::Null);
        let msg = choice.get("message").cloned().unwrap_or(Value::Null);
        let content = msg
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let reasoning = extract_reasoning(&msg);
        let tool_calls = normalize_tool_calls(&msg);
        let finish_reason = choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .map(String::from);
        let usage = parse_usage(&val);
        Ok(Completion {
            content,
            reasoning,
            tool_calls,
            finish_reason,
            usage,
        })
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> Result<Completion> {
        let body = self.build_body(messages, tools, true);
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
        let mut content = String::new();
        let mut reasoning = String::new();
        // index -> (id, name, arguments)
        let mut tool_calls: std::collections::BTreeMap<usize, (String, String, String)> =
            std::collections::BTreeMap::new();
        let mut finish_reason: Option<String> = None;
        let mut usage = Usage::default();

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
                for line in event.lines() {
                    let line = line.strip_prefix("data:").unwrap_or(line).trim();
                    if line.is_empty() || line == "[DONE]" {
                        continue;
                    }
                    let Ok(val) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    // The final usage chunk carries an empty `choices` array, so
                    // parse usage before the choice guard would `continue` past it.
                    // Non-null so the per-chunk `"usage": null` fields don't wipe it.
                    if let Some(u) = val.get("usage").filter(|u| !u.is_null()) {
                        if let Some(p) = parse_usage_obj(u) {
                            usage = p.clone();
                            sink.on_usage(p);
                        }
                    }
                    let Some(choice) = val
                        .get("choices")
                        .and_then(|c| c.as_array())
                        .and_then(|a| a.first())
                    else {
                        continue;
                    };
                    let delta = choice.get("delta").cloned().unwrap_or(Value::Null);
                    if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
                        content.push_str(t);
                        sink.on_text(t);
                    }
                    if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                        reasoning.push_str(r);
                        sink.on_reasoning(r);
                    }
                    if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            let i = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            let entry = tool_calls
                                .entry(i)
                                .or_insert_with(|| (String::new(), String::new(), String::new()));
                            merge_stream_tool_call_delta(entry, tc);
                            sink.on_tool_call(i, &entry.1, &entry.2);
                        }
                    }
                    if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                        finish_reason = Some(fr.to_string());
                    }
                }
            }
        }

        let tool_calls_v: Vec<ToolCall> = tool_calls
            .into_iter()
            .map(|(_, (id, name, args))| ToolCall {
                id,
                kind: "function".into(),
                function: FunctionCall {
                    name,
                    arguments: args,
                },
            })
            .collect();

        if content.is_empty() && tool_calls_v.is_empty() && finish_reason.is_none() {
            return Err(LlmError::Incomplete);
        }

        Ok(Completion {
            content,
            reasoning: if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
            tool_calls: tool_calls_v,
            finish_reason,
            usage,
        })
    }
}

fn parse_usage(val: &Value) -> Usage {
    val.get("usage")
        .and_then(parse_usage_obj)
        .unwrap_or_default()
}

fn parse_usage_obj(u: &Value) -> Option<Usage> {
    // Cache-hit tokens: OpenAI/GLM/Moonshot report `prompt_tokens_details
    // .cached_tokens`; DeepSeek exposes `prompt_cache_hit_tokens` at the usage
    // root; some Moonshot builds use a bare `cached_tokens`. `prompt_tokens`
    // already includes these on every one of them.
    let cached = u
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .or_else(|| u.get("prompt_cache_hit_tokens").and_then(|v| v.as_u64()))
        .or_else(|| u.get("cached_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    Some(Usage {
        input_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        output_tokens: u
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        reasoning_tokens: u
            .get("completion_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cached_input_tokens: cached,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[test]
    fn parses_cache_hits_across_providers() {
        // OpenAI / GLM / Moonshot: prompt_tokens_details.cached_tokens.
        let openai = json!({"prompt_tokens": 1000, "completion_tokens": 50,
            "prompt_tokens_details": {"cached_tokens": 800}});
        let u = parse_usage_obj(&openai).unwrap();
        assert_eq!(
            (u.input_tokens, u.output_tokens, u.cached_input_tokens),
            (1000, 50, 800)
        );
        // DeepSeek: prompt_cache_hit_tokens at the usage root.
        let deepseek = json!({"prompt_tokens": 1000, "completion_tokens": 50,
            "prompt_cache_hit_tokens": 640, "prompt_cache_miss_tokens": 360});
        assert_eq!(parse_usage_obj(&deepseek).unwrap().cached_input_tokens, 640);
        // No cache reported → 0, input still populated.
        let plain = json!({"prompt_tokens": 12, "completion_tokens": 3});
        let u = parse_usage_obj(&plain).unwrap();
        assert_eq!((u.input_tokens, u.cached_input_tokens), (12, 0));
    }

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "read".into(),
                arguments: "{}".into(),
            },
        }
    }

    // #74: a turn interrupted after GLM emitted `tool_calls` but before its
    // `tool` results were persisted leaves a dangling `tool_calls`. GLM
    // tolerates re-sending it; DeepSeek 400s. sanitize must strip the unanswered
    // call so the request stays valid across a model switch.
    #[test]
    fn drops_unanswered_tool_calls() {
        let mut asst = Message::assistant("");
        asst.tool_calls = vec![call("a"), call("b")];
        let msgs = vec![
            Message::user("hi"),
            asst,
            Message::tool("a", "read", "ok"),
            // no reply for "b"
        ];
        let out = OpenAiProvider::sanitize(&msgs);
        let asst_json = &out[1];
        let kept = asst_json["tool_calls"].as_array().unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["id"], "a");
    }

    // When none of an assistant's tool_calls were answered, the whole field is
    // omitted so the message degrades to a plain assistant turn.
    #[test]
    fn omits_tool_calls_when_none_answered() {
        let mut asst = Message::assistant("partial");
        asst.tool_calls = vec![call("x")];
        let out = OpenAiProvider::sanitize(&[asst]);
        assert!(out[0].get("tool_calls").is_none());
        assert_eq!(out[0]["content"], "partial");
    }

    // The symmetric orphan: a `tool` message with no preceding `tool_calls`
    // also 400s on strict endpoints, so it is dropped entirely.
    #[test]
    fn drops_orphan_tool_message() {
        let msgs = vec![Message::user("hi"), Message::tool("ghost", "read", "stale")];
        let out = OpenAiProvider::sanitize(&msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
    }

    // A well-formed pair passes through untouched.
    #[test]
    fn keeps_matched_pair() {
        let mut asst = Message::assistant("");
        asst.tool_calls = vec![call("a")];
        let msgs = vec![asst, Message::tool("a", "read", "ok")];
        let out = OpenAiProvider::sanitize(&msgs);
        assert_eq!(out[0]["tool_calls"].as_array().unwrap().len(), 1);
        assert_eq!(out[1]["tool_call_id"], "a");
    }

    #[test]
    fn stream_delta_keeps_first_non_empty_tool_name() {
        let mut entry = ("".to_string(), "".to_string(), "".to_string());
        merge_stream_tool_call_delta(
            &mut entry,
            &json!({
                "index": 0,
                "id": "call_1",
                "type": "function",
                "function": { "name": "read", "arguments": "" }
            }),
        );
        merge_stream_tool_call_delta(
            &mut entry,
            &json!({
                "index": 0,
                "id": null,
                "type": null,
                "function": { "name": "", "arguments": "{\"file_path\":\"C:/test.txt\"}" }
            }),
        );
        assert_eq!(entry.0, "call_1");
        assert_eq!(entry.1, "read");
        assert_eq!(entry.2, "{\"file_path\":\"C:/test.txt\"}");
    }
}
