//! The real-browser bridge: a loopback listener the Wisp Chrome extension
//! connects to, and the tools that open a tab, read a page, run a script and
//! take a screenshot in the user's own browser session.
//!
//! The extension runs in the user's existing profile, so pages keep its
//! cookies and logins; no separate automation browser is started. The
//! wire protocol is the extension's (`browser-extension/`), as spoken by
//! `src-tauri/src/browser_bridge`. The rules both desktops share (the two
//! Browser settings, the human-verification handoff, the extension's
//! version check, its managed copy and how the user's browser is started)
//! come from `wisp_runs::browser`. This is a smaller bridge than the Tauri
//! shell's: one session, and a conversation may only touch tabs it opened
//! itself, so pages the user has open are never read or driven.
//!
//! What it keeps for the app: the tabs each turn opened, offered for
//! closing (or closed, with **Automatically close browser tabs**) when the
//! turn ends, and the tabs whose page waits for a human to complete a
//! verification challenge.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
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
use sha2::{Digest, Sha256};
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
use wisp_dto::{
    BrowserExtensionSetup, BrowserExtensionStatus, BrowserExtensionUpdateResult,
    BrowserNeedsHumanConfirmResult, BrowserNeedsHumanTab, BrowserTabCleanupItem,
    BrowserTabCleanupPrompt,
};
use wisp_llm::ToolSchema;
use wisp_runs::browser::{
    self as shared, extension, human_verification_handoff, needs_human_block_message, EXTENSION_ID,
    EXTENSION_ORIGIN, REQUIRED_PROTOCOL,
};
use wisp_store::Store;
use wisp_tools::{Approval, ImageData, Tool, ToolEnv, ToolResult};

use crate::browser_filters::{self, BrowserFilters};

/// Where the extension looks for Wisp.
pub const BRIDGE_ADDR: &str = "127.0.0.1:18765";
/// The name the WebView gives its shared browser, which is this bridge's
/// only one; tabs in prompts carry it.
const LANE: &str = "shared";
const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const MAX_TIMEOUT_MS: u64 = 60_000;
/// How long a browser that was started for a tool is waited for.
const AUTO_LAUNCH_WAIT: Duration = Duration::from_secs(15);
/// Longer than the extension's own budget, so its answer wins the race.
const REPLY_GRACE: Duration = Duration::from_secs(5);
const MAX_SCRIPT_BYTES: usize = 64 * 1024;
const MAX_RESULT_CHARS: usize = 200_000;
/// Base64 ceiling of one screenshot: the shared image path's 5 MB decoded.
const MAX_SCREENSHOT_B64: usize = 7 * 1024 * 1024;
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
    capabilities: Vec<String>,
}

/// What the extension last said of itself. Kept after it disconnects, so
/// the update banner can say which version it is waiting to see replaced.
#[derive(Clone, Debug, Default)]
struct Seen {
    protocol: i64,
    version: String,
    capabilities: Vec<String>,
}

/// What the extension answered.
struct Reply {
    value: Value,
    ready: Option<bool>,
    wait: Option<Value>,
    /// Tabs that came into being while the request ran.
    new_tabs: Vec<i64>,
}

/// A tab a turn opened, as it was opened and as it was last seen.
#[derive(Clone, Debug)]
struct Tracked {
    tab: i64,
    initial_url: String,
    url: String,
    title: String,
}

impl Tracked {
    fn item(&self) -> BrowserTabCleanupItem {
        BrowserTabCleanupItem {
            session: LANE.into(),
            tab_id: self.tab,
            url: self.url.clone(),
            title: self.title.clone(),
            initial_url: self.initial_url.clone(),
        }
    }
}

/// The tabs one turn opened, and the conversation it is.
#[derive(Debug, Default)]
struct Ledger {
    frame: String,
    tabs: Vec<Tracked>,
}

/// A turn's tabs waiting for the user's choice, or for the extension to be
/// back so they can be closed as the setting says.
#[derive(Clone, Debug)]
struct Pending {
    prompt: BrowserTabCleanupPrompt,
    auto_close: bool,
}

/// What `complete_turn` did with the tabs the turn opened.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum TabCleanup {
    None,
    Closed,
    Prompt(BrowserTabCleanupPrompt),
}

#[derive(Default)]
struct State {
    /// The origin of the last connection that was turned away.
    refused: Option<String>,
    link: Option<Link>,
    seen: Option<Seen>,
    tabs: Vec<Tab>,
    pending: HashMap<String, oneshot::Sender<Result<Reply, String>>>,
    /// The tabs each conversation opened, oldest first.
    owned: HashMap<String, Vec<i64>>,
    /// The tab each conversation last worked on.
    selected: HashMap<String, i64>,
    /// The tabs each turn opened, until the turn ends.
    ledgers: HashMap<String, Ledger>,
    /// Ended turns' tabs to close or keep, oldest first.
    cleanups: Vec<Pending>,
    /// Tabs whose page waits for a human, by tab.
    needs_human: BTreeMap<i64, BrowserNeedsHumanTab>,
}

/// The bridge as Settings shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserStatus {
    /// The extension is connected.
    pub connected: bool,
    pub extension_version: String,
    /// The connected extension has to be updated: an older protocol, no
    /// capabilities reported, or older than the copy this app ships.
    pub update_required: bool,
    /// The user paused browser control from the extension's popup.
    pub paused: bool,
    /// Pages open in the browser, the user's own included.
    pub tabs: usize,
    /// Something other than the Wisp extension tried to connect.
    pub refused_origin: Option<String>,
}

/// Starts the user's own browser, on `url` or its new-tab page.
pub(crate) type LaunchBrowser = Arc<dyn Fn(Option<String>) -> Result<(), String> + Send + Sync>;
/// Opens the user's browser on its extension-manager page; whether it did.
pub(crate) type OpenExtensionPage = Arc<dyn Fn() -> bool + Send + Sync>;

/// How the host's own browser is started. None in a test, which must not
/// start one.
#[derive(Clone)]
pub(crate) struct Launcher {
    pub(crate) browser: LaunchBrowser,
    pub(crate) extension_page: OpenExtensionPage,
}

impl Launcher {
    fn system() -> Self {
        Self {
            browser: Arc::new(|url| shared::launch::spawn_user_browser(url.as_deref())),
            extension_page: Arc::new(shared::launch::open_extension_page),
        }
    }
}

/// The copy of the extension the app ships, and the managed copy in the
/// app's data folder that the user loads in the browser.
#[derive(Clone, Debug, Default)]
pub(crate) struct ExtensionDirs {
    pub(crate) bundled: Option<PathBuf>,
    pub(crate) managed: Option<PathBuf>,
}

impl ExtensionDirs {
    fn bundled_version(&self) -> Option<String> {
        self.bundled
            .as_deref()
            .and_then(shared::bundled_manifest_version)
    }

    /// Refreshes the managed copy from the bundled one.
    fn sync(&self) -> Result<(), String> {
        match (&self.bundled, &self.managed) {
            (Some(bundled), Some(managed)) => {
                extension::sync(bundled, managed, EXTENSION_ID).map(|_| ())
            }
            (None, _) => Err("this app has no bundled browser extension".into()),
            (Some(_), None) => Ok(()),
        }
    }

    /// The folder to load in the browser, once it is verified to be the
    /// copy this app ships.
    fn verified_path(&self) -> Option<String> {
        let bundled = self.bundled.as_deref()?;
        let folder = match self.managed.as_deref() {
            Some(managed) => {
                extension::verify(bundled, managed, EXTENSION_ID).ok()?;
                managed
            }
            None => {
                extension::inspect(bundled, EXTENSION_ID).ok()?;
                bundled
            }
        };
        dunce::canonicalize(folder)
            .ok()
            .map(|dir| dir.display().to_string())
    }

    /// The verified folder, refreshing the managed copy first when it is
    /// missing or out of date.
    fn prepared_path(&self) -> Option<String> {
        self.verified_path().or_else(|| {
            let _ = self.sync();
            self.verified_path()
        })
    }
}

/// What the bridge is started with besides its address.
#[derive(Clone, Default)]
pub(crate) struct Options {
    /// Where the Browser settings are kept.
    pub(crate) store: Option<Store>,
    pub(crate) extension: ExtensionDirs,
    pub(crate) launcher: Option<Launcher>,
    /// How long a started browser is waited for.
    pub(crate) launch_wait: Duration,
}

pub(crate) struct Bridge {
    state: Mutex<State>,
    next: AtomicU64,
    options: Options,
    bundled_version: Option<String>,
    /// One start of the user's browser at a time.
    launching: tokio::sync::Mutex<()>,
    /// One extension update at a time.
    updating: tokio::sync::Mutex<()>,
    /// Why the last refresh of the managed copy failed.
    extension_error: Mutex<Option<String>>,
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

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

impl Bridge {
    /// Starts listening on `addr`. Fails when the port is taken, which is
    /// what happens while another Wisp holds the bridge.
    #[cfg(test)]
    pub(crate) async fn start(addr: &str) -> Result<Arc<Self>> {
        Self::start_with(addr, Options::default()).await
    }

    pub(crate) async fn start_with(addr: &str, options: Options) -> Result<Arc<Self>> {
        let listener = TcpListener::bind(addr).await.with_context(|| {
            format!("cannot listen on {addr}; quit any other Wisp that is using the browser bridge")
        })?;
        let bridge = Arc::new(Self {
            state: Mutex::default(),
            next: AtomicU64::new(1),
            bundled_version: options.extension.bundled_version(),
            options,
            launching: tokio::sync::Mutex::new(()),
            updating: tokio::sync::Mutex::new(()),
            extension_error: Mutex::default(),
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

    /// Stops listening and drops the extension, and with it what the app
    /// was to be asked about tabs it can no longer reach.
    pub(crate) fn stop(&self) {
        if let Some(task) = self.accepting.lock().unwrap().take() {
            task.abort();
        }
        let mut state = self.state.lock().unwrap();
        state.link = None;
        state.seen = None;
        state.tabs.clear();
        state.ledgers.clear();
        state.cleanups.clear();
        state.needs_human.clear();
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
            update_required: link.is_some_and(|link| {
                shared::extension_requires_update(
                    link.protocol,
                    &link.capabilities,
                    &link.version,
                    self.bundled_version.as_deref(),
                )
            }),
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
                capabilities: Vec::new(),
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
            if self.receive(id, &message, &tx) {
                // Tabs the setting closes but that could not be closed while
                // the extension was away are closed now that it is back.
                let bridge = self.clone();
                tokio::spawn(async move { bridge.retry_auto_close().await });
            }
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

    /// Takes one message of the connection. True when it was the
    /// extension's hello.
    fn receive(
        &self,
        connection: u64,
        message: &Value,
        tx: &mpsc::UnboundedSender<String>,
    ) -> bool {
        let mut state = self.state.lock().unwrap();
        if !state
            .link
            .as_ref()
            .is_some_and(|link| link.id == connection)
        {
            return false;
        }
        match message.get("type").and_then(Value::as_str) {
            Some("ext_ready") => {
                state.tabs = message
                    .get("tabs")
                    .and_then(Value::as_array)
                    .map(|tabs| tabs.iter().filter_map(parse_tab).collect())
                    .unwrap_or_default();
                let seen = Seen {
                    protocol: message
                        .get("protocol_version")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    version: text(message, "extension_version"),
                    capabilities: message
                        .get("capabilities")
                        .and_then(Value::as_array)
                        .map(|caps| {
                            caps.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                if let Some(link) = state.link.as_mut() {
                    link.protocol = seen.protocol;
                    link.version = seen.version.clone();
                    link.capabilities = seen.capabilities.clone();
                    link.paused = message
                        .get("paused")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                }
                state.seen = Some(seen);
                // A tab that is gone is nobody's any more, and nothing is
                // asked about it.
                let open: HashSet<i64> = state.tabs.iter().map(|tab| tab.id).collect();
                for tabs in state.owned.values_mut() {
                    tabs.retain(|tab| open.contains(tab));
                }
                state.selected.retain(|_, tab| open.contains(tab));
                state.needs_human.retain(|tab, _| open.contains(tab));
                for pending in &mut state.cleanups {
                    pending.prompt.tabs.retain(|tab| open.contains(&tab.tab_id));
                }
                state
                    .cleanups
                    .retain(|pending| !pending.prompt.tabs.is_empty());
                true
            }
            Some("ping") => {
                let _ = tx.send(json!({"type": "pong"}).to_string());
                false
            }
            Some(kind @ ("result" | "error")) => {
                let Some(waiting) = message
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| state.pending.remove(id))
                else {
                    return false;
                };
                let _ = waiting.send(if kind == "result" {
                    Ok(parse_reply(message))
                } else {
                    Err(error_text(message))
                });
                false
            }
            _ => false,
        }
    }

    fn connected(&self) -> bool {
        self.state.lock().unwrap().link.is_some()
    }

    /// With **Open browser automatically** on and the extension away,
    /// starts the user's own browser so the extension it has installed can
    /// connect, and waits a while for it. Never in a test that did not hand
    /// the bridge a launcher of its own.
    async fn ensure_extension(&self, url: Option<&str>) {
        if self.connected() {
            return;
        }
        let (Some(launcher), Some(store)) = (&self.options.launcher, &self.options.store) else {
            return;
        };
        if !shared::auto_launch_enabled(store).await {
            return;
        }
        let _one = self.launching.lock().await;
        if self.connected() {
            return;
        }
        if let Err(error) = (launcher.browser)(url.map(str::to_owned)) {
            tracing::warn!(target: "wisp", "browser auto-launch failed: {error}");
            return;
        }
        let deadline = tokio::time::Instant::now() + self.options.launch_wait;
        while tokio::time::Instant::now() < deadline {
            if self.connected() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Sends one request to the extension and waits for its answer.
    async fn request(
        &self,
        tab: Option<i64>,
        code: &str,
        timeout: Duration,
    ) -> Result<Reply, String> {
        self.ensure_extension(None).await;
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

    /// Refuses a command the connected extension cannot carry out.
    fn require_capability(&self, capability: &str) -> Result<(), String> {
        let state = self.state.lock().unwrap();
        let Some(link) = state.link.as_ref() else {
            return Err(DISCONNECTED.into());
        };
        if link.capabilities.iter().any(|have| have == capability) {
            return Ok(());
        }
        Err(format!(
            "the connected browser extension (version {}) cannot do {capability}; update it \
             (browser_setup with action=update_extension) and try again",
            link.version
        ))
    }

    /// Makes `tabs` the conversation `scope`'s, and the turn's to clean up.
    fn adopt(&self, scope: &str, turn: &str, tabs: &[(i64, Option<&Value>)]) {
        let mut state = self.state.lock().unwrap();
        let owned = state.owned.entry(scope.to_owned()).or_default();
        for (tab, _) in tabs {
            if !owned.contains(tab) {
                owned.push(*tab);
            }
        }
        // A tab the extension just made is open, before its next hello
        // says so.
        for (_, value) in tabs {
            if let Some(made) = value.and_then(parse_tab) {
                if !state.tabs.iter().any(|tab| tab.id == made.id) {
                    state.tabs.push(made);
                }
            }
        }
        if turn.is_empty() {
            return;
        }
        let known: HashMap<i64, (String, String)> = state
            .tabs
            .iter()
            .map(|tab| (tab.id, (tab.url.clone(), tab.title.clone())))
            .collect();
        let ledger = state.ledgers.entry(turn.to_owned()).or_default();
        if ledger.frame.is_empty() {
            ledger.frame = scope.to_owned();
        }
        for (tab, value) in tabs {
            if ledger.tabs.iter().any(|tracked| tracked.tab == *tab) {
                continue;
            }
            let (url, title) = match value {
                Some(value) => (text(value, "url"), text(value, "title")),
                None => known.get(tab).cloned().unwrap_or_default(),
            };
            ledger.tabs.push(Tracked {
                tab: *tab,
                initial_url: url.clone(),
                url,
                title,
            });
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

    fn select(&self, scope: &str, tab: i64) {
        self.state
            .lock()
            .unwrap()
            .selected
            .insert(scope.to_owned(), tab);
    }

    fn waits_for_human(&self, tab: i64) -> bool {
        self.state.lock().unwrap().needs_human.contains_key(&tab)
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

    /// Opens a tab at `url` for the conversation `scope`, in its turn `turn`.
    pub(crate) async fn open_tab(
        &self,
        scope: &str,
        turn: &str,
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
        let mut tabs: Vec<(i64, Option<&Value>)> = opened
            .map(|tab| (tab, Some(&reply.value)))
            .into_iter()
            .collect();
        tabs.extend(reply.new_tabs.iter().map(|tab| (*tab, None)));
        self.adopt(scope, turn, &tabs);
        if let Some(tab) = opened {
            self.select(scope, tab);
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

    /// Reads a tab the conversation opened. A page that asks for a human is
    /// marked so, and the result says to stop.
    pub(crate) async fn scan(
        &self,
        scope: &str,
        turn: &str,
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
        self.select(scope, tab);
        self.apply_human_verification(tab, &reply.value, scope, turn);
        let mut result = json!({
            "human_intervention": human_verification_handoff(&reply.value),
            "tab_id": tab,
            "page": reply.value,
        });
        if let Some(ready) = reply.ready {
            result["ready"] = json!(ready);
        }
        if let Some(wait) = reply.wait {
            result["wait"] = wait;
        }
        Ok(result)
    }

    /// Runs a script in a tab the conversation opened, or closes tabs it
    /// opened. Other extension commands are not offered here. A tab that
    /// waits for a human is left alone.
    pub(crate) async fn execute(
        &self,
        scope: &str,
        turn: &str,
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
            if ids.iter().any(|tab| self.waits_for_human(*tab)) {
                return Err(needs_human_block_message());
            }
            let closed = self.close_tabs(&ids, timeout).await?;
            // What is left is the whole browser's; only say what was closed.
            return Ok(json!({"closed": closed}));
        }
        let tab = self.target(scope, tab)?;
        if self.waits_for_human(tab) {
            return Err(needs_human_block_message());
        }
        let reply = self.request(Some(tab), script, timeout).await?;
        // A page this conversation's tab opened is this conversation's too,
        // and its turn's to clean up.
        let popups: Vec<(i64, Option<&Value>)> =
            reply.new_tabs.iter().map(|tab| (*tab, None)).collect();
        self.adopt(scope, turn, &popups);
        self.select(scope, tab);
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

    /// Captures what a tab the conversation opened shows: the viewport as a
    /// JPEG, or with `png` the extension's PNG capture (the whole page, or
    /// one element). The image's base64.
    pub(crate) async fn screenshot(
        &self,
        scope: &str,
        tab: Option<i64>,
        png: Option<(bool, Option<&str>)>,
    ) -> Result<(i64, String), String> {
        let code = match png {
            Some((full_page, selector)) => {
                self.require_capability(if selector.is_some() {
                    "selector_screenshot"
                } else {
                    "full_page_screenshot"
                })?;
                json!({"cmd": "capture", "full_page": full_page, "selector": selector, "format": "png"})
            }
            None => json!({
                "cmd": "cdp",
                "method": "Page.captureScreenshot",
                "params": {"format": "jpeg", "quality": 80}
            }),
        };
        // The command is the bridge's own, sent to a tab this conversation
        // owns; the model cannot send either command itself.
        let tab = self.target(scope, tab)?;
        let reply = self
            .request(
                Some(tab),
                &code.to_string(),
                Duration::from_millis(DEFAULT_TIMEOUT_MS),
            )
            .await?;
        self.select(scope, tab);
        let data = reply
            .value
            .get("data")
            .and_then(Value::as_str)
            .filter(|data| !data.is_empty())
            .ok_or("browser screenshot returned no image data")?;
        if data.len() > MAX_SCREENSHOT_B64 {
            return Err(format!(
                "browser screenshot is too large ({} bytes of base64); reduce the browser window \
                 size and retry",
                data.len()
            ));
        }
        Ok((tab, data.to_owned()))
    }

    /// Closes tabs Wisp opened and forgets them everywhere.
    async fn close_tabs(&self, ids: &[i64], timeout: Duration) -> Result<Vec<i64>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let code = json!({"cmd": "tabs", "method": "close", "tabIds": ids});
        let reply = self.request(None, &code.to_string(), timeout).await?;
        let closed: Vec<i64> = reply
            .value
            .get("closed")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_else(|| ids.to_vec());
        let mut state = self.state.lock().unwrap();
        state.tabs.retain(|tab| !closed.contains(&tab.id));
        for owned in state.owned.values_mut() {
            owned.retain(|tab| !closed.contains(tab));
        }
        state.selected.retain(|_, tab| !closed.contains(tab));
        for ledger in state.ledgers.values_mut() {
            ledger.tabs.retain(|tracked| !closed.contains(&tracked.tab));
        }
        for pending in &mut state.cleanups {
            pending
                .prompt
                .tabs
                .retain(|tab| !closed.contains(&tab.tab_id));
        }
        state
            .cleanups
            .retain(|pending| !pending.prompt.tabs.is_empty());
        state.needs_human.retain(|tab, _| !closed.contains(tab));
        Ok(closed)
    }

    /// Marks a tab as waiting for a human when its page asks for one, and
    /// clears it when the page no longer does.
    fn apply_human_verification(&self, tab: i64, page: &Value, frame: &str, turn: &str) {
        let mut state = self.state.lock().unwrap();
        let Some(handoff) = human_verification_handoff(page) else {
            state.needs_human.remove(&tab);
            return;
        };
        let entry = state
            .needs_human
            .entry(tab)
            .or_insert_with(|| BrowserNeedsHumanTab {
                session: LANE.into(),
                tab_id: tab,
                frame_id: frame.to_owned(),
                turn_id: turn.to_owned(),
                ..Default::default()
            });
        let (url, title) = (text(page, "url"), text(page, "title"));
        if !url.is_empty() {
            entry.url = url;
        }
        if !title.is_empty() {
            entry.title = title;
        }
        entry.reason = handoff
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("captcha_challenge")
            .to_owned();
        if entry.frame_id.is_empty() {
            entry.frame_id = frame.to_owned();
        }
        if !turn.is_empty() {
            entry.turn_id = turn.to_owned();
        }
    }

    /// The tabs that wait for a human, in tab order.
    pub(crate) fn needs_human(&self) -> Vec<BrowserNeedsHumanTab> {
        self.state
            .lock()
            .unwrap()
            .needs_human
            .values()
            .cloned()
            .collect()
    }

    /// Looks again at the tabs the user says they verified (all of them when
    /// none is named). Only tabs the bridge marked are looked at.
    pub(crate) async fn confirm_needs_human(
        &self,
        tabs: &[BrowserNeedsHumanTab],
    ) -> Result<BrowserNeedsHumanConfirmResult, String> {
        let marked = self.needs_human();
        let targets: Vec<BrowserNeedsHumanTab> = if tabs.is_empty() {
            marked
        } else {
            marked
                .into_iter()
                .filter(|known| tabs.iter().any(|tab| tab.tab_id == known.tab_id))
                .collect()
        };
        let mut result = BrowserNeedsHumanConfirmResult::default();
        for tab in targets {
            let gone = {
                let state = self.state.lock().unwrap();
                state.link.is_some() && !state.tabs.iter().any(|open| open.id == tab.tab_id)
            };
            if gone {
                // Closed in the browser: nothing waits on it any more.
                self.state.lock().unwrap().needs_human.remove(&tab.tab_id);
                result.cleared.push(tab);
                continue;
            }
            let code = json!({"cmd": "scan", "mode": "text"});
            let reply = self
                .request(
                    Some(tab.tab_id),
                    &code.to_string(),
                    Duration::from_millis(DEFAULT_TIMEOUT_MS),
                )
                .await?;
            self.apply_human_verification(tab.tab_id, &reply.value, &tab.frame_id, &tab.turn_id);
            let still = self
                .state
                .lock()
                .unwrap()
                .needs_human
                .get(&tab.tab_id)
                .cloned();
            match still {
                Some(still) => result.still_required.push(still),
                None => result.cleared.push(tab),
            }
        }
        Ok(result)
    }

    /// Brings a tab that waits for a human to the front of the browser.
    pub(crate) async fn focus_needs_human(&self, tab: i64) -> Result<(), String> {
        if !self.waits_for_human(tab) {
            return Err(format!("tab {tab} is not waiting for verification"));
        }
        let code = json!({"cmd": "tabs", "method": "switch", "tabId": tab});
        self.request(
            None,
            &code.to_string(),
            Duration::from_millis(DEFAULT_TIMEOUT_MS),
        )
        .await
        .map(|_| ())
    }

    /// The turn is over: its tabs are closed when **Automatically close
    /// browser tabs** is on, else kept for the user to choose. Tabs that are
    /// gone or wait for a human are left out.
    pub(crate) async fn complete_turn(&self, turn: &str) -> TabCleanup {
        let prompt = {
            let mut state = self.state.lock().unwrap();
            let Some(ledger) = state.ledgers.remove(turn) else {
                return TabCleanup::None;
            };
            let connected = state.link.is_some();
            let tabs: Vec<BrowserTabCleanupItem> = ledger
                .tabs
                .iter()
                .filter(|tracked| !state.needs_human.contains_key(&tracked.tab))
                .filter_map(|tracked| {
                    if !connected {
                        // Away: kept until the extension says what is open.
                        return Some(tracked.item());
                    }
                    let live = state.tabs.iter().find(|tab| tab.id == tracked.tab)?;
                    let mut item = tracked.item();
                    item.url = live.url.clone();
                    item.title = live.title.clone();
                    Some(item)
                })
                .collect();
            if tabs.is_empty() {
                return TabCleanup::None;
            }
            BrowserTabCleanupPrompt {
                turn_id: turn.to_owned(),
                frame_id: ledger.frame,
                tabs,
            }
        };
        let auto_close = match &self.options.store {
            Some(store) => shared::auto_close_tabs_enabled(store).await,
            None => false,
        };
        if auto_close {
            let ids: Vec<i64> = prompt.tabs.iter().map(|tab| tab.tab_id).collect();
            if self
                .close_tabs(&ids, Duration::from_millis(DEFAULT_TIMEOUT_MS))
                .await
                .is_ok()
            {
                return TabCleanup::Closed;
            }
        }
        self.stash(prompt.clone(), auto_close);
        TabCleanup::Prompt(prompt)
    }

    fn stash(&self, prompt: BrowserTabCleanupPrompt, auto_close: bool) {
        let mut state = self.state.lock().unwrap();
        state
            .cleanups
            .retain(|pending| pending.prompt.turn_id != prompt.turn_id);
        state.cleanups.push(Pending { prompt, auto_close });
    }

    /// The ended turns' tabs the user is to be asked about, oldest first.
    pub(crate) fn cleanups(&self) -> Vec<BrowserTabCleanupPrompt> {
        self.state
            .lock()
            .unwrap()
            .cleanups
            .iter()
            .map(|pending| pending.prompt.clone())
            .collect()
    }

    /// Closes the chosen tabs of a turn's prompt; only tabs the prompt
    /// offered are closed. On failure the prompt stays.
    pub(crate) async fn confirm_cleanup(
        &self,
        turn: &str,
        selected: &[BrowserTabCleanupItem],
    ) -> Result<Vec<i64>, String> {
        let pending = {
            let mut state = self.state.lock().unwrap();
            let at = state
                .cleanups
                .iter()
                .position(|pending| pending.prompt.turn_id == turn)
                .ok_or("no pending browser tab cleanup for this turn")?;
            state.cleanups.remove(at)
        };
        let offered: HashSet<i64> = pending.prompt.tabs.iter().map(|tab| tab.tab_id).collect();
        let ids: Vec<i64> = selected
            .iter()
            .map(|tab| tab.tab_id)
            .filter(|tab| offered.contains(tab))
            .collect();
        match self
            .close_tabs(&ids, Duration::from_millis(DEFAULT_TIMEOUT_MS))
            .await
        {
            Ok(closed) => Ok(closed),
            Err(error) => {
                self.state.lock().unwrap().cleanups.insert(0, pending);
                Err(error)
            }
        }
    }

    /// Keeps all of a turn's tabs open and stops asking about them.
    pub(crate) fn dismiss_cleanup(&self, turn: &str) {
        self.state
            .lock()
            .unwrap()
            .cleanups
            .retain(|pending| pending.prompt.turn_id != turn);
    }

    /// Closes what the setting was to close while the extension was away.
    async fn retry_auto_close(&self) {
        let due: Vec<Pending> = {
            let mut state = self.state.lock().unwrap();
            let (due, kept) = std::mem::take(&mut state.cleanups)
                .into_iter()
                .partition(|pending| pending.auto_close);
            state.cleanups = kept;
            due
        };
        for pending in due {
            let ids: Vec<i64> = pending.prompt.tabs.iter().map(|tab| tab.tab_id).collect();
            if self
                .close_tabs(&ids, Duration::from_millis(DEFAULT_TIMEOUT_MS))
                .await
                .is_err()
            {
                self.state.lock().unwrap().cleanups.push(pending);
            }
        }
    }

    /// The extension as the update banner shows it.
    pub(crate) fn extension_status(&self) -> BrowserExtensionStatus {
        let path = self.options.extension.verified_path();
        let state = self.state.lock().unwrap();
        let seen = state.seen.as_ref();
        let connected = state.link.is_some();
        BrowserExtensionStatus {
            connected,
            current_version: seen
                .map(|seen| seen.version.clone())
                .filter(|version| !version.is_empty()),
            bundled_version: self.bundled_version.clone(),
            current_protocol: seen.map(|seen| seen.protocol).unwrap_or_default(),
            required_protocol: REQUIRED_PROTOCOL,
            update_required: seen.is_some_and(|seen| {
                shared::extension_requires_update(
                    seen.protocol,
                    &seen.capabilities,
                    &seen.version,
                    self.bundled_version.as_deref(),
                )
            }),
            automatic_reload_available: connected
                && seen.is_some_and(|seen| {
                    seen.capabilities
                        .iter()
                        .any(|capability| capability == "runtime_reload")
                }),
            extension_path_verified: path.is_some(),
            integrity_verified: path.is_some(),
            extension_path: path,
            error: self.extension_error.lock().unwrap().clone(),
        }
    }

    /// Opens the user's browser on its extension-manager page.
    pub(crate) fn open_extension_setup(&self) -> BrowserExtensionSetup {
        BrowserExtensionSetup {
            extension_path: self.options.extension.verified_path(),
            opened: self
                .options
                .launcher
                .as_ref()
                .is_some_and(|launcher| (launcher.extension_page)()),
        }
    }

    /// Refreshes the managed copy, then has a compatible extension reload
    /// itself from it and waits for its new hello; an older one needs the
    /// user's one click on Reload, for which its page is opened.
    pub(crate) async fn update_extension(&self) -> BrowserExtensionUpdateResult {
        let _one = self.updating.lock().await;
        if let Err(error) = self.options.extension.sync() {
            *self.extension_error.lock().unwrap() = Some(error.clone());
            return BrowserExtensionUpdateResult {
                outcome: "error".into(),
                status: self.extension_status(),
                opened: false,
                error: Some(error),
            };
        }
        *self.extension_error.lock().unwrap() = None;
        let status = self.extension_status();
        if !status.update_required {
            return BrowserExtensionUpdateResult {
                outcome: "updated".into(),
                status,
                opened: false,
                error: None,
            };
        }
        if status.automatic_reload_available {
            let reload = json!({"cmd": "runtime", "method": "reload"});
            if self
                .request(
                    None,
                    &reload.to_string(),
                    Duration::from_millis(DEFAULT_TIMEOUT_MS),
                )
                .await
                .is_ok()
            {
                let deadline = tokio::time::Instant::now() + AUTO_LAUNCH_WAIT;
                while tokio::time::Instant::now() < deadline {
                    let now = self.extension_status();
                    if now.connected && !now.update_required {
                        return BrowserExtensionUpdateResult {
                            outcome: "updated".into(),
                            status: now,
                            opened: false,
                            error: None,
                        };
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
        let setup = self.open_extension_setup();
        BrowserExtensionUpdateResult {
            outcome: "manual_reload_required".into(),
            status: self.extension_status(),
            opened: setup.opened,
            error: None,
        }
    }
}

fn scope(env: &dyn ToolEnv) -> String {
    env.frame_id().unwrap_or_default().to_owned()
}

fn turn(env: &dyn ToolEnv) -> String {
    env.turn_id().unwrap_or_default().to_owned()
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
             when a web tool says the extension is disconnected. When Open browser \
             automatically is on and the extension is not connected, this starts the user's own \
             browser (on url, when given) so its installed extension can connect. If \
             update_required is true, call it again with action=update_extension: a compatible \
             extension reloads itself, an older one returns manual_reload_required with the \
             exact folder, which you relay before calling this again to recheck.",
            json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "description": "Omit for the status. update_extension refreshes the extension's folder and reloads a compatible extension."},
                    "url": {"type": "string", "description": "An http(s) address to open when the browser has to be started; otherwise its new-tab page"}
                },
                "additionalProperties": false
            }),
        )
    }

    fn read_only(&self) -> bool {
        true
    }

    fn preview(&self, _: &Value) -> String {
        "check the real-browser bridge".into()
    }

    async fn run(&self, args: &Value, _: &dyn ToolEnv) -> ToolResult {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|action| !action.is_empty());
        match action {
            None => {}
            Some("update_extension") => {
                let result = self.bridge.update_extension().await;
                return match serde_json::to_value(result) {
                    Ok(value) => ToolResult::ok(render(&value)),
                    Err(error) => ToolResult::fail(format!(
                        "serialize browser extension update result: {error}"
                    )),
                };
            }
            Some(_) => return ToolResult::fail("action must be update_extension"),
        }
        let filters = browser_filters::load(&self.store).await;
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty());
        if let Some(url) = url {
            if !matches!(url::Url::parse(url), Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some())
            {
                return ToolResult::fail("url must be an absolute http:// or https:// address");
            }
            if let Some(rule) = filters.blocked(url) {
                return ToolResult::fail(browser_filters::block_message(url, rule));
            }
        }
        self.bridge.ensure_extension(url).await;
        let status = self.bridge.status();
        let mut report = json!({
            "connected": status.connected,
            "extension_version": status.extension_version,
            "update_required": status.update_required,
            "paused_by_user": status.paused,
            "extension_path": self.bridge.options.extension.verified_path(),
            "auto_launch": shared::auto_launch_enabled(&self.store).await,
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
                .open_tab(&scope(env), &turn(env), url, active, &filters)
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
             this conversation's tabs. If the result contains human_intervention.required=true, \
             a verification prompt has been shown to the user: stop browser automation, do not \
             call ask_user, do not click the challenge, and end your turn.",
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
        finish(self.bridge.scan(&scope, &turn(env), tab, mode).await)
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
             If web_scan reported human_intervention.required=true, do not automate the \
             challenge; a verification prompt has been shown to the user.",
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
                    &turn(env),
                    tab,
                    script,
                    Duration::from_millis(timeout),
                    &filters,
                )
                .await,
        )
    }
}

/// Looks at what a tab this conversation opened shows, as the WebView's
/// `web_screenshot` does: the viewport goes to the model as a picture; a
/// whole-page or element capture, or one with `save_path`, is written to
/// the project as a PNG instead.
pub(crate) struct WebScreenshotTool {
    pub(crate) bridge: Arc<Bridge>,
}

#[async_trait]
impl Tool for WebScreenshotTool {
    fn name(&self) -> &str {
        "web_screenshot"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            "Look at what a tab this conversation opened in the user's real Chrome/Chromium \
             session is showing. Waits until the tab's document is complete before capturing. \
             Use it when web_scan's text and element snapshot is not enough: rendered layout, a \
             chart or diagram, a canvas/WebGL page, a QR code, a PDF or image viewer, or a page \
             that looks wrong and needs eyes. Captures the visible viewport of the tab; to reach \
             content below the fold, scroll with web_execute_js first and capture again. Pass \
             'question' to say what should be read out of the screenshot. full_page, selector \
             or save_path write a PNG into the project instead of showing it to you.",
            json!({
                "type": "object",
                "properties": {
                    "switch_tab_id": {"type": ["integer", "string"], "description": "Tab id returned by web_open_tab; selects that tab for this and later calls"},
                    "question": {"type": "string", "description": "What to look for in the screenshot, e.g. 'is the login QR code visible and not expired?'"},
                    "full_page": {"type": "boolean"},
                    "selector": {"type": "string"},
                    "save_path": {"type": "string", "description": "Optional project-relative PNG path. Screenshots are not original figures."}
                }
            }),
        )
    }

    fn minimum_approval(&self) -> Approval {
        Approval::Ask
    }

    fn preview(&self, args: &Value) -> String {
        match args.get("switch_tab_id").filter(|tab| !tab.is_null()) {
            Some(tab) => format!("screenshot real-browser tab {tab}"),
            None => "screenshot the selected real-browser tab".into(),
        }
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let tab = match tab_arg(args) {
            Ok(tab) => tab,
            Err(problem) => return ToolResult::fail(problem),
        };
        let full_page = args
            .get("full_page")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let selector = args.get("selector").and_then(Value::as_str);
        let save_path = args
            .get("save_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let persist = full_page || selector.is_some() || save_path.is_some();
        // Where a PNG goes is checked before the browser is asked.
        let target = match save_path {
            Some(rel) => match shared::project_relative_path(env.project_root(), rel) {
                Ok(path) => Some(path),
                Err(error) => return ToolResult::fail(format!("save_path: {error}")),
            },
            None => None,
        };
        let (tab, data) = match self
            .bridge
            .screenshot(&scope(env), tab, persist.then_some((full_page, selector)))
            .await
        {
            Ok(shot) => shot,
            Err(problem) => return ToolResult::fail(problem),
        };
        if !persist {
            return ToolResult::image(ImageData {
                mime: "image/jpeg".into(),
                data_url: format!("data:image/jpeg;base64,{data}"),
                label: format!(
                    "Screenshot of real browser tab {tab} ({} KB)",
                    data.len() / 1024
                ),
            });
        }
        let bytes = match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &data)
        {
            Ok(bytes) => bytes,
            Err(error) => return ToolResult::fail(format!("decode screenshot: {error}")),
        };
        let target = target.unwrap_or_else(|| {
            env.project_root()
                .join("browser-assets")
                .join("screenshots")
                .join(format!("shot-{}.png", uuid::Uuid::new_v4()))
        });
        if let Some(parent) = target.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                return ToolResult::fail(format!("create screenshot folder: {error}"));
            }
        }
        if let Err(error) = std::fs::write(&target, &bytes) {
            return ToolResult::fail(format!("write screenshot: {error}"));
        }
        ToolResult::ok(render(&json!({
            "tab_id": tab,
            "path": target.display().to_string(),
            "bytes": bytes.len(),
            "sha256": hex::encode(Sha256::digest(&bytes)),
            "note": "screenshot PNG is not an original figure"
        })))
    }
}

const ENABLED_KEY: &str = "desktop.browser.enabled";

/// The bridge of one host: off until the desktop switches it on.
pub(crate) struct Slot {
    bridge: Mutex<Option<Arc<Bridge>>>,
    /// Why it is on but not listening.
    problem: Mutex<Option<String>>,
    addr: Mutex<String>,
    /// How the user's browser is started; none in a test.
    launcher: Mutex<Option<Launcher>>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            bridge: Mutex::default(),
            problem: Mutex::default(),
            addr: Mutex::new(BRIDGE_ADDR.into()),
            launcher: Mutex::new((!cfg!(test)).then(Launcher::system)),
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
    /// **Open browser automatically**: a browser tool that needs the
    /// extension starts the user's browser when it is not connected.
    pub auto_launch: bool,
    /// **Automatically close browser tabs**: a turn's tabs are closed when
    /// it ends, instead of the user being asked.
    pub auto_close_tabs: bool,
}

/// What the app shows about the browser over a project: the update banner,
/// and the questions about ended turns' tabs and tabs that wait for a
/// human, for that project's conversations only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserAlerts {
    /// The extension, when it has to be updated.
    pub extension: Option<BrowserExtensionStatus>,
    /// Ended turns' tabs, oldest first.
    pub cleanups: Vec<BrowserTabCleanupPrompt>,
    pub needs_human: Vec<BrowserNeedsHumanTab>,
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

    /// The extension this app ships and its managed copy in the data folder.
    fn extension_dirs(&self) -> ExtensionDirs {
        ExtensionDirs {
            bundled: wisp_paths::browser_extension_dir(),
            managed: Some(self.0.config.data.join("browser-extension")),
        }
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
        let extension_path = self.extension_dirs().prepared_path();
        Ok(BrowserSettings {
            enabled,
            listening: bridge.is_some(),
            problem,
            status,
            extension_path,
            filters,
            auto_launch: shared::auto_launch_enabled(&self.0.store).await,
            auto_close_tabs: shared::auto_close_tabs_enabled(&self.0.store).await,
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

    /// **Open browser automatically**, under the WebView's settings key.
    pub async fn set_browser_auto_launch(&self, enabled: bool) -> Result<BrowserSettings> {
        self.0
            .store
            .set_setting(
                shared::AUTO_LAUNCH_KEY,
                if enabled { "true" } else { "false" },
            )
            .await?;
        self.browser_settings().await
    }

    /// **Automatically close browser tabs**, under the WebView's key.
    pub async fn set_browser_auto_close_tabs(&self, enabled: bool) -> Result<BrowserSettings> {
        self.0
            .store
            .set_setting(
                shared::AUTO_CLOSE_TABS_KEY,
                if enabled { "true" } else { "false" },
            )
            .await?;
        self.browser_settings().await
    }

    /// Starts the bridge when the browser tools are on. The desktop
    /// calls this once it has opened the host.
    pub async fn start_browser(&self) {
        let slot = &self.0.browser;
        if !self.browser_enabled().await || slot.bridge().is_some() {
            return;
        }
        let extension = self.extension_dirs();
        // The folder the user loads is brought up to the shipped copy.
        let synced = extension.sync().err();
        let options = Options {
            store: Some(self.0.store.clone()),
            extension,
            launcher: slot.launcher.lock().unwrap().clone(),
            launch_wait: AUTO_LAUNCH_WAIT,
        };
        let addr = slot.addr.lock().unwrap().clone();
        match Bridge::start_with(&addr, options).await {
            Ok(bridge) => {
                *bridge.extension_error.lock().unwrap() = synced;
                *slot.bridge.lock().unwrap() = Some(bridge);
                *slot.problem.lock().unwrap() = None;
            }
            Err(error) => *slot.problem.lock().unwrap() = Some(format!("{error:#}")),
        }
    }

    /// A turn is over: what the browser does with the tabs it opened is
    /// settled without holding up what comes after the turn.
    pub(crate) fn browser_turn_over(&self, turn: &str) {
        if let Some(bridge) = self.0.browser.bridge() {
            let turn = turn.to_owned();
            tokio::spawn(async move { bridge.complete_turn(&turn).await });
        }
    }

    /// Whether the conversation `frame` is one of `project`'s.
    async fn frame_in_project(&self, frame: &str, project: &str) -> bool {
        self.0
            .store
            .frame_project_id(frame)
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some(project)
    }

    /// The update banner and the browser questions for `project`.
    pub async fn browser_alerts(&self, project: &str) -> Result<BrowserAlerts> {
        let Some(bridge) = self.0.browser.bridge() else {
            return Ok(BrowserAlerts::default());
        };
        let status = bridge.extension_status();
        let mut alerts = BrowserAlerts {
            extension: status.update_required.then_some(status),
            ..Default::default()
        };
        for prompt in bridge.cleanups() {
            if self.frame_in_project(&prompt.frame_id, project).await {
                alerts.cleanups.push(prompt);
            }
        }
        for tab in bridge.needs_human() {
            if self.frame_in_project(&tab.frame_id, project).await {
                alerts.needs_human.push(tab);
            }
        }
        Ok(alerts)
    }

    fn running_bridge(&self) -> Result<Arc<Bridge>> {
        self.0
            .browser
            .bridge()
            .context("the browser tools are switched off")
    }

    /// Closes the chosen tabs of an ended turn; none chosen keeps them all.
    pub async fn confirm_browser_tab_cleanup(
        &self,
        turn: &str,
        tabs: Vec<BrowserTabCleanupItem>,
    ) -> Result<Vec<i64>> {
        let bridge = self.running_bridge()?;
        if tabs.is_empty() {
            bridge.dismiss_cleanup(turn);
            return Ok(Vec::new());
        }
        bridge
            .confirm_cleanup(turn, &tabs)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Keeps all the tabs of an ended turn.
    pub async fn dismiss_browser_tab_cleanup(&self, turn: &str) -> Result<()> {
        if let Some(bridge) = self.0.browser.bridge() {
            bridge.dismiss_cleanup(turn);
        }
        Ok(())
    }

    /// Looks again at tabs the user says they verified.
    pub async fn confirm_browser_needs_human(
        &self,
        tabs: Vec<BrowserNeedsHumanTab>,
    ) -> Result<BrowserNeedsHumanConfirmResult> {
        self.running_bridge()?
            .confirm_needs_human(&tabs)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Shows a tab that waits for a human in the browser.
    pub async fn focus_browser_needs_human(&self, tab: i64) -> Result<()> {
        self.running_bridge()?
            .focus_needs_human(tab)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Updates the extension the user loaded, as the banner's button does.
    pub async fn update_browser_extension(&self) -> Result<BrowserExtensionUpdateResult> {
        Ok(self.running_bridge()?.update_extension().await)
    }

    /// Opens the user's browser on its extension page.
    pub async fn open_browser_extension_page(&self) -> Result<BrowserExtensionSetup> {
        Ok(match self.0.browser.bridge() {
            Some(bridge) => bridge.open_extension_setup(),
            None => BrowserExtensionSetup {
                extension_path: self.extension_dirs().verified_path(),
                opened: self
                    .0
                    .browser
                    .launcher
                    .lock()
                    .unwrap()
                    .clone()
                    .is_some_and(|launcher| (launcher.extension_page)()),
            },
        })
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
        Box::new(WebScreenshotTool {
            bridge: bridge.clone(),
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

    const CAPABILITIES: [&str; 4] = [
        "tab_scope",
        "runtime_reload",
        "full_page_screenshot",
        "selector_screenshot",
    ];

    async fn hello(socket: &mut Socket, tabs: Value) {
        let ready = json!({
            "type": "ext_ready", "protocol_version": 2, "extension_version": "0.5.1",
            "session": "shared", "paused": false, "tabs": tabs, "capabilities": CAPABILITIES,
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
        assert_eq!(status.extension_version, "0.5.1");
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
                "",
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
            .scan("c", "t", Some(7), "default")
            .await
            .unwrap_err()
            .contains("not opened"));
        assert!(bridge
            .scan("c", "t", None, "default")
            .await
            .unwrap_err()
            .contains("no tab yet"));
        assert_eq!(bridge.owned_tabs("c")["tabs"], json!([]));
        // A blocked host and something that is no web address are refused
        // before the browser hears of them.
        for url in ["https://www.blocked.example/x", "file:///etc/passwd"] {
            assert!(bridge
                .open_tab("c", "t", url, false, &filters)
                .await
                .is_err());
        }

        // Opening a tab makes it the conversation's.
        let opening = {
            let bridge = bridge.clone();
            let filters = filters.clone();
            tokio::spawn(async move {
                bridge
                    .open_tab("c", "t", "https://example.org/paper", false, &filters)
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
            tokio::spawn(async move { bridge.scan("c", "t", None, "text").await })
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
        assert!(bridge
            .scan("other", "t", Some(21), "default")
            .await
            .is_err());
        assert!(bridge.scan("c", "t", None, "everything").await.is_err());

        // A script runs in it; a page it opens is the conversation's too.
        let running = {
            let bridge = bridge.clone();
            let filters = filters.clone();
            tokio::spawn(async move {
                bridge
                    .execute(
                        "c",
                        "t",
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
            let refused = bridge
                .execute("c", "t", None, script, short, &filters)
                .await;
            assert!(refused.is_err(), "{script}");
        }
        assert!(bridge
            .execute("c", "t", Some(7), "1", short, &filters)
            .await
            .is_err());
        let big = "x".repeat(MAX_SCRIPT_BYTES + 1);
        assert!(bridge
            .execute("c", "t", None, &big, short, &filters)
            .await
            .is_err());

        // An error of the extension is passed on with its code.
        let failing = {
            let bridge = bridge.clone();
            let filters = filters.clone();
            tokio::spawn(async move {
                bridge
                    .execute("c", "t", Some(21), "boom()", short, &filters)
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
                    .execute("c", "t", None, script, Duration::from_secs(2), &filters)
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
                "",
                "https://example.org",
                false,
                &BrowserFilters::default(),
            )
            .await
            .unwrap_err();
        assert!(refused.contains("protocol"), "{refused}");
        bridge.stop();
    }

    /// A store for the Browser settings, in a folder dropped with it.
    async fn settings_store() -> (Store, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        (store, temp)
    }

    async fn bridge_with(options: Options) -> Arc<Bridge> {
        Bridge::start_with("127.0.0.1:0", options).await.unwrap()
    }

    /// Has conversation `c` open a tab in turn `turn`, the extension
    /// answering with tab `id`.
    async fn open_as(
        bridge: &Arc<Bridge>,
        extension: &mut Socket,
        turn: &str,
        id: i64,
        url: &str,
    ) -> Value {
        let opening = {
            let (bridge, turn, url) = (bridge.clone(), turn.to_owned(), url.to_owned());
            tokio::spawn(async move {
                bridge
                    .open_tab("c", &turn, &url, false, &BrowserFilters::default())
                    .await
            })
        };
        let request = asked(extension).await;
        let tab = json!({"id": id, "url": url, "title": format!("Tab {id}")});
        answer(extension, &request, json!({"result": tab})).await;
        opening.await.unwrap().unwrap();
        tab
    }

    const CAPTCHA: &str =
        "Are you a robot?\nPlease confirm you are a human by completing the captcha challenge.";

    #[tokio::test]
    async fn the_tabs_a_turn_opened_are_offered_for_closing_when_it_ends() {
        let (store, _temp) = settings_store().await;
        let bridge = bridge_with(Options {
            store: Some(store),
            ..Default::default()
        })
        .await;
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        let bank = json!({"id": 7, "url": "https://bank.example", "title": "Bank"});
        hello(&mut extension, json!([bank])).await;
        settled(&bridge, |status| status.tabs == 1).await;

        // A turn that opened nothing asks nothing.
        assert_eq!(bridge.complete_turn("t0").await, TabCleanup::None);

        let paper = open_as(&bridge, &mut extension, "t1", 21, "https://example.org/a").await;
        hello(&mut extension, json!([bank, paper])).await;
        settled(&bridge, |status| status.tabs == 2).await;
        // The tab is offered, the user's own is not; the setting is off.
        let TabCleanup::Prompt(prompt) = bridge.complete_turn("t1").await else {
            panic!("the turn's tab is offered");
        };
        assert_eq!(
            (prompt.turn_id.as_str(), prompt.frame_id.as_str()),
            ("t1", "c")
        );
        assert_eq!(prompt.tabs.len(), 1);
        assert_eq!(
            (prompt.tabs[0].tab_id, prompt.tabs[0].session.as_str()),
            (21, "shared")
        );
        assert_eq!(bridge.cleanups(), vec![prompt.clone()]);
        // A turn ends once.
        assert_eq!(bridge.complete_turn("t1").await, TabCleanup::None);

        // Only what the prompt offered is closed, whatever the app sends.
        let mut chosen = prompt.tabs.clone();
        chosen.push(BrowserTabCleanupItem {
            session: "shared".into(),
            tab_id: 7,
            ..Default::default()
        });
        let closing = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.confirm_cleanup("t1", &chosen).await })
        };
        let request = asked(&mut extension).await;
        assert_eq!(
            serde_json::from_str::<Value>(request["code"].as_str().unwrap()).unwrap(),
            json!({"cmd": "tabs", "method": "close", "tabIds": [21]})
        );
        answer(
            &mut extension,
            &request,
            json!({"result": {"closed": [21]}}),
        )
        .await;
        assert_eq!(closing.await.unwrap().unwrap(), vec![21]);
        assert!(bridge.cleanups().is_empty());
        assert!(!bridge.owns("c", 21));

        // Keep all: nothing is closed and nothing is asked again.
        let second = open_as(&bridge, &mut extension, "t2", 31, "https://example.org/b").await;
        hello(&mut extension, json!([bank, second])).await;
        settled(&bridge, |status| status.tabs == 2).await;
        assert!(matches!(
            bridge.complete_turn("t2").await,
            TabCleanup::Prompt(_)
        ));
        bridge.dismiss_cleanup("t2");
        assert!(bridge.cleanups().is_empty());
        assert!(bridge.owns("c", 31));
        assert!(bridge.confirm_cleanup("t2", &[]).await.is_err());
        bridge.stop();
    }

    #[tokio::test]
    async fn auto_close_closes_a_turns_tabs_and_waits_for_an_extension_that_is_away() {
        let (store, _temp) = settings_store().await;
        store
            .set_setting(shared::AUTO_CLOSE_TABS_KEY, "true")
            .await
            .unwrap();
        let bridge = bridge_with(Options {
            store: Some(store),
            ..Default::default()
        })
        .await;
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        hello(&mut extension, json!([])).await;
        // Its hello is in: requests are sent from now on.
        settled(&bridge, |status| status.extension_version == "0.5.1").await;

        let paper = open_as(&bridge, &mut extension, "t1", 21, "https://example.org/a").await;
        hello(&mut extension, json!([paper])).await;
        settled(&bridge, |status| status.tabs == 1).await;
        let ending = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.complete_turn("t1").await })
        };
        let request = asked(&mut extension).await;
        assert_eq!(
            serde_json::from_str::<Value>(request["code"].as_str().unwrap()).unwrap(),
            json!({"cmd": "tabs", "method": "close", "tabIds": [21]})
        );
        answer(
            &mut extension,
            &request,
            json!({"result": {"closed": [21]}}),
        )
        .await;
        assert_eq!(ending.await.unwrap(), TabCleanup::Closed);
        assert!(bridge.cleanups().is_empty());

        // The extension is away when the next turn ends: its tab waits.
        let other = open_as(&bridge, &mut extension, "t2", 22, "https://example.org/b").await;
        drop(extension);
        settled(&bridge, |status| !status.connected).await;
        assert!(matches!(
            bridge.complete_turn("t2").await,
            TabCleanup::Prompt(_)
        ));
        assert_eq!(bridge.cleanups().len(), 1);
        // Back, it is closed without asking.
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        hello(&mut extension, json!([other])).await;
        let request = asked(&mut extension).await;
        assert_eq!(
            serde_json::from_str::<Value>(request["code"].as_str().unwrap()).unwrap(),
            json!({"cmd": "tabs", "method": "close", "tabIds": [22]})
        );
        answer(
            &mut extension,
            &request,
            json!({"result": {"closed": [22]}}),
        )
        .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !bridge.cleanups().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the waiting tabs are closed");
        bridge.stop();
    }

    #[tokio::test]
    async fn a_page_that_asks_for_a_human_holds_its_tab_until_the_user_has_answered_it() {
        let (store, _temp) = settings_store().await;
        store
            .set_setting(shared::AUTO_CLOSE_TABS_KEY, "true")
            .await
            .unwrap();
        let bridge = bridge_with(Options {
            store: Some(store),
            ..Default::default()
        })
        .await;
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        hello(&mut extension, json!([])).await;
        // Its hello is in: requests are sent from now on.
        settled(&bridge, |status| status.extension_version == "0.5.1").await;
        let paper = open_as(&bridge, &mut extension, "t1", 21, "https://example.org/a").await;
        hello(&mut extension, json!([paper])).await;
        settled(&bridge, |status| status.tabs == 1).await;
        // Nothing waits for a human that the bridge did not mark.
        assert!(bridge.focus_needs_human(21).await.is_err());

        let reading = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.scan("c", "t1", None, "text").await })
        };
        let request = asked(&mut extension).await;
        let challenge =
            json!({"url": "https://example.org/a", "title": "Just a moment", "text": CAPTCHA});
        answer(&mut extension, &request, json!({"result": challenge})).await;
        let page = reading.await.unwrap().unwrap();
        assert_eq!(page["human_intervention"]["required"], true);
        let held = bridge.needs_human();
        assert_eq!(held.len(), 1);
        assert_eq!(
            (
                held[0].tab_id,
                held[0].frame_id.as_str(),
                held[0].turn_id.as_str()
            ),
            (21, "c", "t1")
        );
        assert_eq!(held[0].title, "Just a moment");

        // The tab is not driven, closed by the model or closed at the end of
        // the turn while it waits.
        let short = Duration::from_millis(200);
        let filters = BrowserFilters::default();
        for script in [
            "document.title",
            r#"{"cmd":"tabs","method":"close","tabIds":[21]}"#,
        ] {
            let refused = bridge
                .execute("c", "t1", None, script, short, &filters)
                .await
                .unwrap_err();
            assert!(refused.contains("human verification"), "{refused}");
        }
        assert_eq!(bridge.complete_turn("t1").await, TabCleanup::None);
        assert!(bridge.cleanups().is_empty());

        // Show this tab brings it forward.
        let showing = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.focus_needs_human(21).await })
        };
        let request = asked(&mut extension).await;
        assert_eq!(
            serde_json::from_str::<Value>(request["code"].as_str().unwrap()).unwrap(),
            json!({"cmd": "tabs", "method": "switch", "tabId": 21})
        );
        answer(&mut extension, &request, json!({"result": {}})).await;
        showing.await.unwrap().unwrap();

        // "I completed verification" while the challenge is still there.
        let checking = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.confirm_needs_human(&[]).await })
        };
        let request = asked(&mut extension).await;
        assert_eq!(request["tabId"], 21);
        answer(&mut extension, &request, json!({"result": challenge})).await;
        let checked = checking.await.unwrap().unwrap();
        assert_eq!(checked.still_required.len(), 1);
        assert!(checked.cleared.is_empty());

        // And once it is gone.
        let checking = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.confirm_needs_human(&held).await })
        };
        let request = asked(&mut extension).await;
        let article =
            json!({"url": "https://example.org/a", "title": "Article", "text": "Abstract"});
        answer(&mut extension, &request, json!({"result": article})).await;
        let checked = checking.await.unwrap().unwrap();
        assert!(checked.still_required.is_empty());
        assert_eq!(checked.cleared[0].frame_id, "c");
        assert!(bridge.needs_human().is_empty());
        // A tab the bridge never marked is not looked at.
        let stranger = BrowserNeedsHumanTab {
            tab_id: 7,
            ..Default::default()
        };
        let checked = bridge.confirm_needs_human(&[stranger]).await.unwrap();
        assert!(checked.cleared.is_empty() && checked.still_required.is_empty());
        bridge.stop();
    }

    #[tokio::test]
    async fn a_screenshot_is_taken_only_of_the_conversations_own_tab() {
        let bridge = bridge_with(Options::default()).await;
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        let bank = json!({"id": 7, "url": "https://bank.example", "title": "Bank"});
        hello(&mut extension, json!([bank])).await;
        settled(&bridge, |status| status.tabs == 1).await;
        // Nothing of its own yet, and the user's tab is refused unasked.
        assert!(bridge.screenshot("c", None, None).await.is_err());
        assert!(bridge.screenshot("c", Some(7), None).await.is_err());

        open_as(&bridge, &mut extension, "t1", 21, "https://example.org/a").await;
        let shooting = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.screenshot("c", None, None).await })
        };
        let request = asked(&mut extension).await;
        assert_eq!(request["tabId"], 21);
        assert_eq!(
            serde_json::from_str::<Value>(request["code"].as_str().unwrap()).unwrap(),
            json!({"cmd": "cdp", "method": "Page.captureScreenshot", "params": {"format": "jpeg", "quality": 80}})
        );
        answer(
            &mut extension,
            &request,
            json!({"result": {"data": "aGVsbG8="}}),
        )
        .await;
        assert_eq!(
            shooting.await.unwrap().unwrap(),
            (21, "aGVsbG8=".to_owned())
        );
        assert!(bridge.screenshot("other", Some(21), None).await.is_err());

        // An element capture is the extension's PNG capture.
        let shooting = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .screenshot("c", None, Some((false, Some("#fig1"))))
                    .await
            })
        };
        let request = asked(&mut extension).await;
        assert_eq!(
            serde_json::from_str::<Value>(request["code"].as_str().unwrap()).unwrap(),
            json!({"cmd": "capture", "full_page": false, "selector": "#fig1", "format": "png"})
        );
        answer(&mut extension, &request, json!({"result": {"data": ""}})).await;
        assert!(shooting.await.unwrap().is_err());

        // An extension without the capability is not asked for it.
        let old = json!({"type": "ext_ready", "protocol_version": 2, "extension_version": "0.3.0", "tabs": [], "capabilities": ["tab_scope"]});
        extension
            .send(Message::Text(old.to_string().into()))
            .await
            .unwrap();
        settled(&bridge, |status| status.extension_version == "0.3.0").await;
        let refused = bridge
            .screenshot("c", Some(21), Some((true, None)))
            .await
            .unwrap_err();
        assert!(refused.contains("full_page_screenshot"), "{refused}");
        bridge.stop();
    }

    #[derive(Default)]
    struct Env {
        root: PathBuf,
    }

    #[async_trait]
    impl ToolEnv for Env {
        fn project_root(&self) -> &std::path::Path {
            &self.root
        }
        async fn confirm(&self, _: &str) -> bool {
            true
        }
        async fn emit(&self, _: wisp_tools::ToolEvent) {}
        fn frame_id(&self) -> Option<&str> {
            Some("c")
        }
        fn turn_id(&self) -> Option<&str> {
            Some("t1")
        }
    }

    #[tokio::test]
    async fn the_screenshot_tool_shows_the_viewport_and_keeps_a_png_inside_the_project() {
        let project = tempfile::tempdir().unwrap();
        let env = Env {
            root: project.path().to_path_buf(),
        };
        let bridge = bridge_with(Options::default()).await;
        let tool = WebScreenshotTool {
            bridge: bridge.clone(),
        };
        assert_eq!(tool.minimum_approval(), Approval::Ask);
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        hello(&mut extension, json!([])).await;
        // Its hello is in: requests are sent from now on.
        settled(&bridge, |status| status.extension_version == "0.5.1").await;
        open_as(&bridge, &mut extension, "t1", 21, "https://example.org/a").await;

        // A PNG may not be written outside the project; the browser is not
        // asked.
        let outside = tool.run(&json!({"save_path": "../shot.png"}), &env).await;
        assert!(!outside.success && outside.content.contains("save_path"));

        let looking = {
            let bridge = bridge.clone();
            let root = env.root.clone();
            tokio::spawn(async move {
                WebScreenshotTool { bridge }
                    .run(&json!({}), &Env { root })
                    .await
            })
        };
        let request = asked(&mut extension).await;
        answer(
            &mut extension,
            &request,
            json!({"result": {"data": "aGVsbG8="}}),
        )
        .await;
        let seen = looking.await.unwrap();
        assert!(seen.success);
        assert_eq!(seen.images.len(), 1);
        assert_eq!(seen.images[0].data_url, "data:image/jpeg;base64,aGVsbG8=");

        let saving = {
            let bridge = bridge.clone();
            let root = env.root.clone();
            tokio::spawn(async move {
                WebScreenshotTool { bridge }
                    .run(&json!({"save_path": "figures/page.png"}), &Env { root })
                    .await
            })
        };
        let request = asked(&mut extension).await;
        answer(
            &mut extension,
            &request,
            json!({"result": {"data": "aGVsbG8="}}),
        )
        .await;
        let saved = saving.await.unwrap();
        assert!(saved.success, "{}", saved.content);
        assert!(saved.images.is_empty());
        assert_eq!(
            std::fs::read(project.path().join("figures/page.png")).unwrap(),
            b"hello"
        );
        bridge.stop();
    }

    #[tokio::test]
    async fn open_browser_automatically_starts_the_browser_only_when_on_and_away() {
        let (store, _temp) = settings_store().await;
        let launches: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
        let seen = launches.clone();
        let bridge = bridge_with(Options {
            store: Some(store.clone()),
            launcher: Some(Launcher {
                browser: Arc::new(move |url| {
                    seen.lock().unwrap().push(url);
                    Ok(())
                }),
                extension_page: Arc::new(|| false),
            }),
            launch_wait: Duration::ZERO,
            ..Default::default()
        })
        .await;
        // On by default, and the extension is away: the browser is started,
        // on the address given.
        bridge.ensure_extension(None).await;
        bridge.ensure_extension(Some("https://example.org/")).await;
        assert_eq!(
            *launches.lock().unwrap(),
            vec![None, Some("https://example.org/".to_owned())]
        );
        // A request that finds it away starts it too, then fails as before.
        assert!(bridge
            .open_tab(
                "c",
                "t",
                "https://example.org",
                false,
                &BrowserFilters::default()
            )
            .await
            .unwrap_err()
            .contains("not connected"));
        assert_eq!(launches.lock().unwrap().len(), 3);
        // Off, nothing is started.
        store
            .set_setting(shared::AUTO_LAUNCH_KEY, "false")
            .await
            .unwrap();
        bridge.ensure_extension(None).await;
        assert_eq!(launches.lock().unwrap().len(), 3);
        // On again, but connected: nothing is started.
        store
            .set_setting(shared::AUTO_LAUNCH_KEY, "true")
            .await
            .unwrap();
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        hello(&mut extension, json!([])).await;
        // Its hello is in: requests are sent from now on.
        settled(&bridge, |status| status.extension_version == "0.5.1").await;
        bridge.ensure_extension(None).await;
        assert_eq!(launches.lock().unwrap().len(), 3);
        bridge.stop();
    }

    /// The extension the repository ships, and a managed copy's folder.
    fn shipped_extension(data: &std::path::Path) -> ExtensionDirs {
        ExtensionDirs {
            bundled: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../browser-extension"),
            ),
            managed: Some(data.join("browser-extension")),
        }
    }

    #[tokio::test]
    async fn an_extension_older_than_the_shipped_copy_is_updated_and_rechecked() {
        let data = tempfile::tempdir().unwrap();
        let bridge = bridge_with(Options {
            extension: shipped_extension(data.path()),
            ..Default::default()
        })
        .await;
        // Away and never seen: no banner.
        assert!(!bridge.extension_status().update_required);

        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        let old = json!({"type": "ext_ready", "protocol_version": 2, "extension_version": "0.4.0", "tabs": [], "capabilities": CAPABILITIES});
        extension
            .send(Message::Text(old.to_string().into()))
            .await
            .unwrap();
        settled(&bridge, |status| status.extension_version == "0.4.0").await;
        let status = bridge.extension_status();
        assert!(status.connected && status.update_required);
        assert!(status.automatic_reload_available);
        assert_eq!(status.current_version.as_deref(), Some("0.4.0"));
        assert_eq!(status.bundled_version.as_deref(), Some("0.5.1"));
        assert!(bridge.status().update_required);

        let updating = {
            let bridge = bridge.clone();
            tokio::spawn(async move { bridge.update_extension().await })
        };
        // The managed copy is written, then the extension is told to reload.
        let request = asked(&mut extension).await;
        assert_eq!(
            serde_json::from_str::<Value>(request["code"].as_str().unwrap()).unwrap(),
            json!({"cmd": "runtime", "method": "reload"})
        );
        answer(&mut extension, &request, json!({"result": {}})).await;
        drop(extension);
        let mut extension = connect(&bridge, EXTENSION_ORIGIN).await.unwrap();
        hello(&mut extension, json!([])).await;
        let result = updating.await.unwrap();
        assert_eq!(result.outcome, "updated");
        assert!(!result.status.update_required);
        let folder = result.status.extension_path.unwrap();
        assert!(folder.ends_with("browser-extension"), "{folder}");
        assert!(std::path::Path::new(&folder)
            .join("manifest.json")
            .is_file());

        // One that cannot reload itself needs the user's click; no browser
        // is started from a test.
        let old = json!({"type": "ext_ready", "protocol_version": 2, "extension_version": "0.3.0", "tabs": [], "capabilities": ["tab_scope"]});
        extension
            .send(Message::Text(old.to_string().into()))
            .await
            .unwrap();
        settled(&bridge, |status| status.extension_version == "0.3.0").await;
        let result = bridge.update_extension().await;
        assert_eq!(result.outcome, "manual_reload_required");
        assert!(!result.opened);
        assert!(result.status.update_required && !result.status.automatic_reload_available);
        // Gone, the banner still says what it waits for.
        drop(extension);
        settled(&bridge, |status| !status.connected).await;
        let status = bridge.extension_status();
        assert!(!status.connected && status.update_required);
        assert_eq!(status.current_version.as_deref(), Some("0.3.0"));
        bridge.stop();
    }
}
