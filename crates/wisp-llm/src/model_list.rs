//! `GET {base}/models` discovery for the model settings form.
//!
//! OpenAI-compatible gateways list their models there; some (Magpie, LiteLLM,
//! OpenRouter-style gateways) add per-model facts. Those facts are explicit
//! endpoint data for an exact model id, never a catalog guess.
use crate::provider::{http_client, ProviderConfig, ProviderKind};
use serde_json::Value;

/// One entry of an endpoint's model list. `None` is "not reported".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EndpointModel {
    pub id: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// `Some(vec![])` means the endpoint says the model takes no effort.
    pub reasoning_levels: Option<Vec<String>>,
}

/// Bound the reply: a model list is small, a misdirected HTML page is not.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_MODELS: usize = 2000;

/// Candidate list URLs, mirroring how the chat request path is built: a base
/// ending in `/v1` gets `/models`; otherwise `{base}/models` first, then
/// `{base}/v1/models` (Anthropic always lives under `/v1`).
pub fn list_urls(base_url: &str, kind: &ProviderKind) -> Vec<String> {
    let base = base_url.trim().trim_end_matches('/');
    let base = [
        "/chat/completions",
        "/responses",
        "/v1/messages",
        "/messages",
    ]
    .into_iter()
    .find_map(|suffix| base.strip_suffix(suffix))
    .unwrap_or(base)
    .trim_end_matches('/');
    if base.ends_with("/v1") {
        return vec![format!("{base}/models")];
    }
    match kind {
        ProviderKind::Anthropic => vec![format!("{base}/v1/models")],
        _ => vec![format!("{base}/models"), format!("{base}/v1/models")],
    }
}

/// Fetch and parse the endpoint's model list through the profile's proxy.
pub async fn fetch(cfg: &ProviderConfig) -> Result<Vec<EndpointModel>, String> {
    let client = http_client(cfg);
    let mut last_error = String::new();
    for url in list_urls(&cfg.base_url, &cfg.kind) {
        let mut request = cfg.request_headers(client.get(&url));
        let key = cfg.api_key.trim();
        if !key.is_empty() {
            request = match cfg.kind {
                ProviderKind::Anthropic => request
                    .header("x-api-key", key)
                    .header("anthropic-version", &cfg.anthropic_version),
                _ => request.bearer_auth(key),
            };
        }
        let response = request
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|error| {
                format!(
                    "Could not reach {url}: {}",
                    crate::provider::error_chain(&error)
                )
            })?;
        let status = response.status();
        if matches!(status.as_u16(), 404 | 405) {
            last_error = format!("{url} returned HTTP {status}");
            continue;
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("Could not read {url}: {error}"))?;
        if !status.is_success() {
            let snippet: String = String::from_utf8_lossy(&bytes).chars().take(300).collect();
            return Err(format!("{url} returned HTTP {status}: {snippet}"));
        }
        if bytes.len() > MAX_BODY_BYTES {
            return Err(format!("{url} returned an oversized model list."));
        }
        return parse(&bytes).ok_or_else(|| format!("{url} did not return a model list."));
    }
    Err(last_error)
}

/// Parse `{"data": [...]}` (OpenAI, Anthropic) or a bare array. Unknown or
/// out-of-range facts read as not reported.
pub fn parse(bytes: &[u8]) -> Option<Vec<EndpointModel>> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let items = match &value {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("data")
            .or_else(|| map.get("models"))
            .and_then(Value::as_array)?,
        _ => return None,
    };
    let mut models: Vec<EndpointModel> = Vec::new();
    for item in items.iter().take(MAX_MODELS) {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        else {
            continue;
        };
        if models.iter().any(|known| known.id == id) {
            continue;
        }
        let limit = |keys: &[&str], min: u64| {
            keys.iter()
                .find_map(|key| item.get(*key).and_then(Value::as_u64))
                .filter(|n| (min..=u32::MAX as u64).contains(n))
        };
        let reasoning_levels = ["reasoning_levels", "efforts"]
            .into_iter()
            .find_map(|key| item.get(key).and_then(Value::as_array))
            .map(|levels| {
                let mut kept: Vec<String> = Vec::new();
                for level in levels.iter().filter_map(Value::as_str).map(str::trim) {
                    if valid_level(level) && !kept.iter().any(|known| known == level) {
                        kept.push(level.to_string());
                    }
                }
                kept
            });
        models.push(EndpointModel {
            id: id.to_string(),
            context_window: limit(
                &["context_length", "context_window", "max_input_tokens"],
                1024,
            ),
            max_output_tokens: limit(&["max_output_tokens", "max_completion_tokens"], 16),
            reasoning_levels,
        });
    }
    Some(models)
}

fn valid_level(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_url_follows_the_chat_path_rule() {
        let openai = ProviderKind::OpenAiCompatible;
        assert_eq!(
            list_urls("http://127.0.0.1:3425/v1/", &openai),
            ["http://127.0.0.1:3425/v1/models"]
        );
        assert_eq!(
            list_urls("https://api.deepseek.com", &openai),
            [
                "https://api.deepseek.com/models",
                "https://api.deepseek.com/v1/models"
            ]
        );
        assert_eq!(
            list_urls("https://gw.example/v1/chat/completions", &openai),
            ["https://gw.example/v1/models"]
        );
        assert_eq!(
            list_urls("https://api.anthropic.com", &ProviderKind::Anthropic),
            ["https://api.anthropic.com/v1/models"]
        );
    }

    #[test]
    fn gateway_facts_are_read_per_exact_id() {
        let body = br#"{"object":"list","data":[
            {"id":"deepseek/deepseek-flash","context_length":1000000,"max_output_tokens":384000,
             "reasoning_levels":["none","low","high","max"]},
            {"id":"plain-model"},
            {"id":"other","max_input_tokens":200000,"reasoning_levels":[]},
            {"id":"plain-model","context_length":5},
            {"id":"bad-facts","context_length":-1,"max_output_tokens":"lots","reasoning_levels":["High","ok"]},
            {"object":"model"}
        ]}"#;
        let models = parse(body).unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            [
                "deepseek/deepseek-flash",
                "plain-model",
                "other",
                "bad-facts"
            ]
        );
        assert_eq!(
            models[0],
            EndpointModel {
                id: "deepseek/deepseek-flash".into(),
                context_window: Some(1_000_000),
                max_output_tokens: Some(384_000),
                reasoning_levels: Some(vec![
                    "none".into(),
                    "low".into(),
                    "high".into(),
                    "max".into()
                ]),
            }
        );
        assert_eq!(
            models[1],
            EndpointModel {
                id: "plain-model".into(),
                ..Default::default()
            }
        );
        assert_eq!(models[2].context_window, Some(200_000));
        assert_eq!(models[2].reasoning_levels, Some(vec![]));
        assert_eq!(models[3].context_window, None);
        assert_eq!(models[3].max_output_tokens, None);
        assert_eq!(models[3].reasoning_levels, Some(vec!["ok".into()]));
    }

    #[test]
    fn non_lists_are_rejected() {
        assert!(parse(b"<html>not found</html>").is_none());
        assert!(parse(br#"{"error":"nope"}"#).is_none());
        assert_eq!(parse(br#"[{"id":"a"}]"#).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fetch_reads_a_local_mock_gateway_with_bearer_auth() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = r#"{"data":[{"id":"m1","context_length":64000}]}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            request
        });
        let mut cfg = ProviderConfig::openai(format!("http://{addr}/v1"), "placeholder", "m1");
        cfg.proxy = Some("none".into());
        let models = fetch(&cfg).await.unwrap();
        assert_eq!(models[0].context_window, Some(64_000));
        let request = server.await.unwrap();
        assert!(request.starts_with("GET /v1/models "), "{request}");
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer placeholder"));
    }
}
