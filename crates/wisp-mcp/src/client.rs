//! Minimal stdio JSON-RPC 2.0 MCP client.
//!
//! Launches any MCP server that speaks newline-delimited JSON over stdio
//! configured by the user,
//! performs the `initialize` handshake, lists tools, and dispatches
//! `tools/call`. Each remote tool is exposed to the agent as a
//! [`wisp_tools::Tool`] via [`McpTool`].

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::Instrument;
use wisp_tools::process::ProcessTree;

/// Technical exchanges are bounded; tools/call deliberately has no deadline.
const CONTROL_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const STDIO_SHUTDOWN_EOF_GRACE: std::time::Duration = std::time::Duration::from_millis(100);
const STDIO_SHUTDOWN_TERM_GRACE: std::time::Duration = std::time::Duration::from_millis(500);
const STDIO_SHUTDOWN_KILL_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
const STDIO_SHUTDOWN_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

    /// Whether a legitimate MCP App instance of this tool may call it. The
    /// spec defaults unset `_meta.ui.visibility` to `["model", "app"]`, so
    /// only an explicit visibility that omits `"app"` hides it from apps.
    pub fn visible_to_app(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|meta| meta.pointer("/ui/visibility"))
            .and_then(Value::as_array)
            .is_none_or(|visibility| visibility.iter().any(|item| item.as_str() == Some("app")))
    }

    /// Human title for UI and audit: explicit `title`, annotated title, then
    /// the tool name.
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .or_else(|| {
                self.annotations
                    .as_ref()
                    .and_then(|annotations| annotations.get("title"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

type StdioWaiters =
    Arc<StdMutex<HashMap<u64, oneshot::Sender<Result<JsonRpcResp, anyhow::Error>>>>>;

enum Transport {
    Stdio {
        stdin: Arc<Mutex<Option<ChildStdin>>>,
        writer: mpsc::Sender<StdioWrite>,
        waiters: StdioWaiters,
        child: Mutex<Option<tokio::process::Child>>,
        process_tree: ProcessTree,
        closing: Arc<AtomicBool>,
        terminated: AtomicBool,
        shutdown_lock: Mutex<()>,
        next_id: AtomicU64,
    },
    Http(HttpTransport),
    Managed(crate::connection::ManagedConnection),
}

struct HttpTransport {
    client: reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    session_id: tokio::sync::Mutex<Option<String>>,
    next_id: AtomicU64,
    closing: Arc<AtomicBool>,
}

struct StdioWrite {
    frame: Vec<u8>,
    request: Option<(u64, Arc<AtomicBool>)>,
    done: oneshot::Sender<Result<()>>,
}

impl HttpTransport {
    fn post(&self) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(&self.url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }
        request
    }
}

#[derive(Debug)]
struct RemoteRpcError(String);
impl std::fmt::Display for RemoteRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MCP error: {}", self.0)
    }
}
impl std::error::Error for RemoteRpcError {}

fn rpc_result(response: JsonRpcResp, expected: u64) -> Result<Value> {
    if response.id != Some(expected) {
        return Err(anyhow!("MCP response id mismatch"));
    }
    if let Some(error) = response.error {
        return Err(RemoteRpcError(error.message).into());
    }
    Ok(response.result.unwrap_or(Value::Null))
}

/// Incremental SSE framing, including CRLF, multiline data and split UTF-8.
/// Limits apply per pending event, not to the lifetime of a long-running stream.
#[derive(Default)]
struct SseDecoder {
    line: Vec<u8>,
    data: Vec<u8>,
}
impl SseDecoder {
    fn feed(&mut self, bytes: &[u8], id: u64) -> Result<Option<Value>> {
        for byte in bytes {
            if *byte != b'\n' {
                self.line.push(*byte);
                if self.line.len() + self.data.len() > MAX_RESPONSE_BYTES {
                    return Err(anyhow!("MCP SSE event too large"));
                }
                continue;
            }
            if self.line.last() == Some(&b'\r') {
                self.line.pop();
            }
            if self.line.is_empty() {
                let data = std::mem::take(&mut self.data);
                if let Ok(response) = serde_json::from_slice::<JsonRpcResp>(&data) {
                    if response.id == Some(id) {
                        return rpc_result(response, id).map(Some);
                    }
                }
            } else if let Some(data) = self.line.strip_prefix(b"data:") {
                let data = data.strip_prefix(b" ").unwrap_or(data);
                self.data.extend_from_slice(data);
                self.data.push(b'\n');
            }
            self.line.clear();
        }
        Ok(None)
    }
}

fn spawn_stdio_writer(
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    waiters: StdioWaiters,
    closing: Arc<AtomicBool>,
) -> mpsc::Sender<StdioWrite> {
    let (tx, mut rx) = mpsc::channel::<StdioWrite>(64);
    tokio::spawn(
        async move {
            while let Some(write) = rx.recv().await {
                // Once writing starts, always complete this frame, even if its caller drops.
                if closing.load(Ordering::SeqCst) {
                    break;
                }
                let result = async {
                    let mut stdin = stdin.lock().await;
                    let stdin = stdin.as_mut().ok_or_else(|| anyhow!("MCP stdin closed"))?;
                    if let Some((id, sent)) = &write.request {
                        if !waiters.lock().unwrap().contains_key(id) {
                            return Ok(());
                        }
                        sent.store(true, Ordering::SeqCst);
                    }
                    stdin.write_all(&write.frame).await?;
                    stdin.flush().await?;
                    Ok(())
                }
                .await;
                let failed = result.is_err();
                let _ = write.done.send(result);
                if failed {
                    closing.store(true, Ordering::SeqCst);
                    tracing::warn!(target: "wisp", "mcp.connection.disconnected: write failure");
                    fail_stdio_waiters(
                        &waiters,
                        "MCP write failed; operation outcome unknown, do not replay",
                    );
                    break;
                }
            }
        }
        .instrument(tracing::Span::current()),
    );
    tx
}

/// Pull the JSON-RPC response with `expected_id` out of a `text/event-stream`
/// body. Each SSE frame carries one JSON object on a `data:` line; we scan
/// every data line and return the first whose id matches.
#[cfg(test)]
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

/// The writer owns frames independently of request futures.
struct StdioWaiterGuard {
    waiters: StdioWaiters,
    writer: mpsc::Sender<StdioWrite>,
    id: u64,
    cancel: bool,
    sent: Arc<AtomicBool>,
}
impl Drop for StdioWaiterGuard {
    fn drop(&mut self) {
        self.waiters.lock().unwrap().remove(&self.id);
        if self.cancel && self.sent.load(Ordering::SeqCst) {
            tracing::info!(target: "wisp", request_id = self.id, "mcp.request.cancelled; external outcome may be unknown");
            let (done, _) = oneshot::channel();
            let frame = format!("{}\n", json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":self.id,"reason":"Caller stopped waiting; no rollback implied"}})).into_bytes();
            let _ = self.writer.try_send(StdioWrite {
                frame,
                request: None,
                done,
            });
        }
    }
}
struct HttpCancellation {
    request: Option<reqwest::RequestBuilder>,
}
impl Drop for HttpCancellation {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = request
                        .timeout(std::time::Duration::from_secs(2))
                        .send()
                        .await;
                });
            }
        }
    }
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

impl McpClient {
    /// Local transport liveness only; never issue a network probe or a tool
    /// call just to advertise the App capability. HTTP remains best-effort.
    pub fn is_connected(&self) -> bool {
        match &self.transport {
            Transport::Stdio {
                closing,
                terminated,
                child,
                ..
            } => {
                !closing.load(Ordering::SeqCst)
                    && !terminated.load(Ordering::SeqCst)
                    && child.try_lock().map_or(true, |mut child| {
                        child
                            .as_mut()
                            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
                    })
            }
            Transport::Http(h) => !h.closing.load(Ordering::SeqCst),
            Transport::Managed(m) => m.is_connected(),
        }
    }
    pub(crate) fn is_stdio(&self) -> bool {
        matches!(self.transport, Transport::Stdio { .. })
    }
    /// Give back the local process of a managed stdio connection that has been
    /// idle for `idle_for`; the next request relaunches it. Other transports
    /// hold no local process and are left alone.
    pub async fn suspend_if_idle(&self, idle_for: std::time::Duration) -> bool {
        match &self.transport {
            Transport::Managed(m) => m.suspend_if_idle(idle_for).instrument(m.span()).await,
            _ => false,
        }
    }
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
        let waiters: StdioWaiters = Arc::new(StdMutex::new(HashMap::new()));
        let closing = Arc::new(AtomicBool::new(false));
        spawn_stdio_reader(stdout, Arc::clone(&waiters), closing.clone());
        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let writer = spawn_stdio_writer(stdin.clone(), waiters.clone(), closing.clone());
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
                                let mut drop_n = t.len() - 2048;
                                while !t.is_char_boundary(drop_n) {
                                    drop_n += 1;
                                }
                                t.drain(..drop_n);
                            }
                        }
                    }
                }
            });
        }

        let client = Self {
            transport: Transport::Stdio {
                stdin,
                writer,
                waiters,
                child: Mutex::new(Some(child)),
                process_tree,
                closing,
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
        Self::connect_http_with_proxy(url, headers, "").await
    }

    /// Same transport with an independent proxy policy: empty inherits, `none`
    /// forces direct, and a URL overrides the ambient proxy.
    pub async fn connect_http_with_proxy(
        url: &str,
        headers: &[(String, String)],
        proxy: &str,
    ) -> Result<Self> {
        let mut builder =
            reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10));
        builder = match proxy.trim() {
            "" => builder,
            "none" => builder.no_proxy(),
            proxy => builder.proxy(reqwest::Proxy::all(proxy)?),
        };
        let http = builder.build()?;
        let client = Self {
            transport: Transport::Http(HttpTransport {
                client: http,
                url: url.to_string(),
                headers: headers.to_vec(),
                session_id: tokio::sync::Mutex::new(None),
                closing: Arc::new(AtomicBool::new(false)),
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

    pub fn managed(connection: crate::connection::ManagedConnection) -> Self {
        Self {
            transport: Transport::Managed(connection),
        }
    }
    pub fn generation(&self) -> u64 {
        match &self.transport {
            Transport::Managed(m) => m.generation(),
            _ => 0,
        }
    }
    pub fn needs_catalog_refresh(&self) -> bool {
        match &self.transport {
            Transport::Managed(m) => m.needs_catalog_refresh(),
            _ => false,
        }
    }
    pub fn mark_catalog_current(&self) {
        if let Transport::Managed(m) = &self.transport {
            m.mark_catalog_current();
        }
    }
    pub async fn tool_call_checked(
        &self,
        expected: &RemoteTool,
        args: &Value,
    ) -> Result<McpCallResult> {
        self.tool_call_checked_generation(expected, args, None)
            .await
    }
    pub async fn tool_call_checked_generation(
        &self,
        expected: &RemoteTool,
        args: &Value,
        generation: Option<u64>,
    ) -> Result<McpCallResult> {
        if let Transport::Managed(m) = &self.transport {
            if generation.is_some() && !m.is_connected() {
                return Err(anyhow!("stale-instance: reopen the MCP App"));
            }
            let prior = m.is_connected().then(|| m.generation());
            let client = m.ready().instrument(m.span()).await?;
            if prior.is_some_and(|g| g != m.generation()) {
                return Err(anyhow!("MCP connection changed while queued; request not sent. Issue a new request after reviewing the previous outcome."));
            }
            let catalog = client.tools_list().await?;
            if generation.is_some_and(|g| g != m.generation()) {
                return Err(anyhow!("stale-instance: reopen the MCP App"));
            }
            if catalog.iter().find(|t| t.name == expected.name) != Some(expected) {
                m.catalog_changed();
                return Err(anyhow!("MCP tool catalog changed; reopen the tool/refresh the conversation before approval and retry. Nothing was sent."));
            }
            crate::tool::validate_tool_arguments(&expected.input_schema, args)
                .map_err(|e| anyhow!(e))?;
            let result = client
                .tool_call_rich(&expected.name, args)
                .instrument(m.span())
                .await;
            m.touch();
            return result;
        }
        self.tool_call_rich(&expected.name, args).await
    }
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let exchange = self.request_exchange(method, params);
        if method == "tools/call" {
            exchange.await
        } else {
            tokio::time::timeout(CONTROL_REQUEST_TIMEOUT, exchange)
                .await
                .map_err(|_| {
                    anyhow!("MCP control request '{method}' timed out; connection not terminated")
                })?
        }
    }
    async fn request_exchange(&self, method: &str, params: Option<Value>) -> Result<Value> {
        match &self.transport {
            Transport::Managed(m) => {
                let prior = m.is_connected().then(|| m.generation());
                let client = m.ready().instrument(m.span()).await?;
                if method == "tools/call" && prior.is_some_and(|g| g != m.generation()) {
                    return Err(anyhow!(
                        "MCP connection changed while queued; request not sent"
                    ));
                }
                // A long tools/call must not look idle the moment it returns.
                let result = Box::pin(client.request(method, params))
                    .instrument(m.span())
                    .await;
                m.touch();
                result
            }
            Transport::Stdio {
                writer,
                waiters,
                closing,
                next_id,
                ..
            } => {
                if closing.load(Ordering::SeqCst) {
                    return Err(anyhow!("MCP connection disconnected; request not sent"));
                }
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let tool = params
                    .as_ref()
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let started = std::time::Instant::now();
                tracing::info!(target: "wisp", request_id=id, method, tool, "mcp.request.started");
                let frame = format!(
                    "{}\n",
                    serde_json::to_string(&JsonRpcReq {
                        jsonrpc: "2.0",
                        id,
                        method: method.into(),
                        params
                    })?
                )
                .into_bytes();
                let (tx, rx) = oneshot::channel();
                waiters.lock().unwrap().insert(id, tx);
                let started_write = Arc::new(AtomicBool::new(false));
                let mut guard = StdioWaiterGuard {
                    sent: started_write.clone(),
                    waiters: waiters.clone(),
                    writer: writer.clone(),
                    id,
                    cancel: method != "initialize",
                };
                let (done, sent) = oneshot::channel();
                writer
                    .send(StdioWrite {
                        frame,
                        request: Some((id, started_write)),
                        done,
                    })
                    .await
                    .map_err(|_| anyhow!("MCP writer closed; request not sent"))?;
                sent.await
                    .map_err(|_| {
                        anyhow!("MCP write interrupted; operation outcome unknown, do not replay")
                    })?
                    .map_err(|error| {
                        if guard.sent.load(Ordering::SeqCst) {
                            error.context(
                                "MCP write failed; operation outcome unknown, do not replay",
                            )
                        } else {
                            error.context("MCP request not sent")
                        }
                    })?;
                let response = rx.await.map_err(|_| {
                    anyhow!("MCP response lost; operation outcome unknown, do not replay")
                })?;
                guard.cancel = false;
                let result = response.and_then(|response| rpc_result(response, id));
                tracing::info!(target: "wisp", request_id=id, method, elapsed_ms=started.elapsed().as_millis() as u64, success=result.is_ok(), "mcp.request.finished");
                result
            }
            Transport::Http(h) => {
                if h.closing.load(Ordering::SeqCst) {
                    return Err(anyhow!(
                        "MCP HTTP connection disconnected; request not sent"
                    ));
                }
                let id = h.next_id.fetch_add(1, Ordering::SeqCst);
                let started = std::time::Instant::now();
                let tool = params
                    .as_ref()
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                tracing::info!(target: "wisp", request_id=id, method, tool, "mcp.request.started");
                let mut rb = h.post().json(&JsonRpcReq {
                    jsonrpc: "2.0",
                    id,
                    method: method.into(),
                    params,
                });
                let mut cancel = h.post().json(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"Caller stopped waiting; no rollback implied"}}));
                if let Some(sid) = h.session_id.lock().await.clone() {
                    rb = rb.header("mcp-session-id", &sid);
                    cancel = cancel.header("mcp-session-id", sid);
                }
                let mut cancellation = HttpCancellation {
                    request: (method != "initialize").then_some(cancel),
                };
                let work = async {
                    let mut response = rb.send().await?;
                    if let Some(sid) = response
                        .headers()
                        .get("mcp-session-id")
                        .and_then(|v| v.to_str().ok())
                    {
                        *h.session_id.lock().await = Some(sid.to_owned());
                    }
                    let status = response.status();
                    if !status.is_success() {
                        h.closing.store(true, Ordering::SeqCst);
                        return Err(anyhow!(
                            "MCP HTTP status {status}; connection invalidated; no automatic replay"
                        ));
                    }
                    let sse = response
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v.contains("text/event-stream"));
                    let mut decoder = SseDecoder::default();
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await? {
                        if sse {
                            if let Some(result) = decoder.feed(&chunk, id)? {
                                return Ok(result);
                            }
                        } else {
                            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                                return Err(anyhow!("MCP response too large"));
                            }
                            bytes.extend_from_slice(&chunk);
                        }
                    }
                    if sse {
                        return Err(anyhow!("MCP SSE closed before response; operation outcome unknown, do not replay"));
                    }
                    rpc_result(serde_json::from_slice(&bytes)?, id)
                };
                let exchange = tokio::select! {
                    result = work => result,
                    _ = async { loop {
                        if h.closing.load(Ordering::SeqCst) { break; }
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    } } => Err(anyhow!("MCP connection closed; operation outcome unknown, do not replay")),
                };
                if !h.closing.load(Ordering::SeqCst) {
                    cancellation.request = None;
                }
                let exchange = exchange.map_err(|error| {
                    if method == "tools/call" && error.downcast_ref::<RemoteRpcError>().is_none() {
                        error.context("MCP HTTP operation outcome unknown; no automatic replay")
                    } else {
                        error
                    }
                });
                if exchange
                    .as_ref()
                    .is_err_and(|e| e.downcast_ref::<RemoteRpcError>().is_none())
                {
                    h.closing.store(true, Ordering::SeqCst);
                }
                tracing::info!(target: "wisp", request_id=id, method, elapsed_ms=started.elapsed().as_millis() as u64, success=exchange.is_ok(), "mcp.request.finished");
                exchange
            }
        }
    }
    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        match &self.transport {
            Transport::Stdio { writer, .. } => {
                let frame = format!(
                    "{}\n",
                    json!({"jsonrpc":"2.0","method":method,"params":params})
                )
                .into_bytes();
                let (done, rx) = oneshot::channel();
                tokio::time::timeout(CONTROL_REQUEST_TIMEOUT, async {
                    writer
                        .send(StdioWrite {
                            frame,
                            request: None,
                            done,
                        })
                        .await
                        .map_err(|_| anyhow!("MCP writer closed"))?;
                    rx.await.map_err(|_| anyhow!("MCP writer stopped"))?
                })
                .await
                .map_err(|_| anyhow!("MCP notification timed out"))?
            }
            Transport::Http(h) => {
                let mut rb = h
                    .post()
                    .json(&json!({"jsonrpc":"2.0","method":method,"params":params}));
                if let Some(sid) = h.session_id.lock().await.clone() {
                    rb = rb.header("mcp-session-id", sid);
                }
                rb.timeout(CONTROL_REQUEST_TIMEOUT)
                    .send()
                    .await?
                    .error_for_status()?;
                Ok(())
            }
            Transport::Managed(m) => Box::pin(m.ready().await?.notify(method, params)).await,
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
    ///
    /// No execution deadline. Dropping the future cancels only this request.
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

    /// Compatibility alias; all callers now have isolated cancellation.
    pub async fn tool_call_rich_isolated(&self, name: &str, args: &Value) -> Result<McpCallResult> {
        self.tool_call_rich(name, args).await
    }

    /// Read one server resource, including MCP Apps `ui://` documents.
    pub async fn resource_read(&self, uri: &str) -> Result<Value> {
        self.request("resources/read", Some(json!({ "uri": uri })))
            .await
    }

    /// Stop this MCP connection and, for stdio transports, its complete child
    /// process tree. Safe to call repeatedly. Drop remains a forceful safety
    /// net for owners that cannot await this method.
    pub async fn shutdown(&self) -> Result<()> {
        match &self.transport {
            Transport::Stdio {
                stdin,
                waiters,
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
                    waiters,
                )
                .await
            }
            Transport::Http(h) => {
                h.closing.store(true, Ordering::SeqCst);
                if let Some(sid) = h.session_id.lock().await.take() {
                    let mut rb = h.client.delete(&h.url).header("mcp-session-id", sid);
                    for (k, v) in &h.headers {
                        rb = rb.header(k, v);
                    }
                    let _ = rb.timeout(STDIO_SHUTDOWN_LOCK_WAIT).send().await;
                }
                Ok(())
            }
            Transport::Managed(m) => m.shutdown().instrument(m.span()).await,
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

fn fail_stdio_waiters(waiters: &StdioWaiters, message: impl Into<String>) {
    let message = message.into();
    let pending = std::mem::take(&mut *waiters.lock().unwrap());
    for (_, tx) in pending {
        let _ = tx.send(Err(anyhow!(message.clone())));
    }
}

fn spawn_stdio_reader(
    stdout: tokio::process::ChildStdout,
    waiters: StdioWaiters,
    closing: Arc<AtomicBool>,
) {
    tokio::spawn(
        async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match (&mut reader)
                    .take((MAX_RESPONSE_BYTES + 1) as u64)
                    .read_line(&mut line)
                    .await
                {
                    Ok(0) => {
                        closing.store(true, Ordering::SeqCst);
                        tracing::warn!(target: "wisp", "mcp.connection.disconnected: stdout EOF");
                        fail_stdio_waiters(
                            &waiters,
                            "MCP server closed stdout; operation outcome unknown, do not replay",
                        );
                        break;
                    }
                    Ok(_) => {
                        if line.len() > MAX_RESPONSE_BYTES {
                            closing.store(true, Ordering::SeqCst);
                            fail_stdio_waiters(
                                &waiters,
                                "MCP response exceeded size limit; connection invalidated",
                            );
                            break;
                        }
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let Ok(resp) = serde_json::from_str::<JsonRpcResp>(trimmed) else {
                            tracing::warn!("ignoring malformed MCP stdio line");
                            continue;
                        };
                        if let Some(id) = resp.id {
                            if let Some(tx) = waiters.lock().unwrap().remove(&id) {
                                let _ = tx.send(Ok(resp));
                            }
                        }
                    }
                    Err(error) => {
                        closing.store(true, Ordering::SeqCst);
                        tracing::warn!(target: "wisp", "mcp.connection.disconnected: read failure");
                        fail_stdio_waiters(
                            &waiters,
                            format!(
                            "MCP server stdout: {error}; operation outcome unknown, do not replay"
                        ),
                        );
                        break;
                    }
                }
            }
        }
        .instrument(tracing::Span::current()),
    );
}

async fn shutdown_stdio(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    child: &Mutex<Option<tokio::process::Child>>,
    process_tree: &ProcessTree,
    closing: &AtomicBool,
    terminated: &AtomicBool,
    shutdown_lock: &Mutex<()>,
    waiters: &StdioWaiters,
) -> Result<()> {
    closing.store(true, Ordering::SeqCst);
    fail_stdio_waiters(waiters, "MCP stdio connection is shutting down");
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
        assert_eq!(tools[0].display_title(), "Open Motif for Claude Science");

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
        // Unset visibility defaults to ["model", "app"], so the presenter and
        // siblings stay callable from an App; an explicit model-only list hides
        // a tool from apps.
        assert!(tools[0].visible_to_app());
        assert!(app_only[0].visible_to_app());
        let model_only = tools_into_remote(vec![json!({
            "name": "motif_hidden",
            "inputSchema": { "type": "object" },
            "_meta": { "ui": { "visibility": ["model"] } }
        })]);
        assert!(model_only[0].visible_to_model());
        assert!(!model_only[0].visible_to_app());
        // Title falls back to the annotated title, then the raw name.
        let annotated = tools_into_remote(vec![json!({
            "name": "motif_named",
            "inputSchema": { "type": "object" },
            "annotations": { "title": "Motif Refresh" }
        })]);
        assert_eq!(annotated[0].display_title(), "Motif Refresh");
        let bare = tools_into_remote(vec![json!({
            "name": "motif_bare",
            "inputSchema": { "type": "object" }
        })]);
        assert_eq!(bare[0].display_title(), "motif_bare");
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

    #[tokio::test]
    async fn http_explicit_proxy_routes_an_unresolvable_mcp_host_through_proxy() {
        // A loopback fake proxy handles MCP itself. The target deliberately
        // cannot resolve, so this succeeds only if the explicit proxy is used.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                serve_http_jsonrpc(stream.unwrap());
            }
        });
        let client = McpClient::connect_http_with_proxy("http://mcp.invalid/mcp", &[], &proxy)
            .await
            .unwrap();
        let response = client
            .tool_call_rich("echo", &json!({"token": "proxied"}))
            .await
            .unwrap();
        assert_eq!(response.structured_content.unwrap()["token"], "proxied");
    }

    #[tokio::test]
    async fn http_invalid_proxy_fails_before_connecting() {
        assert!(McpClient::connect_http_with_proxy(
            "http://mcp.invalid/mcp",
            &[],
            "socks42://localhost:1234"
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn http_concurrent_calls_keep_matching_ids() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let Ok(stream) = stream else {
                    continue;
                };
                std::thread::spawn(move || serve_http_jsonrpc(stream));
            }
        });

        let url = format!("http://{addr}/mcp");
        let client = McpClient::connect_http_with_proxy(&url, &[], "none")
            .await
            .unwrap();
        let slow_args = json!({ "token": "slow", "delay_ms": 180 });
        let fast_args = json!({ "token": "fast", "delay_ms": 20 });
        let slow = client.tool_call_rich("echo", &slow_args);
        let fast = client.tool_call_rich("echo", &fast_args);
        let (slow, fast) = tokio::join!(slow, fast);
        let slow = slow.unwrap();
        let fast = fast.unwrap();
        assert_eq!(slow.structured_content.unwrap()["token"], "slow");
        assert_eq!(fast.structured_content.unwrap()["token"], "fast");
        drop(client);
        let _ = server;
    }

    fn serve_http_jsonrpc(mut stream: std::net::TcpStream) {
        use std::io::{Read, Write};
        let mut buf = Vec::new();
        let mut tmp = [0u8; 2048];
        let n = match stream.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            return;
        };
        let headers = std::str::from_utf8(&buf[..header_end]).unwrap_or("");
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        let body_start = header_end + 4;
        while buf.len() < body_start + content_length {
            match stream.read(&mut tmp) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
            }
        }
        let Ok(body) =
            serde_json::from_slice::<Value>(&buf[body_start..body_start + content_length])
        else {
            return;
        };
        if body.get("id").is_none() {
            let _ = stream.write_all(
                b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            );
            return;
        }
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        let method = body.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake-http", "version": "1" }
            }),
            "tools/list" => json!({ "tools": [{
                "name": "echo",
                "inputSchema": { "type": "object" }
            }] }),
            "tools/call" => {
                let arguments = body
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(json!({}));
                let delay = arguments
                    .get("delay_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if delay > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                }
                json!({
                    "content": [{ "type": "text", "text": "ok" }],
                    "structuredContent": { "token": arguments.get("token").cloned().unwrap_or(Value::Null) },
                    "isError": false
                })
            }
            _ => json!({}),
        };
        let payload = json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
            payload.len()
        );
        let _ = stream.write_all(response.as_bytes());
    }
}
