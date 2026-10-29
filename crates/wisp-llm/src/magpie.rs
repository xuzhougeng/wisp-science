//! Read-only connection managed by Magpie, separate from stored profiles/keys.
use crate::ProviderConfig;
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAX_FILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Deserialize)]
struct Connection {
    #[serde(rename = "baseURL")]
    base_url: String,
    #[serde(rename = "apiKey")]
    api_key: String,
    model: String,
}

pub fn path_from_home(home: &Path) -> PathBuf {
    home.join(".config")
        .join("wisp-science")
        .join("magpie.json")
}

pub fn parse(bytes: &[u8]) -> Option<ProviderConfig> {
    if bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    let c: Connection = serde_json::from_slice(bytes).ok()?;
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
    let mut cfg = ProviderConfig::openai(c.base_url, c.api_key, c.model);
    cfg.user_agent = "wisp-science".into();
    // A local gateway must not accidentally use an ambient system proxy.
    if matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    ) {
        cfg.proxy = Some("none".into());
    }
    Some(cfg)
}

pub fn load_from_path(path: &Path) -> Option<ProviderConfig> {
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
pub fn startup_config() -> Option<ProviderConfig> {
    static CONFIG: OnceLock<Option<ProviderConfig>> = OnceLock::new();
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
        let cfg = parse(good).unwrap();
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

    #[test]
    fn magpie_file_switch_and_disconnect_are_read_only() {
        let dir = std::env::temp_dir().join(format!("wisp-magpie-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("magpie.json");
        assert!(load_from_path(&path).is_none());
        for model in ["fixture/first", "fixture/second"] {
            let text = serde_json::json!({"baseURL":"https://gateway.example/v1","apiKey":"test-key","model":model}).to_string();
            std::fs::write(&path, &text).unwrap();
            assert_eq!(load_from_path(&path).unwrap().model, model);
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
        let data = serde_json::json!({"baseURL":format!("http://{address}/v1"),"apiKey":"fixture-key","model":"fixture/model"}).to_string();
        let provider = crate::build(parse(data.as_bytes()).unwrap());
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
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap()["model"],
            "fixture/model"
        );
    }
}
