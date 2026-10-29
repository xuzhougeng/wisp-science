//! Read-only connection managed by Magpie, separate from stored profiles/keys.
use crate::ProviderConfig;
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAX_FILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Deserialize)]
struct File {
    #[serde(rename = "baseURL")]
    base_url: String,
    #[serde(rename = "apiKey")]
    api_key: String,
    model: String,
    // Untyped so a malformed snapshot is ignored instead of disabling the file.
    #[serde(rename = "modelInfo", default)]
    model_info: Option<serde_json::Value>,
}

/// Facts Magpie published for the connected model. `None`/empty is unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelInfo {
    pub context: Option<u64>,
    pub output: Option<u64>,
    pub efforts: Vec<String>,
}

#[derive(Clone)]
pub struct Connection {
    /// Carries the known output limit and default reasoning effort.
    pub config: ProviderConfig,
    pub info: ModelInfo,
}

impl ModelInfo {
    fn from_snapshot(value: Option<&serde_json::Value>, model: &str) -> Self {
        // A snapshot for another ID is stale after a manual model edit.
        let Some(info) = value
            .and_then(serde_json::Value::as_object)
            .filter(|info| info.get("id").and_then(serde_json::Value::as_str) == Some(model))
        else {
            return Self::default();
        };
        let limit = |key: &str| {
            info.get(key)
                .and_then(serde_json::Value::as_u64)
                .filter(|n| (1..=u32::MAX as u64).contains(n))
        };
        let mut efforts: Vec<String> = Vec::new();
        for effort in info
            .get("efforts")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
        {
            if valid_effort(effort) && !efforts.iter().any(|known| known == effort) {
                efforts.push(effort.into());
            }
        }
        Self {
            context: limit("context"),
            output: limit("output").filter(|n| *n >= 16),
            efforts,
        }
    }
}

// Same names the plugin accepts: ^[a-z][a-z0-9_-]{0,31}$
fn valid_effort(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

/// Default effort for a known level list: medium, then high, then the first.
pub fn default_effort(efforts: &[String]) -> Option<&str> {
    ["medium", "high"]
        .into_iter()
        .find(|level| efforts.iter().any(|effort| effort == level))
        .or_else(|| efforts.first().map(String::as_str))
}

/// The requested effort when the model supports it, otherwise the default.
/// Without a known list the provider default applies: Magpie reports no
/// levels for models that do not accept a reasoning effort.
pub fn resolve_effort(efforts: &[String], requested: &str) -> Option<String> {
    let requested = requested.trim();
    efforts
        .iter()
        .find(|effort| *effort == requested)
        .map(String::as_str)
        .or_else(|| default_effort(efforts))
        .map(str::to_string)
}

pub fn path_from_home(home: &Path) -> PathBuf {
    home.join(".config")
        .join("wisp-science")
        .join("magpie.json")
}

pub fn parse(bytes: &[u8]) -> Option<Connection> {
    if bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    let c: File = serde_json::from_slice(bytes).ok()?;
    for s in [&c.base_url, &c.api_key, &c.model] {
        if s.is_empty() || s.len() > 8192 || s.trim() != s || s.chars().any(|c| c.is_control()) {
            return None;
        }
    }
    let url = url::Url::parse(&c.base_url).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let info = ModelInfo::from_snapshot(c.model_info.as_ref(), &c.model);
    let mut cfg = ProviderConfig::openai(c.base_url, c.api_key, c.model);
    cfg.user_agent = "wisp-science".into();
    if let Some(output) = info.output {
        cfg.max_tokens = output;
    }
    cfg.reasoning_effort = default_effort(&info.efforts).map(str::to_string);
    // A local gateway must not accidentally use an ambient system proxy.
    if matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    ) {
        cfg.proxy = Some("none".into());
    }
    Some(Connection { config: cfg, info })
}

pub fn load_from_path(path: &Path) -> Option<Connection> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    parse(&bytes)
}

/// Stable for the process lifetime. Restart after connect/switch/disconnect;
/// this also keeps live agents, tools and context limits on the same model.
pub fn startup_connection() -> Option<Connection> {
    static CONFIG: OnceLock<Option<Connection>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            let home = std::env::var_os(key).filter(|s| !s.is_empty())?;
            load_from_path(&path_from_home(Path::new(&home)))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magpie_validates_contract_and_keeps_model_id() {
        let good = br#"{"baseURL":"http://127.0.0.1:3425/v1","apiKey":"fixture-key","model":"provider/model","future":true}"#;
        let cfg = parse(good).unwrap().config;
        assert_eq!(cfg.base_url, "http://127.0.0.1:3425/v1");
        assert_eq!(cfg.model, "provider/model");
        assert_eq!(cfg.proxy.as_deref(), Some("none"));
        for bad in ["{}", "null", "[]", "{", r#"{"baseURL":false}"#] {
            assert!(parse(bad.as_bytes()).is_none());
        }
        let text = String::from_utf8_lossy(good);
        for (a, b) in [
            ("fixture-key", ""),
            ("http://127.0.0.1:3425/v1", "file:///tmp/x"),
            ("fixture-key", r"bad\r\nheader"),
            (
                "http://127.0.0.1:3425/v1",
                "https://user:pass@example.com/v1",
            ),
        ] {
            assert!(parse(text.replace(a, b).as_bytes()).is_none());
        }
        assert!(parse(&vec![b' '; MAX_FILE_BYTES + 1]).is_none());
    }

    fn with_info(model_info: serde_json::Value) -> Connection {
        let text = serde_json::json!({"baseURL":"https://gateway.example/v1","apiKey":"k","model":"p/reasoner","modelInfo":model_info});
        parse(text.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn magpie_model_info_for_the_selected_model_sets_limits_and_default_effort() {
        let connection = with_info(serde_json::json!({
            "id": "p/reasoner", "context": 400000, "output": 128000, "efforts": ["low", "medium", "high"],
        }));
        assert_eq!(
            connection.info,
            ModelInfo {
                context: Some(400000),
                output: Some(128000),
                efforts: vec!["low".into(), "medium".into(), "high".into()],
            }
        );
        assert_eq!(connection.config.max_tokens, 128000);
        assert_eq!(
            connection.config.reasoning_effort.as_deref(),
            Some("medium")
        );
        let connection =
            with_info(serde_json::json!({"id": "p/reasoner", "efforts": ["low", "high"]}));
        assert_eq!(connection.config.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(connection.config.max_tokens, 8192);
        assert_eq!(connection.info.context, None);
    }

    #[test]
    fn magpie_model_info_for_another_model_or_malformed_is_ignored() {
        let stale = with_info(serde_json::json!({
            "id": "p/previous", "context": 9000, "output": 1000, "efforts": ["high"],
        }));
        assert_eq!(stale.info, ModelInfo::default());
        assert_eq!(stale.config.max_tokens, 8192);
        assert_eq!(stale.config.reasoning_effort, None);
        for bad in [
            serde_json::json!(null),
            serde_json::json!("p/reasoner"),
            serde_json::json!(["p/reasoner"]),
            serde_json::json!({"id": 7}),
            serde_json::json!({"id": "p/reasoner", "context": -1, "output": 8192.5, "efforts": "high"}),
            serde_json::json!({"id": "p/reasoner", "context": 4294967296u64, "output": 8, "efforts": [false, "High", "", "x y"]}),
        ] {
            let connection = with_info(bad);
            assert_eq!(connection.info, ModelInfo::default());
            assert_eq!(connection.config.model, "p/reasoner");
            assert_eq!(connection.config.max_tokens, 8192);
            assert_eq!(connection.config.reasoning_effort, None);
        }
        // The connection contract itself is unchanged by a valid snapshot.
        let text = serde_json::json!({"baseURL":"file:///tmp","apiKey":"k","model":"p/reasoner","modelInfo":{"id":"p/reasoner","context":1}});
        assert!(parse(text.to_string().as_bytes()).is_none());
    }

    #[test]
    fn magpie_effort_follows_the_request_only_when_supported() {
        let levels = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let known = levels(&["low", "medium", "high", "xhigh"]);
        assert_eq!(resolve_effort(&known, "xhigh").as_deref(), Some("xhigh"));
        assert_eq!(resolve_effort(&known, " low ").as_deref(), Some("low"));
        assert_eq!(resolve_effort(&known, "").as_deref(), Some("medium"));
        assert_eq!(resolve_effort(&known, "max").as_deref(), Some("medium"));
        assert_eq!(default_effort(&levels(&["low", "high"])), Some("high"));
        assert_eq!(default_effort(&levels(&["max", "minimal"])), Some("max"));
        assert_eq!(resolve_effort(&[], "high"), None);
        assert_eq!(resolve_effort(&[], ""), None);
    }

    #[test]
    fn magpie_file_switch_and_disconnect_are_read_only() {
        let dir = std::env::temp_dir().join(format!("wisp-magpie-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("magpie.json");
        assert!(load_from_path(&path).is_none());
        for model in ["fixture/first", "fixture/second"] {
            let text = serde_json::json!({"baseURL":"https://gateway.example/v1","apiKey":"test-key","model":model}).to_string();
            std::fs::write(&path, &text).unwrap();
            assert_eq!(load_from_path(&path).unwrap().config.model, model);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        std::fs::write(&path, "{}").unwrap();
        assert!(load_from_path(&path).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn magpie_provider_sends_configured_model_key_and_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let capture = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut part = [0; 4096];
                let n = stream.read(&mut part).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&part[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"fixture-ok"},"finish_reason":"stop"}]}"#;
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let data = serde_json::json!({"baseURL":format!("http://{address}/v1"),"apiKey":"fixture-key","model":"fixture/model",
            "modelInfo":{"id":"fixture/model","output":32000,"efforts":["low","high"]}}).to_string();
        let provider = crate::build(parse(data.as_bytes()).unwrap().config);
        let result = provider
            .complete(&[crate::Message::user("hello")], &[])
            .await
            .unwrap();
        assert_eq!(result.content, "fixture-ok");
        let wire = capture.await.unwrap();
        assert!(wire.starts_with("POST /v1/chat/completions "));
        assert!(wire
            .to_lowercase()
            .contains("authorization: bearer fixture-key"));
        let (_, body) = wire.split_once("\r\n\r\n").unwrap();
        let body = serde_json::from_str::<serde_json::Value>(body).unwrap();
        assert_eq!(body["model"], "fixture/model");
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["max_tokens"], 32000);
    }
}
