//! End-to-end wire test for the Anthropic provider: a local mock enforces the
//! documented Messages-API constraints (first message is user, strict role
//! alternation, tool_use/tool_result adjacency, non-empty text) the way the
//! real API 400s them. Histories that a cross-provider model switch replays
//! must come out of `build_body` legal — no real API key involved.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use serde_json::{json, Value};
use wisp_llm::anthropic::AnthropicProvider;
use wisp_llm::{FunctionCall, Message, Provider, ProviderConfig, ToolCall};

/// One-shot mock that captures the request body and answers 200 when it
/// satisfies the constraints, 400 with an Anthropic-style envelope otherwise.
struct MockAnthropic {
    url: String,
    captured: std::sync::mpsc::Receiver<Value>,
}

fn start_mock() -> MockAnthropic {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0_u8; 8192];
        let header_end = loop {
            let n = stream.read(&mut tmp).unwrap();
            assert!(n > 0, "connection closed before headers completed");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
        let len = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = buf[header_end..].to_vec();
        while body.len() < len {
            let n = stream.read(&mut tmp).unwrap();
            assert!(n > 0, "connection closed before body completed");
            body.extend_from_slice(&tmp[..n]);
        }
        let request: Value = serde_json::from_slice(&body[..len]).unwrap();
        let (status, payload) = match validate_messages(&request) {
            Some(message) => (
                400,
                json!({
                    "type": "error",
                    "error": {"type": "invalid_request_error", "message": message}
                }),
            ),
            None => (
                200,
                json!({
                    "id": "msg_mock", "type": "message", "role": "assistant",
                    "content": [{"type": "text", "text": "ok"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 10, "output_tokens": 2}
                }),
            ),
        };
        let payload = payload.to_string();
        let reason = if status == 200 { "OK" } else { "Bad Request" };
        write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
            payload.len()
        )
        .unwrap();
        stream.flush().unwrap();
        tx.send(request).unwrap();
    });
    MockAnthropic {
        url: format!("http://127.0.0.1:{port}"),
        captured: rx,
    }
}

/// The constraints Anthropic documents (and 400s on), checked the way the
/// real API reports them.
fn validate_messages(body: &Value) -> Option<String> {
    let messages = body.get("messages")?.as_array()?;
    if messages.is_empty() {
        return Some("messages: at least one message is required".into());
    }
    if messages[0]["role"] != "user" {
        return Some("messages: first message must use the \"user\" role".into());
    }
    let mut prev_role = "";
    let mut pending_tool_ids: Vec<String> = Vec::new();
    for (i, message) in messages.iter().enumerate() {
        let role = message["role"].as_str().unwrap_or("");
        if role == prev_role {
            return Some(format!(
                "messages: roles must alternate between \"user\" and \"assistant\", but found multiple \"{role}\" roles in a row"
            ));
        }
        let blocks = match &message["content"] {
            Value::String(text) => {
                if text.is_empty() {
                    return Some(format!("messages.{i}: text content is empty"));
                }
                vec![json!({ "type": "text", "text": text })]
            }
            Value::Array(items) => items.clone(),
            _ => Vec::new(),
        };
        if blocks.is_empty() {
            return Some(format!("messages.{i}: content must not be empty"));
        }
        for block in &blocks {
            if block["type"] == "text" && block["text"].as_str().unwrap_or("").is_empty() {
                return Some(format!("messages.{i}: text content is empty"));
            }
        }
        match role {
            "assistant" => {
                pending_tool_ids = blocks
                    .iter()
                    .filter(|block| block["type"] == "tool_use")
                    .map(|block| block["id"].as_str().unwrap_or("").to_string())
                    .collect();
            }
            "user" => {
                let result_ids: Vec<&str> = blocks
                    .iter()
                    .filter(|block| block["type"] == "tool_result")
                    .filter_map(|block| block["tool_use_id"].as_str())
                    .collect();
                for id in &result_ids {
                    if !pending_tool_ids.iter().any(|pending| pending == id) {
                        return Some(format!(
                            "messages.{i}: tool_result references an unexpected tool_use id"
                        ));
                    }
                }
                let unanswered: Vec<&String> = pending_tool_ids
                    .iter()
                    .filter(|pending| !result_ids.contains(&pending.as_str()))
                    .collect();
                if !unanswered.is_empty() {
                    return Some(format!(
                        "messages.{i}: tool_use ids were found without tool_result blocks immediately after"
                    ));
                }
                pending_tool_ids = Vec::new();
            }
            _ => {}
        }
        prev_role = role;
    }
    None
}

fn provider_for(url: &str) -> AnthropicProvider {
    // The mock is loopback; keep ambient proxies out of the way. Per-provider
    // config, not process environment: tests in this binary run concurrently.
    let mut cfg = ProviderConfig::anthropic(url, "sk-test", "claude-test");
    cfg.proxy = Some("none".into());
    AnthropicProvider::new(cfg)
}

fn tool_call(id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        kind: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: args.into(),
        },
    }
}

/// The shape a model switch replays: tool calls with mid-turn guidance
/// (consecutive user turns pre-fix) and an empty user text. All of it must
/// come out legal.
#[tokio::test]
async fn cross_provider_replay_passes_anthropic_constraints() {
    let mock = start_mock();
    let provider = provider_for(&mock.url);
    let mut searching = Message::assistant("");
    searching.tool_calls = vec![tool_call("tu_1", "shell", "{\"cmd\":\"fastqc\"}")];
    let mut reading = Message::assistant("duplicates 偏高");
    reading.tool_calls = vec![tool_call("tu_2", "read", "{}")];
    let history = vec![
        Message::user("分析这批数据"),
        searching,
        Message::tool("tu_1", "shell", "QC done"),
        Message::user("顺便看下 duplicates"),
        reading,
        Message::tool("tu_2", "read", "details"),
        Message::user(""),
        Message::assistant("final answer"),
    ];

    let completion = provider
        .complete(&history, &[])
        .await
        .expect("normalized history must satisfy the mock's Anthropic constraints");
    assert_eq!(completion.content, "ok");

    let body = mock.captured.recv().unwrap();
    let messages = body["messages"].as_array().unwrap();
    // Guidance merged into the tool_result user turn.
    let types: Vec<&str> = messages[2]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| block["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["tool_result", "text"]);
    // Empty user text became the placeholder, merged after the second result.
    let last_user = &messages[4]["content"].as_array().unwrap();
    assert_eq!(last_user.last().unwrap()["text"], " ");
}

/// A transcript that opens with an assistant turn must be reshaped to open
/// with user instead.
#[tokio::test]
async fn leading_assistant_history_opens_with_user() {
    let mock = start_mock();
    let provider = provider_for(&mock.url);
    let history = vec![Message::assistant("上次说到一半"), Message::user("继续")];

    provider
        .complete(&history, &[])
        .await
        .expect("leading assistant turn must be dropped, not rejected");

    let body = mock.captured.recv().unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
}

/// Control: the gatekeeper has teeth. A hand-written illegal body (the exact
/// pre-fix shape) gets the real API's 400.
#[test]
fn gatekeeper_rejects_consecutive_user_roles() {
    let mock = start_mock();
    let body = json!({
        "model": "claude-test",
        "max_tokens": 8,
        "messages": [
            {"role": "user", "content": "a"},
            {"role": "user", "content": "b"}
        ]
    })
    .to_string();
    let mut stream = TcpStream::connect(mock.url.strip_prefix("http://").unwrap()).unwrap();
    write!(
        stream,
        "POST /v1/messages HTTP/1.1\r\nhost: mock\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(response.contains("roles must alternate"), "{response}");
}
