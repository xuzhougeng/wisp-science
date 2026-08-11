//! OpenAI first-party Responses API (`/v1/responses`).

use crate::message::{Content, Message, Part, Role, ToolCall, ToolSchema};
use crate::provider::{
    openai_internal_tool_name, openai_wire_tool_name, LlmError, Provider, Result, StreamSink,
};
use crate::{Completion, FunctionCall, Usage};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct OpenAiResponsesProvider {
    cfg: crate::provider::ProviderConfig,
    client: reqwest::Client,
}

impl OpenAiResponsesProvider {
    pub fn new(cfg: crate::provider::ProviderConfig) -> Self {
        let client = crate::provider::http_client(&cfg);
        Self { cfg, client }
    }

    fn endpoint(&self) -> String {
        let base = self.cfg.base_url.trim_end_matches('/');
        if base.ends_with("/responses") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{base}/responses")
        } else {
            format!("{base}/v1/responses")
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

    fn build_body(&self, messages: &[Message], tools: &[ToolSchema]) -> Value {
        // DeepSeek/OpenAI Responses reject unpaired function_call items with
        // "No tool output found for tool call …". Match chat-completions #74:
        // drop unanswered calls (and orphan outputs) before building `input`.
        let input: Vec<Value> = sanitize_messages(messages)
            .iter()
            .flat_map(message_to_input)
            .collect();
        let mut body = json!({
            "model": self.cfg.model,
            "input": input,
            "max_output_tokens": self.cfg.max_tokens,
        });
        let tools_json: Vec<Value> = tools.iter().map(tool_to_responses).collect();
        if !tools_json.is_empty() {
            body["tools"] = json!(tools_json);
        }
        if let Some(effort) = &self.cfg.reasoning_effort {
            body["reasoning"] = json!({ "effort": effort });
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
        Ok(serde_json::from_str(&text)?)
    }
}

/// Keep only tool-call pairings that Responses endpoints accept.
///
/// A turn interrupted after the assistant emitted `function_call` but before
/// its `function_call_output` was persisted leaves a dangling call_id. Strict
/// providers (DeepSeek) 400 with "No tool output found for tool call …"; the
/// chat-completions path already strips these (#74). Symmetrically drop tool
/// outputs with no preceding call.
fn sanitize_messages(messages: &[Message]) -> Vec<Message> {
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
            Role::Assistant => {
                let mut out = m.clone();
                out.tool_calls.retain(|tc| answered.contains(&tc.id));
                // Empty assistant with every call stripped contributes nothing
                // useful on the wire; drop it so resume/`继续` is not preceded
                // by a no-op assistant item.
                if out.content.as_text().is_empty() && out.tool_calls.is_empty() {
                    None
                } else {
                    Some(out)
                }
            }
            Role::Tool => {
                let id = m.tool_call_id.as_deref().unwrap_or("");
                if requested.contains(id) {
                    Some(m.clone())
                } else {
                    None
                }
            }
            _ => Some(m.clone()),
        })
        .collect()
}

fn message_to_input(m: &Message) -> Vec<Value> {
    match m.role {
        Role::System => vec![json!({ "role": "system", "content": m.content.as_text() })],
        Role::User => vec![json!({ "role": "user", "content": content_to_responses(&m.content) })],
        Role::Assistant => {
            // The Responses API is stateless over `input`: an assistant turn that
            // issued tool calls must be replayed as `function_call` items so the
            // later `function_call_output` finds its matching call_id. Otherwise
            // the API rejects with "No tool call found for function call output".
            let mut items = vec![];
            let text = m.content.as_text();
            if !text.is_empty() {
                items.push(json!({ "role": "assistant", "content": text }));
            }
            for tc in &m.tool_calls {
                items.push(json!({
                    "type": "function_call",
                    "call_id": tc.id,
                    "name": openai_wire_tool_name(&tc.function.name),
                    "arguments": crate::provider::valid_json_tool_arguments(&tc.function.arguments),
                }));
            }
            items
        }
        Role::Tool => vec![json!({
            "type": "function_call_output",
            "call_id": m.tool_call_id.clone().unwrap_or_default(),
            "output": m.content.as_text(),
        })],
    }
}

fn content_to_responses(c: &Content) -> Value {
    match c {
        Content::Text(s) => json!(s),
        Content::Parts(parts) => json!(parts.iter().map(part_to_responses).collect::<Vec<_>>()),
    }
}

fn part_to_responses(p: &Part) -> Value {
    match p {
        Part::Text { text, .. } => json!({ "type": "input_text", "text": text }),
        Part::Image { image_url, .. } => {
            json!({ "type": "input_image", "image_url": image_url.url.clone() })
        }
    }
}

fn tool_to_responses(t: &ToolSchema) -> Value {
    json!({
        "type": "function",
        "name": openai_wire_tool_name(&t.function.name),
        "description": t.function.description.clone(),
        "parameters": t.function.parameters.clone(),
    })
}

fn parse_completion(val: &Value) -> Completion {
    let mut content = val
        .get("output_text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut tool_calls = vec![];

    if let Some(output) = val.get("output").and_then(|v| v.as_array()) {
        for item in output {
            match item.get("type").and_then(|v| v.as_str()) {
                Some("message") => {
                    if content.is_empty() {
                        content.push_str(&message_text(item));
                    }
                }
                Some("function_call") => {
                    let id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(openai_internal_tool_name)
                        .unwrap_or("")
                        .to_string();
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    tool_calls.push(ToolCall {
                        id,
                        kind: "function".into(),
                        function: FunctionCall { name, arguments },
                    });
                }
                _ => {}
            }
        }
    }

    let usage = val.get("usage").map(parse_usage).unwrap_or_default();
    let finish_reason = val.get("status").and_then(|v| v.as_str()).map(String::from);
    Completion {
        content,
        reasoning: None,
        tool_calls,
        finish_reason,
        usage,
    }
}

fn message_text(item: &Value) -> String {
    item.get("content")
        .and_then(|v| v.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| {
                    p.get("text")
                        .or_else(|| p.get("output_text"))
                        .and_then(|v| v.as_str())
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        reasoning_tokens: u
            .get("output_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cached_input_tokens: u
            .get("input_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    }
}

#[async_trait]
impl Provider for OpenAiResponsesProvider {
    fn name(&self) -> &str {
        "openai-responses"
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }

    async fn complete(&self, messages: &[Message], tools: &[ToolSchema]) -> Result<Completion> {
        let val = self.request(self.build_body(messages, tools)).await?;
        ensure_completed_response(&val)?;
        Ok(parse_completion(&val))
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> Result<Completion> {
        let comp = self.complete(messages, tools).await?;
        if !comp.content.is_empty() {
            sink.on_text(&comp.content);
        }
        for (i, tc) in comp.tool_calls.iter().enumerate() {
            sink.on_tool_call(&crate::provider::ToolCallSnapshot {
                index: i,
                id: Some(tc.id.clone()),
                name: tc.function.name.clone(),
                arguments_so_far: tc.function.arguments.clone(),
            });
        }
        sink.on_usage(comp.usage.clone());
        Ok(comp)
    }
}

/// A non-streaming Responses request can return HTTP 200 with a terminal
/// `incomplete`, `failed`, or `cancelled` status and partial output. Only
/// `completed` is safe to commit as a successful agent iteration. Status-less
/// responses remain accepted for compatible relays that implement the older
/// subset of this wire format.
fn ensure_completed_response(value: &Value) -> Result<()> {
    match value.get("status").and_then(Value::as_str) {
        None | Some("completed") => Ok(()),
        Some(_) => Err(LlmError::Incomplete),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_with_call(text: &str, call_id: &str, name: &str, args: &str) -> Message {
        let mut m = Message::assistant(text);
        m.tool_calls = vec![ToolCall {
            id: call_id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: args.into(),
            },
        }];
        m
    }

    fn wire_input(messages: &[Message]) -> Vec<Value> {
        sanitize_messages(messages)
            .iter()
            .flat_map(message_to_input)
            .collect()
    }

    /// Regression: a tool-call turn must be replayed as a `function_call` item so
    /// the following `function_call_output` has a matching `call_id`. Without it
    /// the Responses API rejects with "No tool call found for function call output".
    #[test]
    fn tool_call_turn_emits_matching_function_call() {
        let messages = vec![
            Message::user("run the skill"),
            assistant_with_call("", "call_abc", "openalex", "{\"q\":\"x\"}"),
            Message::tool("call_abc", "openalex", "result body"),
        ];

        let input = wire_input(&messages);

        let call = input
            .iter()
            .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("function_call"))
            .expect("function_call item present");
        assert_eq!(call["call_id"], "call_abc");
        assert_eq!(call["name"], "openalex");
        assert_eq!(call["arguments"], "{\"q\":\"x\"}");

        let output = input
            .iter()
            .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("function_call_output"))
            .expect("function_call_output item present");
        assert_eq!(
            output["call_id"], "call_abc",
            "output must match the emitted call_id"
        );
    }

    /// History arguments can be invalid JSON (compaction truncates oversized
    /// arguments mid-string; a `finish_reason: "length"` turn can persist a
    /// half-written call). Strict gateways re-parse them and 400, so the wire
    /// format must replace them with valid JSON.
    #[test]
    fn replaces_invalid_arguments_with_empty_object() {
        let messages = vec![
            Message::user("run the skill"),
            assistant_with_call("", "call_bad", "openalex", "{\"q\":\"x...[ archived ...]"),
            Message::tool("call_bad", "openalex", "result body"),
        ];

        let input = wire_input(&messages);

        let call = input
            .iter()
            .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("function_call"))
            .expect("function_call item present");
        assert_eq!(call["arguments"], "{}");
    }

    /// Interrupted turn: assistant emitted a call, then the user resumed before
    /// a tool result was persisted. DeepSeek 400s with
    /// "No tool output found for tool call …" unless we strip the dangling call.
    #[test]
    fn drops_unanswered_function_call_so_resume_can_retry() {
        let messages = vec![
            Message::user("poll training"),
            assistant_with_call(
                "",
                "call_00_ET_YySmFH64ARi0Sf1W1K7Q6631",
                "shell",
                "{\"cmd\":\"Start-Sleep -Seconds 110\"}",
            ),
            Message::user("继续"),
        ];
        let input = wire_input(&messages);
        assert!(
            input
                .iter()
                .all(|v| v.get("type").and_then(|t| t.as_str()) != Some("function_call")),
            "unanswered function_call must not be sent: {input:?}"
        );
        assert_eq!(input.last().unwrap()["role"], "user");
        assert_eq!(input.last().unwrap()["content"], "继续");
    }

    #[test]
    fn keeps_answered_call_when_sibling_is_unanswered() {
        let mut asst = Message::assistant("");
        asst.tool_calls = vec![
            ToolCall {
                id: "a".into(),
                kind: "function".into(),
                function: FunctionCall {
                    name: "read".into(),
                    arguments: "{}".into(),
                },
            },
            ToolCall {
                id: "b".into(),
                kind: "function".into(),
                function: FunctionCall {
                    name: "shell".into(),
                    arguments: "{}".into(),
                },
            },
        ];
        let messages = vec![
            Message::user("hi"),
            asst,
            Message::tool("a", "read", "ok"),
            // no reply for "b"
            Message::user("继续"),
        ];
        let input = wire_input(&messages);
        let calls: Vec<_> = input
            .iter()
            .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("function_call"))
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["call_id"], "a");
    }

    #[test]
    fn drops_orphan_function_call_output() {
        let messages = vec![Message::user("hi"), Message::tool("ghost", "read", "stale")];
        let input = wire_input(&messages);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
    }

    /// An empty-text tool-call turn must not emit a stray empty assistant message.
    #[test]
    fn empty_assistant_text_emits_only_call() {
        let items = message_to_input(&assistant_with_call("", "c1", "f", "{}"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call");
    }

    /// Assistant text alongside a tool call yields both a message and the call.
    #[test]
    fn assistant_text_and_call_emit_both() {
        let items = message_to_input(&assistant_with_call("thinking", "c1", "f", "{}"));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["role"], "assistant");
        assert_eq!(items[0]["content"], "thinking");
        assert_eq!(items[1]["type"], "function_call");
    }

    #[test]
    fn parse_completion_reads_function_call() {
        let val = json!({
            "output": [
                { "type": "function_call", "call_id": "call_9", "name": "openalex", "arguments": "{\"q\":\"y\"}" }
            ],
            "status": "completed",
            "usage": { "input_tokens": 3, "output_tokens": 5 }
        });
        let comp = parse_completion(&val);
        assert_eq!(comp.tool_calls.len(), 1);
        assert_eq!(comp.tool_calls[0].id, "call_9");
        assert_eq!(comp.tool_calls[0].function.name, "openalex");
        assert_eq!(comp.usage.input_tokens, 3);
        assert_eq!(comp.usage.output_tokens, 5);
    }

    #[test]
    fn rejects_partial_terminal_responses() {
        for status in ["incomplete", "failed", "cancelled", "in_progress"] {
            let value = json!({
                "status": status,
                "output_text": "partial report",
                "incomplete_details": {"reason": "upstream_error"}
            });
            assert!(matches!(
                ensure_completed_response(&value),
                Err(LlmError::Incomplete)
            ));
        }
        assert!(ensure_completed_response(&json!({"status": "completed"})).is_ok());
        assert!(ensure_completed_response(&json!({"output_text": "relay response"})).is_ok());
    }

    #[test]
    fn reserved_python_name_round_trips_through_responses_wire() {
        let schema = ToolSchema::new("python", "Run Python", json!({"type": "object"}));
        assert_eq!(tool_to_responses(&schema)["name"], "wisp_python");

        let input = message_to_input(&assistant_with_call("", "py", "python", "{}"));
        assert_eq!(input[0]["name"], "wisp_python");

        let comp = parse_completion(&json!({
            "output": [{
                "type": "function_call",
                "call_id": "py",
                "name": "wisp_python",
                "arguments": "{}"
            }]
        }));
        assert_eq!(comp.tool_calls[0].function.name, "python");
    }
}
