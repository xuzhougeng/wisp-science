//! Loopback bridge to a user-installed Chrome/Chromium extension.
//!
//! The extension runs inside the user's existing browser profile, so page
//! execution keeps that profile's cookies, login state, extensions, GPU, and
//! browser fingerprint. Wisp never launches a separate automation browser.
//!
//! Design acknowledgement: this bridge is inspired by GenericAgent's GA Web /
//! TMWebDriver real-browser architecture and compatible loopback protocol:
//! https://github.com/lsdefine/GenericAgent (MIT, Copyright 2025 lsdefine).
//! This module is Wisp's independent Rust implementation; see
//! `browser-extension/NOTICE.md` for provenance details.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_hdr_async, WebSocketStream};
use uuid::Uuid;
use wisp_llm::ToolSchema;
use wisp_tools::{Approval, Tool, ToolEnv, ToolResult};

const BRIDGE_ADDR: &str = "127.0.0.1:18765";
const EXTENSION_ORIGIN: &str = "chrome-extension://gnkjgagleagkgdlkkcianolobfdoocnp";
const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const MAX_TIMEOUT_MS: u64 = 60_000;
const MAX_SCRIPT_BYTES: usize = 64 * 1024;
const MAX_RESULT_CHARS: usize = 200_000;

#[derive(Clone, Debug, Serialize)]
pub struct BrowserTab {
    id: i64,
    url: String,
    title: String,
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_id: Option<i64>,
}

#[derive(Clone)]
struct BridgeClient {
    connection_id: u64,
    tx: mpsc::UnboundedSender<Message>,
}

#[derive(Default)]
struct BridgeState {
    client: Option<BridgeClient>,
    tabs: BTreeMap<i64, BrowserTab>,
    selected_tab: Option<i64>,
    pending: HashMap<String, oneshot::Sender<Result<Value, String>>>,
    startup_error: Option<String>,
}

pub struct BrowserBridge {
    state: Mutex<BridgeState>,
    next_connection_id: AtomicU64,
    extension_dir: PathBuf,
}

struct BrowserExecution {
    tab_id: i64,
    value: Value,
}

impl BrowserBridge {
    fn new(extension_dir: PathBuf) -> Self {
        Self {
            state: Mutex::new(BridgeState::default()),
            next_connection_id: AtomicU64::new(1),
            extension_dir,
        }
    }

    pub async fn start(extension_dir: PathBuf) -> Arc<Self> {
        let bridge = Arc::new(Self::new(extension_dir));
        match TcpListener::bind(BRIDGE_ADDR).await {
            Ok(listener) => {
                let task_bridge = bridge.clone();
                tokio::spawn(async move { task_bridge.accept_loop(listener).await });
            }
            Err(error) => {
                bridge.state.lock().await.startup_error = Some(format!(
                    "cannot listen on {BRIDGE_ADDR}: {error}; stop any other TMWebDriver/Wisp browser bridge using this port"
                ));
            }
        }
        bridge
    }

    async fn setup_info(&self) -> Value {
        let state = self.state.lock().await;
        let extension_path = self.verified_extension_path();
        let extension_ready = extension_path.is_some();
        let status = if state.startup_error.is_some() {
            "error"
        } else if !extension_ready {
            "extension_missing"
        } else if state.client.is_some() {
            "connected"
        } else {
            "disconnected"
        };
        let steps = extension_path.as_ref().map_or_else(Vec::new, |path| {
            vec![
                "Start Wisp Science and keep it running.".to_string(),
                "Open chrome://extensions in the Chrome/Chromium profile Wisp should control."
                    .to_string(),
                "Enable Developer mode.".to_string(),
                format!("Click Load unpacked and select this exact folder: {path}"),
                "Open the Wisp Real Browser Bridge extension popup and confirm Connected to Wisp."
                    .to_string(),
            ]
        });

        json!({
            "status": status,
            "connected_tabs": state.tabs.len(),
            "runtime_os": std::env::consts::OS,
            "path_source": "wisp_tauri_resource_dir",
            "extension_path": extension_path,
            "extension_path_verified": extension_ready,
            "extension_id": EXTENSION_ORIGIN.trim_start_matches("chrome-extension://"),
            "bridge_endpoint": format!("ws://{BRIDGE_ADDR}"),
            "install_scope": "once_per_browser_profile",
            "assistant_instruction": if extension_ready {
                "Copy extension_path character-for-character. Never translate, infer, normalize, or replace any path segment."
            } else {
                "The running Wisp build has no verified bundled extension path. Do not invent a path or claim the extension exists."
            },
            "steps": steps,
            "download_automation": {
                "limitation": "GA Web controls web-page tabs. It cannot operate Chrome/Edge toolbar download bubbles or native operating-system Open, Save, and Save As dialogs.",
                "manual_setup_required": true,
                "chrome_settings_url": "chrome://settings/downloads",
                "edge_settings_url": "edge://settings/downloads",
                "setting_to_disable": "Ask where to save each file before downloading",
                "multiple_downloads": {
                    "chrome_settings_url": "chrome://settings/content/automaticDownloads",
                    "edge_settings_url": "edge://settings/content/automaticDownloads",
                    "recommended_action": "Add only the trusted target site to Allowed to automatically download multiple files. If the browser asks on the site's first batch, choose Allow.",
                    "security_note": "Do not allow automatic multiple downloads for untrusted sites."
                },
                "effect": "Downloads save to the browser's configured default download directory without opening a native location prompt. Authorized filesystem tools may process the saved file afterward."
            },
            "error": state.startup_error
        })
    }

    async fn accept_loop(self: Arc<Self>, listener: TcpListener) {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let bridge = self.clone();
                    tokio::spawn(async move {
                        if let Err(error) = bridge.accept_connection(stream).await {
                            tracing::warn!(target: "wisp", "browser bridge connection rejected: {error}");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(target: "wisp", "browser bridge accept failed: {error}");
                }
            }
        }
    }

    async fn accept_connection(
        self: Arc<Self>,
        stream: TcpStream,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        let socket = accept_hdr_async(stream, |request: &Request, response: Response| {
            let origin = request
                .headers()
                .get("origin")
                .and_then(|value| value.to_str().ok());
            if allowed_extension_origin(origin) {
                Ok(response)
            } else {
                Err(forbidden_response())
            }
        })
        .await?;
        self.serve_connection(socket).await;
        Ok(())
    }

    async fn serve_connection(self: Arc<Self>, socket: WebSocketStream<TcpStream>) {
        let connection_id = self.next_connection_id.fetch_add(1, Ordering::Relaxed);
        let (mut writer, mut reader) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.install_client(connection_id, tx.clone()).await;
        let writer_task = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if writer.send(message).await.is_err() {
                    break;
                }
            }
        });

        while let Some(message) = reader.next().await {
            match message {
                Ok(Message::Text(text)) => self.handle_text(connection_id, text.as_str()).await,
                Ok(Message::Ping(payload)) => {
                    let _ = tx.send(Message::Pong(payload));
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
        writer_task.abort();
        self.disconnect_client(connection_id).await;
    }

    async fn install_client(&self, connection_id: u64, tx: mpsc::UnboundedSender<Message>) {
        let mut state = self.state.lock().await;
        fail_pending(&mut state, "browser extension connection was replaced");
        state.client = Some(BridgeClient { connection_id, tx });
        state.tabs.clear();
        state.selected_tab = None;
    }

    async fn disconnect_client(&self, connection_id: u64) {
        let mut state = self.state.lock().await;
        if state
            .client
            .as_ref()
            .is_some_and(|client| client.connection_id == connection_id)
        {
            state.client = None;
            state.tabs.clear();
            state.selected_tab = None;
            fail_pending(&mut state, "browser extension disconnected");
        }
    }

    async fn handle_text(&self, connection_id: u64, text: &str) {
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let message_type = message.get("type").and_then(Value::as_str).unwrap_or("");
        let mut state = self.state.lock().await;
        if !state
            .client
            .as_ref()
            .is_some_and(|client| client.connection_id == connection_id)
        {
            return;
        }
        match message_type {
            "ext_ready" | "tabs_update" => replace_tabs(&mut state, &message),
            "result" | "error" => {
                let Some(id) = message.get("id").and_then(Value::as_str) else {
                    return;
                };
                let Some(sender) = state.pending.remove(id) else {
                    return;
                };
                let result = if message_type == "result" {
                    Ok(message
                        .get("result")
                        .or_else(|| message.get("data"))
                        .cloned()
                        .unwrap_or(Value::Null))
                } else {
                    Err(render_bridge_error(message.get("error")))
                };
                let _ = sender.send(result);
            }
            _ => {}
        }
    }

    async fn execute(
        &self,
        requested_tab: Option<i64>,
        code: &str,
        timeout: Duration,
    ) -> Result<BrowserExecution, String> {
        let id = Uuid::new_v4().to_string();
        let (response_tx, response_rx) = oneshot::channel();
        let tab_id = {
            let mut state = self.state.lock().await;
            if let Some(error) = &state.startup_error {
                return Err(self.unavailable_message(error));
            }
            let Some(client) = state.client.clone() else {
                return Err(self.unavailable_message("browser extension is not connected"));
            };
            let tab_id = select_tab(&state, requested_tab)?;
            state.selected_tab = Some(tab_id);
            state.pending.insert(id.clone(), response_tx);
            let payload = json!({ "id": id, "tabId": tab_id, "code": code }).to_string();
            if client.tx.send(Message::Text(payload.into())).is_err() {
                state.pending.remove(&id);
                return Err("browser extension disconnected before the request was sent".into());
            }
            tab_id
        };

        match tokio::time::timeout(timeout, response_rx).await {
            Ok(Ok(Ok(value))) => Ok(BrowserExecution { tab_id, value }),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err("browser extension disconnected before returning a result".into()),
            Err(_) => {
                self.state.lock().await.pending.remove(&id);
                Err(format!(
                    "browser execution timed out after {} ms",
                    timeout.as_millis()
                ))
            }
        }
    }

    async fn tabs(&self) -> Result<Vec<BrowserTab>, String> {
        let state = self.state.lock().await;
        if let Some(error) = &state.startup_error {
            return Err(self.unavailable_message(error));
        }
        if state.client.is_none() {
            return Err(self.unavailable_message("browser extension is not connected"));
        }
        Ok(state.tabs.values().cloned().collect())
    }

    fn unavailable_message(&self, reason: &str) -> String {
        match self.verified_extension_path() {
            Some(path) => format!(
                "real-browser bridge unavailable: {reason}. In Chrome/Chromium open chrome://extensions, enable Developer mode, and Load unpacked from this exact native {} path: '{path}'. Keep Wisp running; the extension connects only to {BRIDGE_ADDR}.",
                std::env::consts::OS
            ),
            None => format!(
                "real-browser bridge unavailable: {reason}. This Wisp build has no verified bundled browser extension; do not infer an installation path."
            ),
        }
    }

    fn verified_extension_path(&self) -> Option<String> {
        let dir = dunce::canonicalize(&self.extension_dir).ok()?;
        dir.join("manifest.json")
            .is_file()
            .then(|| dir.display().to_string())
    }
}

fn allowed_extension_origin(origin: Option<&str>) -> bool {
    origin == Some(EXTENSION_ORIGIN)
}

fn forbidden_response() -> ErrorResponse {
    tokio_tungstenite::tungstenite::http::Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(Some(
            "Wisp browser bridge accepts Chrome extensions only".into(),
        ))
        .expect("static browser bridge rejection response")
}

fn fail_pending(state: &mut BridgeState, reason: &str) {
    for (_, sender) in state.pending.drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
}

fn replace_tabs(state: &mut BridgeState, message: &Value) {
    let Some(tabs) = message.get("tabs").and_then(Value::as_array) else {
        return;
    };
    state.tabs = tabs
        .iter()
        .filter_map(parse_tab)
        .map(|tab| (tab.id, tab))
        .collect();
    if !state
        .selected_tab
        .is_some_and(|tab_id| state.tabs.contains_key(&tab_id))
    {
        state.selected_tab = state
            .tabs
            .values()
            .find(|tab| tab.active)
            .or_else(|| state.tabs.values().next())
            .map(|tab| tab.id);
    }
}

fn parse_tab(value: &Value) -> Option<BrowserTab> {
    let id = value.get("id").and_then(|id| {
        id.as_i64()
            .or_else(|| id.as_str().and_then(|id| id.parse().ok()))
    })?;
    Some(BrowserTab {
        id,
        url: value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        active: value
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        window_id: value.get("windowId").and_then(Value::as_i64),
    })
}

fn select_tab(state: &BridgeState, requested: Option<i64>) -> Result<i64, String> {
    if state.tabs.is_empty() {
        return Err("browser extension is connected, but no HTTP(S) tabs are available".into());
    }
    if let Some(tab_id) = requested {
        return state
            .tabs
            .contains_key(&tab_id)
            .then_some(tab_id)
            .ok_or_else(|| {
                format!("browser tab {tab_id} is not available; call web_scan with tabs_only=true")
            });
    }
    state
        .selected_tab
        .filter(|tab_id| state.tabs.contains_key(tab_id))
        .or_else(|| state.tabs.values().find(|tab| tab.active).map(|tab| tab.id))
        .or_else(|| state.tabs.keys().next().copied())
        .ok_or_else(|| "no browser tab is selected".into())
}

fn render_bridge_error(error: Option<&Value>) -> String {
    match error {
        Some(Value::String(error)) => error.clone(),
        Some(error) => serde_json::to_string_pretty(error).unwrap_or_else(|_| error.to_string()),
        None => "browser extension returned an unknown error".into(),
    }
}

fn tab_id_arg(args: &Value) -> Result<Option<i64>, String> {
    let Some(value) = args.get("switch_tab_id") else {
        return Ok(None);
    };
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .map(Some)
        .ok_or_else(|| "switch_tab_id must be an integer tab id returned by web_scan".into())
}

fn render_json(value: &Value) -> String {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    if rendered.chars().count() <= MAX_RESULT_CHARS {
        return rendered;
    }
    let mut clipped: String = rendered.chars().take(MAX_RESULT_CHARS).collect();
    clipped.push_str("\n... browser result truncated");
    clipped
}

const SCAN_SCRIPT: &str = r##"(() => {
  const visible = (el) => {
    const s = getComputedStyle(el), r = el.getBoundingClientRect();
    return s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity) > 0 && r.width > 0 && r.height > 0;
  };
  const selector = (el) => {
    if (el.id) {
      const id = '#' + CSS.escape(el.id);
      if (document.querySelectorAll(id).length === 1) return id;
    }
    const parts = [];
    for (let node = el; node && node.nodeType === 1 && parts.length < 6; node = node.parentElement) {
      let part = node.tagName.toLowerCase();
      const siblings = node.parentElement ? [...node.parentElement.children].filter(x => x.tagName === node.tagName) : [];
      if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(node) + 1})`;
      parts.unshift(part);
      const candidate = parts.join(' > ');
      if (document.querySelectorAll(candidate).length === 1) return candidate;
    }
    return parts.join(' > ');
  };
  const query = 'a,button,input,textarea,select,summary,[role],[contenteditable=true],h1,h2,h3,label';
  const elements = [...document.querySelectorAll(query)].filter(visible).slice(0, 400).map((el) => {
    const r = el.getBoundingClientRect(), type = el.getAttribute('type') || '';
    return {
      selector: selector(el), tag: el.tagName.toLowerCase(), role: el.getAttribute('role') || undefined,
      text: (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 500) || undefined,
      aria_label: el.getAttribute('aria-label') || undefined, href: el.href || undefined, type: type || undefined,
      value: type.toLowerCase() === 'password' ? undefined : (el.value || undefined), disabled: !!el.disabled,
      rect: [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]
    };
  });
  return { url: location.href, title: document.title, viewport: [innerWidth, innerHeight],
    text: (document.body?.innerText || '').slice(0, 30000), elements };
})()"##;

const TEXT_SCAN_SCRIPT: &str = r#"(() => ({
  url: location.href,
  title: document.title,
  text: (document.body?.innerText || '').slice(0, 50000)
}))()"#;

pub struct BrowserSetupTool {
    bridge: Arc<BrowserBridge>,
}

impl BrowserSetupTool {
    pub fn new(bridge: Arc<BrowserBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for BrowserSetupTool {
    fn name(&self) -> &str {
        "browser_setup"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Call when the user asks to configure, install, set up, or connect the real browser. The result is derived from the running Wisp binary's native Tauri resource directory and includes the manual settings required for unattended single and multiple downloads. Copy extension_path character-for-character and never convert it between Windows, WSL, macOS, or Linux. If extension_path_verified is false, report the missing bundled extension and never invent a path.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        )
    }

    fn preview(&self, _args: &Value) -> String {
        "show real-browser setup status and extension path".into()
    }

    async fn run(&self, _args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        ToolResult::ok(render_json(&self.bridge.setup_info().await))
    }
}

pub struct WebScanTool {
    bridge: Arc<BrowserBridge>,
}

impl WebScanTool {
    pub fn new(bridge: Arc<BrowserBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for WebScanTool {
    fn name(&self) -> &str {
        "web_scan"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Read visible content and actionable elements from the user's real, persistent Chrome/Chromium session. The browser keeps its existing cookies, login state, extensions, GPU/WebGL behavior, and normal profile fingerprint. Use tabs_only first when the target tab is unclear.",
            json!({
                "type": "object",
                "properties": {
                    "tabs_only": { "type": "boolean", "description": "List connected HTTP(S) tabs without reading page content" },
                    "switch_tab_id": { "type": ["integer", "string"], "description": "Tab id returned by this tool; selects that tab for this and later calls" },
                    "text_only": { "type": "boolean", "description": "Return page text without the actionable-element snapshot" }
                }
            }),
        )
    }

    fn minimum_approval(&self) -> Approval {
        Approval::Ask
    }

    fn preview(&self, args: &Value) -> String {
        if args
            .get("tabs_only")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "list real-browser tabs".into()
        } else {
            args.get("switch_tab_id")
                .map(|tab| format!("scan real-browser tab {tab}"))
                .unwrap_or_else(|| "scan selected real-browser tab".into())
        }
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        if args
            .get("tabs_only")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return match self.bridge.tabs().await {
                Ok(tabs) => ToolResult::ok(render_json(&json!({ "tabs": tabs }))),
                Err(error) => ToolResult::fail(error),
            };
        }
        let tab_id = match tab_id_arg(args) {
            Ok(tab_id) => tab_id,
            Err(error) => return ToolResult::fail(error),
        };
        let text_only = args
            .get("text_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match self
            .bridge
            .execute(
                tab_id,
                if text_only {
                    TEXT_SCAN_SCRIPT
                } else {
                    SCAN_SCRIPT
                },
                Duration::from_millis(DEFAULT_TIMEOUT_MS),
            )
            .await
        {
            Ok(execution) => ToolResult::ok(render_json(&json!({
                "tab_id": execution.tab_id,
                "page": execution.value
            }))),
            Err(error) => ToolResult::fail(error),
        }
    }
}

pub struct WebExecuteJsTool {
    bridge: Arc<BrowserBridge>,
}

impl WebExecuteJsTool {
    pub fn new(bridge: Arc<BrowserBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for WebExecuteJsTool {
    fn name(&self) -> &str {
        "web_execute_js"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Execute JavaScript in a tab from the user's real, persistent Chrome/Chromium session. Call web_scan first and do not guess selectors. A JSON script with cmd='cdp' may call one Chrome DevTools Protocol method for trusted input or other advanced browser actions.",
            json!({
                "type": "object",
                "properties": {
                    "script": { "type": "string", "description": "JavaScript, or a JSON command such as {\"cmd\":\"cdp\",\"method\":\"Input.dispatchMouseEvent\",\"params\":{...}}" },
                    "switch_tab_id": { "type": ["integer", "string"], "description": "Tab id returned by web_scan" },
                    "timeout_ms": { "type": "integer", "minimum": 1, "maximum": 60000, "description": "Execution timeout in milliseconds (default 15000)" }
                },
                "required": ["script"]
            }),
        )
    }

    fn minimum_approval(&self) -> Approval {
        Approval::Ask
    }

    fn preview(&self, args: &Value) -> String {
        let script = args.get("script").and_then(Value::as_str).unwrap_or("");
        let mut preview: String = script.chars().take(240).collect();
        if script.chars().count() > 240 {
            preview.push('…');
        }
        preview
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let Some(script) = args
            .get("script")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|script| !script.is_empty())
        else {
            return ToolResult::fail("missing required argument 'script'");
        };
        if script.len() > MAX_SCRIPT_BYTES {
            return ToolResult::fail(format!(
                "browser script is {} bytes (maximum {MAX_SCRIPT_BYTES})",
                script.len()
            ));
        }
        let tab_id = match tab_id_arg(args) {
            Ok(tab_id) => tab_id,
            Err(error) => return ToolResult::fail(error),
        };
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1, MAX_TIMEOUT_MS);
        match self
            .bridge
            .execute(tab_id, script, Duration::from_millis(timeout_ms))
            .await
        {
            Ok(execution) => ToolResult::ok(render_json(&json!({
                "tab_id": execution.tab_id,
                "result": execution.value
            }))),
            Err(error) => ToolResult::fail(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use sha2::{Digest, Sha256};

    #[test]
    fn manifest_key_matches_the_only_accepted_extension_origin() {
        let manifest_path = wisp_paths::browser_extension_dir()
            .unwrap()
            .join("manifest.json");
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
        let key = manifest["key"].as_str().unwrap();
        let der = base64::engine::general_purpose::STANDARD
            .decode(key)
            .unwrap();
        let digest = Sha256::digest(der);
        let id: String = digest[..16]
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0x0f])
            .map(|nibble| char::from(b'a' + nibble))
            .collect();
        assert_eq!(EXTENSION_ORIGIN, format!("chrome-extension://{id}"));
    }

    #[test]
    fn bridge_accepts_extension_origins_only() {
        assert!(allowed_extension_origin(Some(
            "chrome-extension://gnkjgagleagkgdlkkcianolobfdoocnp"
        )));
        assert!(!allowed_extension_origin(Some(
            "chrome-extension://abcdefghijklmnop"
        )));
        assert!(!allowed_extension_origin(Some("https://example.com")));
        assert!(!allowed_extension_origin(Some("null")));
        assert!(!allowed_extension_origin(None));
    }

    #[test]
    fn page_access_tools_always_require_approval() {
        let bridge = Arc::new(BrowserBridge::new(PathBuf::from("extension")));
        assert_eq!(
            WebScanTool::new(bridge.clone()).minimum_approval(),
            Approval::Ask
        );
        assert_eq!(
            WebExecuteJsTool::new(bridge).minimum_approval(),
            Approval::Ask
        );
    }

    #[tokio::test]
    async fn setup_reports_the_extension_folder_without_requiring_approval() {
        let extension_dir = wisp_paths::browser_extension_dir().unwrap();
        let bridge = Arc::new(BrowserBridge::new(extension_dir.clone()));
        let info = bridge.setup_info().await;
        let expected_path = dunce::canonicalize(extension_dir).unwrap();

        assert_eq!(info["status"], "disconnected");
        assert_eq!(info["runtime_os"], std::env::consts::OS);
        assert_eq!(info["path_source"], "wisp_tauri_resource_dir");
        assert_eq!(info["extension_path"], expected_path.display().to_string());
        assert_eq!(info["extension_path_verified"], true);
        assert_eq!(info["install_scope"], "once_per_browser_profile");
        assert_eq!(
            info["download_automation"]["chrome_settings_url"],
            "chrome://settings/downloads"
        );
        assert_eq!(
            info["download_automation"]["setting_to_disable"],
            "Ask where to save each file before downloading"
        );
        assert_eq!(
            info["download_automation"]["multiple_downloads"]["chrome_settings_url"],
            "chrome://settings/content/automaticDownloads"
        );
        assert!(
            info["download_automation"]["multiple_downloads"]["recommended_action"]
                .as_str()
                .unwrap()
                .contains("trusted target site")
        );
        assert!(info["steps"].as_array().unwrap().iter().any(|step| step
            .as_str()
            .unwrap()
            .contains(info["extension_path"].as_str().unwrap())));
        assert!(bridge
            .unavailable_message("not connected")
            .contains(info["extension_path"].as_str().unwrap()));
        assert_eq!(
            BrowserSetupTool::new(bridge).minimum_approval(),
            Approval::Allow
        );
    }

    #[tokio::test]
    async fn setup_never_offers_an_unverified_extension_path() {
        let missing = std::env::temp_dir().join(format!(
            "wisp-browser-extension-missing-{}",
            std::process::id()
        ));
        let bridge = BrowserBridge::new(missing.clone());
        let info = bridge.setup_info().await;

        assert_eq!(info["status"], "extension_missing");
        assert_eq!(info["extension_path_verified"], false);
        assert!(info["extension_path"].is_null());
        assert!(info["steps"].as_array().unwrap().is_empty());
        assert!(!bridge
            .unavailable_message("not connected")
            .contains(&missing.display().to_string()));
    }

    #[test]
    fn tab_parser_accepts_generic_agent_numeric_and_string_ids() {
        let numeric =
            parse_tab(&json!({ "id": 7, "url": "https://a", "title": "A", "active": true }))
                .unwrap();
        let string = parse_tab(&json!({ "id": "8", "url": "https://b", "title": "B" })).unwrap();
        assert_eq!(numeric.id, 7);
        assert!(numeric.active);
        assert_eq!(string.id, 8);
    }

    #[tokio::test]
    async fn routes_execution_to_the_live_extension_and_correlates_result() {
        let bridge = Arc::new(BrowserBridge::new(PathBuf::from("extension")));
        let (tx, mut rx) = mpsc::unbounded_channel();
        bridge.install_client(1, tx).await;
        bridge
            .handle_text(
                1,
                r#"{"type":"ext_ready","tabs":[{"id":42,"url":"https://example.com","title":"Example","active":true}]}"#,
            )
            .await;

        let running = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .execute(None, "document.title", Duration::from_secs(1))
                    .await
            })
        };
        let outbound = rx.recv().await.unwrap().into_text().unwrap();
        let outbound: Value = serde_json::from_str(&outbound).unwrap();
        assert_eq!(outbound["tabId"], 42);
        assert_eq!(outbound["code"], "document.title");
        let id = outbound["id"].as_str().unwrap();
        bridge
            .handle_text(
                1,
                &json!({ "type": "result", "id": id, "result": "Example" }).to_string(),
            )
            .await;

        let result = running.await.unwrap().unwrap();
        assert_eq!(result.tab_id, 42);
        assert_eq!(result.value, "Example");
    }

    #[test]
    fn browser_results_are_bounded_before_entering_model_context() {
        let rendered = render_json(&json!({ "text": "x".repeat(MAX_RESULT_CHARS * 2) }));
        assert!(rendered.chars().count() <= MAX_RESULT_CHARS + 40);
        assert!(rendered.ends_with("browser result truncated"));
    }
}
