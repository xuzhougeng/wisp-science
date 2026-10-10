//! Wisp Lab: an optional membership layer on top of the relay.
//!
//! With `WISP_LAB=1` the relay keeps one lab under `<root>/lab/`: its members,
//! a pointer to the knowledge-base project and one mailbox per member. The
//! first member to join becomes the leader. Everyone after that needs an
//! invite the leader issued and stays pending until the leader approves them.
//! Without the switch every `/v1/lab/*` route answers 404 and the computers
//! sharing the relay stay independent of each other.
//!
//! The relay never holds the lab key. The knowledge-base pointer and every
//! mail are sealed with it by the desktops, so the relay stores who is in the
//! lab and who wrote to whom, and nothing about what was said.
use crate::http::{authorized, unauthorized, RelayHttpState};
use crate::{sha256_hex, FileRelay, HttpRelay};
use anyhow::{Context, Result};
use axum::{
    extract::{DefaultBodyLimit, Path as RoutePath, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::Mutex;

/// Carries the member key: 64 lowercase hex digits the desktop generated.
pub const LAB_MEMBER_HEADER: &str = "x-wisp-member";
/// AAD of the sealed knowledge-base pointer.
pub const LAB_KB_AAD: &[u8] = b"wisp-lab/v1/kb";
/// One sealed mail. Attachments travel as relay blobs, not inside the mail.
pub const MAX_LAB_MAIL_BYTES: usize = 256 * 1024;
const MAX_LAB_JSON_BYTES: usize = 512 * 1024;
const MAX_SEALED_KB_BYTES: usize = 16 * 1024;
const MAX_MEMBERS: usize = 256;
const MAX_INVITES: usize = 64;
const MAX_INBOX: usize = 500;
const INBOX_PAGE: usize = 50;
const INVITE_TTL_SECS: i64 = 7 * 24 * 3600;
const MAX_NAME_CHARS: usize = 64;
const LAB_INVITE_PREFIX: &str = "wisp-lab:";
const LAB_INVITE_VERSION: u32 = 1;
const INVITE_NONCE_BYTES: usize = 16;

/// AAD of one mail. It names both ends, so the relay can neither deliver a
/// mail to another member nor claim somebody else wrote it.
pub fn lab_mail_aad(from: &str, to: &str) -> Vec<u8> {
    format!("wisp-lab/v1/mail/{from}/{to}").into_bytes()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabRole {
    Leader,
    Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabMemberStatus {
    Pending,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabMember {
    pub id: String,
    pub name: String,
    pub role: LabRole,
    pub status: LabMemberStatus,
    pub joined_at: i64,
}

/// Where the lab's knowledge base lives. `sealed` is that project's device
/// code encrypted with the lab key; the relay only reads `project_id`, to
/// keep everyone but the leader from committing to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabKnowledgeBase {
    pub project_id: String,
    pub sealed: String,
}

/// What one member may see of the lab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabView {
    pub id: String,
    pub name: String,
    pub me: LabMember,
    /// Active members, plus pending ones for the leader. Empty while `me` is
    /// pending.
    #[serde(default)]
    pub members: Vec<LabMember>,
    #[serde(default)]
    pub knowledge_base: Option<LabKnowledgeBase>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabJoinRequest {
    /// Shown to the leader and to other members.
    pub name: String,
    /// Used only by the first member, who founds the lab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lab_name: Option<String>,
    /// The invite nonce. Required from everyone but the first member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<String>,
}

/// One mail waiting in a mailbox. `from` is stamped by the relay from the
/// sender's member key, and `d` is sealed under `lab_mail_aad(from, to)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabMail {
    pub id: String,
    pub from: String,
    pub at: i64,
    pub d: String,
}

/// A refused lab request: the HTTP status and a stable code the desktop turns
/// into a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabRejection {
    pub status: u16,
    pub code: String,
}

impl LabRejection {
    fn new(status: StatusCode, code: &str) -> Self {
        Self {
            status: status.as_u16(),
            code: code.into(),
        }
    }
}

impl std::fmt::Display for LabRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lab request refused ({}): {}", self.status, self.code)
    }
}

impl std::error::Error for LabRejection {}

fn refuse(status: StatusCode, code: &str) -> LabRejection {
    LabRejection::new(status, code)
}

fn storage(error: impl std::fmt::Display) -> LabRejection {
    tracing::warn!("lab storage failed: {error}");
    refuse(StatusCode::INTERNAL_SERVER_ERROR, "storage_error")
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

fn valid_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Ids the relay generated: no separators, so they are safe path components.
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn clean_name(value: &str) -> Result<String, LabRejection> {
    let name = value.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(refuse(StatusCode::BAD_REQUEST, "invalid_name"));
    }
    Ok(name.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMember {
    #[serde(flatten)]
    member: LabMember,
    /// SHA-256 of the member key. The key itself stays on the desktop.
    key_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredInvite {
    /// SHA-256 of the nonce the invited member will present.
    hash: String,
    expires_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LabState {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    members: Vec<StoredMember>,
    #[serde(default)]
    invites: Vec<StoredInvite>,
    #[serde(default)]
    knowledge_base: Option<LabKnowledgeBase>,
}

impl LabState {
    fn member(&self, key: &str) -> Result<&StoredMember, LabRejection> {
        // Hashes are compared, so the comparison leaks nothing about the key.
        let hash = valid_hex64(key).then(|| sha256_hex(key.as_bytes()));
        self.members
            .iter()
            .find(|member| Some(&member.key_hash) == hash.as_ref())
            .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "unknown_member"))
    }

    fn active(&self, key: &str) -> Result<&StoredMember, LabRejection> {
        let member = self.member(key)?;
        if member.member.status != LabMemberStatus::Active {
            return Err(refuse(StatusCode::FORBIDDEN, "not_active"));
        }
        Ok(member)
    }

    fn leader(&self, key: &str) -> Result<&StoredMember, LabRejection> {
        let member = self.active(key)?;
        if member.member.role != LabRole::Leader {
            return Err(refuse(StatusCode::FORBIDDEN, "not_leader"));
        }
        Ok(member)
    }

    fn view(&self, me: &LabMember) -> LabView {
        let active = me.status == LabMemberStatus::Active;
        let leader = active && me.role == LabRole::Leader;
        LabView {
            id: self.id.clone(),
            name: self.name.clone(),
            me: me.clone(),
            members: self
                .members
                .iter()
                .map(|stored| &stored.member)
                .filter(|member| active && (leader || member.status == LabMemberStatus::Active))
                .cloned()
                .collect(),
            knowledge_base: active.then(|| self.knowledge_base.clone()).flatten(),
        }
    }
}

// ponytail: one lock and a whole-state rewrite per change, and mail I/O runs
// under the same lock. A lab is tens of people; split the lock if it grows.
pub struct Lab {
    root: PathBuf,
    state: Mutex<LabState>,
    /// Arrival time of the newest mail, so two mails in one millisecond
    /// still sort in the order they arrived.
    last_mail: AtomicI64,
}

impl Lab {
    pub async fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        tokio::fs::create_dir_all(root.join("inbox")).await?;
        let state = match tokio::fs::read(root.join("state.json")).await {
            Ok(bytes) => serde_json::from_slice(&bytes).context("invalid lab state")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => LabState::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            root,
            state: Mutex::new(state),
            last_mail: AtomicI64::new(0),
        })
    }

    /// Applies `change` to a copy and keeps it only once it is on disk, so
    /// memory never runs ahead of what a restart would load.
    async fn update<T>(
        &self,
        change: impl FnOnce(&mut LabState) -> Result<T, LabRejection>,
    ) -> Result<T, LabRejection> {
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        let output = change(&mut next)?;
        let bytes = serde_json::to_vec_pretty(&next).map_err(storage)?;
        FileRelay::write_atomic(&self.root.join("state.json"), &bytes)
            .await
            .map_err(storage)?;
        *state = next;
        Ok(output)
    }

    fn inbox_dir(&self, member_id: &str) -> PathBuf {
        self.root.join("inbox").join(member_id)
    }

    /// Founds the lab for the first caller and adds everyone else as pending.
    /// Repeating a join with a known key returns that member unchanged.
    // ponytail: whoever holds the relay token and joins first leads the lab.
    // The operator joins right after enabling it; add a founding secret if
    // relays ever enable the lab while strangers already hold the token.
    pub async fn join(&self, key: &str, request: LabJoinRequest) -> Result<LabView, LabRejection> {
        if !valid_hex64(key) {
            return Err(refuse(StatusCode::BAD_REQUEST, "invalid_member_key"));
        }
        let name = clean_name(&request.name)?;
        self.update(|state| {
            if let Ok(existing) = state.member(key) {
                return Ok(state.view(&existing.member));
            }
            let founding = state.members.is_empty();
            if founding {
                state.id = uuid::Uuid::new_v4().to_string();
                state.name = match request.lab_name.as_deref().map(str::trim) {
                    Some(lab_name) if !lab_name.is_empty() => clean_name(lab_name)?,
                    _ => "Wisp Lab".into(),
                };
            } else {
                let invite = request
                    .invite
                    .as_deref()
                    .map(str::trim)
                    .filter(|invite| !invite.is_empty())
                    .ok_or_else(|| refuse(StatusCode::FORBIDDEN, "invite_required"))?;
                let hash = sha256_hex(invite.as_bytes());
                let now = now_millis() / 1000;
                let position = state
                    .invites
                    .iter()
                    .position(|stored| stored.hash == hash && stored.expires_at > now)
                    .ok_or_else(|| refuse(StatusCode::FORBIDDEN, "invalid_invite"))?;
                if state.members.len() >= MAX_MEMBERS {
                    return Err(refuse(StatusCode::CONFLICT, "lab_full"));
                }
                // Single use: the invite is spent whether or not the leader
                // later approves this member.
                state.invites.remove(position);
            }
            let member = LabMember {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                role: if founding {
                    LabRole::Leader
                } else {
                    LabRole::Member
                },
                status: if founding {
                    LabMemberStatus::Active
                } else {
                    LabMemberStatus::Pending
                },
                joined_at: now_millis() / 1000,
            };
            state.members.push(StoredMember {
                member: member.clone(),
                key_hash: sha256_hex(key.as_bytes()),
            });
            Ok(state.view(&member))
        })
        .await
    }

    pub async fn view(&self, key: &str) -> Result<LabView, LabRejection> {
        let state = self.state.lock().await;
        let me = state.member(key)?;
        Ok(state.view(&me.member))
    }

    /// Registers the hash of an invite nonce the leader generated.
    pub async fn add_invite(&self, key: &str, hash: &str) -> Result<(), LabRejection> {
        if !valid_hex64(hash) {
            return Err(refuse(StatusCode::BAD_REQUEST, "invalid_invite"));
        }
        self.update(|state| {
            state.leader(key)?;
            let now = now_millis() / 1000;
            state.invites.retain(|invite| invite.expires_at > now);
            if state.invites.iter().any(|invite| invite.hash == hash) {
                return Ok(());
            }
            if state.invites.len() >= MAX_INVITES {
                return Err(refuse(StatusCode::CONFLICT, "too_many_invites"));
            }
            state.invites.push(StoredInvite {
                hash: hash.into(),
                expires_at: now + INVITE_TTL_SECS,
            });
            Ok(())
        })
        .await
    }

    pub async fn approve(&self, key: &str, member_id: &str) -> Result<(), LabRejection> {
        self.update(|state| {
            state.leader(key)?;
            let target = state
                .members
                .iter_mut()
                .find(|stored| stored.member.id == member_id)
                .ok_or_else(|| refuse(StatusCode::NOT_FOUND, "member_not_found"))?;
            target.member.status = LabMemberStatus::Active;
            Ok(())
        })
        .await
    }

    /// The leader removes a member or rejects a pending one; a member removes
    /// themselves to leave. Their key stops working and their mailbox goes.
    // ponytail: the leader cannot leave and leadership cannot move. Add a
    // transfer once a lab outlives its founder.
    pub async fn remove(&self, key: &str, member_id: &str) -> Result<(), LabRejection> {
        self.update(|state| {
            let actor = state.member(key)?.member.clone();
            let position = state
                .members
                .iter()
                .position(|stored| stored.member.id == member_id)
                .ok_or_else(|| refuse(StatusCode::NOT_FOUND, "member_not_found"))?;
            if actor.id == member_id {
                if actor.role == LabRole::Leader {
                    return Err(refuse(StatusCode::FORBIDDEN, "leader_cannot_leave"));
                }
            } else {
                state.leader(key)?;
            }
            state.members.remove(position);
            Ok(())
        })
        .await?;
        if valid_id(member_id) {
            let _ = tokio::fs::remove_dir_all(self.inbox_dir(member_id)).await;
        }
        Ok(())
    }

    pub async fn set_knowledge_base(
        &self,
        key: &str,
        knowledge_base: LabKnowledgeBase,
    ) -> Result<(), LabRejection> {
        if FileRelay::validate_component(&knowledge_base.project_id, "project id").is_err()
            || knowledge_base.sealed.is_empty()
            || knowledge_base.sealed.len() > MAX_SEALED_KB_BYTES
        {
            return Err(refuse(StatusCode::BAD_REQUEST, "invalid_knowledge_base"));
        }
        self.update(|state| {
            state.leader(key)?;
            state.knowledge_base = Some(knowledge_base);
            Ok(())
        })
        .await
    }

    /// Only the leader commits to the knowledge base. Every other project on
    /// the relay is untouched by the lab.
    pub async fn may_commit(&self, project_id: &str, key: &str) -> bool {
        let state = self.state.lock().await;
        match &state.knowledge_base {
            Some(knowledge_base) if knowledge_base.project_id == project_id => {
                state.leader(key).is_ok()
            }
            _ => true,
        }
    }

    pub async fn send_mail(
        &self,
        key: &str,
        to: &str,
        sealed: &str,
    ) -> Result<LabMail, LabRejection> {
        if sealed.is_empty() || sealed.len() > MAX_LAB_MAIL_BYTES {
            return Err(refuse(StatusCode::BAD_REQUEST, "invalid_mail"));
        }
        let state = self.state.lock().await;
        let from = state.active(key)?.member.id.clone();
        let recipient = state
            .members
            .iter()
            .find(|stored| stored.member.id == to)
            .ok_or_else(|| refuse(StatusCode::NOT_FOUND, "member_not_found"))?;
        if recipient.member.status != LabMemberStatus::Active || recipient.member.id == from {
            return Err(refuse(StatusCode::BAD_REQUEST, "invalid_recipient"));
        }
        let dir = self.inbox_dir(&recipient.member.id);
        if mail_ids(&dir).await.map_err(storage)?.len() >= MAX_INBOX {
            return Err(refuse(StatusCode::TOO_MANY_REQUESTS, "inbox_full"));
        }
        let at = now_millis().max(self.last_mail.load(Ordering::Relaxed) + 1);
        self.last_mail.store(at, Ordering::Relaxed);
        let mail = LabMail {
            // Zero-padded time first, so file order is arrival order.
            id: format!("{at:013}-{}", uuid::Uuid::new_v4().simple()),
            from,
            at: at / 1000,
            d: sealed.into(),
        };
        let bytes = serde_json::to_vec(&mail).map_err(storage)?;
        FileRelay::write_atomic(&dir.join(format!("{}.json", mail.id)), &bytes)
            .await
            .map_err(storage)?;
        Ok(mail)
    }

    /// The oldest waiting mails. They stay until acknowledged, so a reply
    /// lost on the way to the desktop is delivered again.
    pub async fn inbox(&self, key: &str) -> Result<Vec<LabMail>, LabRejection> {
        let state = self.state.lock().await;
        let dir = self.inbox_dir(&state.active(key)?.member.id);
        let mut mails = Vec::new();
        for id in mail_ids(&dir)
            .await
            .map_err(storage)?
            .into_iter()
            .take(INBOX_PAGE)
        {
            if let Ok(bytes) = tokio::fs::read(dir.join(format!("{id}.json"))).await {
                if let Ok(mail) = serde_json::from_slice::<LabMail>(&bytes) {
                    mails.push(mail);
                }
            }
        }
        Ok(mails)
    }

    pub async fn ack(&self, key: &str, mail_id: &str) -> Result<(), LabRejection> {
        if !valid_id(mail_id) {
            return Err(refuse(StatusCode::BAD_REQUEST, "invalid_mail"));
        }
        let state = self.state.lock().await;
        let path = self
            .inbox_dir(&state.active(key)?.member.id)
            .join(format!("{mail_id}.json"));
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(storage(error)),
        }
    }
}

/// Mail ids in a mailbox, oldest first. A member nobody wrote to has no
/// directory yet.
async fn mail_ids(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut ids = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        // Skips the dot-prefixed temporary files of a write in progress.
        if let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) {
            if valid_id(id) {
                ids.push(id.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

// --------------------------------------------------------------------- HTTP

pub(crate) fn routes() -> Router<RelayHttpState> {
    Router::new()
        .route("/v1/lab", get(view))
        .route("/v1/lab/join", post(join))
        .route("/v1/lab/invites", post(add_invite))
        .route("/v1/lab/members/{member_id}", delete(remove))
        .route("/v1/lab/members/{member_id}/approve", post(approve))
        .route("/v1/lab/members/{member_id}/mail", post(send_mail))
        .route("/v1/lab/knowledge-base", put(set_knowledge_base))
        .route("/v1/lab/mail", get(inbox))
        .route("/v1/lab/mail/{mail_id}", delete(ack))
        // Lab requests are small JSON; only blobs need the relay-wide limit.
        .layer(DefaultBodyLimit::max(MAX_LAB_JSON_BYTES))
}

pub(crate) fn member_key(headers: &HeaderMap) -> &str {
    headers
        .get(LAB_MEMBER_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
}

fn rejection(rejection: LabRejection) -> Response {
    (
        StatusCode::from_u16(rejection.status).unwrap_or(StatusCode::BAD_REQUEST),
        rejection.code,
    )
        .into_response()
}

/// The relay token first, then the switch: a relay without a lab answers 404
/// to its own users and 401 to everyone else.
fn gate<'a>(
    state: &'a RelayHttpState,
    headers: &'a HeaderMap,
) -> Result<(&'a Lab, &'a str), Response> {
    if !authorized(headers, state) {
        return Err(unauthorized());
    }
    let lab = state
        .lab
        .as_deref()
        .ok_or_else(|| rejection(refuse(StatusCode::NOT_FOUND, "lab_disabled")))?;
    Ok((lab, member_key(headers)))
}

fn done(result: Result<(), LabRejection>) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => rejection(error),
    }
}

fn json<T: Serialize>(result: Result<T, LabRejection>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => rejection(error),
    }
}

async fn view(State(state): State<RelayHttpState>, headers: HeaderMap) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => json(lab.view(key).await),
        Err(response) => response,
    }
}

async fn join(
    State(state): State<RelayHttpState>,
    headers: HeaderMap,
    Json(request): Json<LabJoinRequest>,
) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => json(lab.join(key, request).await),
        Err(response) => response,
    }
}

#[derive(Serialize, Deserialize)]
struct InviteBody {
    hash: String,
}

async fn add_invite(
    State(state): State<RelayHttpState>,
    headers: HeaderMap,
    Json(body): Json<InviteBody>,
) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => done(lab.add_invite(key, &body.hash).await),
        Err(response) => response,
    }
}

async fn approve(
    State(state): State<RelayHttpState>,
    RoutePath(member_id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => done(lab.approve(key, &member_id).await),
        Err(response) => response,
    }
}

async fn remove(
    State(state): State<RelayHttpState>,
    RoutePath(member_id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => done(lab.remove(key, &member_id).await),
        Err(response) => response,
    }
}

async fn set_knowledge_base(
    State(state): State<RelayHttpState>,
    headers: HeaderMap,
    Json(body): Json<LabKnowledgeBase>,
) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => done(lab.set_knowledge_base(key, body).await),
        Err(response) => response,
    }
}

#[derive(Serialize, Deserialize)]
struct MailBody {
    d: String,
}

async fn send_mail(
    State(state): State<RelayHttpState>,
    RoutePath(member_id): RoutePath<String>,
    headers: HeaderMap,
    Json(body): Json<MailBody>,
) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => json(lab.send_mail(key, &member_id, &body.d).await),
        Err(response) => response,
    }
}

async fn inbox(State(state): State<RelayHttpState>, headers: HeaderMap) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => json(lab.inbox(key).await),
        Err(response) => response,
    }
}

async fn ack(
    State(state): State<RelayHttpState>,
    RoutePath(mail_id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    match gate(&state, &headers) {
        Ok((lab, key)) => done(lab.ack(key, &mail_id).await),
        Err(response) => response,
    }
}

// ------------------------------------------------------------------- client

impl HttpRelay {
    /// Sends a lab request built with [`HttpRelay::with_lab_member`]'s key.
    /// A refusal comes back as a [`LabRejection`] inside the error.
    async fn lab_call(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = request.send().await?;
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let code = Self::bounded_body(response, 256)
            .await
            .map(|body| String::from_utf8_lossy(&body).trim().to_string())
            .unwrap_or_default();
        Err(LabRejection {
            status,
            // A relay older than the lab has no such route and no body.
            code: if status == 404 && code.is_empty() {
                "lab_disabled".into()
            } else {
                code
            },
        }
        .into())
    }

    pub async fn lab_join(&self, request: &LabJoinRequest) -> Result<LabView> {
        let call = self
            .request(reqwest::Method::POST, "v1/lab/join")?
            .json(request);
        Self::bounded_json(self.lab_call(call).await?).await
    }

    pub async fn lab_view(&self) -> Result<LabView> {
        let call = self.request(reqwest::Method::GET, "v1/lab")?;
        Self::bounded_json(self.lab_call(call).await?).await
    }

    pub async fn lab_add_invite(&self, invite: &LabInvite) -> Result<()> {
        let call = self
            .request(reqwest::Method::POST, "v1/lab/invites")?
            .json(&InviteBody {
                hash: invite.nonce_hash(),
            });
        self.lab_call(call).await.map(drop)
    }

    pub async fn lab_approve(&self, member_id: &str) -> Result<()> {
        Self::component(member_id, "member id")?;
        let call = self.request(
            reqwest::Method::POST,
            &format!("v1/lab/members/{member_id}/approve"),
        )?;
        self.lab_call(call).await.map(drop)
    }

    pub async fn lab_remove(&self, member_id: &str) -> Result<()> {
        Self::component(member_id, "member id")?;
        let call = self.request(
            reqwest::Method::DELETE,
            &format!("v1/lab/members/{member_id}"),
        )?;
        self.lab_call(call).await.map(drop)
    }

    pub async fn lab_set_knowledge_base(&self, knowledge_base: &LabKnowledgeBase) -> Result<()> {
        let call = self
            .request(reqwest::Method::PUT, "v1/lab/knowledge-base")?
            .json(knowledge_base);
        self.lab_call(call).await.map(drop)
    }

    /// Leaves `sealed` in `to`'s mailbox and returns the mail as stored.
    pub async fn lab_send_mail(&self, to: &str, sealed: &str) -> Result<LabMail> {
        Self::component(to, "member id")?;
        let call = self
            .request(reqwest::Method::POST, &format!("v1/lab/members/{to}/mail"))?
            .json(&MailBody { d: sealed.into() });
        Self::bounded_json(self.lab_call(call).await?).await
    }

    pub async fn lab_inbox(&self) -> Result<Vec<LabMail>> {
        let call = self.request(reqwest::Method::GET, "v1/lab/mail")?;
        let body = Self::bounded_body(
            self.lab_call(call).await?,
            INBOX_PAGE * (MAX_LAB_MAIL_BYTES + 1024),
        )
        .await?;
        serde_json::from_slice(&body).context("relay returned malformed JSON")
    }

    pub async fn lab_ack(&self, mail_id: &str) -> Result<()> {
        Self::component(mail_id, "mail id")?;
        let call = self.request(reqwest::Method::DELETE, &format!("v1/lab/mail/{mail_id}"))?;
        self.lab_call(call).await.map(drop)
    }
}

// ------------------------------------------------------------------- invite

/// What the leader hands to a new member, out of band. It carries the lab
/// key, so it is a secret like a project device code. The relay token is
/// deliberately not part of it.
#[derive(Clone, PartialEq, Eq)]
pub struct LabInvite {
    pub relay_url: String,
    pub lab_key: [u8; 32],
    nonce: [u8; INVITE_NONCE_BYTES],
}

#[derive(Serialize, Deserialize)]
struct InviteWire {
    v: u32,
    relay_url: String,
    key: String,
    nonce: String,
}

impl LabInvite {
    pub fn generate(relay_url: &str, lab_key: [u8; 32]) -> Result<Self> {
        let mut nonce = [0_u8; INVITE_NONCE_BYTES];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| anyhow::anyhow!("could not generate a lab invite"))?;
        Ok(Self {
            relay_url: relay_url.trim().to_string(),
            lab_key,
            nonce,
        })
    }

    pub fn encode(&self) -> String {
        let wire = InviteWire {
            v: LAB_INVITE_VERSION,
            relay_url: self.relay_url.clone(),
            key: URL_SAFE_NO_PAD.encode(self.lab_key),
            nonce: self.nonce(),
        };
        format!(
            "{LAB_INVITE_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&wire).unwrap_or_default())
        )
    }

    pub fn parse(text: &str) -> Result<Self> {
        let encoded = text
            .trim()
            .strip_prefix(LAB_INVITE_PREFIX)
            .context("this is not a Wisp Lab invite code")?;
        let wire: InviteWire = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(encoded)
                .context("invalid Wisp Lab invite code")?,
        )
        .context("invalid Wisp Lab invite code")?;
        if wire.v != LAB_INVITE_VERSION {
            anyhow::bail!("unsupported Wisp Lab invite code");
        }
        let lab_key = URL_SAFE_NO_PAD
            .decode(&wire.key)
            .ok()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .context("invalid Wisp Lab invite code")?;
        let nonce = hex::decode(&wire.nonce)
            .ok()
            .and_then(|bytes| <[u8; INVITE_NONCE_BYTES]>::try_from(bytes).ok())
            .context("invalid Wisp Lab invite code")?;
        Ok(Self {
            relay_url: wire.relay_url,
            lab_key,
            nonce,
        })
    }

    /// What the invited member presents to the relay when joining.
    pub fn nonce(&self) -> String {
        hex::encode(self.nonce)
    }

    /// What the leader registers. The relay cannot join with a hash, so an
    /// unused invite is worthless to someone who only reads the relay's disk.
    pub fn nonce_hash(&self) -> String {
        sha256_hex(self.nonce().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        open_frame, relay_router, seal_frame, CommitOutcome, CommitRequest, SyncRevision,
        SyncTransport, SYNC_PROTOCOL_VERSION,
    };
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn key(byte: u8) -> String {
        hex::encode([byte; 32])
    }

    fn joining(name: &str, invite: Option<&LabInvite>) -> LabJoinRequest {
        LabJoinRequest {
            name: name.into(),
            lab_name: Some("Genomics".into()),
            invite: invite.map(LabInvite::nonce),
        }
    }

    async fn temp_lab() -> (PathBuf, Lab) {
        let root = std::env::temp_dir().join(format!("wisp-lab-{}", uuid::Uuid::new_v4()));
        let lab = Lab::open(&root).await.unwrap();
        (root, lab)
    }

    fn code(result: Result<impl std::fmt::Debug, LabRejection>) -> String {
        result.unwrap_err().code
    }

    /// Leader plus one invited member, still pending.
    async fn lab_with_pending_member() -> (PathBuf, Lab, LabView, LabView) {
        let (root, lab) = temp_lab().await;
        let leader = lab.join(&key(1), joining("Ada", None)).await.unwrap();
        let invite = LabInvite::generate("https://relay.example.test", [7; 32]).unwrap();
        lab.add_invite(&key(1), &invite.nonce_hash()).await.unwrap();
        let member = lab
            .join(&key(2), joining("Lin", Some(&invite)))
            .await
            .unwrap();
        (root, lab, leader, member)
    }

    #[tokio::test]
    async fn the_first_member_leads_and_later_ones_need_an_invite_and_approval() {
        let (root, lab) = temp_lab().await;
        let leader = lab.join(&key(1), joining("Ada", None)).await.unwrap();
        assert_eq!(leader.name, "Genomics");
        assert_eq!(leader.me.role, LabRole::Leader);
        assert_eq!(leader.me.status, LabMemberStatus::Active);
        // Joining again with the same key is a retry, not a second member.
        let again = lab.join(&key(1), joining("Ada", None)).await.unwrap();
        assert_eq!(again.me.id, leader.me.id);

        assert_eq!(
            code(lab.join(&key(2), joining("Lin", None)).await),
            "invite_required"
        );
        let invite = LabInvite::generate("https://relay.example.test", [7; 32]).unwrap();
        assert_eq!(
            code(lab.join(&key(2), joining("Lin", Some(&invite))).await),
            "invalid_invite"
        );
        // Only the leader issues invites, and each one admits one member.
        lab.add_invite(&key(1), &invite.nonce_hash()).await.unwrap();
        let member = lab
            .join(&key(2), joining("Lin", Some(&invite)))
            .await
            .unwrap();
        assert_eq!(member.me.role, LabRole::Member);
        assert_eq!(member.me.status, LabMemberStatus::Pending);
        assert_eq!(
            code(lab.join(&key(3), joining("Mallory", Some(&invite))).await),
            "invalid_invite"
        );

        // A pending member sees the lab's name and nothing else.
        assert!(member.members.is_empty());
        assert_eq!(
            code(lab.add_invite(&key(2), &invite.nonce_hash()).await),
            "not_active"
        );
        assert_eq!(code(lab.inbox(&key(2)).await), "not_active");
        assert_eq!(lab.view(&key(1)).await.unwrap().members.len(), 2);

        assert_eq!(
            code(lab.approve(&key(2), &member.me.id).await),
            "not_active"
        );
        lab.approve(&key(1), &member.me.id).await.unwrap();
        let member = lab.view(&key(2)).await.unwrap();
        assert_eq!(member.me.status, LabMemberStatus::Active);
        assert_eq!(member.members.len(), 2);
        assert_eq!(
            code(lab.approve(&key(2), &leader.me.id).await),
            "not_leader"
        );
        assert_eq!(code(lab.view(&key(9)).await), "unknown_member");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn the_lab_survives_a_relay_restart() {
        let (root, lab, leader, member) = lab_with_pending_member().await;
        lab.approve(&key(1), &member.me.id).await.unwrap();
        drop(lab);
        let lab = Lab::open(&root).await.unwrap();
        let view = lab.view(&key(2)).await.unwrap();
        assert_eq!(view.id, leader.id);
        assert_eq!(view.me.status, LabMemberStatus::Active);
        // The founding slot is taken: a stranger cannot become a second leader.
        assert_eq!(
            code(lab.join(&key(3), joining("Eve", None)).await),
            "invite_required"
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn mail_reaches_only_its_recipient_and_waits_until_acknowledged() {
        let (root, lab, leader, member) = lab_with_pending_member().await;
        assert_eq!(
            code(lab.send_mail(&key(1), &member.me.id, "sealed").await),
            "invalid_recipient"
        );
        lab.approve(&key(1), &member.me.id).await.unwrap();
        assert_eq!(
            code(lab.send_mail(&key(1), &leader.me.id, "sealed").await),
            "invalid_recipient"
        );
        assert_eq!(
            code(lab.send_mail(&key(1), "nobody", "sealed").await),
            "member_not_found"
        );
        assert_eq!(
            code(lab.send_mail(&key(1), &member.me.id, "").await),
            "invalid_mail"
        );

        let first = lab
            .send_mail(&key(1), &member.me.id, "first")
            .await
            .unwrap();
        let second = lab
            .send_mail(&key(1), &member.me.id, "second")
            .await
            .unwrap();
        assert_eq!(first.from, leader.me.id);
        assert!(lab.inbox(&key(1)).await.unwrap().is_empty());
        let waiting = lab.inbox(&key(2)).await.unwrap();
        assert_eq!(waiting, vec![first.clone(), second.clone()]);
        // Not acknowledged yet, so it is delivered again.
        assert_eq!(lab.inbox(&key(2)).await.unwrap().len(), 2);
        // Another member cannot acknowledge it away.
        lab.ack(&key(1), &first.id).await.unwrap();
        assert_eq!(lab.inbox(&key(2)).await.unwrap().len(), 2);
        lab.ack(&key(2), &first.id).await.unwrap();
        lab.ack(&key(2), &first.id).await.unwrap();
        assert_eq!(lab.inbox(&key(2)).await.unwrap(), vec![second]);
        assert_eq!(code(lab.ack(&key(2), "../state").await), "invalid_mail");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn removing_a_member_revokes_the_key_and_drops_the_mailbox() {
        let (root, lab, leader, member) = lab_with_pending_member().await;
        lab.approve(&key(1), &member.me.id).await.unwrap();
        lab.send_mail(&key(1), &member.me.id, "sealed")
            .await
            .unwrap();
        assert_eq!(code(lab.remove(&key(2), &leader.me.id).await), "not_leader");
        assert_eq!(
            code(lab.remove(&key(1), &leader.me.id).await),
            "leader_cannot_leave"
        );
        lab.remove(&key(1), &member.me.id).await.unwrap();
        assert_eq!(code(lab.view(&key(2)).await), "unknown_member");
        assert_eq!(code(lab.inbox(&key(2)).await), "unknown_member");
        assert!(!root.join("inbox").join(&member.me.id).exists());
        assert_eq!(lab.view(&key(1)).await.unwrap().members.len(), 1);

        // A member may leave by removing themselves.
        let invite = LabInvite::generate("https://relay.example.test", [7; 32]).unwrap();
        lab.add_invite(&key(1), &invite.nonce_hash()).await.unwrap();
        let other = lab
            .join(&key(3), joining("Kai", Some(&invite)))
            .await
            .unwrap();
        lab.remove(&key(3), &other.me.id).await.unwrap();
        assert_eq!(code(lab.view(&key(3)).await), "unknown_member");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn only_the_leader_commits_to_the_knowledge_base() {
        let (root, lab, _, member) = lab_with_pending_member().await;
        lab.approve(&key(1), &member.me.id).await.unwrap();
        let knowledge_base = LabKnowledgeBase {
            project_id: "kb-project".into(),
            sealed: "sealed-device-code".into(),
        };
        assert_eq!(
            code(
                lab.set_knowledge_base(&key(2), knowledge_base.clone())
                    .await
            ),
            "not_leader"
        );
        assert!(lab.may_commit("kb-project", &key(2)).await);
        lab.set_knowledge_base(&key(1), knowledge_base.clone())
            .await
            .unwrap();
        assert_eq!(
            lab.view(&key(2)).await.unwrap().knowledge_base,
            Some(knowledge_base)
        );
        assert!(lab.may_commit("kb-project", &key(1)).await);
        assert!(!lab.may_commit("kb-project", &key(2)).await);
        assert!(!lab.may_commit("kb-project", "").await);
        // Every other project on the relay is none of the lab's business.
        assert!(lab.may_commit("someone-elses-project", "").await);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[test]
    fn an_invite_round_trips_and_mail_is_bound_to_both_ends() {
        let invite = LabInvite::generate(" https://relay.example.test ", [7; 32]).unwrap();
        let parsed = LabInvite::parse(&invite.encode()).unwrap();
        assert!(parsed == invite);
        assert_eq!(parsed.relay_url, "https://relay.example.test");
        assert_eq!(parsed.nonce_hash(), sha256_hex(parsed.nonce().as_bytes()));
        assert!(LabInvite::parse("wisp-sync:abc").is_err());
        assert!(LabInvite::parse("wisp-lab:not-base64!").is_err());

        let sealed = seal_frame(&invite.lab_key, &lab_mail_aad("ada", "lin"), b"hello").unwrap();
        assert_eq!(
            open_frame(&invite.lab_key, &lab_mail_aad("ada", "lin"), &sealed).unwrap(),
            b"hello"
        );
        // A relay that redirects the mail or renames its sender is caught.
        assert!(open_frame(&invite.lab_key, &lab_mail_aad("ada", "kai"), &sealed).is_err());
        assert!(open_frame(&invite.lab_key, &lab_mail_aad("eve", "lin"), &sealed).is_err());
        assert!(open_frame(&invite.lab_key, LAB_KB_AAD, &sealed).is_err());
    }

    async fn status(app: &Router, method: &str, path: &str, token: Option<&str>) -> StatusCode {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        app.clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn a_relay_without_the_switch_has_no_lab() {
        let root = std::env::temp_dir().join(format!("wisp-lab-off-{}", uuid::Uuid::new_v4()));
        let relay = FileRelay::open(&root).await.unwrap();
        let app = relay_router(RelayHttpState::new(relay, "token").unwrap());
        assert_eq!(
            status(&app, "GET", "/v1/lab", None).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(&app, "GET", "/v1/lab", Some("token")).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(&app, "GET", "/v1/lab/mail", Some("token")).await,
            StatusCode::NOT_FOUND
        );
        assert!(!root.join("lab").exists());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn the_client_drives_a_lab_end_to_end_over_http() {
        let root = std::env::temp_dir().join(format!("wisp-lab-http-{}", uuid::Uuid::new_v4()));
        let relay = FileRelay::open(&root).await.unwrap();
        let lab = Lab::open(root.join("lab")).await.unwrap();
        let app = relay_router(RelayHttpState::new(relay, "token").unwrap().with_lab(lab));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = |member: u8| {
            HttpRelay::new(&url, "token")
                .unwrap()
                .with_lab_member(key(member))
        };
        let rejected = |error: anyhow::Error| error.downcast::<LabRejection>().unwrap().code;

        let wrong_token = HttpRelay::new(&url, "wrong")
            .unwrap()
            .with_lab_member(key(1));
        assert_eq!(
            rejected(wrong_token.lab_view().await.unwrap_err()),
            "unauthorized"
        );
        assert_eq!(
            rejected(client(1).lab_view().await.unwrap_err()),
            "unknown_member"
        );

        let leader = client(1).lab_join(&joining("Ada", None)).await.unwrap();
        let invite = LabInvite::generate(&url, [7; 32]).unwrap();
        assert_eq!(
            rejected(client(2).lab_add_invite(&invite).await.unwrap_err()),
            "unknown_member"
        );
        client(1).lab_add_invite(&invite).await.unwrap();
        let member = client(2)
            .lab_join(&joining("Lin", Some(&invite)))
            .await
            .unwrap();
        client(1).lab_approve(&member.me.id).await.unwrap();

        let sealed = seal_frame(
            &invite.lab_key,
            &lab_mail_aad(&leader.me.id, &member.me.id),
            b"which reference genome?",
        )
        .unwrap();
        let sent = client(1)
            .lab_send_mail(&member.me.id, &sealed)
            .await
            .unwrap();
        let waiting = client(2).lab_inbox().await.unwrap();
        assert_eq!(waiting, vec![sent.clone()]);
        assert_eq!(
            open_frame(
                &invite.lab_key,
                &lab_mail_aad(&waiting[0].from, &member.me.id),
                &waiting[0].d
            )
            .unwrap(),
            b"which reference genome?"
        );
        client(2).lab_ack(&sent.id).await.unwrap();
        assert!(client(2).lab_inbox().await.unwrap().is_empty());

        // The knowledge base is an ordinary synced project only the leader
        // may commit to.
        let blob = b"encrypted-placeholder";
        let blob_id = sha256_hex(blob);
        client(2).put_blob(&blob_id, blob.to_vec()).await.unwrap();
        client(1)
            .lab_set_knowledge_base(&LabKnowledgeBase {
                project_id: "kb-project".into(),
                sealed: seal_frame(&invite.lab_key, LAB_KB_AAD, b"wisp-sync:code").unwrap(),
            })
            .await
            .unwrap();
        let commit = || CommitRequest {
            base_revision: None,
            revision: SyncRevision {
                protocol_version: SYNC_PROTOCOL_VERSION,
                project_id: "kb-project".into(),
                revision_id: "revision-1".into(),
                parent_revision: None,
                device_id: "device-1".into(),
                created_at: 1,
                metadata_blob: blob_id.clone(),
                manifest_blob: blob_id.clone(),
                workspace_blobs: vec![],
                state_hash: sha256_hex(b"state"),
                auth_tag: sha256_hex(b"auth"),
            },
        };
        let refused = client(2).commit("kb-project", commit()).await.unwrap_err();
        assert!(refused.to_string().contains("403"), "{refused}");
        assert!(client(2).head("kb-project").await.unwrap().is_none());
        assert!(matches!(
            client(1).commit("kb-project", commit()).await.unwrap(),
            CommitOutcome::Committed(_)
        ));
        // Members still read it.
        assert!(client(2).head("kb-project").await.unwrap().is_some());

        client(1).lab_remove(&member.me.id).await.unwrap();
        assert_eq!(
            rejected(client(2).lab_inbox().await.unwrap_err()),
            "unknown_member"
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
