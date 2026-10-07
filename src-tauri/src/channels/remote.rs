//! Remote web access (#1460). The desktop dials out to a self-hosted
//! `wisp-relay`; browsers holding the connection code reach the shared native
//! broker through a short allowlist. Frames are sealed end to end, so the relay
//! sees neither requests nor transcripts.
use super::{
    emit_channels_updated, get_setting, load_secret, set_status, status_snapshot, ChannelManager,
    ChannelStatus,
};
use crate::native_settings::Broker;
use crate::AppState;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, protocol::WebSocketConfig, Message,
};
use wisp_dto::native_conversations::{ApprovalRequest, ApprovalScope, CreateRequest, SendRequest};
use wisp_dto::native_settings::{Request, Response, SCHEMA};
use wisp_dto::RemoteAccessStatus;
use wisp_store::secrets::Secret;
use wisp_sync::{open_frame, seal_frame, HostFrame, RemoteCode, CLIENT_TO_HOST, HOST_TO_CLIENT};

const ENABLED_KEY: &str = "remote_access_enabled";
const URL_KEY: &str = "remote_relay_url";
const TOKEN_SECRET: &str = "remote_relay_token";
const CODE_SECRET: &str = "remote_access_code";

/// What a remote browser may run. Terminal input, kernel execution and file
/// writes stay off the tunnel until identity, roles and audit exist (#1460).
const REMOTE_COMMANDS: &[&str] = &[
    "list_projects",
    "remote_sessions",
    "native_conversation_create",
    "native_conversation_snapshot",
    "native_conversation_send",
    "native_conversation_stop",
    "native_conversation_approve",
];
const MAX_CLIENTS: usize = 8;
const HEARTBEAT: Duration = Duration::from_secs(25);
const RELAY_SILENCE: Duration = Duration::from_secs(75);

#[derive(Debug, Deserialize)]
struct RemoteRequest {
    nonce: String,
    seq: u64,
    id: String,
    command: String,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    args: Value,
}

struct Peer {
    nonce: String,
    seq: u64,
}

impl ChannelManager {
    pub fn stop_remote(&self) {
        if let Some(tx) = self.remote.lock().unwrap().take() {
            let _ = tx.send(true);
        }
        self.remote_clients.store(0, Ordering::Relaxed);
        set_status(&self.remote_status, "stopped", "");
    }

    pub async fn start_remote(&self, app: &AppHandle) {
        self.stop_remote();
        let state = app.state::<AppState>();
        let relay_url = get_setting(&state.store, URL_KEY).await;
        let token = load_secret(TOKEN_SECRET).await;
        if relay_url.trim().is_empty() || token.is_empty() {
            return set_status(&self.remote_status, "error", MISSING_RELAY);
        }
        let code = match load_code(true).await {
            Ok(Some(code)) => code,
            Ok(None) => return,
            Err(error) => return set_status(&self.remote_status, "error", &error),
        };
        let (tx, rx) = watch::channel(false);
        *self.remote.lock().unwrap() = Some(tx);
        let status = self.remote_status.clone();
        let clients = self.remote_clients.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            run(app, relay_url, token, code, status, clients, rx).await;
        });
    }
}

const MISSING_RELAY: &str =
    "Enter the relay server URL and access token before enabling remote access.";

pub(super) async fn autostart(app: &AppHandle) {
    let state = app.state::<AppState>();
    if get_setting(&state.store, ENABLED_KEY).await == "true" {
        app.state::<ChannelManager>().start_remote(app).await;
    }
}

async fn write_secret(name: &'static str, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || Secret::set(name, &value))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

async fn load_code(create: bool) -> Result<Option<RemoteCode>, String> {
    if let Ok(code) = RemoteCode::parse(&load_secret(CODE_SECRET).await) {
        return Ok(Some(code));
    }
    if !create {
        return Ok(None);
    }
    let code = RemoteCode::generate().map_err(|error| error.to_string())?;
    write_secret(CODE_SECRET, code.display()).await?;
    Ok(Some(code))
}

pub(super) async fn status(app: &AppHandle) -> RemoteAccessStatus {
    let state = app.state::<AppState>();
    let mgr = app.state::<ChannelManager>();
    let relay_url = get_setting(&state.store, URL_KEY).await;
    let code = load_code(false).await.ok().flatten();
    let runtime = status_snapshot(&mgr.remote_status);
    RemoteAccessStatus {
        enabled: get_setting(&state.store, ENABLED_KEY).await == "true",
        has_token: !load_secret(TOKEN_SECRET).await.is_empty(),
        state: runtime.state,
        detail: runtime.detail,
        link: code
            .as_ref()
            .and_then(|code| wisp_sync::remote_endpoints(&relay_url, code).ok())
            .map(|(_, link)| link),
        code: code.map(|code| code.display()),
        relay_url,
        clients: mgr.remote_clients.load(Ordering::Relaxed),
    }
}

#[tauri::command]
pub(crate) async fn set_remote_access(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
    app: AppHandle,
    enabled: bool,
    relay_url: String,
    relay_token: String,
) -> Result<(), String> {
    let relay_url = relay_url.trim().to_string();
    if !relay_url.is_empty() {
        wisp_sync::relay_base(&relay_url).map_err(|error| error.to_string())?;
    }
    state
        .store
        .set_setting(URL_KEY, &relay_url)
        .await
        .map_err(|error| error.to_string())?;
    let token = relay_token.trim().to_string();
    if !token.is_empty() {
        // Empty input keeps the stored token, mirroring the other channels.
        write_secret(TOKEN_SECRET, token).await?;
    }
    if enabled && (relay_url.is_empty() || load_secret(TOKEN_SECRET).await.is_empty()) {
        return Err(MISSING_RELAY.into());
    }
    state
        .store
        .set_setting(ENABLED_KEY, if enabled { "true" } else { "false" })
        .await
        .map_err(|error| error.to_string())?;
    if enabled {
        mgr.start_remote(&app).await;
    } else {
        mgr.stop_remote();
    }
    emit_channels_updated(&app);
    Ok(())
}

/// A new code invalidates every link and disconnects open browsers.
#[tauri::command]
pub(crate) async fn reset_remote_access_code(
    state: State<'_, AppState>,
    mgr: State<'_, ChannelManager>,
    app: AppHandle,
) -> Result<(), String> {
    let code = RemoteCode::generate().map_err(|error| error.to_string())?;
    write_secret(CODE_SECRET, code.display()).await?;
    if get_setting(&state.store, ENABLED_KEY).await == "true" {
        mgr.start_remote(&app).await;
    }
    emit_channels_updated(&app);
    Ok(())
}

async fn run(
    app: AppHandle,
    relay_url: String,
    token: String,
    code: RemoteCode,
    status: Arc<StdMutex<ChannelStatus>>,
    clients: Arc<AtomicU32>,
    mut stop: watch::Receiver<bool>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        set_status(&status, "connecting", "");
        emit_channels_updated(&app);
        let started = Instant::now();
        let error = tokio::select! {
            error = session(&app, &relay_url, &token, &code, &status, &clients) => error,
            _ = stop.changed() => return,
        };
        clients.store(0, Ordering::Relaxed);
        if *stop.borrow() {
            return;
        }
        tracing::warn!(target: "wisp", %error, "remote access tunnel dropped");
        set_status(&status, "error", &error);
        emit_channels_updated(&app);
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

/// One relay connection; returns why it ended.
async fn session(
    app: &AppHandle,
    relay_url: &str,
    token: &str,
    code: &RemoteCode,
    status: &Arc<StdMutex<ChannelStatus>>,
    clients: &Arc<AtomicU32>,
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
    // ponytail: dials directly; add proxy support if relays sit behind one.
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
    set_status(status, "running", "");
    emit_channels_updated(app);

    let key = code.key();
    let broker = crate::native_settings::remote_broker(app);
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
                    Some(Ok(Message::Close(_))) | None => return "The relay closed the connection".into(),
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
                            });
                            peers.insert(c, Peer { nonce, seq: 0 });
                            clients.store(peers.len() as u32, Ordering::Relaxed);
                            emit_channels_updated(app);
                            match seal(&key, &hello) {
                                Ok(d) => HostFrame::Msg { c, d },
                                Err(error) => return error,
                            }
                        }
                    }
                    HostFrame::Msg { c, d } => {
                        // Forged, replayed or reordered frames are dropped silently.
                        let Some(request) = peers.get_mut(&c).and_then(|peer| open_request(&key, &d, peer)) else {
                            continue;
                        };
                        let broker = broker.clone();
                        let out = out_tx.clone();
                        tauri::async_runtime::spawn(async move {
                            let result = dispatch(&broker, &request).await;
                            let response = Response {
                                schema: SCHEMA.into(),
                                id: request.id,
                                result: result.as_ref().ok().cloned(),
                                error: result.err(),
                            };
                            if let Ok(d) = seal(&key, &json!({"type": "response", "response": response})) {
                                let _ = out.send(HostFrame::Msg { c, d }).await;
                            }
                        });
                        continue;
                    }
                    HostFrame::Close { c } => {
                        peers.remove(&c);
                        clients.store(peers.len() as u32, Ordering::Relaxed);
                        emit_channels_updated(app);
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

fn seal(key: &[u8; 32], value: &Value) -> Result<String, String> {
    seal_frame(key, HOST_TO_CLIENT, value.to_string().as_bytes()).map_err(|error| error.to_string())
}

/// Requests are bound to the browser connection's hello nonce and strictly
/// increasing, so the relay can neither replay nor reorder them.
fn open_request(key: &[u8; 32], frame: &str, peer: &mut Peer) -> Option<RemoteRequest> {
    let plaintext = open_frame(key, CLIENT_TO_HOST, frame).ok()?;
    let request: RemoteRequest = serde_json::from_slice(&plaintext).ok()?;
    if request.nonce != peer.nonce || request.seq <= peer.seq {
        return None;
    }
    peer.seq = request.seq;
    Some(request)
}

fn host_name() -> String {
    // ponytail: env only; macOS GUI apps get the fallback until a hostname API is linked.
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| "Wisp".into())
}

fn decode<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|error| error.to_string())
}

/// Validates the remote subset, then reuses the native broker. The actor comes
/// from the broker's `remote` stamp, never from the request.
async fn dispatch(broker: &Broker, request: &RemoteRequest) -> Result<Value, String> {
    check(request)?;
    let project_id = request.project_id.clone().filter(|id| !id.is_empty());
    if request.command == "remote_sessions" {
        let project = project_id.ok_or("A project is required")?;
        return crate::native_settings::invoke_command(
            broker,
            Some(project.clone()),
            "search_sessions",
            json!({"query": "", "limit": 100, "projectId": project}),
        )
        .await;
    }
    crate::native_settings::dispatch(
        broker,
        &Request {
            schema: SCHEMA.into(),
            id: request.id.clone(),
            project_id,
            command: request.command.clone(),
            args: request.args.clone(),
        },
    )
    .await
}

fn check(request: &RemoteRequest) -> Result<(), String> {
    if !REMOTE_COMMANDS.contains(&request.command.as_str()) || !request.args.is_object() {
        return Err("This command is not available remotely".into());
    }
    match request.command.as_str() {
        "native_conversation_send" => {
            let args: SendRequest = decode(&request.args)?;
            if !args.attachments.is_empty() || !args.references.is_empty() {
                return Err("Remote messages cannot attach files yet".into());
            }
        }
        "native_conversation_approve" => {
            let args: ApprovalRequest = decode(&request.args)?;
            if args.scope != ApprovalScope::Once {
                return Err("Remote approvals apply to one request only".into());
            }
        }
        "native_conversation_create" => {
            let args: CreateRequest = decode(&request.args)?;
            if args.acp_agent_id.is_some() {
                return Err("ACP conversations can only be continued on the desktop".into());
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(command: &str, args: Value) -> RemoteRequest {
        RemoteRequest {
            nonce: "n".into(),
            seq: 1,
            id: "1".into(),
            command: command.into(),
            project_id: Some("p".into()),
            args,
        }
    }

    #[test]
    fn remote_allowlist_excludes_code_execution_and_permanent_grants() {
        for command in [
            "native_conversation_terminal_write",
            "native_conversation_panel_runtime_execute",
            "native_conversation_panel_savefile",
            "native_conversation_panel_file_action",
            "native_conversation_enqueue",
            "native_conversation_attach",
            "write_terminal",
            "channels_status",
        ] {
            assert!(check(&request(command, json!({}))).is_err(), "{command}");
        }
        assert!(check(&request("list_projects", json!({}))).is_ok());
        assert!(check(&request("list_projects", json!([]))).is_err());
        let approve = |scope: &str| json!({"session_id": "s", "approval_id": "a", "approved": true, "scope": scope});
        assert!(check(&request("native_conversation_approve", approve("once"))).is_ok());
        assert!(check(&request("native_conversation_approve", approve("project"))).is_err());
        let send = json!({"session_id": "s", "request_id": "r", "message": "hi"});
        assert!(check(&request("native_conversation_send", send)).is_ok());
        let attached = json!({"session_id": "s", "request_id": "r", "message": "hi", "attachments": ["a.txt"]});
        assert!(check(&request("native_conversation_send", attached)).is_err());
        let acp = json!({"acp_agent_id": "codex"});
        assert!(check(&request("native_conversation_create", acp)).is_err());
    }

    #[test]
    fn requests_are_bound_to_the_connection_and_never_replayed() {
        let code = RemoteCode::generate().unwrap();
        let key = code.key();
        let mut peer = Peer {
            nonce: "conn-1".into(),
            seq: 0,
        };
        let frame = |nonce: &str, seq: u64| {
            let body = json!({"nonce": nonce, "seq": seq, "id": "x", "command": "list_projects", "args": {}});
            seal_frame(&key, CLIENT_TO_HOST, body.to_string().as_bytes()).unwrap()
        };
        let first = frame("conn-1", 1);
        assert!(open_request(&key, &first, &mut peer).is_some());
        assert!(open_request(&key, &first, &mut peer).is_none(), "replay");
        assert!(
            open_request(&key, &frame("conn-0", 2), &mut peer).is_none(),
            "other connection"
        );
        assert!(open_request(&key, &frame("conn-1", 3), &mut peer).is_some());
        assert!(
            open_request(&key, &frame("conn-1", 2), &mut peer).is_none(),
            "reordered"
        );
        // A host frame reflected back by the relay does not authenticate.
        let reflected = seal(&key, &json!({"nonce": "conn-1", "seq": 9})).unwrap();
        assert!(open_request(&key, &reflected, &mut peer).is_none());
        let other = RemoteCode::generate().unwrap().key();
        let forged = seal_frame(&other, CLIENT_TO_HOST, b"{}").unwrap();
        assert!(open_request(&key, &forged, &mut peer).is_none());
    }
}
