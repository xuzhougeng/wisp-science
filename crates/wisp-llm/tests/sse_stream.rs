//! Stream parsing over real HTTP for all three provider families. One event
//! list per provider is rendered with LF and with CRLF line endings and cut
//! into network chunks at several offsets (including inside the event
//! separator); every combination must produce the same completion.

mod common;

use base64::Engine as _;
use common::{chunkings, render_sse, serve_sse, split_at, LineEnding, SseEvent};
use serde_json::json;
use wisp_llm::anthropic::AnthropicProvider;
use wisp_llm::openai::OpenAiProvider;
use wisp_llm::responses::OpenAiResponsesProvider;
use wisp_llm::{
    is_retriable, is_stream_interruption, Completion, LlmError, Message, Provider, ProviderConfig,
    StreamSink, Usage,
};

#[derive(Default)]
struct Recorder {
    text: String,
    reasoning: String,
}

impl StreamSink for Recorder {
    fn on_text(&mut self, delta: &str) {
        self.text.push_str(delta);
    }
    fn on_reasoning(&mut self, delta: &str) {
        self.reasoning.push_str(delta);
    }
    fn on_tool_call(&mut self, _: usize, _: &str, _: &str) {}
    fn on_usage(&mut self, _: Usage) {}
}

/// Stream `events` through `provider_for(base_url)` once per line ending and
/// chunking, handing each result to `check` with a label for failures.
async fn for_every_wire_shape<P: Provider>(
    events: &[SseEvent],
    provider_for: impl Fn(&str) -> P,
    check: impl Fn(wisp_llm::Result<Completion>, Recorder, &str),
) {
    for eol in LineEnding::ALL {
        let body = render_sse(events, eol);
        for (name, chunks) in chunkings(&body, eol) {
            let label = format!("{eol:?}, {name}");
            let (base_url, server) = serve_sse(chunks).await;
            let provider = provider_for(&base_url);
            let mut sink = Recorder::default();
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                provider.stream(&[Message::user("test")], &[], &mut sink),
            )
            .await
            .unwrap_or_else(|_| panic!("stream timed out ({label})"));
            server.await.unwrap();
            check(result, sink, &label);
        }
    }
}

fn openai_provider(base_url: &str) -> OpenAiProvider {
    let mut cfg = ProviderConfig::openai(base_url, "", "gpt-test");
    cfg.proxy = Some("none".into());
    OpenAiProvider::new(cfg)
}

fn anthropic_provider(base_url: &str) -> AnthropicProvider {
    let mut cfg = ProviderConfig::anthropic(base_url, "sk-test", "claude-test");
    cfg.proxy = Some("none".into());
    AnthropicProvider::new(cfg)
}

/// The Responses SSE parser only runs on the subscription (Codex) endpoint,
/// which needs a token carrying an account id. The token is synthetic.
fn codex_provider(base_url: &str) -> OpenAiResponsesProvider {
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"test-account"}}"#);
    let mut cfg =
        ProviderConfig::openai_codex(base_url, format!("e30.{claims}.synthetic"), "gpt-test");
    cfg.proxy = Some("none".into());
    OpenAiResponsesProvider::new(cfg)
}

fn chat_events() -> Vec<SseEvent> {
    vec![
        SseEvent::data(
            json!({"choices": [{"delta": {"reasoning_content": "想一想"}, "finish_reason": null}]}),
        ),
        SseEvent::data(json!({"choices": [{"delta": {"content": "你好"}, "finish_reason": null}]})),
        SseEvent::data(
            json!({"choices": [{"delta": {"content": ", world"}, "finish_reason": null}]}),
        ),
        SseEvent::data(json!({
            "choices": [{"delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 4}
        })),
        SseEvent::data("[DONE]"),
    ]
}

fn anthropic_events() -> Vec<SseEvent> {
    let delta = |index: u64, delta: serde_json::Value| {
        SseEvent::named(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": index, "delta": delta}),
        )
    };
    vec![
        SseEvent::named(
            "message_start",
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 3, "output_tokens": 1}}}),
        ),
        SseEvent::named(
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
        ),
        delta(0, json!({"type": "thinking_delta", "thinking": "先想"})),
        delta(
            0,
            json!({"type": "thinking_delta", "thinking": ", then answer"}),
        ),
        delta(0, json!({"type": "signature_delta", "signature": "sig"})),
        SseEvent::named(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        SseEvent::named(
            "content_block_start",
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
        ),
        delta(1, json!({"type": "text_delta", "text": "你好"})),
        delta(1, json!({"type": "text_delta", "text": ", world"})),
        SseEvent::named(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 1}),
        ),
        SseEvent::named(
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 4}}),
        ),
        SseEvent::named("message_stop", json!({"type": "message_stop"})),
    ]
}

fn codex_events() -> Vec<SseEvent> {
    vec![
        SseEvent::data(json!({"type": "response.reasoning_summary_text.delta", "delta": "想一想"})),
        SseEvent::data(json!({"type": "response.output_text.delta", "delta": "你好"})),
        SseEvent::data(json!({"type": "response.output_text.delta", "delta": ", world"})),
        SseEvent::data(json!({"type": "response.completed", "response": {
            "status": "completed", "output": [],
            "usage": {"input_tokens": 3, "output_tokens": 4}
        }})),
    ]
}

#[tokio::test]
async fn chat_completions_stream_parses_lf_and_crlf_at_any_chunking() {
    for_every_wire_shape(&chat_events(), openai_provider, |result, sink, label| {
        let completion = result.unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(completion.content, "你好, world", "{label}");
        assert_eq!(completion.reasoning.as_deref(), Some("想一想"), "{label}");
        assert_eq!(completion.finish_reason.as_deref(), Some("stop"), "{label}");
        assert_eq!(completion.usage.output_tokens, 4, "{label}");
        assert_eq!(sink.text, "你好, world", "{label}");
    })
    .await;
}

#[tokio::test]
async fn anthropic_stream_parses_lf_and_crlf_at_any_chunking() {
    for_every_wire_shape(
        &anthropic_events(),
        anthropic_provider,
        |result, sink, label| {
            let completion = result.unwrap_or_else(|error| panic!("{label}: {error}"));
            assert_eq!(completion.content, "你好, world", "{label}");
            assert_eq!(completion.finish_reason.as_deref(), Some("stop"), "{label}");
            assert_eq!(completion.usage.output_tokens, 4, "{label}");
            assert_eq!(sink.text, "你好, world", "{label}");
        },
    )
    .await;
}

/// Streaming must return the same reasoning the non-streaming path does, not
/// only forward it to the sink.
#[tokio::test]
async fn anthropic_stream_returns_accumulated_thinking() {
    for_every_wire_shape(
        &anthropic_events(),
        anthropic_provider,
        |result, sink, label| {
            let completion = result.unwrap_or_else(|error| panic!("{label}: {error}"));
            assert_eq!(
                completion.reasoning.as_deref(),
                Some("先想, then answer"),
                "{label}"
            );
            assert_eq!(sink.reasoning, "先想, then answer", "{label}");
        },
    )
    .await;
}

#[tokio::test]
async fn anthropic_stream_without_thinking_returns_no_reasoning() {
    let events: Vec<SseEvent> = anthropic_events().into_iter().skip(6).collect();
    let body = render_sse(&events, LineEnding::Lf);
    let (base_url, server) = serve_sse(split_at(&body, &[])).await;
    let completion = anthropic_provider(&base_url)
        .stream(&[Message::user("test")], &[], &mut Recorder::default())
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(completion.content, "你好, world");
    assert_eq!(completion.reasoning, None);
}

#[tokio::test]
async fn responses_stream_parses_lf_and_crlf_at_any_chunking() {
    for_every_wire_shape(&codex_events(), codex_provider, |result, sink, label| {
        let completion = result.unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(completion.content, "你好, world", "{label}");
        assert_eq!(completion.reasoning.as_deref(), Some("想一想"), "{label}");
        assert_eq!(completion.usage.output_tokens, 4, "{label}");
        assert_eq!(sink.text, "你好, world", "{label}");
    })
    .await;
}

/// A stream that fails after some text: the events before `event: error`.
fn anthropic_events_then_error(kind: &str, message: &str) -> Vec<SseEvent> {
    let mut events: Vec<SseEvent> = anthropic_events().into_iter().take(9).collect();
    events.push(SseEvent::named(
        "error",
        json!({"type": "error", "error": {"type": kind, "message": message}}),
    ));
    events
}

#[tokio::test]
async fn anthropic_stream_error_event_reports_the_provider_message() {
    let events = anthropic_events_then_error("overloaded_error", "Overloaded");
    for_every_wire_shape(&events, anthropic_provider, |result, _, label| {
        let error = result.expect_err(label);
        let LlmError::Api { status, body } = &error else {
            panic!("{label}: expected an API error, got {error:?}");
        };
        assert_eq!(*status, 529, "{label}");
        assert_eq!(body, "overloaded_error: Overloaded", "{label}");
        assert!(error.to_string().contains("Overloaded"), "{label}");
        // Retried as a request; not reconnected as if the stream were cut.
        assert!(is_retriable(&error), "{label}");
        assert!(!is_stream_interruption(&error), "{label}");
    })
    .await;
}

#[tokio::test]
async fn anthropic_stream_permanent_error_is_not_retried() {
    let events = anthropic_events_then_error(
        "invalid_request_error",
        "messages.3: tool_use ids were found without tool_result blocks",
    );
    let body = render_sse(&events, LineEnding::CrLf);
    let (base_url, server) = serve_sse(split_at(&body, &[])).await;
    let error = anthropic_provider(&base_url)
        .stream(&[Message::user("test")], &[], &mut Recorder::default())
        .await
        .unwrap_err();
    server.await.unwrap();
    assert!(
        matches!(&error, LlmError::Api { status: 400, body } if body.contains("tool_use ids were found")),
        "{error:?}"
    );
    assert!(!is_retriable(&error));
    assert!(!is_stream_interruption(&error));
}
