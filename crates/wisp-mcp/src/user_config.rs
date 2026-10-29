//! User-level MCP servers from `~/.wisp/mcp.json`, in the common `mcpServers`
//! layout other agents and agent managers write:
//! `{"mcpServers": {"<name>": {"type", "command", "args", "env", "cwd", "url", "headers"}}}`.
//!
//! Desktop and CLI read the file as-is on each load and never copy its values
//! into settings or the keyring. `${NAME}` and `${NAME:-default}` expand from
//! the process environment; a server referencing an unset variable is skipped,
//! so a placeholder is never sent as a token.

use crate::McpClient;
use anyhow::Result;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `~/.wisp/mcp.json`.
pub fn path(home: &Path) -> PathBuf {
    home.join(".wisp").join("mcp.json")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserServer {
    /// `user:<name>`: stable across edits, distinct from other connector IDs.
    pub id: String,
    pub name: String,
    pub transport: UserTransport,
}

/// Values are already expanded; never log them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UserTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
    },
    Http {
        url: String,
        headers: Vec<(String, String)>,
    },
}

/// Servers that can start, plus one message per entry (or file) that cannot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserMcpConfig {
    pub servers: Vec<UserServer>,
    pub errors: Vec<String>,
}

impl UserMcpConfig {
    /// Read `path` against the process environment. A missing file has no
    /// servers and no errors.
    pub fn load(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => return Self::failed(format!("{}: {error}", path.display())),
        };
        let mut config = Self::parse(&text, |name| std::env::var(name).ok());
        for error in &mut config.errors {
            *error = format!("{}: {error}", path.display());
        }
        config
    }

    /// Parse file contents; `lookup` resolves variable references.
    pub fn parse(text: &str, lookup: impl Fn(&str) -> Option<String>) -> Self {
        let root = match serde_json::from_str::<Value>(text) {
            Ok(root) => root,
            Err(error) => return Self::failed(format!("invalid JSON: {error}")),
        };
        let Some(root) = root.as_object() else {
            return Self::failed("expected a JSON object".into());
        };
        let entries = match root.get("mcpServers") {
            None | Some(Value::Null) => return Self::default(),
            Some(Value::Object(entries)) => entries,
            Some(_) => return Self::failed("`mcpServers` must be an object".into()),
        };
        let mut config = Self::default();
        for (name, entry) in entries {
            // Claude Desktop and Cline spell a switched-off server this way.
            if entry.get("disabled").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            match server(name, entry, &lookup) {
                Ok(server) => config.servers.push(server),
                Err(error) => config.errors.push(format!("MCP server '{name}': {error}")),
            }
        }
        config.servers.sort_by(|a, b| a.name.cmp(&b.name));
        config
    }

    fn failed(error: String) -> Self {
        Self {
            servers: Vec::new(),
            errors: vec![error],
        }
    }
}

impl UserServer {
    /// Launch or connect. `proxy` is an MCP proxy policy: empty inherits the
    /// environment, `none` forces direct, anything else is a proxy URL.
    pub async fn connect(&self, proxy: &str) -> Result<McpClient> {
        match &self.transport {
            UserTransport::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let mut cmd = tokio::process::Command::new(command);
                cmd.args(args).envs(env);
                if let Some(cwd) = cwd {
                    cmd.current_dir(cwd);
                }
                cmd.envs(wisp_tools::network::proxy_env(proxy));
                McpClient::launch_with_command(cmd).await
            }
            UserTransport::Http { url, headers } => {
                McpClient::connect_http_with_proxy(url, headers, proxy).await
            }
        }
    }
}

fn server(
    name: &str,
    entry: &Value,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<UserServer, String> {
    if name.trim().is_empty() || name.chars().any(char::is_control) {
        return Err("invalid server name".into());
    }
    let entry = entry.as_object().ok_or("expected an object")?;
    let resolve = |value: &str| expand(value, lookup);
    let transport = match string(entry, "type")?.as_deref() {
        None | Some("stdio") => {
            let command = resolve(&string(entry, "command")?.ok_or("missing `command`")?)?;
            if command.trim().is_empty() {
                return Err("`command` is empty".into());
            }
            let args = strings(entry, "args")?
                .iter()
                .map(|arg| resolve(arg))
                .collect::<Result<_, _>>()?;
            let mut env = BTreeMap::new();
            for (key, value) in string_map(entry, "env")? {
                if key.is_empty() || key.contains(['=', '\0']) {
                    return Err(format!("invalid environment variable name '{key}'"));
                }
                env.insert(key, resolve(&value)?);
            }
            let cwd = string(entry, "cwd")?
                .map(|cwd| resolve(&cwd))
                .transpose()?
                .filter(|cwd| !cwd.is_empty());
            UserTransport::Stdio {
                command,
                args,
                env,
                cwd,
            }
        }
        Some("http") => {
            let url = resolve(&string(entry, "url")?.ok_or("missing `url`")?)?;
            if !reqwest::Url::parse(&url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            {
                return Err("`url` must be an http(s) URL".into());
            }
            let mut headers = Vec::new();
            for (key, value) in string_map(entry, "headers")? {
                let value = resolve(&value)?;
                if reqwest::header::HeaderName::from_bytes(key.as_bytes()).is_err()
                    || reqwest::header::HeaderValue::from_str(&value).is_err()
                {
                    return Err(format!("invalid header '{key}'"));
                }
                headers.push((key, value));
            }
            UserTransport::Http { url, headers }
        }
        Some("sse") => {
            return Err(
                "legacy SSE transport is not supported; use \"type\": \"http\" (Streamable HTTP)"
                    .into(),
            )
        }
        Some(other) => return Err(format!("unsupported type '{other}'")),
    };
    Ok(UserServer {
        id: format!("user:{name}"),
        name: name.into(),
        transport,
    })
}

fn string(entry: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match entry.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn strings(entry: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    match entry.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("`{key}` must be an array of strings")),
        Some(_) => Err(format!("`{key}` must be an array of strings")),
    }
}

fn string_map(entry: &Map<String, Value>, key: &str) -> Result<Vec<(String, String)>, String> {
    match entry.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(items)) => items
            .iter()
            .map(|(name, value)| value.as_str().map(|value| (name.clone(), value.into())))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("`{key}` values must be strings")),
        Some(_) => Err(format!("`{key}` must be an object of strings")),
    }
}

/// Expand `${NAME}` and `${NAME:-default}` (the default applies only when
/// NAME is unset). Any other `${` is an error rather than literal text.
/// Messages name the variable, never a value.
fn expand(value: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or("unterminated `${` variable reference")?;
        let (name, default) = match after[..end].split_once(":-") {
            Some((name, default)) => (name, Some(default)),
            None => (&after[..end], None),
        };
        let valid = name.starts_with(|c: char| c == '_' || c.is_ascii_alphabetic())
            && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
        if !valid {
            return Err(format!("invalid variable name '{name}'"));
        }
        if default.is_some_and(|default| default.contains("${")) {
            return Err(format!("nested reference in the default for {name}"));
        }
        let value = lookup(name)
            .or_else(|| default.map(str::to_string))
            .ok_or_else(|| format!("environment variable {name} is not set"))?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env(name: &str) -> Option<String> {
        match name {
            "GITHUB_TOKEN" => Some("gh-secret".into()),
            "EMPTY" => Some(String::new()),
            _ => None,
        }
    }

    #[test]
    fn a_disabled_server_is_left_out_without_an_error() {
        let text = json!({"mcpServers": {
            "off": {"command": "npx", "env": {"TOKEN": "${UNSET_TOKEN}"}, "disabled": true},
            "on": {"command": "uvx", "disabled": false},
        }})
        .to_string();
        let config = UserMcpConfig::parse(&text, env);
        assert_eq!(config.errors, Vec::<String>::new());
        let ids: Vec<_> = config.servers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["user:on"]);
    }

    #[test]
    fn parses_stdio_and_http_servers_and_ignores_unknown_keys() {
        let text = json!({
            "otherTool": {"keep": true},
            "mcpServers": {
                "zeta": {"type": "http", "url": "https://mcp.example/v1",
                    "headers": {"Authorization": "Bearer ${GITHUB_TOKEN}"}, "note": "ignored"},
                "alpha": {"command": "npx", "args": ["-y", "@scope/server"],
                    "env": {"TOKEN": "${GITHUB_TOKEN}"}, "cwd": "/tmp", "disabled": false},
                "beta": {"type": "stdio", "command": "uvx"},
            }
        })
        .to_string();
        let config = UserMcpConfig::parse(&text, env);
        assert_eq!(config.errors, Vec::<String>::new());
        let ids: Vec<_> = config.servers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["user:alpha", "user:beta", "user:zeta"]);
        assert_eq!(
            config.servers[0].transport,
            UserTransport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "@scope/server".into()],
                env: BTreeMap::from([("TOKEN".into(), "gh-secret".into())]),
                cwd: Some("/tmp".into()),
            }
        );
        assert_eq!(
            config.servers[2].transport,
            UserTransport::Http {
                url: "https://mcp.example/v1".into(),
                headers: vec![("Authorization".into(), "Bearer gh-secret".into())],
            }
        );
    }

    #[test]
    fn expands_references_and_skips_servers_with_unset_variables() {
        let lookup = |name: &str| env(name);
        assert_eq!(expand("${GITHUB_TOKEN}", &lookup).unwrap(), "gh-secret");
        assert_eq!(
            expand("a-${MISSING:-fallback}-b", &lookup).unwrap(),
            "a-fallback-b"
        );
        assert_eq!(
            expand("${GITHUB_TOKEN:-unused}", &lookup).unwrap(),
            "gh-secret"
        );
        assert_eq!(expand("${EMPTY:-unused}", &lookup).unwrap(), "");
        assert_eq!(expand("${MISSING:-}", &lookup).unwrap(), "");
        assert_eq!(expand("$HOME and $", &lookup).unwrap(), "$HOME and $");
        for bad in ["${", "${OPEN", "${}", "${1X}", "${A-B}", "${A:-${B}}"] {
            let error = expand(bad, &lookup).unwrap_err();
            assert!(!error.contains("gh-secret"), "{error}");
        }

        let text = json!({"mcpServers": {
            "ok": {"command": "server", "args": ["--token=${GITHUB_TOKEN}"]},
            "arg": {"command": "server", "args": ["${API_KEY}"]},
            "env": {"command": "server", "env": {"KEY": "${API_KEY}"}},
            "cmd": {"command": "${SERVER_BIN}"},
            "url": {"type": "http", "url": "${MCP_URL}"},
            "header": {"type": "http", "url": "https://mcp.example",
                "headers": {"X-Api-Key": "${API_KEY}"}},
        }})
        .to_string();
        let config = UserMcpConfig::parse(&text, env);
        let ids: Vec<_> = config.servers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["user:ok"]);
        assert_eq!(config.errors.len(), 5, "{:?}", config.errors);
        for (name, variable) in [
            ("arg", "API_KEY"),
            ("env", "API_KEY"),
            ("cmd", "SERVER_BIN"),
            ("url", "MCP_URL"),
            ("header", "API_KEY"),
        ] {
            assert!(
                config.errors.contains(&format!(
                    "MCP server '{name}': environment variable {variable} is not set"
                )),
                "{:?}",
                config.errors
            );
        }
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("${"), "{rendered}");
    }

    #[test]
    fn unsupported_and_malformed_entries_are_reported_per_server() {
        let text = json!({"mcpServers": {
            "legacy": {"type": "sse", "url": "https://mcp.example/sse"},
            "future": {"type": "websocket", "url": "wss://mcp.example"},
            "no-command": {"args": []},
            "bad-args": {"command": "x", "args": "--flag"},
            "bad-env": {"command": "x", "env": {"A": 1}},
            "bad-url": {"type": "http", "url": "file:///etc/passwd"},
            "bad-header": {"type": "http", "url": "https://mcp.example", "headers": {"X": "a\r\nb"}},
            "not-object": "npx server",
            "fine": {"command": "x"},
        }})
        .to_string();
        let config = UserMcpConfig::parse(&text, env);
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.errors.len(), 8, "{:?}", config.errors);
        assert!(config
            .errors
            .iter()
            .any(|e| e.starts_with("MCP server 'legacy': legacy SSE transport is not supported")));
        assert!(config
            .errors
            .contains(&"MCP server 'future': unsupported type 'websocket'".to_string()));
    }

    #[test]
    fn invalid_json_is_one_error_and_a_missing_file_is_empty() {
        for text in ["{", "[]", r#"{"mcpServers": []}"#] {
            let config = UserMcpConfig::parse(text, env);
            assert!(config.servers.is_empty());
            assert_eq!(config.errors.len(), 1, "{text}: {:?}", config.errors);
        }
        for text in ["{}", r#"{"mcpServers": null}"#, r#"{"mcpServers": {}}"#] {
            assert_eq!(UserMcpConfig::parse(text, env), UserMcpConfig::default());
        }

        let home = std::env::temp_dir().join(format!("wisp-user-mcp-{}", std::process::id()));
        let file = path(&home);
        assert_eq!(UserMcpConfig::load(&file), UserMcpConfig::default());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{ not json").unwrap();
        let config = UserMcpConfig::load(&file);
        assert!(config.servers.is_empty());
        assert_eq!(config.errors.len(), 1);
        assert!(config.errors[0].starts_with(&file.display().to_string()));
        std::fs::remove_dir_all(home).ok();
    }

    #[tokio::test]
    async fn http_server_receives_expanded_headers() {
        use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
        type Seen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;
        async fn rpc(
            State(seen): State<Seen>,
            headers: HeaderMap,
            Json(rpc): Json<Value>,
        ) -> axum::response::Response {
            use axum::response::IntoResponse;
            if let Some(value) = headers.get("authorization") {
                seen.lock().unwrap().push(value.to_str().unwrap().into());
            }
            if rpc["id"].is_null() {
                return axum::http::StatusCode::ACCEPTED.into_response();
            }
            let result = match rpc["method"].as_str() {
                Some("tools/list") => json!({"tools": []}),
                _ => json!({"capabilities": {"tools": {}}}),
            };
            Json(json!({"jsonrpc": "2.0", "id": rpc["id"], "result": result})).into_response()
        }
        let seen = Seen::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new().route("/", post(rpc)).with_state(seen.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let text = json!({"mcpServers": {"remote": {"type": "http", "url": url,
            "headers": {"Authorization": "Bearer ${GITHUB_TOKEN}"}}}})
        .to_string();
        let config = UserMcpConfig::parse(&text, env);
        let client = config.servers[0].connect("none").await.unwrap();
        assert!(client.tools_list().await.unwrap().is_empty());
        let seen = seen.lock().unwrap().clone();
        assert!(!seen.is_empty());
        assert!(
            seen.iter().all(|value| value == "Bearer gh-secret"),
            "{seen:?}"
        );
        task.abort();
    }
}
