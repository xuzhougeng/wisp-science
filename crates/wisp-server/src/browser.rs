//! The real-browser bridge: a loopback listener the Wisp Chrome extension
//! connects to, and the tools that open a tab, read a page and run a
//! script in the user's own browser session.
//!
//! The extension runs in the user's existing profile, so pages keep its
//! cookies and logins; no separate automation browser is started. The
//! wire protocol is the extension's (`browser-extension/`), as spoken by
//! `src-tauri/src/browser_bridge`, which cannot be shared from here
//! because it lives in the Tauri shell. This is a smaller bridge than
//! that one: one session, and a conversation may only touch tabs it
//! opened itself, so pages the user has open are never read or driven.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
        Message,
    },
};
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{Approval, Tool, ToolEnv, ToolResult};

use crate::browser_filters::{self, BrowserFilters};

/// Where the extension looks for Wisp.
pub const BRIDGE_ADDR: &str = "127.0.0.1:18765";
const EXTENSION_ORIGIN: &str = "chrome-extension://gnkjgagleagkgdlkkcianolobfdoocnp";
const REQUIRED_PROTOCOL: i64 = 2;
const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const MAX_TIMEOUT_MS: u64 = 60_000;
/// Longer than the extension's own budget, so its answer wins the race.
const REPLY_GRACE: Duration = Duration::from_secs(5);
const MAX_SCRIPT_BYTES: usize = 64 * 1024;
const MAX_RESULT_CHARS: usize = 200_000;
const DISCONNECTED: &str = "The Wisp browser extension is not connected, so there is no live web \
    retrieval in this turn. Do not answer live, latest or URL-specific questions from memory. \
    Tell the user to open chrome://extensions in Chrome or Chromium, turn on Developer mode, \
    press Load unpacked and choose the extension folder named by browser_setup, then try again.";

/// A tab as the extension reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tab {
    id: i64,
    url: String,
    title: String,
    active: bool,
}

struct Link {
    id: u64,
    tx: mpsc::UnboundedSender<String>,
    protocol: i64,
    version: String,
    paused: bool,
}

/// What the extension answered.
struct Reply {
    value: Value,
    ready: Option<bool>,
    wait: Option<Value>,
    /// Tabs that came into being while the request ran.
    new_tabs: Vec<i64>,
}

#[derive(Default)]
struct State {
    /// The origin of the last connection that was turned away.
    refused: Option<String>,
    link: Option<Link>,
    tabs: Vec<Tab>,
    pending: HashMap<String, oneshot::Sender<Result<Reply, String>>>,
    /// The tabs each conversation opened, oldest first.
    owned: HashMap<String, Vec<i64>>,
    /// The tab each conversation last worked on.
    selected: HashMap<String, i64>,
}

/// The bridge as Settings shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserStatus {
    /// The extension is connected.
    pub connected: bool,
    pub extension_version: String,
    /// The connected extension speaks another protocol than this build.
    pub update_required: bool,
    /// The user paused browser control from the extension's popup.
    pub paused: bool,
    /// Pages open in the browser, the user's own included.
    pub tabs: usize,
    /// Something other than the Wisp extension tried to connect.
    pub refused_origin: Option<String>,
}

pub(crate) struct Bridge {
    state: Mutex<State>,
    next: AtomicU64,
    #[cfg(test)]
    addr: std::net::SocketAddr,
    accepting: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

fn parse_tab(value: &Value) -> Option<Tab> {
    Some(Tab {
        id: value.get("id")?.as_i64()?,
        url: value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        active: value
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_reply(message: &Value) -> Reply {
    Reply {
        value: message
            .get("result")
            .or_else(|| message.get("data"))
            .cloned()
            .unwrap_or(Value::Null),
        ready: message.get("ready").and_then(Value::as_bool),
        wait: message.get("wait").cloned().filter(|wait| !wait.is_null()),
        new_tabs: message
            .get("newTabs")
            .and_then(Value::as_array)
            .map(|tabs| {
                tabs.iter()
                    .filter_map(|tab| tab.get("id")?.as_i64())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn error_text(message: &Value) -> String {
    match message.get("error") {
        Some(Value::String(text)) => text.clone(),
        Some(error) => match (
            error.get("code").and_then(Value::as_str),
            error.get("message").and_then(Value::as_str),
        ) {
            (Some(code), Some(text)) => format!("{code}: {text}"),
            (None, Some(text)) => text.to_owned(),
            _ => error.to_string(),
        },
        None => "the browser extension reported an error".into(),
    }
}

fn render(value: &Value) -> String {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    if rendered.chars().count() <= MAX_RESULT_CHARS {
        return rendered;
    }
    let mut clipped: String = rendered.chars().take(MAX_RESULT_CHARS).collect();
    clipped.push_str("\n… (clipped)");
    clipped
}

fn out_of_scope(tab: i64) -> String {
    format!(
        "tab {tab} was not opened by this conversation. Only tabs opened with web_open_tab \
         here (and tabs they open) can be read or driven; the user's own tabs are off limits."
    )
}

/// A script that is really a command for the extension, as the
/// extension itself tells one from JavaScript.
fn command(script: &str) -> Option<Value> {
    serde_json::from_str::<Value>(script)
        .ok()
        .filter(|value| value.is_object() && value.get("cmd").is_some())
}

impl Bridge {
    /// Starts listening on `addr`. Fails when the port is taken, which is
    /// what happens while another Wisp holds the bridge.
    pub(crate) async fn start(addr: &str) -> Result<Arc<Self>> {
        let listener = TcpListener::bind(addr).await.with_context(|| {
            format!("cannot listen on {addr}; quit any other Wisp that is using the browser bridge")
        })?;
        let bridge = Arc::new(Self {
            state: Mutex::default(),
            next: AtomicU64::new(1),
            #[cfg(test)]
            addr: listener.local_addr()?,
            accepting: Mutex::default(),
        });
        let accepting = Arc::downgrade(&bridge);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let Some(bridge) = accepting.upgrade() else {
                    break;
                };
                tokio::spawn(async move { bridge.serve(stream).await });
            }
        });
        *bridge.accepting.lock().unwrap() = Some(task);
        Ok(bridge)
    }

    #[cfg(test)]
    pub(crate) fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    /// Stops listening and drops the extension.
    pub(crate) fn stop(&self) {
        if let Some(task) = self.accepting.lock().unwrap().take() {
            task.abort();
        }
        let mut state = self.state.lock().unwrap();
        state.link = None;
        state.tabs.clear();
        for (_, waiting) in state.pending.drain() {
            let _ = waiting.send(Err("the browser bridge was switched off".into()));
        }
    }

    pub(crate) fn status(&self) -> BrowserStatus {
        let state = self.state.lock().unwrap();
        let link = state.link.as_ref();
        BrowserStatus {
            connected: link.is_some(),
            extension_version: link.map(|link| link.version.clone()).unwrap_or_default(),
            update_required: link.is_some_and(|link| link.protocol != REQUIRED_PROTOCOL),
            paused: link.is_some_and(|link| link.paused),
            tabs: state.tabs.len(),
            refused_origin: state.refused.clone(),
        }
    }

    /// One connection: only the Wisp extension is let in. The newest
    /// connection is the one that is used.
    async fn serve(self: Arc<Self>, stream: TcpStream) {
        let refused: Arc<Mutex<Option<String>>> = Arc::default();
        let seen = refused.clone();
        let socket = accept_hdr_async(stream, move |request: &Request, response: Response| {
            let origin = request
                .headers()
                .get("origin")
                .and_then(|value| value.to_str().ok());
            if origin == Some(EXTENSION_ORIGIN) {
                return Ok(response);
            }
            *seen.lock().unwrap() = Some(origin.unwrap_or_default().to_owned());
            let mut refusal = ErrorResponse::new(Some(
                "Wisp browser bridge accepts the Wisp Chrome extension only".into(),
            ));
            *refusal.status_mut() = StatusCode::FORBIDDEN;
            Err(refusal)
        })
        .await;
        let Ok(socket) = socket else {
            if let Some(origin) = refused.lock().unwrap().take() {
                self.state.lock().unwrap().refused = Some(origin);
            }
            return;
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        {
            let mut state = self.state.lock().unwrap();
            state.refused = None;
            for (_, waiting) in state.pending.drain() {
                let _ = waiting.send(Err("the browser extension reconnected".into()));
            }
            state.link = Some(Link {
                id,
                tx: tx.clone(),
                protocol: 0,
                version: String::new(),
                paused: false,
            });
        }
        let (mut sink, mut stream) = socket.split();
        let writer = tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });
        while let Some(Ok(message)) = stream.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(message) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            self.receive(id, &message, &tx);
        }
        writer.abort();
        let mut state = self.state.lock().unwrap();
        if state.link.as_ref().is_some_and(|link| link.id == id) {
            state.link = None;
            state.tabs.clear();
            for (_, waiting) in state.pending.drain() {
                let _ = waiting.send(Err(DISCONNECTED.into()));
            }
        }
    }

    fn receive(&self, connection: u64, message: &Value, tx: &mpsc::UnboundedSender<String>) {
        let mut state = self.state.lock().unwrap();
        if !state
            .link
            .as_ref()
            .is_some_and(|link| link.id == connection)
        {
            return;
        }
        match message.get("type").and_then(Value::as_str) {
            Some("ext_ready") => {
                state.tabs = message
                    .get("tabs")
                    .and_then(Value::as_array)
                    .map(|tabs| tabs.iter().filter_map(parse_tab).collect())
                    .unwrap_or_default();
                if let Some(link) = state.link.as_mut() {
                    link.protocol = message
                        .get("protocol_version")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    link.version = message
                        .get("extension_version")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into();
                    link.paused = message
                        .get("paused")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                }
                // A tab that is gone is nobody's any more.
                let open: Vec<i64> = state.tabs.iter().map(|tab| tab.id).collect();
                for tabs in state.owned.values_mut() {
                    tabs.retain(|tab| open.contains(tab));
                }
                state.selected.retain(|_, tab| open.contains(tab));
            }
            Some("ping") => {
                let _ = tx.send(json!({"type": "pong"}).to_string());
            }
            Some(kind @ ("result" | "error")) => {
                let Some(waiting) = message
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| state.pending.remove(id))
                else {
                    return;
                };
                let _ = waiting.send(if kind == "result" {
                    Ok(parse_reply(message))
                } else {
                    Err(error_text(message))
                });
            }
            _ => {}
        }
    }

    /// Sends one request to the extension and waits for its answer.
    async fn request(
        &self,
        tab: Option<i64>,
        code: &str,
        timeout: Duration,
    ) -> Result<Reply, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (answer, waiting) = oneshot::channel();
        {
            let mut state = self.state.lock().unwrap();
            let Some(link) = state.link.as_ref() else {
                return Err(DISCONNECTED.into());
            };
            if link.protocol != REQUIRED_PROTOCOL {
                return Err(format!(
                    "the connected browser extension (version {}) does not speak this Wisp's \
                     protocol; reload it from the folder named by browser_setup",
                    link.version
                ));
            }
            let mut payload = json!({
                "id": id,
                "code": code,
                "timeoutMs": timeout.as_millis() as u64,
            });
            if let Some(tab) = tab {
                payload["tabId"] = json!(tab);
            }
            if link.tx.send(payload.to_string()).is_err() {
                return Err(DISCONNECTED.into());
            }
            state.pending.insert(id.clone(), answer);
        }
        match tokio::time::timeout(timeout + REPLY_GRACE, waiting).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(DISCONNECTED.into()),
            Err(_) => {
                self.state.lock().unwrap().pending.remove(&id);
                Err(format!(
                    "the browser did not answer within {} ms",
                    timeout.as_millis()
                ))
            }
        }
    }

    fn adopt(&self, scope: &str, tabs: impl IntoIterator<Item = i64>) {
        let mut state = self.state.lock().unwrap();
        let owned = state.owned.entry(scope.to_owned()).or_default();
        for tab in tabs {
            if !owned.contains(&tab) {
                owned.push(tab);
            }
        }
    }

    fn owns(&self, scope: &str, tab: i64) -> bool {
        self.state
            .lock()
            .unwrap()
            .owned
            .get(scope)
            .is_some_and(|tabs| tabs.contains(&tab))
    }

    /// The tab a call works on: the one named, else the one worked on
    /// last, else the one opened last. Always one this conversation owns.
    fn target(&self, scope: &str, named: Option<i64>) -> Result<i64, String> {
        let state = self.state.lock().unwrap();
        let owned = state
            .owned
            .get(scope)
            .map(Vec::as_slice)
            .unwrap_or_default();
        match named {
            Some(tab) if owned.contains(&tab) => Ok(tab),
            Some(tab) => Err(out_of_scope(tab)),
            None => state
                .selected
                .get(scope)
                .filter(|tab| owned.contains(tab))
                .or(owned.last())
                .copied()
                .ok_or_else(|| {
                    "this conversation has no tab yet; open one with web_open_tab first".into()
                }),
        }
    }

    /// The tabs this conversation opened, as the browser last reported them.
    fn owned_tabs(&self, scope: &str) -> Value {
        let state = self.state.lock().unwrap();
        let owned = state
            .owned
            .get(scope)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let tabs: Vec<Value> = state
            .tabs
            .iter()
            .filter(|tab| owned.contains(&tab.id))
            .map(|tab| json!({"id": tab.id, "url": tab.url, "title": tab.title, "active": tab.active}))
            .collect();
        json!({
            "tabs": tabs,
            "scope": "Only the tabs this conversation opened are listed; the user's own tabs are off limits.",
        })
    }

    /// Opens a tab at `url` for the conversation `scope`.
    pub(crate) async fn open_tab(
        &self,
        scope: &str,
        url: &str,
        active: bool,
        filters: &BrowserFilters,
    ) -> Result<Value, String> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("url must be an absolute http:// or https:// address".into());
        }
        if let Some(rule) = filters.blocked(url) {
            return Err(browser_filters::block_message(url, rule));
        }
        let code = json!({"cmd": "tabs", "method": "create", "url": url, "active": active});
        let reply = self
            .request(
                None,
                &code.to_string(),
                Duration::from_millis(DEFAULT_TIMEOUT_MS),
            )
            .await?;
        let opened = reply.value.get("id").and_then(Value::as_i64);
        self.adopt(
            scope,
            opened.into_iter().chain(reply.new_tabs.iter().copied()),
        );
        if let Some(tab) = opened {
            self.state
                .lock()
                .unwrap()
                .selected
                .insert(scope.to_owned(), tab);
        }
        let mut result = json!({"tab": reply.value});
        if !filters.prefer.is_empty() {
            let preferred = filters.is_preferred(url);
            result["preferred"] = json!(preferred);
            if !preferred {
                result["prefer_hosts"] = json!(filters
                    .prefer
                    .iter()
                    .map(|rule| &rule.host)
                    .collect::<Vec<_>>());
            }
        }
        if let Some(ready) = reply.ready {
            result["ready"] = json!(ready);
        }
        if let Some(wait) = reply.wait {
            result["wait"] = wait;
        }
        Ok(result)
    }

    /// Reads a tab the conversation opened.
    pub(crate) async fn scan(
        &self,
        scope: &str,
        tab: Option<i64>,
        mode: &str,
    ) -> Result<Value, String> {
        if !matches!(mode, "default" | "text" | "article") {
            return Err("mode must be default, text or article".into());
        }
        let tab = self.target(scope, tab)?;
        let code = json!({"cmd": "scan", "mode": mode});
        let reply = self
            .request(
                Some(tab),
                &code.to_string(),
                Duration::from_millis(DEFAULT_TIMEOUT_MS),
            )
            .await?;
        self.state
            .lock()
            .unwrap()
            .selected
            .insert(scope.to_owned(), tab);
        let mut result = json!({"tab_id": tab, "page": reply.value});
        if let Some(ready) = reply.ready {
            result["ready"] = json!(ready);
        }
        if let Some(wait) = reply.wait {
            result["wait"] = wait;
        }
        Ok(result)
    }

    /// Runs a script in a tab the conversation opened, or closes tabs it
    /// opened. Other extension commands are not offered here.
    pub(crate) async fn execute(
        &self,
        scope: &str,
        tab: Option<i64>,
        script: &str,
        timeout: Duration,
        filters: &BrowserFilters,
    ) -> Result<Value, String> {
        let script = script.trim();
        if script.is_empty() {
            return Err("missing required argument 'script'".into());
        }
        if script.len() > MAX_SCRIPT_BYTES {
            return Err(format!(
                "browser script is {} bytes (maximum {MAX_SCRIPT_BYTES})",
                script.len()
            ));
        }
        if script.starts_with("window.close()") {
            return Err("window.close() cannot close ordinary browser tabs. Send \
                 {\"cmd\":\"tabs\",\"method\":\"close\",\"tabIds\":[...]} with tab ids returned \
                 by web_open_tab or web_scan."
                .into());
        }
        if let Some((url, rule)) = filters.blocked_navigation(script) {
            return Err(browser_filters::block_message(&url, rule));
        }
        if let Some(command) = command(script) {
            // A command can name any tab of the browser, so only the one
            // that is safe by construction is passed on: closing tabs
            // this conversation opened.
            let closing = command.get("cmd").and_then(Value::as_str) == Some("tabs")
                && command.get("method").and_then(Value::as_str) == Some("close");
            if !closing {
                return Err(
                    "this desktop runs page JavaScript and the tabs close command only; other \
                     extension commands (cdp, tabs create or switch, download, capture) are \
                     not offered. Use web_open_tab to open a page."
                        .into(),
                );
            }
            let ids: Vec<i64> = command
                .get("tabIds")
                .and_then(Value::as_array)
                .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            if ids.is_empty() {
                return Err("tabIds must list the tabs to close".into());
            }
            if let Some(stranger) = ids.iter().find(|tab| !self.owns(scope, **tab)) {
                return Err(out_of_scope(*stranger));
            }
            let code = json!({"cmd": "tabs", "method": "close", "tabIds": ids});
            let reply = self.request(None, &code.to_string(), timeout).await?;
            let mut state = self.state.lock().unwrap();
            if let Some(owned) = state.owned.get_mut(scope) {
                owned.retain(|tab| !ids.contains(tab));
            }
            state.selected.retain(|_, tab| !ids.contains(tab));
            // What is left is the whole browser's; only say what was closed.
            return Ok(json!({"closed": reply.value.get("closed").cloned().unwrap_or(json!(ids))}));
        }
        let tab = self.target(scope, tab)?;
        let reply = self.request(Some(tab), script, timeout).await?;
        // A page this conversation's tab opened is this conversation's too.
        self.adopt(scope, reply.new_tabs.iter().copied());
        self.state
            .lock()
            .unwrap()
            .selected
            .insert(scope.to_owned(), tab);
        let mut result = json!({"tab_id": tab, "result": reply.value});
        if !reply.new_tabs.is_empty() {
            result["new_tabs"] = json!(reply.new_tabs);
        }
        if let Some(ready) = reply.ready {
            result["ready"] = json!(ready);
        }
        if let Some(wait) = reply.wait {
            result["wait"] = wait;
        }
        Ok(result)
    }
}

fn scope(env: &dyn ToolEnv) -> String {
    env.frame_id().unwrap_or_default().to_owned()
}

fn tab_arg(args: &Value) -> Result<Option<i64>, String> {
    let Some(value) = args.get("switch_tab_id").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .map(Some)
        .ok_or_else(|| "switch_tab_id must be an integer tab id returned by web_scan".into())
}

fn finish(result: Result<Value, String>) -> ToolResult {
    match result {
        Ok(value) => ToolResult::ok(render(&value)),
        Err(problem) => ToolResult::fail(problem),
    }
}

/// Says whether the browser is reachable and how to set it up.
pub(crate) struct BrowserSetupTool {
    pub(crate) bridge: Arc<Bridge>,
    pub(crate) store: Store,
}

#[async_trait]
impl Tool for BrowserSetupTool {
    fn name(&self) -> &str {
        "browser_setup"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Report whether the Wisp Chrome extension is connected to this desktop, and how to \
             install it when it is not. Call this before the first web_open_tab of a task, and \
             when a web tool says the extension is disconnected. It changes nothing.",
            json!({"type": "object", "properties": {}}),
        )
    }

    fn read_only(&self) -> bool {
        true
    }

    fn preview(&self, _: &Value) -> String {
        "check the real-browser bridge".into()
    }

    async fn run(&self, _: &Value, _: &dyn ToolEnv) -> ToolResult {
        let status = self.bridge.status();
        let filters = browser_filters::load(&self.store).await;
        let extension = wisp_paths::browser_extension_dir();
        let mut report = json!({
            "connected": status.connected,
            "extension_version": status.extension_version,
            "update_required": status.update_required,
            "paused_by_user": status.paused,
            "extension_path": extension.map(|path| path.to_string_lossy().into_owned()),
            "url_filters": {
                "prefer": filters.prefer.iter().map(|rule| &rule.host).collect::<Vec<_>>(),
                "blocked_hosts": filters.block.len(),
            },
            "scope": "A conversation reads and drives only the tabs it opened with web_open_tab.",
        });
        if !status.connected {
            report["instruction"] = json!(DISCONNECTED);
        }
        if let Some(origin) = status.refused_origin {
            report["refused_origin"] = json!(origin);
        }
        ToolResult::ok(render(&report))
    }
}

pub(crate) struct WebOpenTabTool {
    pub(crate) bridge: Arc<Bridge>,
    pub(crate) store: Store,
}

#[async_trait]
impl Tool for WebOpenTabTool {
    fn name(&self) -> &str {
        "web_open_tab"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Open a new tab at an http(s) URL in the user's real, persistent Chrome/Chromium \
             session, with its cookies and logins. Waits until the new tab's document is \
             complete (or until timeout). The result includes the new tab id plus ready; if \
             ready is false, call web_scan before acting. Pass the tab id as switch_tab_id to \
             web_scan or web_execute_js. Hosts the user blocked in Settings, Browser are \
             refused before the tab opens. When the result lists prefer_hosts, prefer those \
             hosts for literature and similar retrieval.",
            json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "Absolute http:// or https:// URL to open"},
                    "active": {"type": "boolean", "description": "Focus the new tab (default false)"}
                },
                "required": ["url"]
            }),
        )
    }

    fn minimum_approval(&self) -> Approval {
        Approval::Ask
    }

    fn preview(&self, args: &Value) -> String {
        let url = args.get("url").and_then(Value::as_str).unwrap_or_default();
        format!("open real-browser tab at {url}")
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let Some(url) = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())
        else {
            return ToolResult::fail("missing required argument 'url'");
        };
        let active = args.get("active").and_then(Value::as_bool).unwrap_or(false);
        let filters = browser_filters::load(&self.store).await;
        finish(
            self.bridge
                .open_tab(&scope(env), url, active, &filters)
                .await,
        )
    }
}

pub(crate) struct WebScanTool {
    pub(crate) bridge: Arc<Bridge>,
}

#[async_trait]
impl Tool for WebScanTool {
    fn name(&self) -> &str {
        "web_scan"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Read visible content and actionable elements from a tab this conversation opened \
             in the user's real Chrome/Chromium session. Waits until the tab's document is \
             complete before reading (or until timeout). The result includes ready; if ready \
             is false, scan again instead of acting on a partial page. Use tabs_only to list \
             this conversation's tabs. If the page shows a human-verification challenge, do \
             not try to solve or bypass it: stop and tell the user.",
            json!({
                "type": "object",
                "properties": {
                    "tabs_only": {"type": "boolean", "description": "List this conversation's tabs without reading a page"},
                    "switch_tab_id": {"type": ["integer", "string"], "description": "Tab id returned by web_open_tab; selects that tab for this and later calls"},
                    "text_only": {"type": "boolean", "description": "Return page text without the actionable-element snapshot"},
                    "mode": {"type": "string", "description": "default | text | article. article adds images[], figures[], code_blocks[]"}
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
            "list this conversation's real-browser tabs".into()
        } else {
            match args.get("switch_tab_id").filter(|tab| !tab.is_null()) {
                Some(tab) => format!("scan real-browser tab {tab}"),
                None => "scan the selected real-browser tab".into(),
            }
        }
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let scope = scope(env);
        if args
            .get("tabs_only")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return ToolResult::ok(render(&self.bridge.owned_tabs(&scope)));
        }
        let tab = match tab_arg(args) {
            Ok(tab) => tab,
            Err(problem) => return ToolResult::fail(problem),
        };
        let text_only = args
            .get("text_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mode = match args.get("mode").and_then(Value::as_str) {
            Some(mode) => mode,
            None if text_only => "text",
            None => "default",
        };
        finish(self.bridge.scan(&scope, tab, mode).await)
    }
}

pub(crate) struct WebExecuteJsTool {
    pub(crate) bridge: Arc<Bridge>,
    pub(crate) store: Store,
}

#[async_trait]
impl Tool for WebExecuteJsTool {
    fn name(&self) -> &str {
        "web_execute_js"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Execute JavaScript in a tab this conversation opened in the user's real \
             Chrome/Chromium session. The extension waits until the tab's document is \
             complete before running the script, and waits again if the script navigates. \
             The result includes ready; if ready is false, scan again before clicking. Call \
             web_scan first and do not guess selectors. To close tabs, never call \
             window.close(); send {\"cmd\":\"tabs\",\"method\":\"close\",\"tabIds\":[...]} \
             with ids returned by web_open_tab. Other extension commands are not available. \
             Never automate a human-verification challenge.",
            json!({
                "type": "object",
                "properties": {
                    "script": {"type": "string", "description": "JavaScript to run in the page, or the tabs close command"},
                    "switch_tab_id": {"type": ["integer", "string"], "description": "Tab id returned by web_open_tab"},
                    "timeout_ms": {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_MS, "description": "Execution timeout in milliseconds (default 15000)"}
                },
                "required": ["script"]
            }),
        )
    }

    fn minimum_approval(&self) -> Approval {
        Approval::Ask
    }

    fn preview(&self, args: &Value) -> String {
        let script = args
            .get("script")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut preview: String = script.chars().take(240).collect();
        if script.chars().count() > 240 {
            preview.push('…');
        }
        preview
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let script = args
            .get("script")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tab = match tab_arg(args) {
            Ok(tab) => tab,
            Err(problem) => return ToolResult::fail(problem),
        };
        let timeout = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1, MAX_TIMEOUT_MS);
        let filters = browser_filters::load(&self.store).await;
        finish(
            self.bridge
                .execute(
                    &scope(env),
                    tab,
                    script,
                    Duration::from_millis(timeout),
                    &filters,
                )
                .await,
        )
    }
}

const ENABLED_KEY: &str = "desktop.browser.enabled";

/// The bridge of one host: off until the desktop switches it on.
pub(crate) struct Slot {
    bridge: Mutex<Option<Arc<Bridge>>>,
    /// Why it is on but not listening.
    problem: Mutex<Option<String>>,
    addr: Mutex<String>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            bridge: Mutex::default(),
            problem: Mutex::default(),
            addr: Mutex::new(BRIDGE_ADDR.into()),
        }
    }
}

impl Slot {
    pub(crate) fn bridge(&self) -> Option<Arc<Bridge>> {
        self.bridge.lock().unwrap().clone()
    }

    pub(crate) fn stop(&self) {
        if let Some(bridge) = self.bridge.lock().unwrap().take() {
            bridge.stop();
        }
        *self.problem.lock().unwrap() = None;
    }

    /// Listens somewhere else than the extension's own port, for a test.
    #[cfg(test)]
    pub(crate) fn listen_on(&self, addr: &str) {
        *self.addr.lock().unwrap() = addr.into();
    }
}

/// The browser tools as Settings shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserSettings {
    /// Conversations are offered the browser tools.
    pub enabled: bool,
    /// The bridge is listening for the extension.
    pub listening: bool,
    /// Why it is on but not listening.
    pub problem: Option<String>,
    pub status: BrowserStatus,
    /// The folder to load in the browser as an unpacked extension.
    pub extension_path: Option<String>,
    pub filters: BrowserFilters,
}

impl crate::Server {
    async fn browser_enabled(&self) -> bool {
        self.0
            .store
            .get_setting(ENABLED_KEY)
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some("true")
    }

    /// The browser tools as they stand.
    pub async fn browser_settings(&self) -> Result<BrowserSettings> {
        // Read before anything is awaited: a lock is not held across it.
        let bridge = self.0.browser.bridge();
        let problem = self.0.browser.problem.lock().unwrap().clone();
        let status = bridge
            .as_ref()
            .map(|bridge| bridge.status())
            .unwrap_or_default();
        let enabled = self.browser_enabled().await;
        let filters = browser_filters::load(&self.0.store).await;
        Ok(BrowserSettings {
            enabled,
            listening: bridge.is_some(),
            problem,
            status,
            extension_path: wisp_paths::browser_extension_dir()
                .map(|path| path.to_string_lossy().into_owned()),
            filters,
        })
    }

    /// Switches the browser tools on or off. On, the bridge listens on
    /// this computer only and conversations get the tools from their
    /// next turn.
    pub async fn set_browser_enabled(&self, enabled: bool) -> Result<BrowserSettings> {
        self.0
            .store
            .set_setting(ENABLED_KEY, if enabled { "true" } else { "false" })
            .await?;
        if enabled {
            self.start_browser().await;
        } else {
            self.0.browser.stop();
        }
        self.browser_settings().await
    }

    /// Keeps the lists of blocked and preferred hosts.
    pub async fn save_browser_filters(&self, filters: BrowserFilters) -> Result<BrowserSettings> {
        browser_filters::save(&self.0.store, filters).await?;
        self.browser_settings().await
    }

    /// Starts the bridge when the browser tools are on. The desktop
    /// calls this once it has opened the host.
    pub async fn start_browser(&self) {
        let slot = &self.0.browser;
        if !self.browser_enabled().await || slot.bridge().is_some() {
            return;
        }
        let addr = slot.addr.lock().unwrap().clone();
        match Bridge::start(&addr).await {
            Ok(bridge) => {
                *slot.bridge.lock().unwrap() = Some(bridge);
                *slot.problem.lock().unwrap() = None;
            }
            Err(error) => *slot.problem.lock().unwrap() = Some(format!("{error:#}")),
        }
    }
}

/// The tools a conversation gets while the bridge is on.
pub(crate) fn tools(bridge: &Arc<Bridge>, store: &Store) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(BrowserSetupTool {
            bridge: bridge.clone(),
            store: store.clone(),
        }),
        Box::new(WebOpenTabTool {
            bridge: bridge.clone(),
            store: store.clone(),
        }),
        Box::new(WebScanTool {
            bridge: bridge.clone(),
        }),
        Box::new(WebExecuteJsTool {
            bridge: bridge.clone(),
            store: store.clone(),
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// Connects the way the extension does, from `origin`.
    async fn connect(bridge: &Bridge, origin: &str) -> Result<Socket, String> {
        let mut request = format!("ws://{}", bridge.addr())
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        tokio_tungstenite::connect_async(request)
            .await
            .map(|(socket, _)| socket)
            .map_err(|error| error.to_string())
    }

    async fn hello(socket: &mut Socket, tabs: Value) {
        let ready = json!({
            "type": "ext_ready", "protocol_version": 2, "extension_version": "0.5.0",
            "session": "shared", "paused": false, "tabs": tabs,
        });
        socket
            .send(Message::Text(ready.to_string().into()))
            .await
            .unwrap();
    }

    /// The next request the bridge sent, as the extension would read it.
    async fn asked(socket: &mut Socket) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value.get("id").is_some() {
                        return value;
                    }
                }
            }
        })
        .await
        .expect("the bridge sends a request")
    }

    async fn answer(socket: &mut Socket, request: &Value, extra: Value) {
        let mut reply = json!({"type": "result", "id": request["id"], "ready": true});
        for (key, value) in extra.as_object().unwrap() {
            reply[key] = value.clone();
        }
        socket
            .send(Message::Text(reply.to_string().into()))
            .await
            .unwrap();
    }

    async fn settled(bridge: &Bridge, ready: impl Fn(&BrowserStatus) -> bool) -> BrowserStatus {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let status = bridge.status();
                if ready(&status) {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the bridge reaches the state")
    }

    #[tokio::test]
    async fn only_the_wisp_extension_is_let_in() {
        let bridge = Bridge::start("127.0.0.1:0").await.unwrap();
        assert_eq!(bridge.status(), BrowserStatus::default());
        // A web page, another extension and a client with no origin at all.
        for origin in ["https://example.com", "chrome-extension://someoneelse"] {
            assert!(connect(&bridge, origin).await.is_err());
            let status = settled(&bridge, |status| status.refused_origin.is_some()).await;
            assert_eq!(status.refused_origin.as_deref(), Some(origin));
            assert!(!status.connected);
        }
        assert!(
            tokio_tungstenite::connect_async(format!("ws://{}", bridge.addr()))
                .await
                .is_err()
        );

        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        hello(
            &mut extension,
            json!([{"id": 7, "url": "https://bank.example", "title": "Bank"}]),
        )
        .await;
        let status = settled(&bridge, |status| status.tabs == 1).await;
        assert!(status.connected && !status.update_required && !status.paused);
        assert_eq!(status.extension_version, "0.5.0");
        assert_eq!(status.refused_origin, None);

        // Its keep-alive is answered.
        extension
            .send(Message::Text(json!({"type": "ping"}).to_string().into()))
            .await
            .unwrap();
        let pong = tokio::time::timeout(Duration::from_secs(5), extension.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            pong,
            Message::Text(json!({"type": "pong"}).to_string().into())
        );

        // Gone, the bridge says so and a request fails with what to do.
        drop(extension);
        settled(&bridge, |status| !status.connected).await;
        let failed = bridge
            .open_tab(
                "c",
                "https://example.org",
                false,
                &BrowserFilters::default(),
            )
            .await
            .unwrap_err();
        assert!(failed.contains("not connected"), "{failed}");
        bridge.stop();
    }

    #[tokio::test]
    async fn a_conversation_only_touches_the_tabs_it_opened() {
        let bridge = Bridge::start("127.0.0.1:0").await.unwrap();
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        // The user has their bank open.
        let bank = json!({"id": 7, "url": "https://bank.example", "title": "Bank"});
        hello(&mut extension, json!([bank])).await;
        settled(&bridge, |status| status.tabs == 1).await;
        let filters = BrowserFilters {
            block: vec![crate::browser_filters::BrowserHostRule {
                host: "blocked.example".into(),
                reason: "no".into(),
            }],
            prefer: vec![crate::browser_filters::BrowserHostRule {
                host: "pubmed.example".into(),
                reason: String::new(),
            }],
        };

        // Nothing is this conversation's yet: the user's tab is refused
        // and never asked about.
        assert!(bridge
            .scan("c", Some(7), "default")
            .await
            .unwrap_err()
            .contains("not opened"));
        assert!(bridge
            .scan("c", None, "default")
            .await
            .unwrap_err()
            .contains("no tab yet"));
        assert_eq!(bridge.owned_tabs("c")["tabs"], json!([]));
        // A blocked host and something that is no web address are refused
        // before the browser hears of them.
        for url in ["https://www.blocked.example/x", "file:///etc/passwd"] {
            assert!(bridge.open_tab("c", url, false, &filters).await.is_err());
        }

        // Opening a tab makes it the conversation's.
        let opening = {
            let bridge = bridge.clone();
            let filters = filters.clone();
            tokio::spawn(async move {
                bridge
                    .open_tab("c", "https://example.org/paper", false, &filters)
                    .await
            })
        };
        let request = asked(&mut extension).await;
        assert!(request.get("tabId").is_none());
        let sent: Value = serde_json::from_str(request["code"].as_str().unwrap()).unwrap();
        assert_eq!(
            sent,
            json!({"cmd": "tabs", "method": "create", "url": "https://example.org/paper", "active": false})
        );
        let paper = json!({"id": 21, "url": "https://example.org/paper", "title": "Paper"});
        answer(&mut extension, &request, json!({"result": paper})).await;
        let opened = opening.await.unwrap().unwrap();
        assert_eq!(opened["tab"]["id"], 21);
        assert_eq!(opened["preferred"], false);
        assert_eq!(opened["prefer_hosts"], json!(["pubmed.example"]));
        hello(&mut extension, json!([bank, paper])).await;
        settled(&bridge, |status| status.tabs == 2).await;
        assert_eq!(bridge.owned_tabs("c")["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(bridge.owned_tabs("other")["tabs"], json!([]));

        // It is read without naming it, and another conversation cannot.
        let reading = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.scan("c", None, "text").await })
        };
        let request = asked(&mut extension).await;
        assert_eq!(request["tabId"], 21);
        assert_eq!(
            serde_json::from_str::<Value>(request["code"].as_str().unwrap()).unwrap(),
            json!({"cmd": "scan", "mode": "text"})
        );
        answer(
            &mut extension,
            &request,
            json!({"result": {"text": "Abstract"}}),
        )
        .await;
        let page = reading.await.unwrap().unwrap();
        assert_eq!(page["page"]["text"], "Abstract");
        assert_eq!(page["ready"], true);
        assert!(bridge.scan("other", Some(21), "default").await.is_err());
        assert!(bridge.scan("c", None, "everything").await.is_err());

        // A script runs in it; a page it opens is the conversation's too.
        let running = {
            let bridge = bridge.clone();
            let filters = filters.clone();
            tokio::spawn(async move {
                bridge
                    .execute(
                        "c",
                        None,
                        " document.title ",
                        Duration::from_secs(2),
                        &filters,
                    )
                    .await
            })
        };
        let request = asked(&mut extension).await;
        assert_eq!(
            (request["tabId"].as_i64(), request["code"].as_str()),
            (Some(21), Some("document.title"))
        );
        assert_eq!(request["timeoutMs"], 2_000);
        answer(
            &mut extension,
            &request,
            json!({"result": "Paper", "newTabs": [{"id": 22, "url": "https://example.org/pdf"}]}),
        )
        .await;
        let ran = running.await.unwrap().unwrap();
        assert_eq!(ran["result"], "Paper");
        assert_eq!(ran["new_tabs"], json!([22]));
        assert!(bridge.owns("c", 22));

        // What a script may not do is refused before the browser hears of it.
        let short = Duration::from_millis(200);
        for script in [
            "",
            "window.close()",
            r#"location.href = "https://blocked.example/""#,
            r#"{"cmd":"cdp","method":"Page.navigate","params":{"url":"https://example.org"},"tabId":7}"#,
            r#"{"cmd":"tabs","method":"switch","tabId":7}"#,
            r#"{"cmd":"tabs","method":"close","tabIds":[7]}"#,
            r#"{"cmd":"tabs","method":"close","tabIds":[21,7]}"#,
            r#"{"cmd":"tabs","method":"close"}"#,
        ] {
            let refused = bridge.execute("c", None, script, short, &filters).await;
            assert!(refused.is_err(), "{script}");
        }
        assert!(bridge
            .execute("c", Some(7), "1", short, &filters)
            .await
            .is_err());
        let big = "x".repeat(MAX_SCRIPT_BYTES + 1);
        assert!(bridge
            .execute("c", None, &big, short, &filters)
            .await
            .is_err());

        // An error of the extension is passed on with its code.
        let failing = {
            let bridge = bridge.clone();
            let filters = filters.clone();
            tokio::spawn(async move {
                bridge
                    .execute("c", Some(21), "boom()", short, &filters)
                    .await
            })
        };
        let request = asked(&mut extension).await;
        assert_eq!(request["code"], "boom()");
        let error = json!({
            "type": "error", "id": request["id"],
            "error": {"code": "USER_CONTROLLING", "message": "user paused browser control"}
        });
        extension
            .send(Message::Text(error.to_string().into()))
            .await
            .unwrap();
        assert_eq!(
            failing.await.unwrap().unwrap_err(),
            "USER_CONTROLLING: user paused browser control"
        );

        // Its own tabs are closed by id, and are then no longer its.
        let closing = {
            let bridge = bridge.clone();
            let filters = filters.clone();
            tokio::spawn(async move {
                let script = r#"{"cmd":"tabs","method":"close","tabIds":[21,22]}"#;
                bridge
                    .execute("c", None, script, Duration::from_secs(2), &filters)
                    .await
            })
        };
        let request = asked(&mut extension).await;
        assert!(request.get("tabId").is_none());
        answer(
            &mut extension,
            &request,
            json!({"result": {"closed": [21, 22], "remaining": [bank]}}),
        )
        .await;
        let closed = closing.await.unwrap().unwrap();
        // What else is open in the browser is not reported back.
        assert_eq!(closed, json!({"closed": [21, 22]}));
        assert!(!bridge.owns("c", 21) && !bridge.owns("c", 22));

        bridge.stop();
        assert!(bridge.status() == BrowserStatus::default());
    }

    #[tokio::test]
    async fn an_extension_of_another_protocol_is_shown_but_not_used() {
        let bridge = Bridge::start("127.0.0.1:0").await.unwrap();
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        let old = json!({"type": "ext_ready", "protocol_version": 1, "extension_version": "0.1.0", "tabs": []});
        extension
            .send(Message::Text(old.to_string().into()))
            .await
            .unwrap();
        let status = settled(&bridge, |status| status.extension_version == "0.1.0").await;
        assert!(status.connected && status.update_required);
        let refused = bridge
            .open_tab(
                "c",
                "https://example.org",
                false,
                &BrowserFilters::default(),
            )
            .await
            .unwrap_err();
        assert!(refused.contains("protocol"), "{refused}");
        bridge.stop();
    }
}
