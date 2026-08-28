use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wisp_llm::openai::OpenAiProvider;
use wisp_llm::responses::OpenAiResponsesProvider;
use wisp_llm::{Completion, Message, Provider, ProviderConfig, StreamSink, Usage};

async fn capture_json_requests(
    responses: Vec<(&'static str, &'static str)>,
) -> (String, tokio::task::JoinHandle<Vec<(String, Value)>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut captured = Vec::with_capacity(responses.len());
        for (content_type, response_body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let (header_end, content_length) = loop {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "request ended before its headers");
                request.extend_from_slice(&chunk[..read]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let end = end + 4;
                    let head = String::from_utf8_lossy(&request[..end]);
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .expect("JSON request must carry content-length");
                    break (end, length);
                }
            };
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "request body ended early");
                request.extend_from_slice(&chunk[..read]);
            }
            let head = String::from_utf8_lossy(&request[..header_end]);
            let path = head
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap()
                .to_string();
            let body: Value =
                serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap();
            captured.push((path, body));

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        captured
    });
    (format!("http://{address}"), task)
}

#[derive(Default)]
struct Sink;

impl StreamSink for Sink {
    fn on_text(&mut self, _: &str) {}
    fn on_reasoning(&mut self, _: &str) {}
    fn on_tool_call(&mut self, _: usize, _: &str, _: &str) {}
    fn on_usage(&mut self, _: Usage) {}
}

fn chat_response() -> &'static str {
    r#"{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#
}

fn chat_stream_response() -> &'static str {
    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n\
     data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n\
     data: [DONE]\n\n"
}

fn responses_response() -> &'static str {
    r#"{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1}}"#
}

#[tokio::test]
async fn chat_completions_actual_http_body_sends_fast_for_stream_and_non_stream() {
    let (base_url, requests) = capture_json_requests(vec![
        ("application/json", chat_response()),
        ("text/event-stream", chat_stream_response()),
    ])
    .await;
    let mut cfg = ProviderConfig::openai(&base_url, "", "gpt-5.6-sol");
    cfg.proxy = Some("none".into());
    cfg.service_tier = Some("priority".into());
    let provider = OpenAiProvider::new(cfg);

    let _: Completion = provider
        .complete(&[Message::user("test")], &[])
        .await
        .unwrap();
    provider
        .stream(&[Message::user("test")], &[], &mut Sink)
        .await
        .unwrap();

    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|(path, body)| {
        path.ends_with("/chat/completions") && body["service_tier"] == "priority"
    }));
    assert_eq!(requests[0].1["stream"], false);
    assert_eq!(requests[1].1["stream"], true);
    println!("service_tier_http_capture provider=openai endpoint_kind=chat_completions stream=false service_tier=priority top_level=true");
    println!("service_tier_http_capture provider=openai endpoint_kind=chat_completions stream=true service_tier=priority top_level=true");
}

#[tokio::test]
async fn chat_completions_actual_http_body_omits_provider_default() {
    let (base_url, requests) = capture_json_requests(vec![
        ("application/json", chat_response()),
        ("text/event-stream", chat_stream_response()),
    ])
    .await;
    let mut cfg = ProviderConfig::openai(&base_url, "", "gpt-5.6-sol");
    cfg.proxy = Some("none".into());
    let provider = OpenAiProvider::new(cfg);
    provider
        .complete(&[Message::user("test")], &[])
        .await
        .unwrap();
    provider
        .stream(&[Message::user("test")], &[], &mut Sink)
        .await
        .unwrap();
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|(_, body)| body.get("service_tier").is_none()));
    println!("service_tier_http_capture provider=openai endpoint_kind=chat_completions stream=false service_tier=omitted top_level=false");
    println!("service_tier_http_capture provider=openai endpoint_kind=chat_completions stream=true service_tier=omitted top_level=false");
}

#[tokio::test]
async fn responses_actual_http_body_sends_fast_at_the_top_level() {
    let (base_url, requests) =
        capture_json_requests(vec![("application/json", responses_response())]).await;
    let mut cfg = ProviderConfig::openai_responses(&base_url, "", "gpt-5.6-sol");
    cfg.proxy = Some("none".into());
    cfg.service_tier = Some("priority".into());
    let provider = OpenAiResponsesProvider::new(cfg);
    provider
        .complete(&[Message::user("test")], &[])
        .await
        .unwrap();
    let requests = requests.await.unwrap();
    assert!(requests[0].0.ends_with("/v1/responses"));
    assert_eq!(requests[0].1["service_tier"], "priority");
    println!("service_tier_http_capture provider=openai_responses endpoint_kind=responses stream=false service_tier=priority top_level=true");
}

#[tokio::test]
async fn responses_actual_http_body_omits_provider_default() {
    let (base_url, requests) =
        capture_json_requests(vec![("application/json", responses_response())]).await;
    let mut cfg = ProviderConfig::openai_responses(&base_url, "", "gpt-5.6-sol");
    cfg.proxy = Some("none".into());
    let provider = OpenAiResponsesProvider::new(cfg);
    provider
        .complete(&[Message::user("test")], &[])
        .await
        .unwrap();
    let requests = requests.await.unwrap();
    assert!(requests[0].1.get("service_tier").is_none());
    println!("service_tier_http_capture provider=openai_responses endpoint_kind=responses stream=false service_tier=omitted top_level=false");
}
