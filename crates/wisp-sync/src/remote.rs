//! Remote web access rendezvous (#1460).
//!
//! A desktop host dials out to `/v1/remote/host/{sid}`; browsers open
//! `/remote#<code>` and connect to `/v1/remote/client/{sid}`. The relay pairs
//! sockets by `sid` and forwards opaque AES-GCM frames. The connection code
//! never reaches the relay: it lives in the URL fragment, and the relay only
//! sees `sid = SHA-256("wisp-remote/sid/v1" || code)`, from which the frame key
//! `SHA-256("wisp-remote/key/v1" || code)` cannot be derived.
use crate::http::{authorized, unauthorized, RelayHttpState};
use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use ring::{
    aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM},
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    tungstenite::{
        handshake::derive_accept_key,
        protocol::{frame::CloseFrame, Role, WebSocketConfig},
        Message,
    },
    WebSocketStream,
};
use url::Url;

type WebSocket = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;

pub const REMOTE_CODE_BYTES: usize = 16;
/// AAD labels bind each frame to its direction, so the relay cannot reflect a
/// host frame back to the host (or a browser frame back to a browser).
pub const HOST_TO_CLIENT: &[u8] = b"wisp-remote/v1/h2c";
pub const CLIENT_TO_HOST: &[u8] = b"wisp-remote/v1/c2h";
/// Host frames carry transcript snapshots; browser frames only carry requests.
pub const REMOTE_HOST_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const CLIENT_MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_HOSTS: usize = 1024;
const MAX_CLIENTS_PER_HOST: usize = 16;
/// Below common reverse-proxy idle timeouts (nginx defaults to 60 s).
const PING_EVERY: Duration = Duration::from_secs(25);
/// WebSocket close code a browser receives while no host serves its code.
pub const HOST_OFFLINE_CLOSE: u16 = 4404;

/// Secret shared by link/code between the desktop and its browsers.
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteCode([u8; REMOTE_CODE_BYTES]);

impl RemoteCode {
    pub fn generate() -> Result<Self> {
        let mut bytes = [0_u8; REMOTE_CODE_BYTES];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| anyhow::anyhow!("could not generate a remote access code"))?;
        Ok(Self(bytes))
    }

    /// Accepts the grouped display form; separators and case are ignored.
    pub fn parse(text: &str) -> Result<Self> {
        let hex_text: String = text.chars().filter(char::is_ascii_hexdigit).collect();
        let bytes = hex::decode(hex_text.to_ascii_lowercase())
            .ok()
            .and_then(|bytes| <[u8; REMOTE_CODE_BYTES]>::try_from(bytes).ok())
            .context("invalid remote access code")?;
        Ok(Self(bytes))
    }

    /// `xxxx-xxxx-…`, eight groups of four lowercase hex digits.
    pub fn display(&self) -> String {
        hex::encode(self.0)
            .as_bytes()
            .chunks(4)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("-")
    }

    fn derive(&self, label: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(label);
        hasher.update(self.0);
        hasher.finalize().into()
    }

    /// Public rendezvous id; the only code-derived value the relay sees.
    pub fn sid(&self) -> String {
        hex::encode(&self.derive(b"wisp-remote/sid/v1")[..16])
    }

    pub fn key(&self) -> [u8; 32] {
        self.derive(b"wisp-remote/key/v1")
    }
}

/// `base64(iv || ciphertext || tag)`, matching WebCrypto AES-GCM output.
pub fn seal_frame(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Result<String> {
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, key).map_err(|_| anyhow::anyhow!("invalid frame key"))?,
    );
    let mut iv = [0_u8; 12];
    SystemRandom::new()
        .fill(&mut iv)
        .map_err(|_| anyhow::anyhow!("could not generate a frame nonce"))?;
    let mut out = iv.to_vec();
    let mut body = plaintext.to_vec();
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(iv), Aad::from(aad), &mut body)
        .map_err(|_| anyhow::anyhow!("could not encrypt remote frame"))?;
    out.extend_from_slice(&body);
    Ok(STANDARD.encode(out))
}

pub fn open_frame(key: &[u8; 32], aad: &[u8], frame: &str) -> Result<Vec<u8>> {
    let bytes = STANDARD.decode(frame).context("invalid remote frame")?;
    if bytes.len() < 12 + AES_256_GCM.tag_len() {
        anyhow::bail!("invalid remote frame");
    }
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, key).map_err(|_| anyhow::anyhow!("invalid frame key"))?,
    );
    let iv: [u8; 12] = bytes[..12].try_into()?;
    let mut body = bytes[12..].to_vec();
    let len = key
        .open_in_place(Nonce::assume_unique_for_key(iv), Aad::from(aad), &mut body)
        .map_err(|_| anyhow::anyhow!("remote frame authentication failed"))?
        .len();
    body.truncate(len);
    Ok(body)
}

fn loopback_host(host: Option<url::Host<&str>>) -> bool {
    match host {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

/// Whether plain HTTP to `host` is acceptable: this computer, or an address
/// that only exists inside a private network (RFC 1918, link-local, IPv6
/// unique local). On such a link the relay token and a lab member key travel
/// unencrypted, while everything they give access to is still end-to-end
/// encrypted: sync blobs, the lab knowledge base and lab mail.
// ponytail: address literals only. A host name is never enough, since whoever
// answers DNS decides where it leads, and the carrier-grade NAT range
// (100.64.0.0/10) is shared with strangers on some networks. Add an explicit
// per-relay opt-in if an intranet name or an overlay network needs HTTP.
pub(crate) fn plain_http_host(host: Option<url::Host<&str>>) -> bool {
    match host {
        Some(url::Host::Ipv4(address)) => {
            address.is_loopback() || address.is_private() || address.is_link_local()
        }
        Some(url::Host::Ipv6(address)) => {
            let head = address.segments()[0];
            address.is_loopback() || head & 0xfe00 == 0xfc00 || head & 0xffc0 == 0xfe80
        }
        host => loopback_host(host),
    }
}

pub(crate) const HTTPS_OR_PRIVATE: &str =
    "relay URL must use HTTPS (HTTP is allowed only for localhost and private network addresses)";

/// Normalized relay base URL for project sync and Wisp Lab: HTTPS, or HTTP
/// to this computer or to a private network address.
pub fn relay_base(relay_url: &str) -> Result<Url> {
    let mut base = Url::parse(relay_url.trim()).context("invalid relay URL")?;
    if base.scheme() != "https" && !(base.scheme() == "http" && plain_http_host(base.host())) {
        anyhow::bail!(HTTPS_OR_PRIVATE);
    }
    if base.cannot_be_a_base() {
        anyhow::bail!("invalid relay base URL");
    }
    base.set_query(None);
    base.set_fragment(None);
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    Ok(base)
}

/// Relay base URL for remote web access, which is stricter: a browser gives
/// the page its encryption (WebCrypto) only in a secure context, so an
/// intranet relay reached over HTTP would hand out links that cannot work.
pub fn remote_relay_base(relay_url: &str) -> Result<Url> {
    let base = relay_base(relay_url)?;
    if base.scheme() == "http" && !loopback_host(base.host()) {
        anyhow::bail!("relay URL must use HTTPS (HTTP is allowed only for localhost)");
    }
    Ok(base)
}

/// Host WebSocket URL and the shareable browser link for `relay_url`.
pub fn remote_endpoints(relay_url: &str, code: &RemoteCode) -> Result<(Url, String)> {
    let base = remote_relay_base(relay_url)?;
    let mut host = base.join(&format!("v1/remote/host/{}", code.sid()))?;
    host.set_scheme(if base.scheme() == "https" {
        "wss"
    } else {
        "ws"
    })
    .map_err(|_| anyhow::anyhow!("invalid relay URL scheme"))?;
    let mut link = base.join("remote")?;
    link.set_fragment(Some(&code.display()));
    Ok((host, link.into()))
}

/// Plaintext envelope between the relay and the host socket. `c` names one
/// browser connection; `d` is an opaque sealed frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum HostFrame {
    Open { c: u64 },
    Msg { c: u64, d: String },
    Close { c: u64 },
}

struct Host {
    generation: u64,
    tx: mpsc::Sender<HostFrame>,
    clients: HashMap<u64, mpsc::Sender<String>>,
}

// ponytail: in-memory only; a relay restart drops sockets and hosts redial.
#[derive(Default)]
pub(crate) struct RemoteHub {
    hosts: Mutex<HashMap<String, Host>>,
    next_id: AtomicU64,
}

fn valid_sid(sid: &str) -> bool {
    sid.len() == 32
        && sid
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(crate) async fn page() -> Response {
    static_asset("text/html; charset=utf-8", include_str!("remote.html"))
}

pub(crate) async fn script() -> Response {
    static_asset("text/javascript; charset=utf-8", include_str!("remote.js"))
}

/// Lets a phone keep the page on its home screen. Every URL is relative, so
/// a path prefix in front of the relay keeps working. The page restores the
/// connection code itself: `start_url` cannot carry a fragment per device.
pub(crate) async fn manifest() -> Response {
    static_asset(
        "application/manifest+json",
        r##"{"name":"Wisp Remote","short_name":"Wisp","start_url":"remote","display":"standalone","background_color":"#faf9f6","theme_color":"#faf9f6","icons":[{"src":"remote-icon-192.png","sizes":"192x192","type":"image/png"},{"src":"remote-icon-512.png","sizes":"512x512","type":"image/png"}]}"##,
    )
}

/// The desktop app's own icons, so the two cannot drift apart.
pub(crate) const ICON_192: &[u8] =
    include_bytes!("../../../src-tauri/icons/android/mipmap-xxxhdpi/ic_launcher.png");
pub(crate) const ICON_512: &[u8] = include_bytes!("../../../src-tauri/icons/icon.png");
/// iOS draws its own rounded corners over a full square.
pub(crate) const TOUCH_ICON: &[u8] =
    include_bytes!("../../../src-tauri/icons/ios/AppIcon-60x60@3x.png");

pub(crate) async fn icon(bytes: &'static [u8]) -> Response {
    static_asset("image/png", bytes)
}

fn static_asset(content_type: &'static str, body: impl Into<Body>) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; \
                 connect-src 'self'; img-src data: 'self'; manifest-src 'self'; \
                 base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            ),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body.into(),
    )
        .into_response()
}

/// RFC 6455 server upgrade on the tungstenite the desktop already links,
/// instead of axum's `ws` feature (which pins a second tungstenite).
fn upgrade<F, Fut>(request: Request, max_message: usize, run: F) -> Response
where
    F: FnOnce(WebSocket) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let headers = request.headers();
    let has = |name: header::HeaderName, token: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case(token))
            })
    };
    let Some(key) = headers.get(header::SEC_WEBSOCKET_KEY) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !has(header::UPGRADE, "websocket")
        || !has(header::CONNECTION, "upgrade")
        || !has(header::SEC_WEBSOCKET_VERSION, "13")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let accept = derive_accept_key(key.as_bytes());
    tokio::spawn(async move {
        let Ok(upgraded) = hyper::upgrade::on(request).await else {
            return;
        };
        let config = WebSocketConfig::default().max_message_size(Some(max_message));
        run(
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(config))
                .await,
        )
        .await;
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_ACCEPT, accept)
        .body(Body::empty())
        .unwrap()
}

pub(crate) async fn host_socket(
    State(state): State<RelayHttpState>,
    Path(sid): Path<String>,
    request: Request,
) -> Response {
    if !authorized(request.headers(), &state) {
        return unauthorized();
    }
    if !valid_sid(&sid) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    upgrade(request, REMOTE_HOST_MAX_FRAME_BYTES, move |socket| {
        run_host(state, sid, socket)
    })
}

pub(crate) async fn client_socket(
    State(state): State<RelayHttpState>,
    Path(sid): Path<String>,
    request: Request,
) -> Response {
    if !valid_sid(&sid) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    upgrade(request, CLIENT_MAX_FRAME_BYTES, move |socket| {
        run_client(state, sid, socket)
    })
}

async fn run_host(state: RelayHttpState, sid: String, socket: WebSocket) {
    let hub = &state.remote;
    let (tx, mut rx) = mpsc::channel(256);
    let generation = hub.next_id.fetch_add(1, Ordering::Relaxed);
    {
        let mut hosts = hub.hosts.lock().unwrap();
        if hosts.len() >= MAX_HOSTS && !hosts.contains_key(&sid) {
            return;
        }
        // A redial replaces a half-open predecessor. Dropping it drops its
        // client senders, which closes those browsers so they reconnect here.
        hosts.insert(
            sid.clone(),
            Host {
                generation,
                tx,
                clients: HashMap::new(),
            },
        );
    }
    tracing::info!(sid = %&sid[..8], "remote host connected");
    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING_EVERY);
    loop {
        tokio::select! {
            frame = rx.recv() => {
                let Some(frame) = frame else { break };
                let text = serde_json::to_string(&frame).unwrap_or_default();
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            message = stream.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<HostFrame>(&text) {
                        Ok(HostFrame::Msg { c, d }) => {
                            let mut hosts = hub.hosts.lock().unwrap();
                            if let Some(host) = hosts.get_mut(&sid).filter(|h| h.generation == generation) {
                                // A browser that cannot keep up is dropped, not
                                // allowed to stall every other browser.
                                if host.clients.get(&c).is_some_and(|client| client.try_send(d).is_err()) {
                                    host.clients.remove(&c);
                                }
                            }
                        }
                        Ok(HostFrame::Close { c }) => {
                            if let Some(host) = hub.hosts.lock().unwrap().get_mut(&sid).filter(|h| h.generation == generation) {
                                host.clients.remove(&c);
                            }
                        }
                        Ok(HostFrame::Open { .. }) | Err(_) => {}
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                if sink.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
        }
    }
    let mut hosts = hub.hosts.lock().unwrap();
    if hosts.get(&sid).is_some_and(|h| h.generation == generation) {
        hosts.remove(&sid);
    }
    tracing::info!(sid = %&sid[..8], "remote host disconnected");
}

async fn run_client(state: RelayHttpState, sid: String, mut socket: WebSocket) {
    let hub = &state.remote;
    let (tx, mut rx) = mpsc::channel::<String>(64);
    let id = hub.next_id.fetch_add(1, Ordering::Relaxed);
    let host = {
        let mut hosts = hub.hosts.lock().unwrap();
        hosts
            .get_mut(&sid)
            .filter(|host| host.clients.len() < MAX_CLIENTS_PER_HOST)
            .map(|host| {
                host.clients.insert(id, tx);
                host.tx.clone()
            })
    };
    let Some(host) = host else {
        return close_offline(socket).await;
    };
    if host.send(HostFrame::Open { c: id }).await.is_err() {
        return close_offline(socket).await;
    }
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut host_gone = false;
    loop {
        tokio::select! {
            frame = rx.recv() => match frame {
                Some(frame) => {
                    if socket.send(Message::Text(frame.into())).await.is_err() {
                        break;
                    }
                }
                None => {
                    host_gone = true;
                    break;
                }
            },
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    if host.send(HostFrame::Msg { c: id, d: text.as_str().to_owned() }).await.is_err() {
                        host_gone = true;
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                if socket.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
        }
    }
    if let Some(entry) = hub.hosts.lock().unwrap().get_mut(&sid) {
        entry.clients.remove(&id);
    }
    let _ = host.try_send(HostFrame::Close { c: id });
    if host_gone {
        close_offline(socket).await;
    }
}

async fn close_offline(mut socket: WebSocket) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: HOST_OFFLINE_CLOSE.into(),
            reason: "host offline".into(),
        })))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{relay_router, FileRelay};
    use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

    fn code() -> RemoteCode {
        RemoteCode((0..16).collect::<Vec<u8>>().try_into().unwrap())
    }

    #[test]
    fn code_derivation_and_frames_match_the_browser_client() {
        let code = code();
        assert_eq!(RemoteCode::parse(&code.display()).unwrap().0, code.0);
        assert_eq!(code.display(), "0001-0203-0405-0607-0809-0a0b-0c0d-0e0f");
        assert!(RemoteCode::parse("0001-0203").is_err());
        // Fixed vector shared with remote.js (WebCrypto SHA-256 + AES-GCM).
        assert_eq!(code.sid(), "cf4778d13d24d0dd1313bca1709267bb");
        let browser_frame = "AAECAwQFBgcICQoL9OOvsOOON1awkl3qncEpESEISdpba7HB8iw=";
        assert_eq!(
            open_frame(&code.key(), CLIENT_TO_HOST, browser_frame).unwrap(),
            br#"{"ping":1}"#
        );

        let sealed = seal_frame(&code.key(), HOST_TO_CLIENT, b"snapshot").unwrap();
        assert_eq!(
            open_frame(&code.key(), HOST_TO_CLIENT, &sealed).unwrap(),
            b"snapshot"
        );
        // Direction-bound: a host frame cannot be reflected back to the host.
        assert!(open_frame(&code.key(), CLIENT_TO_HOST, &sealed).is_err());
        let other = RemoteCode::generate().unwrap();
        assert!(open_frame(&other.key(), HOST_TO_CLIENT, &sealed).is_err());
    }

    #[test]
    fn endpoints_require_tls_except_on_loopback() {
        let code = code();
        let (host, link) = remote_endpoints("https://relay.example.test/wisp", &code).unwrap();
        assert_eq!(
            host.as_str(),
            format!(
                "wss://relay.example.test/wisp/v1/remote/host/{}",
                code.sid()
            )
        );
        assert_eq!(
            link,
            format!("https://relay.example.test/wisp/remote#{}", code.display())
        );
        let (host, _) = remote_endpoints("http://127.0.0.1:8787", &code).unwrap();
        assert_eq!(host.scheme(), "ws");
        assert!(remote_endpoints("http://relay.example.test", &code).is_err());
        // An intranet relay over HTTP serves sync and the lab, but not the
        // browser page: its link would open in an insecure context.
        assert!(relay_base("http://192.168.1.20:8787").is_ok());
        let refused = remote_endpoints("http://192.168.1.20:8787", &code).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "relay URL must use HTTPS (HTTP is allowed only for localhost)"
        );
        assert!(remote_relay_base("https://192.168.1.20").is_ok());
    }

    #[test]
    fn plain_http_is_for_this_computer_and_private_network_addresses_only() {
        for allowed in [
            "http://localhost:8787",
            "http://LOCALHOST:8787",
            "http://127.0.0.1:8787",
            "http://127.8.9.10",
            "http://[::1]:8787",
            "http://10.0.0.5:8787",
            "http://10.10.3.27:8787",
            "http://172.16.0.1",
            "http://172.31.255.254:8787/wisp",
            "http://192.168.1.20:8787",
            "http://169.254.10.20:8787",
            "http://[fd12:3456:789a::1]:8787",
            "http://[fe80::1]:8787",
        ] {
            assert!(relay_base(allowed).is_ok(), "{allowed}");
        }
        for refused in [
            "http://relay.example.test",
            // A name can point anywhere, even one that looks internal.
            "http://relay.lab.internal:8787",
            "http://nas.local",
            "http://8.8.8.8",
            // Just outside 172.16.0.0/12, and the carrier-grade NAT range.
            "http://172.32.0.1",
            "http://172.15.255.255",
            "http://100.64.0.1:8787",
            "http://192.169.1.1",
            "http://[2001:db8::1]:8787",
            "ftp://192.168.1.20",
        ] {
            let error = relay_base(refused).unwrap_err();
            assert_eq!(error.to_string(), HTTPS_OR_PRIVATE, "{refused}");
        }
        assert_eq!(
            relay_base(" http://192.168.1.20:8787/wisp?x=1#frag ")
                .unwrap()
                .as_str(),
            "http://192.168.1.20:8787/wisp/"
        );
    }

    async fn next_text(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> tungstenite::Message {
        loop {
            match socket.next().await.unwrap().unwrap() {
                tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_) => continue,
                other => return other,
            }
        }
    }

    #[tokio::test]
    async fn relay_pairs_browsers_with_the_authenticated_host_by_sid() {
        let root = std::env::temp_dir().join(format!("wisp-remote-{}", uuid::Uuid::new_v4()));
        let relay = FileRelay::open(&root).await.unwrap();
        let app = relay_router(RelayHttpState::new(relay, "token").unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let sid = code().sid();
        let client_url = format!("ws://{addr}/v1/remote/client/{sid}");

        // No host yet: the browser is told the host is offline.
        let (mut early, _) = tokio_tungstenite::connect_async(&client_url).await.unwrap();
        match next_text(&mut early).await {
            tungstenite::Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), HOST_OFFLINE_CLOSE)
            }
            other => panic!("expected offline close, got {other:?}"),
        }

        let host_url = format!("ws://{addr}/v1/remote/host/{sid}");
        assert!(tokio_tungstenite::connect_async(&host_url).await.is_err());
        let mut request = host_url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer token".parse().unwrap());
        let (mut host, _) = tokio_tungstenite::connect_async(request).await.unwrap();

        let (mut browser, _) = tokio_tungstenite::connect_async(&client_url).await.unwrap();
        let tungstenite::Message::Text(open) = next_text(&mut host).await else {
            panic!("expected open frame");
        };
        let HostFrame::Open { c } = serde_json::from_str(&open).unwrap() else {
            panic!("expected open frame");
        };

        browser
            .send(tungstenite::Message::Text("sealed-request".into()))
            .await
            .unwrap();
        let tungstenite::Message::Text(forwarded) = next_text(&mut host).await else {
            panic!("expected forwarded frame");
        };
        assert_eq!(
            serde_json::from_str::<HostFrame>(&forwarded).unwrap(),
            HostFrame::Msg {
                c,
                d: "sealed-request".into()
            }
        );

        let reply = serde_json::to_string(&HostFrame::Msg {
            c,
            d: "sealed-reply".into(),
        })
        .unwrap();
        host.send(tungstenite::Message::Text(reply.into()))
            .await
            .unwrap();
        assert_eq!(
            next_text(&mut browser).await,
            tungstenite::Message::Text("sealed-reply".into())
        );

        // The host leaving closes its browsers as offline.
        host.close(None).await.unwrap();
        match next_text(&mut browser).await {
            tungstenite::Message::Close(Some(frame)) => {
                assert_eq!(u16::from(frame.code), HOST_OFFLINE_CLOSE)
            }
            other => panic!("expected offline close, got {other:?}"),
        }
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
