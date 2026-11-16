//! The StickS3 desk display over the local network, a port of the
//! WebView's `src-tauri/src/device_bridge.rs` and `device_hub.rs`.
//!
//! The listener is off unless asked for and binds one concrete IPv4
//! address. `/health` says little; every other route wants the token kept
//! in the keychain under the WebView's name. The device reads what the
//! agent is doing (`/state`) and the pet's frames (`/pet/manifest`,
//! `/pet/frame`), and may send three actions: `ping`, `focus_session`
//! (the desktop opens that conversation) and `acknowledge` (the done or
//! failed notice is put away). Nothing it sends reaches an agent or a tool.
//!
//! The WebView folds its live agent events into what the device shows.
//! This host has no such bus, so the same states are read from each
//! conversation's live state whenever the device asks: running, waiting
//! on an approval, being reviewed, or ended since it was last looked at
//! (done, or failed with an error).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{bail, ensure, Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Query, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG},
        HeaderMap, StatusCode, Uri,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use image::{codecs::png::PngEncoder, imageops::FilterType, ImageEncoder, ImageFormat};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{oneshot, watch};

use crate::Server;

pub const DEFAULT_PORT: u16 = 18_766;
const TOKEN_SECRET: &str = "sticks3_device_bridge_token";
const ENABLED: &str = "device_bridge_enabled";
const MODE: &str = "device_bridge_mode";
const BIND_IPV4: &str = "device_bridge_bind_ipv4";
const PORT: &str = "device_bridge_port";
const TOKEN_HEADER: &str = "x-wisp-device-token";
const ACTION_HISTORY_LIMIT: usize = 50;
const MAX_ACTION_ID_BYTES: usize = 160;
const PET_FRAME_WIDTH: u32 = 120;
const PET_FRAME_HEIGHT: u32 = 130;
const PET_FRAME_INTERVAL_MS: u32 = 180;
const PET_CELL_WIDTH: u32 = 192;
const PET_CELL_HEIGHT: u32 = 208;
const PET_FRAME_CACHE_LIMIT: usize = 48;
const MAX_PET_FRAME_BYTES: usize = 512 * 1024;

/// The device's names for the states, the sheet's, the sheet row, and
/// how many frames a row has when the pet does not say.
#[derive(Clone, Copy)]
struct PetStateSpec {
    bridge_state: &'static str,
    atlas_state: &'static str,
    row: u32,
    default_frames: u8,
}

const PET_STATE_SPECS: [PetStateSpec; 6] = [
    PetStateSpec {
        bridge_state: "idle",
        atlas_state: "idle",
        row: 0,
        default_frames: 7,
    },
    PetStateSpec {
        bridge_state: "working",
        atlas_state: "running",
        row: 7,
        default_frames: 6,
    },
    PetStateSpec {
        bridge_state: "review",
        atlas_state: "review",
        row: 8,
        default_frames: 6,
    },
    PetStateSpec {
        bridge_state: "needs_user",
        atlas_state: "waiting",
        row: 6,
        default_frames: 6,
    },
    PetStateSpec {
        bridge_state: "done",
        atlas_state: "jumping",
        row: 4,
        default_frames: 5,
    },
    PetStateSpec {
        bridge_state: "failed",
        atlas_state: "failed",
        row: 5,
        default_frames: 8,
    },
];

/// What the agent is doing, as the device shows it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PetState {
    #[default]
    Idle,
    Working,
    Review,
    NeedsUser,
    Done,
    Failed,
}

impl PetState {
    fn priority(self) -> u8 {
        match self {
            Self::NeedsUser => 5,
            Self::Failed => 4,
            Self::Review => 3,
            Self::Working => 2,
            Self::Done => 1,
            Self::Idle => 0,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Idle => "Agent is idle",
            Self::Working => "Agent is working",
            Self::Review => "Review in progress",
            Self::NeedsUser => "Needs your attention",
            Self::Done => "Agent is done",
            Self::Failed => "Agent failed",
        }
    }
}

/// `/state`, as the device's firmware reads it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetStateSnapshot {
    #[serde(rename = "type")]
    message_type: &'static str,
    pub state: PetState,
    project: &'static str,
    label: &'static str,
    pub session_id: Option<String>,
    pub seq: u64,
    updated_at: i64,
}

/// One conversation as the device last saw it.
#[derive(Clone, Debug, Default)]
struct Activity {
    base: PetState,
    needs_user: bool,
    /// The live state's generation when last looked at; a change means a
    /// turn began or ended since.
    generation: u64,
    changed_seq: u64,
}

impl Activity {
    fn displayed(&self) -> PetState {
        if self.needs_user {
            PetState::NeedsUser
        } else {
            self.base
        }
    }
}

/// What a conversation's live state says now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Observed {
    pub generation: u64,
    pub running: bool,
    pub waiting: bool,
    pub reviewing: bool,
    pub failed: bool,
}

#[derive(Default)]
pub(crate) struct Hub {
    sessions: HashMap<String, Activity>,
    seq: u64,
    updated_at: i64,
}

impl Hub {
    /// Takes in what each conversation is doing now.
    pub(crate) fn observe(&mut self, now: impl IntoIterator<Item = (String, Observed)>) {
        for (session, seen) in now {
            let mut activity = self.sessions.remove(&session).unwrap_or_default();
            let before = (activity.base, activity.needs_user);
            let ended_since = seen.generation != activity.generation;
            if seen.running {
                activity.base = if seen.reviewing {
                    PetState::Review
                } else {
                    PetState::Working
                };
                activity.needs_user = seen.waiting;
            } else if seen.reviewing {
                activity.base = PetState::Review;
                activity.needs_user = false;
            } else if matches!(activity.base, PetState::Working | PetState::Review) || ended_since {
                // A turn (or its review) ended since the last look; like the
                // WebView's hub, every turn since the app started counts.
                activity.base = if seen.failed {
                    PetState::Failed
                } else {
                    PetState::Done
                };
                activity.needs_user = false;
            }
            activity.generation = seen.generation;
            if (activity.base, activity.needs_user) != before {
                self.seq = self.seq.saturating_add(1);
                self.updated_at = chrono::Utc::now().timestamp();
                activity.changed_seq = self.seq;
            }
            self.sessions.insert(session, activity);
        }
    }

    fn selected(&self) -> Option<(&str, &Activity)> {
        self.sessions
            .iter()
            .max_by_key(|(_, activity)| (activity.displayed().priority(), activity.changed_seq))
            .map(|(session, activity)| (session.as_str(), activity))
    }

    pub(crate) fn snapshot(&self) -> PetStateSnapshot {
        let (session_id, state) = self
            .selected()
            .map(|(session, activity)| (Some(session.to_owned()), activity.displayed()))
            .unwrap_or((None, PetState::Idle));
        PetStateSnapshot {
            message_type: "pet_state",
            state,
            project: "Wisp Science",
            label: state.label(),
            session_id,
            seq: self.seq,
            updated_at: self.updated_at,
        }
    }

    /// Puts away a done or failed notice, of `session` or else the one
    /// shown.
    pub(crate) fn acknowledge(&mut self, session: Option<&str>) -> bool {
        let Some(session) = session
            .map(str::to_owned)
            .or_else(|| self.selected().map(|(session, _)| session.to_owned()))
        else {
            return false;
        };
        let Some(activity) = self.sessions.get_mut(&session) else {
            return false;
        };
        if !matches!(activity.displayed(), PetState::Done | PetState::Failed) {
            return false;
        }
        activity.base = PetState::Idle;
        activity.needs_user = false;
        self.seq = self.seq.saturating_add(1);
        self.updated_at = chrono::Utc::now().timestamp();
        activity.changed_seq = self.seq;
        true
    }
}

/// Where the listener stands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeviceBridgeState {
    #[default]
    Stopped,
    Listening,
    Error,
}

/// The device bridge as Settings shows it. The token is not part of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceBridge {
    pub enabled: bool,
    pub has_token: bool,
    pub state: DeviceBridgeState,
    pub bind_ipv4: String,
    pub port: u16,
    /// The address to give the device, while listening.
    pub url: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionRecord {
    id: String,
    action: String,
    session_id: Option<String>,
    ok: bool,
    timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActionRequest {
    id: String,
    action: String,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameQuery {
    revision: String,
    state: String,
    frame: u8,
}

#[derive(Default)]
struct FrameCache {
    revision: Option<String>,
    entries: VecDeque<((String, String, u8), Bytes)>,
}

impl FrameCache {
    fn prepare(&mut self, revision: &str) {
        if self.revision.as_deref() != Some(revision) {
            self.entries.clear();
            self.revision = Some(revision.to_owned());
        }
    }

    fn get(&mut self, key: &(String, String, u8)) -> Option<Bytes> {
        self.prepare(&key.0);
        self.entries
            .iter()
            .find(|(cached, _)| cached == key)
            .map(|(_, bytes)| bytes.clone())
    }

    fn insert(&mut self, key: (String, String, u8), bytes: Bytes) {
        self.prepare(&key.0);
        if self.entries.iter().any(|(cached, _)| cached == &key) {
            return;
        }
        if self.entries.len() == PET_FRAME_CACHE_LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back((key, bytes));
    }
}

/// The running listener and what it shares with its handlers.
#[derive(Default)]
pub(crate) struct Slot {
    token: Arc<RwLock<Option<Vec<u8>>>>,
    hub: Arc<Mutex<Hub>>,
    actions: Arc<Mutex<VecDeque<ActionRecord>>>,
    frames: Arc<Mutex<FrameCache>>,
    running: tokio::sync::Mutex<Option<(SocketAddrV4, oneshot::Sender<()>)>>,
    status: Arc<Mutex<(DeviceBridgeState, String)>>,
    /// The conversation the device last asked to open.
    focus: Mutex<Option<watch::Sender<Option<(String, String)>>>>,
}

impl Slot {
    pub(crate) async fn stop(&self) {
        // The token goes first, so no request still connected gets through.
        *self.token.write().unwrap() = None;
        if let Some((_, stop)) = self.running.lock().await.take() {
            let _ = stop.send(());
        }
        *self.status.lock().unwrap() = (DeviceBridgeState::Stopped, String::new());
    }

    fn focus_sender(&self) -> watch::Sender<Option<(String, String)>> {
        self.focus
            .lock()
            .unwrap()
            .get_or_insert_with(|| watch::channel(None).0)
            .clone()
    }
}

#[derive(Clone)]
struct Http {
    server: Server,
}

/// A bind address the listener may use: one concrete IPv4 address.
fn parse_address(bind_ipv4: &str, port: u32) -> Result<SocketAddrV4> {
    let ip: Ipv4Addr = bind_ipv4
        .trim()
        .parse()
        .ok()
        .context("Bind address must be a concrete IPv4 address.")?;
    ensure!(
        !ip.is_unspecified(),
        "Device Bridge cannot bind 0.0.0.0; choose one specific IPv4 address."
    );
    let port = u16::try_from(port)
        .ok()
        .filter(|port| *port > 0)
        .context("Device Bridge port must be between 1 and 65535.")?;
    Ok(SocketAddrV4::new(ip, port))
}

pub(crate) fn generate_token() -> Result<String> {
    let bytes =
        wisp_sync::random_project_key().context("Could not generate a secure device token.")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// The token compared as SHA-256 digests, so the comparison takes the
/// same time wherever the tokens differ.
fn token_matches(expected: &[u8], supplied: &[u8]) -> bool {
    let (a, b) = (Sha256::digest(expected), Sha256::digest(supplied));
    a.iter()
        .zip(b.iter())
        .fold(0u8, |diff, (x, y)| diff | (x ^ y))
        == 0
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "ok": false, "error": "Unauthorized" })),
    )
        .into_response()
}

fn frame_error(status: StatusCode, error: &'static str) -> Response {
    (status, Json(json!({ "ok": false, "error": error }))).into_response()
}

/// The pet the device draws, read fresh from its folder.
struct DevicePet {
    id: String,
    name: String,
    sheet: Vec<u8>,
    frames: BTreeMap<String, u8>,
}

fn frame_count(pet: &DevicePet, spec: PetStateSpec) -> u8 {
    pet.frames
        .get(spec.atlas_state)
        .copied()
        .unwrap_or(spec.default_frames)
}

fn revision(pet: &DevicePet) -> String {
    let mut revision = Sha256::new();
    revision.update(b"wisp-sticks3-pet-protocol-1\0");
    revision.update(pet.id.as_bytes());
    revision.update(Sha256::digest(&pet.sheet));
    revision.update(b"\0png\0");
    for value in [
        PET_FRAME_WIDTH,
        PET_FRAME_HEIGHT,
        PET_FRAME_INTERVAL_MS,
        PET_CELL_WIDTH,
        PET_CELL_HEIGHT,
    ] {
        revision.update(value.to_le_bytes());
    }
    revision.update(b"resize:lanczos3\0");
    for spec in PET_STATE_SPECS {
        revision.update(spec.bridge_state.as_bytes());
        revision.update([0]);
        revision.update(spec.atlas_state.as_bytes());
        revision.update([0]);
        revision.update(spec.row.to_le_bytes());
        revision.update([frame_count(pet, spec)]);
    }
    format!("{:x}", revision.finalize())
}

fn render_frame(pet: &DevicePet, spec: PetStateSpec, frame: u8) -> Result<Vec<u8>> {
    let format = if pet.sheet.starts_with(b"RIFF") {
        ImageFormat::WebP
    } else {
        ImageFormat::Png
    };
    let atlas = image::load_from_memory_with_format(&pet.sheet, format)
        .ok()
        .context("Pet spritesheet could not be decoded")?
        .into_rgba8();
    ensure!(
        atlas.dimensions() == (PET_CELL_WIDTH * 8, PET_CELL_HEIGHT * 11),
        "Pet spritesheet dimensions changed"
    );
    let cell = image::imageops::crop_imm(
        &atlas,
        u32::from(frame) * PET_CELL_WIDTH,
        spec.row * PET_CELL_HEIGHT,
        PET_CELL_WIDTH,
        PET_CELL_HEIGHT,
    )
    .to_image();
    let resized = image::imageops::resize(
        &cell,
        PET_FRAME_WIDTH,
        PET_FRAME_HEIGHT,
        FilterType::Lanczos3,
    );
    let mut output = Vec::new();
    PngEncoder::new(&mut output)
        .write_image(
            resized.as_raw(),
            PET_FRAME_WIDTH,
            PET_FRAME_HEIGHT,
            image::ExtendedColorType::Rgba8,
        )
        .ok()
        .context("Pet frame could not be encoded")?;
    ensure!(
        output.len() <= MAX_PET_FRAME_BYTES,
        "Pet frame exceeds output limit"
    );
    Ok(output)
}

impl Http {
    fn slot(&self) -> &Slot {
        &self.server.0.device
    }

    fn authorized(&self, headers: &HeaderMap) -> bool {
        let supplied = headers
            .get(TOKEN_HEADER)
            .map(|value| value.as_bytes())
            .unwrap_or_default();
        self.slot()
            .token
            .read()
            .unwrap()
            .as_deref()
            .is_some_and(|expected| token_matches(expected, supplied))
    }

    async fn pet(&self) -> Result<DevicePet, &'static str> {
        let pet = crate::pet::read(&self.server.0.store).await;
        if !pet.enabled {
            return Err("Pet is disabled.");
        }
        if pet.directory.trim().is_empty() {
            return Err("Pet is not configured.");
        }
        let look = pet.look.ok_or("Configured Pet is invalid.")?;
        let manifest: Value = std::fs::read(std::path::Path::new(&pet.directory).join("pet.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .ok_or("Configured Pet is invalid.")?;
        let sheet = std::fs::read(&look.sheet).map_err(|_| "Configured Pet is invalid.")?;
        Ok(DevicePet {
            id: manifest["id"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_owned(),
            name: look.name,
            sheet,
            frames: look.frames,
        })
    }

    fn record(&self, request: &ActionRequest, ok: bool, error: Option<&str>) {
        let mut actions = self.slot().actions.lock().unwrap();
        if actions.len() == ACTION_HISTORY_LIMIT {
            actions.pop_front();
        }
        actions.push_back(ActionRecord {
            id: request.id.clone(),
            action: request.action.clone(),
            session_id: request.session_id.clone(),
            ok,
            timestamp: chrono::Utc::now().timestamp(),
            error: error.map(str::to_owned),
        });
    }

    fn refused(&self, request: &ActionRequest, error: &str) -> Response {
        self.record(request, false, Some(error));
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": error })),
        )
            .into_response()
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "wisp-device-bridge", "protocol": 1 }))
}

async fn state(State(http): State<Http>, headers: HeaderMap) -> Response {
    if !http.authorized(&headers) {
        return unauthorized();
    }
    Json(http.server.observe_devices()).into_response()
}

async fn actions(State(http): State<Http>, headers: HeaderMap) -> Response {
    if !http.authorized(&headers) {
        return unauthorized();
    }
    let actions = http.slot().actions.lock().unwrap().clone();
    Json(json!({ "actions": actions })).into_response()
}

async fn manifest(State(http): State<Http>, headers: HeaderMap) -> Response {
    if !http.authorized(&headers) {
        return unauthorized();
    }
    let pet = match http.pet().await {
        Ok(pet) => pet,
        Err(reason) => {
            *http.slot().frames.lock().unwrap() = FrameCache::default();
            return Json(json!({
                "type": "pet_manifest", "protocol": 1, "enabled": false, "reason": reason,
            }))
            .into_response();
        }
    };
    let revision = revision(&pet);
    http.slot().frames.lock().unwrap().prepare(&revision);
    let counts: BTreeMap<&str, u8> = PET_STATE_SPECS
        .into_iter()
        .map(|spec| (spec.bridge_state, frame_count(&pet, spec)))
        .collect();
    Json(json!({
        "type": "pet_manifest",
        "protocol": 1,
        "enabled": true,
        "id": pet.id,
        "displayName": pet.name,
        "revision": revision,
        "format": "png",
        "frameWidth": PET_FRAME_WIDTH,
        "frameHeight": PET_FRAME_HEIGHT,
        "frameIntervalMs": PET_FRAME_INTERVAL_MS,
        "frameCounts": counts,
    }))
    .into_response()
}

async fn frame(State(http): State<Http>, headers: HeaderMap, uri: Uri) -> Response {
    if !http.authorized(&headers) {
        return unauthorized();
    }
    let Ok(Query(query)) = Query::<FrameQuery>::try_from_uri(&uri) else {
        return frame_error(StatusCode::BAD_REQUEST, "Invalid Pet frame request.");
    };
    let Some(spec) = PET_STATE_SPECS
        .into_iter()
        .find(|spec| spec.bridge_state == query.state)
    else {
        return frame_error(StatusCode::BAD_REQUEST, "Unknown Pet state.");
    };
    let Ok(pet) = http.pet().await else {
        *http.slot().frames.lock().unwrap() = FrameCache::default();
        return frame_error(StatusCode::NOT_FOUND, "No valid Pet is available.");
    };
    let revision = revision(&pet);
    if query.revision != revision {
        return frame_error(StatusCode::CONFLICT, "Pet revision changed.");
    }
    if query.frame >= frame_count(&pet, spec) {
        return frame_error(StatusCode::NOT_FOUND, "Pet frame does not exist.");
    }
    let key = (revision.clone(), spec.bridge_state.to_owned(), query.frame);
    let cached = http.slot().frames.lock().unwrap().get(&key);
    let bytes = match cached {
        Some(bytes) => bytes,
        None => {
            let number = query.frame;
            match tokio::task::spawn_blocking(move || render_frame(&pet, spec, number)).await {
                Ok(Ok(bytes)) => {
                    let bytes = Bytes::from(bytes);
                    http.slot()
                        .frames
                        .lock()
                        .unwrap()
                        .insert(key.clone(), bytes.clone());
                    bytes
                }
                _ => {
                    return frame_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Pet frame could not be rendered.",
                    )
                }
            }
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "image/png")
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .header(CACHE_CONTROL, "private, max-age=31536000, immutable")
        .header(ETAG, format!("\"{}-{}-{}\"", key.0, key.1, key.2))
        .body(Body::from(bytes))
        .unwrap()
}

async fn action(
    State(http): State<Http>,
    headers: HeaderMap,
    Json(request): Json<ActionRequest>,
) -> Response {
    if !http.authorized(&headers) {
        return unauthorized();
    }
    if request.id.trim().is_empty() || request.id.len() > MAX_ACTION_ID_BYTES {
        return http.refused(&request, "Action id is required and must be short.");
    }
    let session = request
        .session_id
        .as_deref()
        .filter(|id| !id.trim().is_empty());
    let result = match request.action.as_str() {
        "ping" => Ok(json!({ "ok": true, "id": request.id, "action": "ping" })),
        "focus_session" => match session {
            Some(session) => http.server.focus_from_device(session).await.map(|()| {
                json!({
                    "ok": true, "id": request.id, "action": "focus_session", "sessionId": session,
                })
            }),
            None => Err("focus_session requires sessionId.".to_owned()),
        },
        "acknowledge" => {
            http.server.observe_devices();
            let acknowledged = http.slot().hub.lock().unwrap().acknowledge(session);
            Ok(json!({
                "ok": true, "id": request.id, "action": "acknowledge", "acknowledged": acknowledged,
            }))
        }
        _ => return http.refused(&request, "Unknown device action."),
    };
    match result {
        Ok(value) => {
            http.record(&request, true, None);
            Json(value).into_response()
        }
        Err(error) => http.refused(&request, &error),
    }
}

impl Server {
    async fn device_setting(&self, key: &str) -> String {
        crate::channels::get_setting(&self.0.store, key).await
    }

    fn device_token(&self) -> Option<String> {
        self.0
            .secrets
            .get(TOKEN_SECRET)
            .filter(|token| !token.is_empty())
    }

    /// What every conversation is doing, folded into what the device shows.
    pub(crate) fn observe_devices(&self) -> PetStateSnapshot {
        let sessions: Vec<(String, Arc<crate::output::Session>)> = self
            .0
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect();
        let observed: Vec<(String, Observed)> = sessions
            .into_iter()
            .map(|(id, session)| {
                let live = session.live.lock().unwrap();
                let seen = Observed {
                    generation: live.generation,
                    running: live.running,
                    waiting: !live.approvals.is_empty(),
                    reviewing: live.reviewing,
                    failed: live.error.is_some(),
                };
                (id, seen)
            })
            .collect();
        let mut hub = self.0.device.hub.lock().unwrap();
        hub.observe(observed);
        hub.snapshot()
    }

    /// The device asked for a conversation: the window opens it.
    async fn focus_from_device(&self, session: &str) -> Result<(), String> {
        let store = &self.0.store;
        let project = store
            .frame_project_id(session)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Session does not exist.".to_owned())?;
        if store
            .get_project(&project)
            .await
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err("Session project does not exist.".into());
        }
        let _ = self
            .0
            .device
            .focus_sender()
            .send(Some((project, session.to_owned())));
        Ok(())
    }

    /// Waits for the device to ask for a conversation; the project and the
    /// conversation it asked for.
    pub async fn next_device_focus(&self) -> (String, String) {
        let mut focus = self.0.device.focus_sender().subscribe();
        loop {
            if focus.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
            if let Some(asked) = focus.borrow_and_update().clone() {
                return asked;
            }
        }
    }

    /// The device bridge as it stands.
    pub async fn device_bridge(&self) -> DeviceBridge {
        let (state, detail) = self.0.device.status.lock().unwrap().clone();
        let bind_ipv4 = self.device_setting(BIND_IPV4).await;
        let port = self
            .device_setting(PORT)
            .await
            .parse()
            .unwrap_or(DEFAULT_PORT);
        DeviceBridge {
            enabled: self.device_setting(ENABLED).await == "true",
            has_token: self.device_token().is_some(),
            state,
            url: (state == DeviceBridgeState::Listening)
                .then(|| format!("http://{bind_ipv4}:{port}")),
            bind_ipv4,
            port,
            detail,
        }
    }

    /// Keeps the address and turns the listener on or off. Turning it off
    /// also forgets the token, so a device that knew it is locked out.
    pub async fn set_device_bridge(
        &self,
        enabled: bool,
        bind_ipv4: &str,
        port: u32,
    ) -> Result<DeviceBridge> {
        let address = parse_address(bind_ipv4, port)?;
        let store = &self.0.store;
        store.set_setting(MODE, "lan").await?;
        store
            .set_setting(BIND_IPV4, &address.ip().to_string())
            .await?;
        store.set_setting(PORT, &address.port().to_string()).await?;
        store
            .set_setting(ENABLED, if enabled { "true" } else { "false" })
            .await?;
        if enabled {
            if self.device_token().is_none() {
                self.0.secrets.set(TOKEN_SECRET, &generate_token()?)?;
            }
            self.start_device_bridge().await?;
        } else {
            self.0.device.stop().await;
            self.0.secrets.set(TOKEN_SECRET, "")?;
        }
        Ok(self.device_bridge().await)
    }

    /// The token to enter on the device.
    pub fn device_bridge_token(&self) -> Result<String> {
        self.device_token()
            .context("No Device Bridge token exists. Turn the bridge on first.")
    }

    /// A new token; the device has to be given it again.
    pub fn rotate_device_bridge_token(&self) -> Result<String> {
        let token = generate_token()?;
        self.0.secrets.set(TOKEN_SECRET, &token)?;
        if self.0.device.token.read().unwrap().is_some() {
            *self.0.device.token.write().unwrap() = Some(token.clone().into_bytes());
        }
        Ok(token)
    }

    /// Starts listening when the bridge was left on. The desktop calls
    /// this once it has opened the host.
    pub async fn start_device_bridge(&self) -> Result<()> {
        if self.device_setting(ENABLED).await != "true" {
            return Ok(());
        }
        let slot = &self.0.device;
        if !matches!(self.device_setting(MODE).await.trim(), "" | "lan") {
            *slot.status.lock().unwrap() = (
                DeviceBridgeState::Error,
                "Device Bridge relay transport is not available in this release.".into(),
            );
            bail!("Device Bridge relay transport is not available in this release.");
        }
        let port: u32 = self
            .device_setting(PORT)
            .await
            .parse()
            .unwrap_or(u32::from(DEFAULT_PORT));
        let address = match parse_address(&self.device_setting(BIND_IPV4).await, port) {
            Ok(address) => address,
            Err(error) => {
                *slot.status.lock().unwrap() = (DeviceBridgeState::Error, error.to_string());
                return Err(error);
            }
        };
        let token = match self.device_token() {
            Some(token) => token,
            None => {
                let token = generate_token()?;
                self.0.secrets.set(TOKEN_SECRET, &token)?;
                token
            }
        };
        let mut running = slot.running.lock().await;
        if running.as_ref().is_some_and(|(bound, _)| *bound == address) {
            *slot.token.write().unwrap() = Some(token.into_bytes());
            *slot.status.lock().unwrap() = (DeviceBridgeState::Listening, String::new());
            return Ok(());
        }
        if let Some((_, stop)) = running.take() {
            let _ = stop.send(());
        }
        let listener = match tokio::net::TcpListener::bind(SocketAddr::V4(address)).await {
            Ok(listener) => listener,
            Err(error) => {
                let detail = format!("Cannot listen on {address}: {error}");
                *slot.status.lock().unwrap() = (DeviceBridgeState::Error, detail.clone());
                bail!(detail);
            }
        };
        *slot.token.write().unwrap() = Some(token.into_bytes());
        let router = Router::new()
            .route("/health", get(health))
            .route("/state", get(state))
            .route("/pet/manifest", get(manifest))
            .route("/pet/frame", get(frame))
            .route("/action", post(action))
            .route("/actions", get(actions))
            .layer(DefaultBodyLimit::max(4 * 1024))
            .with_state(Http {
                server: self.clone(),
            });
        let (stop, stopped) = oneshot::channel();
        let status = slot.status.clone();
        tokio::spawn(async move {
            let served = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await;
            if let Err(error) = served {
                *status.lock().unwrap() = (
                    DeviceBridgeState::Error,
                    format!("Device Bridge stopped unexpectedly: {error}"),
                );
            }
        });
        *running = Some((address, stop));
        *slot.status.lock().unwrap() = (DeviceBridgeState::Listening, String::new());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(
        generation: u64,
        running: bool,
        waiting: bool,
        reviewing: bool,
        failed: bool,
    ) -> Observed {
        Observed {
            generation,
            running,
            waiting,
            reviewing,
            failed,
        }
    }

    #[test]
    fn the_device_follows_a_turn_from_working_to_done() {
        let mut hub = Hub::default();
        assert_eq!(hub.snapshot().state, PetState::Idle);
        hub.observe([("a".to_owned(), seen(1, true, false, false, false))]);
        assert_eq!(hub.snapshot().state, PetState::Working);
        assert_eq!(hub.snapshot().session_id.as_deref(), Some("a"));
        hub.observe([("a".to_owned(), seen(1, true, true, false, false))]);
        assert_eq!(hub.snapshot().state, PetState::NeedsUser);
        hub.observe([("a".to_owned(), seen(2, false, false, true, false))]);
        assert_eq!(hub.snapshot().state, PetState::Review);
        hub.observe([("a".to_owned(), seen(2, false, false, false, false))]);
        assert_eq!(hub.snapshot().state, PetState::Done);
        // Nothing changed: no new seq.
        let seq = hub.snapshot().seq;
        hub.observe([("a".to_owned(), seen(2, false, false, false, false))]);
        assert_eq!(hub.snapshot().seq, seq);
        assert!(hub.acknowledge(None));
        assert_eq!(hub.snapshot().state, PetState::Idle);
        assert!(!hub.acknowledge(None));
    }

    #[test]
    fn a_turn_that_ended_between_looks_still_shows_and_attention_comes_first() {
        let mut hub = Hub::default();
        hub.observe([("a".to_owned(), seen(1, true, false, false, false))]);
        // It ended with an error before the next look.
        hub.observe([("a".to_owned(), seen(2, false, false, false, true))]);
        assert_eq!(hub.snapshot().state, PetState::Failed);
        hub.observe([("b".to_owned(), seen(1, true, true, false, false))]);
        let shown = hub.snapshot();
        assert_eq!(shown.state, PetState::NeedsUser);
        assert_eq!(shown.session_id.as_deref(), Some("b"));
        // A conversation that never ran a turn tells nothing; one first
        // seen after a turn ended shows it.
        let mut fresh = Hub::default();
        fresh.observe([("c".to_owned(), seen(0, false, false, false, false))]);
        assert_eq!(fresh.snapshot().state, PetState::Idle);
        fresh.observe([("d".to_owned(), seen(2, false, false, false, false))]);
        assert_eq!(fresh.snapshot().state, PetState::Done);
    }

    #[test]
    fn only_one_concrete_ipv4_address_is_listened_on() {
        assert!(parse_address("192.168.1.20", 18766).is_ok());
        assert!(parse_address("0.0.0.0", 18766).is_err());
        assert!(parse_address("localhost", 18766).is_err());
        assert!(parse_address("192.168.1.20", 0).is_err());
        assert!(parse_address("192.168.1.20", 70_000).is_err());
        let token = generate_token().unwrap();
        assert_eq!(token.len(), 43);
        assert!(token_matches(token.as_bytes(), token.as_bytes()));
        assert!(!token_matches(token.as_bytes(), b"wrong"));
    }
}
