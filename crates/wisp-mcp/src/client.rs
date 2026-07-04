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

/// Path to the vendored bio-tools MCP servers bundled with the app.
pub fn bundled_bio_tools_dir() -> Option<PathBuf> {
    wisp_paths::bio_tools_dir()
}

#[derive(Debug, Clone)]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
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
    // Http variant added in Task A2
}

pub struct McpClient {
    transport: Transport,
}

impl McpClient {
    /// Spawn `command args...` and perform the MCP initialize handshake.
    pub async fn launch(command: &str, args: &[String]) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        wisp_tools::process::hide_console_async(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| anyhow!("spawn MCP server '{command}': {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        // The server is long-lived for the session; leak the child so dropping
        // the client doesn't kill it mid-call. (A graceful shutdown can be
        // added later via an explicit close.)
        std::mem::forget(child);

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
        let resp = client.request("initialize", Some(init_params)).await?;
        let _ = resp; // server capabilities acknowledged
        // initialized notification (no id, no response expected)
        client.notify("notifications/initialized", json!({})).await?;
        Ok(client)
    }

    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        match &self.transport {
            Transport::Stdio { stdin, stdout, next_id } => {
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let req = JsonRpcReq { jsonrpc: "2.0", id, method: method.to_string(), params };
                let val = serde_json::to_value(&req)?;
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
                    if n == 0 { return Err(anyhow!("MCP server closed stdout")); }
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    let resp: JsonRpcResp = serde_json::from_str(trimmed)?;
                    if resp.id == Some(id) {
                        if let Some(e) = resp.error { return Err(anyhow!("MCP error: {}", e.message)); }
                        return Ok(resp.result.unwrap_or(Value::Null));
                    }
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
        }
    }

    /// `tools/list` -> the server's tool catalog.
    pub async fn tools_list(&self) -> Result<Vec<RemoteTool>> {
        let result = self.request("tools/list", None).await?;
        let tools = result.get("tools").and_then(|t| t.as_array()).cloned().unwrap_or_default();
        Ok(toals_into_remote(tools))
    }

    /// `tools/call` -> concatenated text content blocks.
    pub async fn tool_call(&self, name: &str, arguments: &Value) -> Result<String> {
        let params = json!({ "name": name, "arguments": arguments });
        let result = self.request("tools/call", Some(params)).await?;
        let content = result.get("content").and_then(|c| c.as_array()).cloned().unwrap_or_default();
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
    /// must already have the bio-tools dependencies installed.
    pub async fn launch_bio_tools(python: &std::path::Path, pkg: &str) -> Result<Self> {
        let dir = bundled_bio_tools_dir().ok_or_else(|| anyhow!("bundled bio-tools dir not found"))?;
        let run_server = dir.join("run_server.py");
        let args = vec![run_server.to_string_lossy().to_string(), pkg.to_string()];
        Self::launch(&python.to_string_lossy(), &args).await
    }
}

fn toals_into_remote(tools: Vec<Value>) -> Vec<RemoteTool> {
    tools
        .into_iter()
        .map(|t| RemoteTool {
            name: t.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            description: t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            input_schema: t.get("inputSchema").cloned().unwrap_or(json!({"type": "object", "properties": {}})),
        })
        .collect()
}
