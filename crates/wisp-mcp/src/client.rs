//! Minimal stdio JSON-RPC 2.0 MCP client.
//!
//! Launches any MCP server that speaks newline-delimited JSON over stdio
//! (the upstream `mcp-servers/bio-tools/run_server.py <pkg>` among them),
//! performs the `initialize` handshake, lists tools, and dispatches
//! `tools/call`. Each remote tool is exposed to the agent as a
//! [`wisp_tools::Tool`] via [`McpTool`].

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// Hard cap on a single stdio JSON-RPC exchange, matching the HTTP transport's
/// request timeout. Without it a hung server blocks the agent turn forever.
const STDIO_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Path to the vendored bio-tools MCP servers bundled with the app.
pub fn bundled_bio_tools_dir() -> Option<PathBuf> {
    wisp_paths::bio_tools_dir()
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Serialize)]
struct JsonRpcReq {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcResp {
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcError {
    message: String,
}

enum Transport {
    Stdio {
        stdin: Arc<Mutex<ChildStdin>>,
        stdout: Arc<Mutex<BufReader<ChildStdout>>>,
        next_id: AtomicU64,
    },
    Http(HttpTransport),
}

struct HttpTransport {
    client: reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    session_id: tokio::sync::Mutex<Option<String>>,
    next_id: AtomicU64,
}

/// Pull the JSON-RPC response with `expected_id` out of a `text/event-stream`
/// body. Each SSE frame carries one JSON object on a `data:` line; we scan
/// every data line and return the first whose id matches.
fn parse_jsonrpc_from_sse(body: &str, expected_id: u64) -> Result<Value> {
    for line in body.lines() {
        let line = line.trim_start();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(resp) = serde_json::from_str::<JsonRpcResp>(data) else {
            continue;
        };
        if resp.id == Some(expected_id) {
            if let Some(e) = resp.error {
                return Err(anyhow!("MCP error: {}", e.message));
            }
            return Ok(resp.result.unwrap_or(Value::Null));
        }
    }
    Err(anyhow!(
        "no JSON-RPC response for id {expected_id} in SSE stream"
    ))
}

pub struct McpClient {
    transport: Transport,
}

impl McpClient {
    /// Spawn `command args...` and perform the MCP initialize handshake.
    pub async fn launch(command: &str, args: &[String]) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args);
        Self::launch_with_command(cmd).await
    }

    /// Spawn a caller-built `Command` (already carrying env/cwd/args) and
    /// perform the MCP initialize handshake. Lets callers configure the
    /// child process beyond what `launch(command, args)` exposes.
    pub async fn launch_with_command(mut cmd: tokio::process::Command) -> Result<Self> {
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        wisp_tools::process::hide_console_async(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| anyhow!("spawn MCP server: {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let stderr = child.stderr.take();
        // Drain stderr in the background so a chatty server cannot fill the
        // pipe; keep a short tail for initialize failures.
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        if let Some(err) = stderr {
            let tail = Arc::clone(&stderr_tail);
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut err = err;
                let mut buf = [0u8; 1024];
                loop {
                    match err.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]);
                            let mut t = tail.lock().await;
                            t.push_str(&chunk);
                            // Keep last ~2 KiB.
                            if t.len() > 2048 {
                                let drop_n = t.len() - 2048;
                                t.drain(..drop_n);
                            }
                        }
                    }
                }
            });
        }

        let client = Self {
            transport: Transport::Stdio {
                stdin: Arc::new(Mutex::new(stdin)),
                stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
                next_id: AtomicU64::new(1),
            },
        };

        // initialize
        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "wisp-science", "version": env!("CARGO_PKG_VERSION") }
        });
        if let Err(e) = client.request("initialize", Some(init_params)).await {
            // Give the stderr drain a moment to capture a crash traceback.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let tail = stderr_tail.lock().await.clone();
            let tail = tail.trim();
            let _ = child.kill().await;
            if tail.is_empty() {
                return Err(e);
            }
            return Err(anyhow!("{e}; stderr: {}", tail.chars().take(800).collect::<String>()));
        }
        // The server is long-lived for the session; leak the child so dropping
        // the client doesn't kill it mid-call. (A graceful shutdown can be
        // added later via an explicit close.)
        std::mem::forget(child);
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    /// Connect to an MCP server over Streamable HTTP: POST JSON-RPC to `url`,
    /// accepting either a plain JSON response or an SSE stream. `headers` are
    /// caller-supplied auth headers (e.g. `Authorization`) injected on every
    /// request.
    pub async fn connect_http(url: &str, headers: &[(String, String)]) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            // ponytail: 120s request ceiling so a connected-but-hung host eventually
            // errors instead of blocking a turn forever; raise if a legit HTTP MCP
            // tool call needs longer than this.
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let client = Self {
            transport: Transport::Http(HttpTransport {
                client: http,
                url: url.to_string(),
                headers: headers.to_vec(),
                session_id: tokio::sync::Mutex::new(None),
                next_id: AtomicU64::new(1),
            }),
        };
        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "wisp-science", "version": env!("CARGO_PKG_VERSION") }
        });
        let _ = client.request("initialize", Some(init_params)).await?;
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        match &self.transport {
            Transport::Stdio {
                stdin,
                stdout,
                next_id,
            } => {
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let req = JsonRpcReq {
                    jsonrpc: "2.0",
                    id,
                    method: method.to_string(),
                    params,
                };
                let val = serde_json::to_value(&req)?;
                let exchange = async {
                    // send
                    {
                        let mut w = stdin.lock().await;
                        w.write_all(val.to_string().as_bytes()).await?;
                        w.write_all(b"\n").await?;
                        w.flush().await?;
                    }
                    // read matching id
                    loop {
                        let mut line = String::new();
                        let mut r = stdout.lock().await;
                        let n = r.read_line(&mut line).await?;
                        drop(r);
                        if n == 0 {
                            return Err(anyhow!("MCP server closed stdout"));
                        }
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let resp: JsonRpcResp = serde_json::from_str(trimmed)?;
                        if resp.id == Some(id) {
                            if let Some(e) = resp.error {
                                return Err(anyhow!("MCP error: {}", e.message));
                            }
                            return Ok(resp.result.unwrap_or(Value::Null));
                        }
                    }
                };
                match tokio::time::timeout(STDIO_REQUEST_TIMEOUT, exchange).await {
                    Ok(res) => res,
                    Err(_) => Err(anyhow!(
                        "MCP stdio request '{method}' timed out after {}s",
                        STDIO_REQUEST_TIMEOUT.as_secs()
                    )),
                }
            }
            Transport::Http(h) => {
                let id = h.next_id.fetch_add(1, Ordering::SeqCst);
                let req = JsonRpcReq {
                    jsonrpc: "2.0",
                    id,
                    method: method.to_string(),
                    params,
                };
                let mut rb = h
                    .client
                    .post(&h.url)
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .json(&req);
                if let Some(sid) = h.session_id.lock().await.clone() {
                    rb = rb.header("mcp-session-id", sid);
                }
                for (k, v) in &h.headers {
                    rb = rb.header(k.as_str(), v.as_str());
                }
                let resp = rb
                    .send()
                    .await
                    .map_err(|e| anyhow!("http mcp request: {e}"))?;
                if let Some(sid) = resp
                    .headers()
                    .get("mcp-session-id")
                    .and_then(|v| v.to_str().ok())
                {
                    *h.session_id.lock().await = Some(sid.to_string());
                }
                let ctype = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let status = resp.status();
                let text = resp
                    .text()
                    .await
                    .map_err(|e| anyhow!("http mcp body: {e}"))?;
                if !status.is_success() {
                    return Err(anyhow!(
                        "http mcp {status}: {}",
                        text.chars().take(200).collect::<String>()
                    ));
                }
                if ctype.contains("text/event-stream") {
                    parse_jsonrpc_from_sse(&text, id)
                } else {
                    let resp: JsonRpcResp = serde_json::from_str(text.trim())?;
                    if let Some(e) = resp.error {
                        return Err(anyhow!("MCP error: {}", e.message));
                    }
                    Ok(resp.result.unwrap_or(Value::Null))
                }
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let val = json!({ "jsonrpc": "2.0", "method": method, "params": params });
                let mut w = stdin.lock().await;
                w.write_all(val.to_string().as_bytes()).await?;
                w.write_all(b"\n").await?;
                w.flush().await?;
                Ok(())
            }
            Transport::Http(h) => {
                let val = json!({ "jsonrpc": "2.0", "method": method, "params": params });
                let mut rb = h
                    .client
                    .post(&h.url)
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .json(&val);
                if let Some(sid) = h.session_id.lock().await.clone() {
                    rb = rb.header("mcp-session-id", sid);
                }
                for (k, v) in &h.headers {
                    rb = rb.header(k.as_str(), v.as_str());
                }
                let _ = rb
                    .send()
                    .await
                    .map_err(|e| anyhow!("http mcp notify: {e}"))?;
                Ok(())
            }
        }
    }

    /// `tools/list` -> the server's tool catalog.
    pub async fn tools_list(&self) -> Result<Vec<RemoteTool>> {
        let result = self.request("tools/list", None).await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools_into_remote(tools))
    }

    /// `tools/call` -> concatenated text content blocks.
    pub async fn tool_call(&self, name: &str, arguments: &Value) -> Result<String> {
        let params = json!({ "name": name, "arguments": arguments });
        let result = self.request("tools/call", Some(params)).await?;
        let content = result
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        let mut text = String::new();
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(s) = block.get("text").and_then(|t| t.as_str()) {
                    text.push_str(s);
                    text.push('\n');
                }
            }
        }
        Ok(text.trim().to_string())
    }

    /// Launch a bundled bio-tools server (`<bundled>/run_server.py <pkg>`)
    /// using `python` (typically a uv-provisioned venv interpreter). The venv
    /// must already have the bio-tools dependencies installed. `envs` are
    /// extra environment variables (e.g. service API keys) for the server.
    pub async fn launch_bio_tools(
        python: &std::path::Path,
        pkg: &str,
        envs: &[(String, String)],
    ) -> Result<Self> {
        let dir =
            bundled_bio_tools_dir().ok_or_else(|| anyhow!("bundled bio-tools dir not found"))?;
        let run_server = dir.join("run_server.py");
        let mut cmd = tokio::process::Command::new(python);
        cmd.arg(run_server).arg(pkg);
        cmd.envs(envs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        Self::launch_with_command(cmd).await
    }
}

fn tools_into_remote(tools: Vec<Value>) -> Vec<RemoteTool> {
    tools
        .into_iter()
        .map(|t| RemoteTool {
            name: t
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            description: t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            input_schema: t
                .get("inputSchema")
                .cloned()
                .unwrap_or(json!({"type": "object", "properties": {}})),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_body_yields_matching_jsonrpc_result() {
        // An MCP server may answer over text/event-stream. Frames are
        // `data: <json>` lines separated by blank lines. We want the result
        // whose id == expected_id, ignoring unrelated notifications.
        let body = "event: message\n\
                    data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\
                    \n\
                    event: message\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"tools\":[]}}\n\
                    \n";
        let got = parse_jsonrpc_from_sse(body, 7).unwrap();
        assert_eq!(got, serde_json::json!({ "tools": [] }));
    }

    #[test]
    fn sse_body_surfaces_jsonrpc_error() {
        let body = "data: {\"jsonrpc\":\"2.0\",\"id\":3,\"error\":{\"message\":\"boom\"}}\n\n";
        let err = parse_jsonrpc_from_sse(body, 3).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }
}
