//! Remote web access from the desktop: this host dials out to a
//! self-hosted `wisp-relay`, and a browser that holds the connection code
//! reaches the same short list of commands a headless server offers. No
//! inbound port is opened. Frames are sealed end to end with a key made
//! from the code, so the relay sees neither requests nor transcripts.
//!
//! The tunnel follows `src-tauri/src/channels/remote.rs`, which cannot be
//! shared from here because it lives in the Tauri shell. Whether it is on
//! and the relay's address are kept under the WebView's settings keys;
//! the relay token and the code are kept in the secret store under this
//! data directory's own names, never in SQLite.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::{ensure, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, protocol::WebSocketConfig, Message,
};
use wisp_dto::{native_settings, remote_access::RemoteRequest};
use wisp_sync::{
    open_frame, seal_frame, DirectRemoteHost, HostFrame, RemoteCode, CLIENT_TO_HOST,
    HOST_TO_CLIENT, REMOTE_HOST_MAX_FRAME_BYTES,
};

use crate::Server;

const ENABLED_KEY: &str = "remote_access_enabled";
const URL_KEY: &str = "remote_relay_url";
/// Room for the relay's envelope around a sealed reply.
const MAX_REPLY_BYTES: usize = REMOTE_HOST_MAX_FRAME_BYTES - 1024;
const MAX_CLIENTS: usize = 8;
const HEARTBEAT: Duration = Duration::from_secs(25);
const RELAY_SILENCE: Duration = Duration::from_secs(75);
const MISSING_RELAY: &str =
    "Enter the relay server URL and access token before enabling remote access.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RemoteState {
    #[default]
    Off,
    Connecting,
    Connected,
    /// The tunnel dropped and is being dialled again.
    Failed,
}

/// Remote web access as Settings shows it. The token is never part of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteAccess {
    pub enabled: bool,
    pub relay_url: String,
    pub has_token: bool,
    pub state: RemoteState,
    /// Why the tunnel is not up, when it is not.
    pub detail: String,
    /// The address a browser opens; as secret as the code in it.
    pub link: Option<String>,
    pub code: Option<String>,
    /// Browsers connected right now.
    pub clients: u32,
}

/// The running tunnel, when there is one.
#[derive(Default)]
pub(crate) struct Tunnel {
    stop: Mutex<Option<watch::Sender<bool>>>,
    status: Arc<Mutex<(RemoteState, String)>>,
    clients: Arc<AtomicU32>,
}

impl Tunnel {
    pub(crate) fn stop(&self) {
        if let Some(stop) = self.stop.lock().unwrap().take() {
            let _ = stop.send(true);
        }
        self.clients.store(0, Ordering::Relaxed);
        *self.status.lock().unwrap() = (RemoteState::Off, String::new());
    }
}

struct Peer {
    nonce: String,
    seq: u64,
}

fn token_secret(key_id: &str) -> String {
    format!("desktop.remote.{key_id}.token")
}

fn code_secret(key_id: &str) -> String {
    format!("desktop.remote.{key_id}.code")
}

fn seal(key: &[u8; 32], value: &Value) -> Result<String, String> {
    seal_frame(key, HOST_TO_CLIENT, value.to_string().as_bytes()).map_err(|error| error.to_string())
}

/// The relay drops the whole tunnel on an oversized frame, so a reply
/// that cannot fit becomes an error for that one request instead.
fn seal_reply(key: &[u8; 32], id: &str, result: Result<Value, String>) -> Result<String, String> {
    let reply = |result: Result<Value, String>| {
        let response = native_settings::Response {
            schema: native_settings::SCHEMA.into(),
            id: id.to_owned(),
            result: result.as_ref().ok().cloned(),
            error: result.err(),
        };
        seal(key, &json!({"type": "response", "response": response}))
    };
    match reply(result)? {
        sealed if sealed.len() > MAX_REPLY_BYTES => {
            reply(Err("This is too large to show remotely".into()))
        }
        sealed => Ok(sealed),
    }
}

/// Requests are bound to the browser connection's hello nonce and
/// strictly increasing, so the relay can neither replay nor reorder them.
fn open_request(key: &[u8; 32], frame: &str, peer: &mut Peer) -> Option<RemoteRequest> {
    let plaintext = open_frame(key, CLIENT_TO_HOST, frame).ok()?;
    let request: RemoteRequest = serde_json::from_slice(&plaintext).ok()?;
    if request.nonce != peer.nonce || request.seq <= peer.seq || request.id.trim().is_empty() {
        return None;
    }
    peer.seq = request.seq;
    Some(request)
}

fn host_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| "Wisp".into())
}

/// One connection to the relay; returns why it ended.
async fn session(
    server: &Server,
    relay_url: &str,
    token: &str,
    code: &RemoteCode,
    status: &Mutex<(RemoteState, String)>,
    clients: &AtomicU32,
) -> String {
    let (url, _) = match wisp_sync::remote_endpoints(relay_url, code) {
        Ok(endpoints) => endpoints,
        Err(error) => return error.to_string(),
    };
    let mut request = match url.as_str().into_client_request() {
        Ok(request) => request,
        Err(error) => return error.to_string(),
    };
    match format!("Bearer {token}").parse() {
        Ok(value) => request.headers_mut().insert("authorization", value),
        Err(_) => return "Invalid relay token".into(),
    };
    // Relay-to-host frames only carry browser requests.
    let config = WebSocketConfig::default().max_message_size(Some(1024 * 1024));
    let socket = match tokio::time::timeout(
        Duration::from_secs(20),
        tokio_tungstenite::connect_async_with_config(request, Some(config), false),
    )
    .await
    {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(error)) => return error.to_string(),
        Err(_) => return "Timed out connecting to the relay".into(),
    };
    *status.lock().unwrap() = (RemoteState::Connected, String::new());

    let key = code.key();
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<HostFrame>(64);
    let mut peers = HashMap::<u64, Peer>::new();
    let mut last_seen = Instant::now();
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    loop {
        let outgoing = tokio::select! {
            message = stream.next() => {
                last_seen = Instant::now();
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None => {
                        return "The relay closed the connection".into()
                    }
                    Some(Err(error)) => return error.to_string(),
                    Some(Ok(_)) => continue,
                };
                let Ok(frame) = serde_json::from_str::<HostFrame>(&text) else { continue };
                match frame {
                    HostFrame::Open { c } => {
                        if peers.len() >= MAX_CLIENTS {
                            HostFrame::Close { c }
                        } else {
                            let nonce = uuid::Uuid::new_v4().simple().to_string();
                            let hello = json!({
                                "type": "hello",
                                "nonce": nonce,
                                "name": host_name(),
                                "version": env!("CARGO_PKG_VERSION"),
                                // The relay serves the page, so it can be
                                // newer than this host: it offers only
                                // what is listed.
                                "commands": server.commands(),
                            });
                            peers.insert(c, Peer { nonce, seq: 0 });
                            clients.store(peers.len() as u32, Ordering::Relaxed);
                            match seal(&key, &hello) {
                                Ok(d) => HostFrame::Msg { c, d },
                                Err(error) => return error,
                            }
                        }
                    }
                    HostFrame::Msg { c, d } => {
                        // Forged, replayed or reordered frames are dropped.
                        let Some(request) =
                            peers.get_mut(&c).and_then(|peer| open_request(&key, &d, peer))
                        else {
                            continue;
                        };
                        let (server, out) = (server.clone(), out_tx.clone());
                        tokio::spawn(async move {
                            let id = request.id.clone();
                            let known = server.commands().contains(&request.command.as_str());
                            let result = if known && request.args.is_object() {
                                server.dispatch(request).await
                            } else {
                                Err("This command is not available on this host".into())
                            };
                            if let Ok(d) = seal_reply(&key, &id, result) {
                                let _ = out.send(HostFrame::Msg { c, d }).await;
                            }
                        });
                        continue;
                    }
                    HostFrame::Close { c } => {
                        peers.remove(&c);
                        clients.store(peers.len() as u32, Ordering::Relaxed);
                        continue;
                    }
                }
            }
            Some(frame) = out_rx.recv() => frame,
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > RELAY_SILENCE {
                    return "The relay stopped responding".into();
                }
                if let Err(error) = sink.send(Message::Ping(Default::default())).await {
                    return error.to_string();
                }
                continue;
            }
        };
        let text = serde_json::to_string(&outgoing).unwrap_or_default();
        if let Err(error) = sink.send(Message::Text(text.into())).await {
            return error.to_string();
        }
    }
}

/// Keeps the tunnel up until told to stop, dialling again after a drop
/// with a wait that grows to half a minute.
async fn run(
    server: Server,
    relay_url: String,
    token: String,
    code: RemoteCode,
    status: Arc<Mutex<(RemoteState, String)>>,
    clients: Arc<AtomicU32>,
    mut stop: watch::Receiver<bool>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        *status.lock().unwrap() = (RemoteState::Connecting, String::new());
        let started = Instant::now();
        let error = tokio::select! {
            error = session(&server, &relay_url, &token, &code, &status, &clients) => error,
            _ = stop.changed() => return,
        };
        clients.store(0, Ordering::Relaxed);
        if *stop.borrow() {
            return;
        }
        *status.lock().unwrap() = (RemoteState::Failed, error);
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = stop.changed() => return,
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

impl Server {
    async fn remote_setting(&self, key: &str) -> String {
        self.0
            .store
            .get_setting(key)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn remote_secret(&self, name: &str) -> String {
        self.0.secrets.get(name).unwrap_or_default()
    }

    /// The code kept for this data directory; made when `create` asks.
    async fn remote_code(&self, create: bool) -> Result<Option<RemoteCode>> {
        let key_id = match (self.known_key_id().await, create) {
            (Some(id), _) => id,
            (None, true) => self.key_id().await?,
            (None, false) => return Ok(None),
        };
        if let Ok(code) = RemoteCode::parse(&self.remote_secret(&code_secret(&key_id))) {
            return Ok(Some(code));
        }
        if !create {
            return Ok(None);
        }
        let code = RemoteCode::generate()?;
        self.0.secrets.set(&code_secret(&key_id), &code.display())?;
        Ok(Some(code))
    }

    /// Remote web access as it stands.
    pub async fn remote_access(&self) -> Result<RemoteAccess> {
        let relay_url = self.remote_setting(URL_KEY).await;
        let code = self.remote_code(false).await?;
        let has_token = match self.known_key_id().await {
            Some(key_id) => !self.remote_secret(&token_secret(&key_id)).is_empty(),
            None => false,
        };
        let (state, detail) = self.0.remote.status.lock().unwrap().clone();
        Ok(RemoteAccess {
            enabled: self.remote_setting(ENABLED_KEY).await == "true",
            has_token,
            state,
            detail,
            link: code
                .as_ref()
                .and_then(|code| wisp_sync::remote_endpoints(&relay_url, code).ok())
                .map(|(_, link)| link),
            code: code.map(|code| code.display()),
            relay_url,
            clients: self.0.remote.clients.load(Ordering::Relaxed),
        })
    }

    /// Keeps the relay's address, a typed token (none keeps the stored
    /// one), and turns the tunnel on or off. A refused form changes
    /// nothing.
    pub async fn set_remote_access(
        &self,
        enabled: bool,
        relay_url: &str,
        token: Option<String>,
    ) -> Result<RemoteAccess> {
        let relay_url = relay_url.trim();
        if !relay_url.is_empty() {
            // The browser page needs HTTPS for its encryption.
            wisp_sync::remote_relay_base(relay_url)?;
        }
        let typed = token.as_deref().map(str::trim).unwrap_or_default();
        ensure!(
            typed.len() <= 4_096 && !typed.contains(char::is_whitespace),
            "A token is one word of at most 4096 characters"
        );
        let key_id = self.key_id().await?;
        let stored = self.remote_secret(&token_secret(&key_id));
        ensure!(
            !enabled || (!relay_url.is_empty() && !(typed.is_empty() && stored.is_empty())),
            MISSING_RELAY
        );
        if !typed.is_empty() {
            self.0.secrets.set(&token_secret(&key_id), typed)?;
        }
        self.0.store.set_setting(URL_KEY, relay_url).await?;
        self.0
            .store
            .set_setting(ENABLED_KEY, if enabled { "true" } else { "false" })
            .await?;
        if enabled {
            self.start_remote_access().await;
        } else {
            self.0.remote.stop();
        }
        self.remote_access().await
    }

    /// Makes a new code. Every old link stops working and connected
    /// browsers are dropped.
    pub async fn reset_remote_access_code(&self) -> Result<RemoteAccess> {
        let key_id = self.key_id().await?;
        let code = RemoteCode::generate()?;
        self.0.secrets.set(&code_secret(&key_id), &code.display())?;
        if self.remote_setting(ENABLED_KEY).await == "true" {
            self.start_remote_access().await;
        }
        self.remote_access().await
    }

    /// Dials the relay when remote access is on. The desktop calls this
    /// once it has opened the host; a change in Settings calls it again.
    pub async fn start_remote_access(&self) {
        let tunnel = &self.0.remote;
        tunnel.stop();
        if self.remote_setting(ENABLED_KEY).await != "true" {
            return;
        }
        let failed = |why: String| *tunnel.status.lock().unwrap() = (RemoteState::Failed, why);
        let relay_url = self.remote_setting(URL_KEY).await;
        let token = match self.known_key_id().await {
            Some(key_id) => self.remote_secret(&token_secret(&key_id)),
            None => String::new(),
        };
        if relay_url.trim().is_empty() || token.is_empty() {
            return failed(MISSING_RELAY.into());
        }
        let code = match self.remote_code(true).await {
            Ok(Some(code)) => code,
            Ok(None) => return,
            Err(error) => return failed(error.to_string()),
        };
        let (stop, stopped) = watch::channel(false);
        *tunnel.stop.lock().unwrap() = Some(stop);
        tokio::spawn(run(
            self.clone(),
            relay_url,
            token,
            code,
            tunnel.status.clone(),
            tunnel.clients.clone(),
            stopped,
        ));
    }
}
