//! Minimal stdio JSON-RPC 2.0 MCP client.
//!
//! Launches any MCP server that speaks newline-delimited JSON over stdio
//! (the upstream `mcp-servers/bio-tools/run_server.py <pkg>` among them),
//! performs the `initialize` handshake, lists tools, and dispatches
//! `tools/call`. Each remote tool is exposed to the agent as a
//! [`wisp_tools::Tool`] via [`McpTool`].

use crate::process_tree::ProcessTree;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// Hard cap on a single stdio JSON-RPC exchange, matching the HTTP transport's
/// request timeout. Without it a hung server blocks the agent turn forever.
/// Exceeding this cap closes the stdio transport and terminates its process
/// tree because a late JSON-RPC response cannot be safely reused.
const STDIO_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const STDIO_SHUTDOWN_EOF_GRACE: std::time::Duration = std::time::Duration::from_millis(100);
const STDIO_SHUTDOWN_TERM_GRACE: std::time::Duration = std::time::Duration::from_millis(500);
const STDIO_SHUTDOWN_KILL_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
const STDIO_SHUTDOWN_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// Path to the vendored bio-tools MCP servers bundled with the app.
pub fn bundled_bio_tools_dir() -> Option<PathBuf> {
    wisp_paths::bio_tools_dir()
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(rename = "outputSchema", skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
}

impl RemoteTool {
    pub fn ui_resource_uri(&self) -> Option<&str> {
        self.meta
            .as_ref()
            .and_then(|meta| {
                meta.pointer("/ui/resourceUri")
                    .or_else(|| meta.get("ui/resourceUri"))
            })
            .and_then(Value::as_str)
    }

    /// The server's `readOnlyHint`: this tool only retrieves. Plan mode uses it
    /// to keep retrieval available while blocking everything else, so an absent
    /// or `false` hint has to stay blocked.
    pub fn read_only(&self) -> bool {
        self.annotations
            .as_ref()
            .and_then(|annotations| annotations.get("readOnlyHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// MCP Apps may publish app-only helper tools. Those remain discoverable
    /// on the server connection but must not enter the agent's tool catalog.
    pub fn visible_to_model(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|meta| meta.pointer("/ui/visibility"))
            .and_then(Value::as_array)
            .is_none_or(|visibility| visibility.iter().any(|item| item.as_str() == Some("model")))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpCallResult {
    pub content: Vec<Value>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

impl McpCallResult {
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }
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
        stdin: Arc<Mutex<Option<ChildStdin>>>,
        stdout: Arc<Mutex<BufReader<ChildStdout>>>,
        child: Mutex<Option<tokio::process::Child>>,
        process_tree: ProcessTree,
        closing: AtomicBool,
        terminated: AtomicBool,
        shutdown_lock: Mutex<()>,
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

struct CancellationCleanup<'a> {
    process_tree: &'a ProcessTree,
    child: &'a Mutex<Option<tokio::process::Child>>,
    closing: &'a AtomicBool,
    armed: bool,
}

impl CancellationCleanup<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancellationCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.closing.store(true, Ordering::SeqCst);
            if let Err(error) = self.process_tree.terminate_forcefully() {
                tracing::warn!(%error, "failed to terminate MCP process tree after cancellation");
            }
            // No await is legal in Drop. Taking the Child invokes the existing
            // kill_on_drop guard and hands Unix reaping to Tokio's orphan
            // queue. If explicit shutdown already owns this lock, that path is
            // responsible for the bounded wait/reap instead.
            if let Ok(mut child) = self.child.try_lock() {
                child.take();
            }
        }
    }
}

fn bio_tools_command(
    python: &std::path::Path,
    run_server: &std::path::Path,
    pkg: &str,
    envs: &[(String, String)],
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(run_server).arg(pkg);
    cmd.envs(envs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    cmd.env("PYTHONDONTWRITEBYTECODE", "1");
    cmd
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
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        wisp_tools::process::hide_console_async(&mut cmd);
        ProcessTree::configure(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| anyhow!("spawn MCP server: {e}"))?;
        let process_tree = ProcessTree::attach(&child).map_err(|error| {
            let _ = child.start_kill();
            anyhow!("attach MCP server process tree: {error}")
        })?;
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
                stdin: Arc::new(Mutex::new(Some(stdin))),
                stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
                child: Mutex::new(Some(child)),
                process_tree,
                closing: AtomicBool::new(false),
                terminated: AtomicBool::new(false),
                shutdown_lock: Mutex::new(()),
                next_id: AtomicU64::new(1),
            },
        };

        // initialize
        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "extensions": {
                    "io.modelcontextprotocol/ui": {
                        "mimeTypes": ["text/html;profile=mcp-app"]
                    }
                }
            },
            "clientInfo": { "name": "wisp-science", "version": env!("CARGO_PKG_VERSION") }
        });
        if let Err(e) = client.request("initialize", Some(init_params)).await {
            // Give the stderr drain a moment to capture a crash traceback.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let tail = stderr_tail.lock().await.clone();
            let tail = tail.trim();
            let _ = client.shutdown().await;
            if tail.is_empty() {
                return Err(e);
            }
            return Err(anyhow!(
                "{e}; stderr: {}",
                tail.chars().take(800).collect::<String>()
            ));
        }
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
            "capabilities": {
                "extensions": {
                    "io.modelcontextprotocol/ui": {
                        "mimeTypes": ["text/html;profile=mcp-app"]
                    }
                }
            },
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
                child,
                process_tree,
                closing,
                terminated,
                shutdown_lock,
                next_id,
            } => {
                if closing.load(Ordering::SeqCst) {
                    return Err(anyhow!("MCP stdio connection is shutting down"));
                }
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
                        let w = w
                            .as_mut()
                            .ok_or_else(|| anyhow!("MCP server stdin is closed"))?;
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
                let mut cancellation = CancellationCleanup {
                    process_tree,
                    child,
                    closing,
                    armed: true,
                };
                match tokio::time::timeout(STDIO_REQUEST_TIMEOUT, exchange).await {
                    Ok(Ok(result)) => {
                        cancellation.disarm();
                        Ok(result)
                    }
                    Ok(Err(error)) => {
                        cancellation.disarm();
                        Err(error)
                    }
                    Err(_) => {
                        let cleanup = shutdown_stdio(
                            stdin,
                            child,
                            process_tree,
                            closing,
                            terminated,
                            shutdown_lock,
                        )
                        .await;
                        cancellation.disarm();
                        let message = format!(
                            "MCP stdio request '{method}' timed out after {}s",
                            STDIO_REQUEST_TIMEOUT.as_secs()
                        );
                        match cleanup {
                            Ok(()) => Err(anyhow!(message)),
                            Err(error) => Err(anyhow!("{message}; shutdown failed: {error}")),
                        }
                    }
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
                let w = w
                    .as_mut()
                    .ok_or_else(|| anyhow!("MCP server stdin is closed"))?;
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
        Ok(self.tool_call_rich(name, arguments).await?.text_content())
    }

    /// `tools/call` preserving structured content, embedded resources, error
    /// state, and MCP Apps metadata for hosts that can render them.
    pub async fn tool_call_rich(&self, name: &str, arguments: &Value) -> Result<McpCallResult> {
        let params = json!({ "name": name, "arguments": arguments });
        let result = self.request("tools/call", Some(params)).await?;
        let content = result
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(McpCallResult {
            content,
            structured_content: result.get("structuredContent").cloned(),
            meta: result.get("_meta").cloned(),
            is_error: result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// Read one server resource, including MCP Apps `ui://` documents.
    pub async fn resource_read(&self, uri: &str) -> Result<Value> {
        self.request("resources/read", Some(json!({ "uri": uri })))
            .await
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
        let cmd = bio_tools_command(python, &run_server, pkg, envs);
        Self::launch_with_command(cmd).await
    }

    /// Stop this MCP connection and, for stdio transports, its complete child
    /// process tree. Safe to call repeatedly. Drop remains a forceful safety
    /// net for owners that cannot await this method.
    pub async fn shutdown(&self) -> Result<()> {
        match &self.transport {
            Transport::Stdio {
                stdin,
                child,
                process_tree,
                closing,
                terminated,
                shutdown_lock,
                ..
            } => {
                shutdown_stdio(
                    stdin,
                    child,
                    process_tree,
                    closing,
                    terminated,
                    shutdown_lock,
                )
                .await
            }
            Transport::Http(_) => Ok(()),
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Transport::Stdio {
            process_tree,
            closing,
            ..
        } = &self.transport
        {
            closing.store(true, Ordering::SeqCst);
            let _ = process_tree.terminate_forcefully();
        }
    }
}

async fn shutdown_stdio(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    child: &Mutex<Option<tokio::process::Child>>,
    process_tree: &ProcessTree,
    closing: &AtomicBool,
    terminated: &AtomicBool,
    shutdown_lock: &Mutex<()>,
) -> Result<()> {
    closing.store(true, Ordering::SeqCst);
    let _shutdown = match tokio::time::timeout(STDIO_SHUTDOWN_LOCK_WAIT, shutdown_lock.lock()).await
    {
        Ok(guard) => guard,
        Err(_) => {
            // A prior shutdown normally completes within 2.6s. Never let a
            // stalled caller make App exit unbounded; the tree-wide kill is
            // synchronous even though this path cannot own/reap `child`.
            process_tree.terminate_forcefully()?;
            return Err(anyhow!("timed out waiting for concurrent MCP shutdown"));
        }
    };
    if terminated.load(Ordering::SeqCst) {
        return Ok(());
    }

    let mut cancellation = CancellationCleanup {
        process_tree,
        child,
        closing,
        armed: true,
    };
    let result = async {
        // A cancelled request may still be unwinding while holding this mutex,
        // and a blocked write must never make App exit unbounded. Close stdin
        // only when immediately available; otherwise continue to the bounded
        // tree signals.
        if let Ok(mut stdin) = stdin.try_lock() {
            stdin.take();
        }
        let mut child = child.lock().await;

        #[cfg(unix)]
        {
            signal_unix_tree_before_reap(process_tree).await?;
        }

        #[cfg(not(unix))]
        {
            if wait_for_process_tree(&mut child, process_tree, STDIO_SHUTDOWN_EOF_GRACE).await? {
                process_tree.disarm();
                terminated.store(true, Ordering::SeqCst);
                return Ok(());
            }
            process_tree.terminate_gracefully()?;
            if wait_for_process_tree(&mut child, process_tree, STDIO_SHUTDOWN_TERM_GRACE).await? {
                process_tree.disarm();
                terminated.store(true, Ordering::SeqCst);
                return Ok(());
            }
            process_tree.terminate_forcefully()?;
        }

        if !wait_for_process_tree(&mut child, process_tree, STDIO_SHUTDOWN_KILL_WAIT).await? {
            // Keep the FORCE_SENT state: it prohibits any later numeric PGID
            // signal while still allowing a repeated shutdown to poll/reap the
            // actual tree instead of reporting a false success.
            return Err(anyhow!(
                "MCP server process tree did not exit after forceful shutdown"
            ));
        }
        process_tree.disarm();
        terminated.store(true, Ordering::SeqCst);
        Ok(())
    }
    .await;
    if result.is_ok() {
        cancellation.disarm();
    }
    result
}

#[cfg(unix)]
async fn signal_unix_tree_before_reap(process_tree: &ProcessTree) -> Result<()> {
    // This helper deliberately cannot access Child. Keeping the process-group
    // leader's PID occupied as a zombie, if it exits during either grace
    // period, makes numeric PGID reuse impossible before SIGKILL. Once the
    // force state is recorded, ProcessTree will never signal that PGID again
    // and the caller may safely reap/poll.
    tokio::time::sleep(STDIO_SHUTDOWN_EOF_GRACE).await;
    process_tree.terminate_gracefully()?;
    tokio::time::sleep(STDIO_SHUTDOWN_TERM_GRACE).await;
    process_tree.terminate_forcefully()?;
    Ok(())
}

async fn wait_for_process_tree(
    child: &mut Option<tokio::process::Child>,
    process_tree: &ProcessTree,
    timeout: std::time::Duration,
) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(child) = child.as_mut() {
            let _ = child.try_wait()?;
        }
        if !process_tree.is_running()? {
            child.take();
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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
            title: t.get("title").and_then(Value::as_str).map(str::to_string),
            description: t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            input_schema: t
                .get("inputSchema")
                .cloned()
                .unwrap_or(json!({"type": "object", "properties": {}})),
            output_schema: t.get("outputSchema").cloned(),
            meta: t.get("_meta").cloned(),
            annotations: t.get("annotations").cloned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bio_tools_command_forces_python_not_to_write_bytecode() {
        let envs = vec![("PYTHONDONTWRITEBYTECODE".to_string(), "0".to_string())];
        let cmd = bio_tools_command(
            std::path::Path::new("python"),
            std::path::Path::new("run_server.py"),
            "mcp_bio",
            &envs,
        );

        let value = cmd
            .as_std()
            .get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new("PYTHONDONTWRITEBYTECODE"))
            .and_then(|(_, value)| value);
        assert_eq!(value, Some(std::ffi::OsStr::new("1")));
    }

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

    #[test]
    fn tool_catalog_preserves_mcp_app_metadata() {
        let tools = tools_into_remote(vec![json!({
            "name": "motif_open_workbench",
            "title": "Open Motif for Claude Science",
            "description": "Open Motif",
            "inputSchema": { "type": "object" },
            "outputSchema": { "type": "object" },
            "annotations": { "readOnlyHint": true },
            "_meta": {
                "ui": { "resourceUri": "ui://motif/workbench.html" },
                "ui/resourceUri": "ui://motif/workbench.html"
            }
        })]);
        assert_eq!(tools.len(), 1);
        assert_eq!(
            tools[0].title.as_deref(),
            Some("Open Motif for Claude Science")
        );
        assert_eq!(
            tools[0].ui_resource_uri(),
            Some("ui://motif/workbench.html")
        );
        assert!(tools[0].output_schema.is_some());
        assert!(tools[0].annotations.is_some());
        assert!(tools[0].visible_to_model());
        // Plan mode's retrieval passthrough reads exactly this hint.
        assert!(tools[0].read_only());

        let app_only = tools_into_remote(vec![json!({
            "name": "motif_refresh",
            "inputSchema": { "type": "object" },
            "_meta": { "ui": { "visibility": ["app"] } }
        })]);
        assert!(!app_only[0].visible_to_model());
        assert!(
            !app_only[0].read_only(),
            "no hint means unclassified, not read-only"
        );
    }

    #[test]
    fn rich_result_text_excludes_embedded_html() {
        let result = McpCallResult {
            content: vec![
                json!({ "type": "text", "text": "Prepared workbench" }),
                json!({ "type": "resource", "resource": {
                    "uri": "motif://artifact/demo.html",
                    "mimeType": "text/html",
                    "text": "<html>large artifact</html>"
                } }),
            ],
            structured_content: Some(json!({ "filename": "demo.html" })),
            meta: None,
            is_error: false,
        };
        assert_eq!(result.text_content(), "Prepared workbench");
    }
}
