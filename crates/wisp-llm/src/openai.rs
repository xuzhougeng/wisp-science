//! OpenAI-compatible `/chat/completions` provider (OpenAI, DeepSeek, Qwen,
//! MiniMax, Ollama, LM Studio, any OpenAI-compatible endpoint).
//!
//! Reasoning fields are normalized across vendors:
//! - DeepSeek: `reasoning_content` (string)
//! - Qwen / some OpenAI-compat: `reasoning` (string)
//! - MiniMax: `reasoning_details` (array of `{text}`)

use crate::message::{Content, Message, Part, Role, ToolCall, ToolSchema};
use crate::provider::{
    openai_internal_tool_name, openai_wire_tool_name, LlmError, Provider, Result, StreamSink,
    Utf8Stream,
};
use crate::{Completion, FunctionCall, Usage};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

pub struct OpenAiProvider {
    cfg: crate::provider::ProviderConfig,
    client: reqwest::Client,
    selected_endpoint: std::sync::OnceLock<String>,
}

impl OpenAiProvider {
    pub fn new(cfg: crate::provider::ProviderConfig) -> Self {
        let client = crate::provider::http_client(&cfg);
        Self {
            cfg,
            client,
            selected_endpoint: std::sync::OnceLock::new(),
        }
    }

    fn endpoint_candidates(&self) -> Vec<String> {
        if let Some(endpoint) = self.selected_endpoint.get() {
            return vec![endpoint.clone()];
        }
        let base = self.cfg.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            vec![base.to_string()]
        } else if base.ends_with("/v1") {
            vec![format!("{base}/chat/completions")]
        } else {
            vec![
                format!("{base}/chat/completions"),
                format!("{base}/v1/chat/completions"),
            ]
        }
    }

    fn can_try_next_endpoint(status: u16) -> bool {
        matches!(status, 404 | 405)
    }

    fn response_is_html(resp: &reqwest::Response) -> bool {
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"))
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
        let (answered, requested) = crate::tool_call_pairing(messages);
        // Never replay chain-of-thought. Re-sending historical `reasoning_content`
        // bloats the request (~37% of an 86K kimi-k3 session) and feeds a
        // "thinking" model its own past flailing, reinforcing repeat loops.
        // Chat-completions models regenerate reasoning each turn and don't require
        // prior reasoning (unlike the encrypted-reasoning Responses API). Dropping
        // it uniformly — not per-turn — is also what keeps the prefix cacheable:
        // every assistant message serializes identically on every turn, so the
        // provider's prompt cache keeps hitting. (Keeping "only the last turn's"
        // would mutate a message once it's no longer last, breaking the prefix.)
        // Same stance as pi / grok-build, which never bulk-replay raw CoT.
        let mut out = Vec::new();
        // Chat Completions only accepts text content on `role: tool`. Keep
        // every tool result in its strict pairing position, then append the
        // images from the complete contiguous result batch as one multimodal
        // user message. Flushing only after the batch matters when an
        // assistant issued several parallel tool calls: no user message may
        // split their corresponding tool rows.
        let mut pending_tool_image_parts = Vec::new();
        for m in messages {
            if m.role != Role::Tool {
                flush_tool_images(&mut out, &mut pending_tool_image_parts);
            }
            let wire = match m.role {
                Role::System => Some(json!({ "role": "system", "content": m.content.as_text() })),
                Role::User => {
                    Some(json!({ "role": "user", "content": sanitize_user_content(&m.content) }))
                }
                Role::Assistant => {
                    let kept: Vec<ToolCall> = m
                        .tool_calls
                        .iter()
                        .filter(|tc| answered.contains(&tc.id))
                        .cloned()
                        .map(|mut tc| {
                            tc.function.name = openai_wire_tool_name(&tc.function.name).into();
                            tc.function.arguments =
                                crate::provider::valid_json_tool_arguments(&tc.function.arguments);
                            tc
                        })
                        .collect();
                    let text = m.content.as_text();
                    // Reasoning-only / interrupted turns persist as empty
                    // assistant messages and replay as `content: ""`, which
                    // strict endpoints reject — kimi-k3 400s with "the message
                    // ... must not be empty" (verified live; DeepSeek
                    // tolerates it). Nothing replayable is left, so drop the
                    // turn. Empty text alongside tool_calls is fine on both.
                    if kept.is_empty() && text.is_empty() {
                        None
                    } else {
                        let mut o = json!({ "role": "assistant", "content": text });
                        if !kept.is_empty() {
                            o["tool_calls"] = serde_json::to_value(&kept).unwrap_or(Value::Null);
                        }
                        Some(o)
                    }
                }
                Role::Tool => {
                    let id = m.tool_call_id.clone().unwrap_or_default();
                    if !requested.contains(&id) {
                        continue;
                    }
                    if let Some(parts) = tool_image_parts(&m.content) {
                        pending_tool_image_parts.extend(parts);
                    }
                    Some(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": m.content.as_text(),
                    }))
                }
            };
            if let Some(wire) = wire {
                out.push(wire);
            }
        }
        flush_tool_images(&mut out, &mut pending_tool_image_parts);
        out
    }

    fn build_body(&self, messages: &[Message], tools: &[ToolSchema], stream: bool) -> Value {
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                let mut t = t.clone();
                t.function.name = openai_wire_tool_name(&t.function.name).into();
                serde_json::to_value(t).unwrap_or(Value::Null)
            })
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
        if let Some(tier) = &self.cfg.service_tier {
            body["service_tier"] = json!(tier);
        }
        body
    }

    fn log_dispatch(&self, endpoint: &str, body: &Value, stream: bool) {
        let included = body.get("service_tier").is_some();
        let service_tier = body
            .get("service_tier")
            .and_then(Value::as_str)
            .unwrap_or("omitted");
        tracing::info!(
            target: "wisp",
            provider = "openai",
            model = %self.cfg.model,
            endpoint_kind = "chat_completions",
            endpoint_host = %endpoint_host(endpoint),
            service_tier,
            service_tier_in_body = included,
            stream,
            "llm_request_dispatch"
        );
    }

    async fn request(&self, body: Value) -> Result<Value> {
        let endpoints = self.endpoint_candidates();
        for (index, endpoint) in endpoints.iter().enumerate() {
            let has_next = index + 1 < endpoints.len();
            self.log_dispatch(endpoint, &body, false);
            let resp = self
                .client
                .post(endpoint)
                .headers(self.headers())
                .json(&body)
                .send()
                .await?;
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            if status >= 400 {
                if has_next && Self::can_try_next_endpoint(status) {
                    continue;
                }
                return Err(LlmError::Api { status, body: text });
            }
            match serde_json::from_str::<Value>(&text) {
                Ok(value)
                    if value
                        .get("choices")
                        .and_then(Value::as_array)
                        .is_some_and(|choices| !choices.is_empty()) =>
                {
                    let _ = self.selected_endpoint.set(endpoint.clone());
                    return Ok(value);
                }
                Ok(_) if has_next => continue,
                Ok(_) => {
                    return Err(LlmError::Config(format!(
                        "OpenAI-compatible endpoint returned an unexpected response from {endpoint}"
                    )))
                }
                Err(_) if has_next => continue,
                Err(error) => return Err(LlmError::Decode(error)),
            }
        }
        unreachable!("OpenAI-compatible endpoint candidates are never empty")
    }

    async fn streaming_response(&self, body: &Value) -> Result<(String, reqwest::Response)> {
        let endpoints = self.endpoint_candidates();
        for (index, endpoint) in endpoints.iter().enumerate() {
            let has_next = index + 1 < endpoints.len();
            self.log_dispatch(endpoint, body, true);
            let resp = self
                .client
                .post(endpoint)
                .headers(self.headers())
                .json(body)
                .send()
                .await?;
            let status = resp.status().as_u16();
            if status >= 400 {
                let text = resp.text().await.unwrap_or_default();
                if has_next && Self::can_try_next_endpoint(status) {
                    continue;
                }
                return Err(LlmError::Api { status, body: text });
            }
            if has_next && Self::response_is_html(&resp) {
                continue;
            }
            return Ok((endpoint.clone(), resp));
        }
        unreachable!("OpenAI-compatible endpoint candidates are never empty")
    }
}

fn sanitize_user_content(c: &Content) -> Value {
    match c {
        Content::Text(s) => json!(s),
        Content::Parts(parts) => json!(openai_content_parts(parts)),
    }
}

fn openai_content_parts(parts: &[Part]) -> Vec<Value> {
    parts
        .iter()
        .map(|p| match p {
            Part::Text { text, .. } => json!({ "type": "text", "text": text }),
            Part::Image { image_url, .. } => {
                json!({ "type": "image_url", "image_url": { "url": image_url.url } })
            }
        })
        .collect()
}

fn tool_image_parts(content: &Content) -> Option<Vec<Value>> {
    let Content::Parts(parts) = content else {
        return None;
    };
    parts
        .iter()
        .any(|part| matches!(part, Part::Image { .. }))
        .then(|| openai_content_parts(parts))
}

fn flush_tool_images(out: &mut Vec<Value>, pending: &mut Vec<Value>) {
    if !pending.is_empty() {
        out.push(json!({
            "role": "user",
            "content": std::mem::take(pending),
        }));
    }
}

/// Visible assistant text from a Chat Completions `message`.
///
/// Compatible gateways do not agree on the JSON type: a string, `null`, an
/// array of text parts, or (when the model was asked for JSON) a parsed
/// object. `as_str()` alone turns every non-string into `""`, which made
/// Reader report "no JSON object" on an otherwise successful call (#1019).
fn extract_message_content(msg: &Value) -> String {
    match msg.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => extract_content_array(parts),
        Some(Value::Object(_)) | Some(Value::Number(_)) | Some(Value::Bool(_)) => {
            msg["content"].to_string()
        }
        Some(Value::Null) | None => String::new(),
    }
}

fn extract_content_array(parts: &[Value]) -> String {
    let mut text = String::new();
    let mut saw_part = false;
    for part in parts {
        if let Some(piece) = content_part_text(part) {
            saw_part = true;
            text.push_str(&piece);
        }
    }
    if saw_part {
        text
    } else {
        Value::Array(parts.to_vec()).to_string()
    }
}

fn content_part_text(part: &Value) -> Option<String> {
    if let Some(text) = part.as_str() {
        return Some(text.to_string());
    }
    match part.get("type").and_then(Value::as_str) {
        Some("text") | Some("output_text") | None => part
            .get("text")
            .or_else(|| part.get("output_text"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
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
            .map(openai_internal_tool_name)
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
    let function = tc.get("function").and_then(|v| v.as_object());
    let name = function
        .and_then(|f| f.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            tc.get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        });
    if let Some(name) = name {
        entry.1 = openai_internal_tool_name(name).to_string();
    }
    let arguments = function
        .and_then(|f| f.get("arguments"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| tc.get("arguments").and_then(|v| v.as_str()));
    if let Some(arguments) = arguments {
        entry.2.push_str(arguments);
    }
}

fn ensure_named_tool_calls(calls: &[ToolCall]) -> Result<()> {
    if calls
        .iter()
        .any(|call| call.function.name.trim().is_empty())
    {
        return Err(LlmError::Config(
            "provider returned a tool call without a function name; for Sub2API/OpenAI subscription models, select the OpenAI Responses provider instead of OpenAI-compatible"
                .into(),
        ));
    }
    Ok(())
}

/// Detect an in-band error payload on an otherwise-healthy SSE stream and
/// carry the upstream detail out as `LlmError::Api`, so the user sees the
/// provider's actual reason instead of a generic "stream cut" message (#798).
/// Shapes seen from compatible relays: `{"error":{"message":..,"code":..}}`,
/// `{"error":"plain text"}`, and `{"type":"error","message":..}`. A `"error":
/// null` field on a normal chunk is not an error. When the payload carries a
/// numeric HTTP-like code, use it as the status so retry (429/5xx) and
/// context-overflow (400) handling keep working; otherwise report the 200 the
/// relay actually sent.
fn in_band_stream_error(val: &Value) -> Option<LlmError> {
    let err = val.get("error").filter(|e| !e.is_null());
    if err.is_none() && val.get("type").and_then(Value::as_str) != Some("error") {
        return None;
    }
    let detail = err.unwrap_or(val);
    let message = detail
        .as_str()
        .or_else(|| detail.get("message").and_then(Value::as_str))
        .or_else(|| val.get("message").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| detail.to_string());
    let status = detail
        .get("code")
        .or_else(|| detail.get("status"))
        .and_then(|code| match code {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        })
        .and_then(|code| u16::try_from(code).ok())
        .filter(|code| (100..=599).contains(code))
        .unwrap_or(200);
    Some(LlmError::Api {
        status,
        body: message,
    })
}

/// Merge one SSE `content` / `reasoning_content` value into the assembled
/// string. Compatible relays disagree on whether the field is a fragment or
/// the full snapshot so far; treating snapshots as fragments is O(n²) in both
/// the assembled text and every live UI event (#985).
///
/// Returns the byte offset of the newly appended suffix, or `None` when the
/// chunk is empty or a duplicate snapshot.
fn apply_stream_delta(acc: &mut String, incoming: &str) -> Option<usize> {
    if incoming.is_empty() || incoming == acc.as_str() {
        return None;
    }
    if incoming.starts_with(acc.as_str()) {
        let start = acc.len();
        acc.push_str(&incoming[start..]);
        return (start < acc.len()).then_some(start);
    }
    let start = acc.len();
    acc.push_str(incoming);
    Some(start)
}

fn stream_reasoning_delta(delta: &Value) -> Option<&str> {
    delta
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .or_else(|| delta.get("reasoning").and_then(|v| v.as_str()))
}

fn append_stream_content_delta(
    delta: &Value,
    content: &mut String,
    reasoning: &mut String,
    sink: &mut dyn StreamSink,
) {
    // Some compatible endpoints (notably Alibaba/DashScope thinking streams)
    // include `content: ""` beside every non-empty `reasoning_content` delta.
    // Emitting that empty text event breaks an otherwise contiguous reasoning
    // run into one UI disclosure per token fragment. `apply_stream_delta`
    // already drops empty strings; keep using it so a snapshot replay of the
    // same text also stays silent.
    if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
        if let Some(start) = apply_stream_delta(content, t) {
            sink.on_text(&content[start..]);
        }
    }
    if let Some(r) = stream_reasoning_delta(delta) {
        if let Some(start) = apply_stream_delta(reasoning, r) {
            sink.on_reasoning(&reasoning[start..]);
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
        let content = extract_message_content(&msg);
        let reasoning = extract_reasoning(&msg);
        let tool_calls = normalize_tool_calls(&msg);
        ensure_named_tool_calls(&tool_calls)?;
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
        let (endpoint, resp) = self.streaming_response(&body).await?;
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
        let mut saw_done = false;

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
                    if line == "[DONE]" {
                        saw_done = true;
                        continue;
                    }
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(val) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    // Some OpenAI-compatible relays keep the HTTP/SSE response
                    // at 200 after the upstream connection has failed, then
                    // encode the failure as a normal `data:` payload and may
                    // still append `[DONE]`. Treating `[DONE]` alone as success
                    // in that case commits a partial answer and ends the turn.
                    if let Some(error) = in_band_stream_error(&val) {
                        return Err(error);
                    }
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
                    append_stream_content_delta(&delta, &mut content, &mut reasoning, sink);
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
        ensure_named_tool_calls(&tool_calls_v)?;

        if content.is_empty() && tool_calls_v.is_empty() && finish_reason.is_none() {
            return Err(LlmError::Incomplete);
        }
        if crate::provider::stream_was_cut(finish_reason.is_some() || saw_done, sink.is_cancelled())
        {
            return Err(LlmError::Incomplete);
        }

        let _ = self.selected_endpoint.set(endpoint);

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

fn endpoint_host(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
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
    use crate::provider::Provider;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Default)]
    struct RecordingSink {
        text: Vec<String>,
        reasoning: Vec<String>,
    }

    impl StreamSink for RecordingSink {
        fn on_text(&mut self, delta: &str) {
            self.text.push(delta.into());
        }

        fn on_reasoning(&mut self, delta: &str) {
            self.reasoning.push(delta.into());
        }

        fn on_tool_call(&mut self, _: usize, _: &str, _: &str) {}

        fn on_usage(&mut self, _: Usage) {}
    }

    async fn serve_responses(
        responses: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut paths = vec![];
            for (status, content_type, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 16 * 1024];
                let read = socket.read(&mut request).await.unwrap();
                let head = String::from_utf8_lossy(&request[..read]);
                let path = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or_default()
                    .to_string();
                paths.push(path);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            paths
        });
        (format!("http://{address}"), task)
    }

    fn local_provider(base_url: &str) -> OpenAiProvider {
        let mut cfg = crate::ProviderConfig::openai(base_url, "test-key", "test-model");
        cfg.proxy = Some("none".into());
        OpenAiProvider::new(cfg)
    }

    #[test]
    fn builds_common_chat_completion_endpoint_candidates() {
        let provider = local_provider("https://example.test/");
        assert_eq!(
            provider.endpoint_candidates(),
            [
                "https://example.test/chat/completions",
                "https://example.test/v1/chat/completions",
            ]
        );

        let provider = local_provider("https://example.test/v1");
        assert_eq!(
            provider.endpoint_candidates(),
            ["https://example.test/v1/chat/completions"]
        );

        let provider = local_provider("https://example.test/custom/chat/completions");
        assert_eq!(
            provider.endpoint_candidates(),
            ["https://example.test/custom/chat/completions"]
        );
    }

    #[tokio::test]
    async fn complete_falls_back_from_html_site_to_v1_api() {
        let (base_url, requests) = serve_responses(vec![
            ("200 OK", "text/html", "<html>site shell</html>"),
            (
                "200 OK",
                "application/json",
                r#"{"choices":[{"message":{"content":"OK"},"finish_reason":"stop"}]}"#,
            ),
        ])
        .await;
        let provider = local_provider(&base_url);

        let completion = provider
            .complete(&[Message::user("Reply with OK.")], &[])
            .await
            .unwrap();

        assert_eq!(completion.content, "OK");
        assert_eq!(
            requests.await.unwrap(),
            ["/chat/completions", "/v1/chat/completions"]
        );
    }

    #[test]
    fn extract_message_content_joins_text_parts() {
        let msg = json!({
            "content": [
                {"type": "text", "text": "{\"summary\":"},
                {"type": "text", "text": "\"hit\"}"}
            ]
        });
        assert_eq!(extract_message_content(&msg), "{\"summary\":\"hit\"}");
    }

    #[test]
    fn extract_message_content_serializes_object_payload() {
        let msg = json!({ "content": { "summary": "hit", "evidence": [] } });
        let parsed: Value = serde_json::from_str(&extract_message_content(&msg)).unwrap();
        assert_eq!(parsed["summary"], "hit");
        assert_eq!(parsed["evidence"], json!([]));
    }

    #[test]
    fn extract_message_content_treats_null_as_empty() {
        assert_eq!(extract_message_content(&json!({ "content": null })), "");
    }

    #[tokio::test]
    async fn complete_reads_array_content_parts() {
        let (base_url, requests) = serve_responses(vec![(
            "200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":[{"type":"text","text":"OK"}]},"finish_reason":"stop"}]}"#,
        )])
        .await;
        let provider = local_provider(&base_url);
        let completion = provider
            .complete(&[Message::user("Reply with OK.")], &[])
            .await
            .unwrap();
        assert_eq!(completion.content, "OK");
        assert_eq!(requests.await.unwrap(), ["/chat/completions"]);
    }

    #[tokio::test]
    async fn complete_keeps_reasoning_when_content_is_null() {
        let (base_url, requests) = serve_responses(vec![(
            "200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":null,"reasoning_content":"think"},"finish_reason":"stop"}]}"#,
        )])
        .await;
        let provider = local_provider(&base_url);
        let completion = provider
            .complete(&[Message::user("Reply with OK.")], &[])
            .await
            .unwrap();
        assert_eq!(completion.content, "");
        assert_eq!(completion.reasoning.as_deref(), Some("think"));
        assert_eq!(requests.await.unwrap(), ["/chat/completions"]);
    }

    #[tokio::test]
    async fn stream_falls_back_from_html_site_to_v1_api() {
        let (base_url, requests) = serve_responses(vec![
            ("200 OK", "text/html", "<html>site shell</html>"),
            (
                "200 OK",
                "text/event-stream",
                "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            ),
        ])
        .await;
        let provider = local_provider(&base_url);
        let mut sink = RecordingSink::default();

        let completion = provider
            .stream(&[Message::user("Reply with OK.")], &[], &mut sink)
            .await
            .unwrap();

        assert_eq!(completion.content, "OK");
        assert_eq!(sink.text, ["OK"]);
        assert_eq!(
            requests.await.unwrap(),
            ["/chat/completions", "/v1/chat/completions"]
        );
    }

    // #798: the upstream failure reason must survive to the user instead of
    // collapsing into a generic "stream cut" message.
    #[tokio::test]
    async fn stream_rejects_in_band_error_even_when_relay_appends_done() {
        let (base_url, requests) = serve_responses(vec![(
            "200 OK",
            "text/event-stream",
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\ndata: {\"error\":{\"message\":\"upstream connection reset\"}}\n\ndata: [DONE]\n\n",
        )])
        .await;
        let provider = local_provider(&base_url);
        let mut sink = RecordingSink::default();

        let error = provider
            .stream(&[Message::user("finish the report")], &[], &mut sink)
            .await
            .unwrap_err();

        assert!(
            matches!(&error, LlmError::Api { status: 200, body } if body == "upstream connection reset"),
            "in-band error must carry the upstream message, got: {error}"
        );
        assert_eq!(sink.text, ["partial"]);
        assert_eq!(requests.await.unwrap(), ["/chat/completions"]);
    }

    // #798: detail extraction across the payload shapes compatible relays use,
    // without misreading `"error": null` on healthy chunks as a failure.
    #[test]
    fn in_band_stream_error_extracts_detail_and_spares_null_error_fields() {
        let cases = [
            (
                json!({"error": {"message": "insufficient balance", "code": "1113"}}),
                (200u16, "insufficient balance"),
            ),
            (
                json!({"error": {"message": "rate limit reached", "code": 429}}),
                (429, "rate limit reached"),
            ),
            (
                json!({"error": "plain relay failure"}),
                (200, "plain relay failure"),
            ),
            (
                json!({"type": "error", "message": "upstream timeout"}),
                (200, "upstream timeout"),
            ),
        ];
        for (val, (want_status, want_body)) in cases {
            match in_band_stream_error(&val) {
                Some(LlmError::Api { status, body }) => {
                    assert_eq!((status, body.as_str()), (want_status, want_body), "{val}");
                }
                other => panic!("{val} should be an Api error, got {other:?}"),
            }
        }
        // A `"error": null` field beside normal deltas (some relays emit it on
        // every chunk) must not fail the whole stream.
        assert!(in_band_stream_error(
            &json!({"error": null, "choices": [{"delta": {"content": "hi"}}]})
        )
        .is_none());
        assert!(in_band_stream_error(&json!({"choices": []})).is_none());
    }

    #[tokio::test]
    async fn does_not_retry_authentication_failures() {
        let (base_url, requests) = serve_responses(vec![(
            "401 Unauthorized",
            "application/json",
            r#"{"error":{"message":"bad key"}}"#,
        )])
        .await;
        let provider = local_provider(&base_url);

        let error = provider
            .complete(&[Message::user("Reply with OK.")], &[])
            .await
            .unwrap_err();

        assert!(matches!(error, LlmError::Api { status: 401, .. }));
        assert_eq!(requests.await.unwrap(), ["/chat/completions"]);
    }

    #[test]
    fn ignores_empty_content_between_alibaba_reasoning_deltas() {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut sink = RecordingSink::default();

        for delta in [
            json!({"content": "", "reasoning_content": "column in"}),
            json!({"content": "", "reasoning_content": " test fixtures"}),
            json!({"content": "", "reasoning_content": ""}),
            json!({"content": "Fixed.", "reasoning_content": null}),
        ] {
            append_stream_content_delta(&delta, &mut content, &mut reasoning, &mut sink);
        }

        assert_eq!(content, "Fixed.");
        assert_eq!(reasoning, "column in test fixtures");
        assert_eq!(sink.text, ["Fixed."]);
        assert_eq!(sink.reasoning, ["column in", " test fixtures"]);
    }

    #[test]
    fn apply_stream_delta_keeps_true_fragments() {
        let mut acc = String::new();
        assert_eq!(apply_stream_delta(&mut acc, "Hel"), Some(0));
        assert_eq!(apply_stream_delta(&mut acc, "lo"), Some(3));
        assert_eq!(apply_stream_delta(&mut acc, " world"), Some(5));
        assert_eq!(acc, "Hello world");
    }

    #[test]
    fn apply_stream_delta_collapses_growing_snapshots() {
        let mut acc = String::new();
        let mut emitted = String::new();
        for snap in ["H", "He", "Hel", "Hell", "Hello", "Hello", "Hello!"] {
            if let Some(start) = apply_stream_delta(&mut acc, snap) {
                emitted.push_str(&acc[start..]);
            }
        }
        assert_eq!(acc, "Hello!");
        assert_eq!(emitted, "Hello!");
    }

    #[test]
    fn apply_stream_delta_snapshot_replay_is_linear() {
        let mut acc = String::new();
        let mut emitted = 0usize;
        for n in 1..=256 {
            let snap = "x".repeat(n);
            if let Some(start) = apply_stream_delta(&mut acc, &snap) {
                emitted += acc.len() - start;
            }
        }
        assert_eq!(acc.len(), 256);
        assert_eq!(emitted, 256);
    }

    #[test]
    fn append_stream_collapses_cumulative_content_and_reasoning() {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut sink = RecordingSink::default();

        for delta in [
            json!({"content": "", "reasoning_content": "think"}),
            json!({"content": "", "reasoning_content": "think more"}),
            json!({"content": "Hi", "reasoning_content": "think more"}),
            json!({"content": "Hi there", "reasoning_content": "think more"}),
            json!({"content": "Hi there", "reasoning_content": "think more", "reasoning": "ignored"}),
        ] {
            append_stream_content_delta(&delta, &mut content, &mut reasoning, &mut sink);
        }

        assert_eq!(content, "Hi there");
        assert_eq!(reasoning, "think more");
        assert_eq!(sink.text, ["Hi", " there"]);
        assert_eq!(sink.reasoning, ["think", " more"]);
    }

    #[test]
    fn append_stream_accepts_qwen_reasoning_field() {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut sink = RecordingSink::default();
        append_stream_content_delta(
            &json!({"reasoning": "step"}),
            &mut content,
            &mut reasoning,
            &mut sink,
        );
        append_stream_content_delta(
            &json!({"reasoning": "step 2"}),
            &mut content,
            &mut reasoning,
            &mut sink,
        );
        assert_eq!(reasoning, "step 2");
        assert_eq!(sink.reasoning, ["step", " 2"]);
    }

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

    fn image_tool_result(id: &str) -> Message {
        let mut message = Message::tool(id, "view_image", "plot.png");
        message.content = Content::Parts(vec![
            Part::Text {
                kind: "text".into(),
                text: "plot.png".into(),
            },
            Part::Image {
                kind: "image_url".into(),
                image_url: crate::ImageUrl {
                    url: "data:image/png;base64,AAAA".into(),
                },
            },
        ]);
        message
    }

    #[test]
    fn native_tool_images_follow_the_complete_chat_tool_result_batch() {
        let mut asst = Message::assistant("");
        asst.tool_calls = vec![call("image"), call("text")];
        let out = OpenAiProvider::sanitize(&[
            asst,
            image_tool_result("image"),
            Message::tool("text", "read", "ok"),
        ]);

        let roles: Vec<_> = out.iter().map(|message| message["role"].as_str()).collect();
        assert_eq!(
            roles,
            [Some("assistant"), Some("tool"), Some("tool"), Some("user")]
        );
        assert_eq!(out[1]["tool_call_id"], "image");
        assert_eq!(out[1]["content"], "plot.png");
        assert_eq!(out[2]["tool_call_id"], "text");
        let image_message = out[3]["content"].as_array().unwrap();
        assert_eq!(image_message[0]["type"], "text");
        assert_eq!(image_message[0]["text"], "plot.png");
        assert_eq!(image_message[1]["type"], "image_url");
        assert_eq!(
            image_message[1]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
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

    // Context compaction truncates oversized arguments mid-string, and a
    // `finish_reason: "length"` turn can persist a half-written call. Strict
    // gateways re-parse history arguments and 400 ("Unterminated string") on
    // such values, so the wire format must replace them with valid JSON.
    #[test]
    fn replaces_invalid_tool_arguments_with_empty_object() {
        let mut truncated = call("a");
        truncated.function.arguments =
            "{\"path\":\"/tmp/long...[... tool arguments archived ...]".into();
        let mut empty = call("b");
        empty.function.arguments = String::new();
        let mut asst = Message::assistant("");
        asst.tool_calls = vec![truncated, empty];
        let msgs = vec![
            asst,
            Message::tool("a", "read", "ok"),
            Message::tool("b", "read", "ok"),
        ];
        let out = OpenAiProvider::sanitize(&msgs);
        let kept = out[0]["tool_calls"].as_array().unwrap();
        assert_eq!(kept[0]["function"]["arguments"], "{}");
        assert_eq!(kept[1]["function"]["arguments"], "{}");
    }

    #[test]
    fn keeps_valid_tool_arguments_verbatim() {
        let mut valid = call("a");
        valid.function.arguments = "{\"path\":\"/tmp/x\"}".into();
        let mut asst = Message::assistant("");
        asst.tool_calls = vec![valid];
        let msgs = vec![asst, Message::tool("a", "read", "ok")];
        let out = OpenAiProvider::sanitize(&msgs);
        assert_eq!(
            out[0]["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"/tmp/x\"}"
        );
    }

    /// Reasoning-only / interrupted turns persist as empty assistant messages;
    /// replayed as `content: ""` they 400 on strict endpoints (kimi-k3: "the
    /// message ... must not be empty", verified live). With nothing
    /// replayable left, the turn must be dropped.
    #[test]
    fn drops_empty_assistant_turn_without_tool_calls() {
        let msgs = vec![
            Message::user("hi"),
            Message::assistant(""),
            Message::user("continue"),
        ];
        let out = OpenAiProvider::sanitize(&msgs);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|m| m["role"] != "assistant"));
    }

    /// Empty text alongside tool_calls is accepted by both DeepSeek and kimi
    /// (verified live), so the call turn stays — only truly empty turns drop.
    #[test]
    fn keeps_empty_text_assistant_with_answered_tool_calls() {
        let mut asst = Message::assistant("");
        asst.tool_calls = vec![call("a")];
        let msgs = vec![Message::user("run"), asst, Message::tool("a", "read", "ok")];
        let out = OpenAiProvider::sanitize(&msgs);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1]["role"], "assistant");
        assert_eq!(out[1]["content"], "");
        assert_eq!(out[1]["tool_calls"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn aliases_reserved_python_name_on_openai_wire() {
        let provider = OpenAiProvider::new(crate::ProviderConfig::openai(
            "https://example.test/v1",
            "",
            "codex",
        ));
        let tools = vec![ToolSchema::new(
            "python",
            "Run Python",
            json!({"type": "object"}),
        )];
        let body = provider.build_body(&[Message::user("analyze")], &tools, false);
        assert_eq!(body["tools"][0]["function"]["name"], "wisp_python");

        let mut asst = Message::assistant("");
        let mut python_call = call("py");
        python_call.function.name = "python".into();
        asst.tool_calls = vec![python_call];
        let history = OpenAiProvider::sanitize(&[asst, Message::tool("py", "python", "ok")]);
        assert_eq!(
            history[0]["tool_calls"][0]["function"]["name"],
            "wisp_python"
        );

        let calls = normalize_tool_calls(&json!({
            "tool_calls": [{
                "id": "py",
                "function": {"name": "wisp_python", "arguments": "{}"}
            }]
        }));
        assert_eq!(calls[0].function.name, "python");
    }

    #[test]
    fn fast_service_tier_is_top_level_and_independent_of_effort() {
        let mut cfg = crate::ProviderConfig::openai("https://example.test/v1", "", "gpt-5.6-sol");
        cfg.reasoning_effort = Some("high".into());
        cfg.service_tier = Some("priority".into());
        let provider = OpenAiProvider::new(cfg);
        for stream in [false, true] {
            let body = provider.build_body(&[Message::user("hi")], &[], stream);
            assert_eq!(body["service_tier"], "priority");
            assert_eq!(body["reasoning_effort"], "high");
        }
    }

    #[test]
    fn provider_default_omits_service_tier_from_chat_completions() {
        let provider = OpenAiProvider::new(crate::ProviderConfig::openai(
            "https://example.test/v1",
            "",
            "gpt-5.6-sol",
        ));
        for stream in [false, true] {
            let body = provider.build_body(&[Message::user("hi")], &[], stream);
            assert!(body.get("service_tier").is_none());
        }
    }

    // reasoning_content is never replayed — not even for the most recent turn.
    // Bulk-replaying past CoT bloats context and reinforces repeat loops; keeping
    // it off *every* assistant message keeps each one byte-stable across turns so
    // the provider's prefix cache keeps hitting.
    #[test]
    fn never_replays_reasoning() {
        let mut old = Message::assistant("first");
        old.reasoning = Some("old thinking".into());
        let mut recent = Message::assistant("second");
        recent.reasoning = Some("fresh thinking".into());
        let msgs = vec![old, Message::user("more"), recent];
        let out = OpenAiProvider::sanitize(&msgs);
        assert!(out[0].get("reasoning_content").is_none());
        assert!(
            out[2].get("reasoning_content").is_none(),
            "even the last turn's reasoning is dropped for cache stability"
        );
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

    #[test]
    fn stream_delta_accepts_flattened_relay_fields() {
        let mut entry = ("".to_string(), "".to_string(), "".to_string());
        merge_stream_tool_call_delta(
            &mut entry,
            &json!({
                "index": 0,
                "id": "call_1",
                "name": "wisp_python",
                "arguments": "{\"file_path\":\"C:/test.txt\"}"
            }),
        );
        assert_eq!(entry.0, "call_1");
        assert_eq!(entry.1, "python");
        assert_eq!(entry.2, "{\"file_path\":\"C:/test.txt\"}");
    }

    #[test]
    fn rejects_anonymous_tool_call_instead_of_dispatching_it() {
        let calls = vec![ToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "".into(),
                arguments: "{\"path\":\"x\"}".into(),
            },
        }];
        let err = ensure_named_tool_calls(&calls).unwrap_err();
        assert!(err.to_string().contains("without a function name"));
        assert!(err.to_string().contains("OpenAI Responses"));
    }
}
