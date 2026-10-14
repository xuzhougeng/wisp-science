//! Serve the existing encrypted mobile protocol directly from a headless host.
//! The independent ciphertext-only relay keeps its existing routes and behavior.
use crate::{open_frame, remote, seal_frame, RemoteCode, CLIENT_TO_HOST, HOST_TO_CLIENT};
use async_trait::async_trait;
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use wisp_dto::{native_settings, remote_access::RemoteRequest};

#[async_trait]
pub trait DirectRemoteHost: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn commands(&self) -> &'static [&'static str];
    /// Mutations must return after durable acceptance, not after the agent turn.
    /// A client disconnect must never own or cancel an accepted task.
    async fn dispatch(&self, request: RemoteRequest) -> Result<Value, String>;
}

#[derive(Clone)]
struct DirectState {
    code: RemoteCode,
    host: Arc<dyn DirectRemoteHost>,
    clients: Arc<Semaphore>,
}

pub fn direct_remote_router(code: RemoteCode, host: Arc<dyn DirectRemoteHost>) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::temporary("remote") }))
        .route("/healthz", get(|| async { "ok" }))
        .route("/remote", get(remote::page))
        .route("/remote.js", get(remote::script))
        .route("/remote.webmanifest", get(remote::manifest))
        .route(
            "/remote-icon-192.png",
            get(|| remote::icon(remote::ICON_192)),
        )
        .route(
            "/remote-icon-512.png",
            get(|| remote::icon(remote::ICON_512)),
        )
        .route(
            "/remote-touch-icon.png",
            get(|| remote::icon(remote::TOUCH_ICON)),
        )
        .route("/v1/remote/client/{sid}", get(connect))
        .with_state(DirectState {
            code,
            host,
            clients: Arc::new(Semaphore::new(16)),
        })
}

async fn connect(
    State(state): State<DirectState>,
    Path(sid): Path<String>,
    request: Request,
) -> Response {
    if sid != state.code.sid() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(permit) = state.clients.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    remote::upgrade(request, 1024 * 1024, move |socket| async move {
        let _permit = permit;
        serve(state, socket).await;
    })
}

struct Peer {
    nonce: String,
    sequence: u64,
}

impl Peer {
    fn open(&mut self, key: &[u8; 32], text: &str) -> Option<RemoteRequest> {
        let bytes = open_frame(key, CLIENT_TO_HOST, text).ok()?;
        let request: RemoteRequest = serde_json::from_slice(&bytes).ok()?;
        if request.nonce != self.nonce
            || request.seq <= self.sequence
            || request.id.trim().is_empty()
        {
            return None;
        }
        self.sequence = request.seq;
        Some(request)
    }
}

async fn send<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    key: &[u8; 32],
    value: Value,
) -> Result<(), ()> {
    let bytes = serde_json::to_vec(&value).map_err(|_| ())?;
    let frame = seal_frame(key, HOST_TO_CLIENT, &bytes).map_err(|_| ())?;
    tokio::time::timeout(
        Duration::from_secs(10),
        socket.send(Message::Text(frame.into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    state: DirectState,
    mut socket: WebSocketStream<S>,
) {
    let key = state.code.key();
    let mut peer = Peer {
        nonce: uuid::Uuid::new_v4().to_string(),
        sequence: 0,
    };
    if send(
        &mut socket,
        &key,
        json!({
            "type":"hello", "nonce":peer.nonce, "name":state.host.name(),
            "version":env!("CARGO_PKG_VERSION"), "commands":state.host.commands()
        }),
    )
    .await
    .is_err()
    {
        return;
    }
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    // A sid is public. Unauthenticated peers cannot occupy slots indefinitely.
    let deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            _ = ping.tick() => {
                if tokio::time::timeout(Duration::from_secs(10), socket.send(Message::Ping(Default::default())))
                    .await.map_or(true, |result| result.is_err()) { break; }
            }
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let Some(request) = peer.open(&key, &text) else { break; };
                    deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(75));
                    let id = request.id.clone();
                    let result = if !state.host.commands().contains(&request.command.as_str()) || !request.args.is_object() {
                        Err("This command is not available on this host".into())
                    } else {
                        state.host.dispatch(request).await
                    };
                    let mut response = native_settings::Response {
                        schema: native_settings::SCHEMA.into(), id,
                        result: result.as_ref().ok().cloned(), error: result.err(),
                    };
                    // Allow for base64 encryption overhead and envelope bytes.
                    if serde_json::to_vec(&response).map_or(true, |bytes| bytes.len() > 10 * 1024 * 1024) {
                        response.result = None;
                        response.error = Some("Response is too large; request a smaller page".into());
                    }
                    if send(&mut socket, &key, json!({"type":"response", "response":response})).await.is_err() { break; }
                }
                Some(Ok(Message::Pong(_))) => {
                    if peer.sequence > 0 {
                        deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(75));
                    }
                }
                Some(Ok(Message::Ping(_))) => {}
                _ => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[derive(Default)]
    struct Host(std::sync::atomic::AtomicUsize);
    #[async_trait]
    impl DirectRemoteHost for Host {
        fn name(&self) -> &str {
            "offline test"
        }
        fn commands(&self) -> &'static [&'static str] {
            &["list_projects"]
        }
        async fn dispatch(&self, _: RemoteRequest) -> Result<Value, String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(json!([]))
        }
    }

    #[tokio::test]
    async fn direct_routes_serve_mobile_assets_without_exposing_relay_or_native_invoke() {
        let app = direct_remote_router(RemoteCode::generate().unwrap(), Arc::new(Host::default()));
        for (path, expected) in [
            ("/remote", 200),
            ("/remote.js", 200),
            ("/healthz", 200),
            ("/invoke", 404),
            ("/v1/remote/host/anything", 404),
            ("/v1/remote/client/wrong", 404),
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected, "{path}");
        }
    }

    #[tokio::test]
    async fn encrypted_socket_dispatches_allowlisted_requests_and_closes_on_replay() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let code = RemoteCode::generate().unwrap();
        let host = Arc::new(Host::default());
        let state = DirectState {
            code: code.clone(),
            host: host.clone(),
            clients: Arc::new(Semaphore::new(1)),
        };
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
        let mut client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let task = tokio::spawn(serve(state, server));
        async fn receive(
            client: &mut WebSocketStream<tokio::io::DuplexStream>,
            key: &[u8; 32],
        ) -> Value {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match client.next().await.unwrap().unwrap() {
                        Message::Text(text) => {
                            return serde_json::from_slice(
                                &open_frame(key, HOST_TO_CLIENT, &text).unwrap(),
                            )
                            .unwrap()
                        }
                        Message::Ping(_) => {
                            client.flush().await.unwrap();
                        }
                        other => panic!("Unexpected frame: {other:?}"),
                    }
                }
            })
            .await
            .unwrap()
        }
        let hello = receive(&mut client, &code.key()).await;
        assert_eq!(hello["type"], "hello");
        for (seq, command) in [(1, "list_projects"), (2, "forbidden")] {
            let bytes = serde_json::to_vec(
                &json!({"nonce":hello["nonce"],"seq":seq,"id":"r","command":command,"args":{}}),
            )
            .unwrap();
            let encrypted = seal_frame(&code.key(), CLIENT_TO_HOST, &bytes).unwrap();
            client
                .send(Message::Text(encrypted.clone().into()))
                .await
                .unwrap();
            let reply = receive(&mut client, &code.key()).await;
            if seq == 1 {
                assert_eq!(reply["response"]["result"], json!([]));
            } else {
                assert!(reply["response"]["error"].is_string());
                client.send(Message::Text(encrypted.into())).await.unwrap();
            }
        }
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(host.0.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn requests_require_key_direction_nonce_and_monotonic_sequence() {
        let code = RemoteCode::generate().unwrap();
        let mut peer = Peer {
            nonce: "connection-a".into(),
            sequence: 0,
        };
        let bytes = serde_json::to_vec(&json!({"nonce":"connection-a", "seq":1, "id":"r", "command":"list_projects", "args":{}})).unwrap();
        let wrong_direction = seal_frame(&code.key(), HOST_TO_CLIENT, &bytes).unwrap();
        assert!(peer.open(&code.key(), &wrong_direction).is_none());
        let valid = seal_frame(&code.key(), CLIENT_TO_HOST, &bytes).unwrap();
        assert!(peer
            .open(&RemoteCode::generate().unwrap().key(), &valid)
            .is_none());
        assert!(peer.open(&code.key(), &valid).is_some());
        assert!(peer.open(&code.key(), &valid).is_none());
        let mut reconnect = Peer {
            nonce: "connection-b".into(),
            sequence: 0,
        };
        assert!(reconnect.open(&code.key(), &valid).is_none());
    }
}
