//! Wisp Lab on the desktop (docs/wisp-lab.md): this computer's membership in
//! the research group hosted by a `wisp-relay` started with `WISP_LAB=1`.
//!
//! The relay knows who belongs to the lab. The lab key stays on the members'
//! computers: it arrives inside the leader's invite code and seals the
//! knowledge-base pointer and every mail before they leave this process.
use super::load_secret;
use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use wisp_dto::{LabKnowledgeBaseInfo, LabMemberInfo, LabStatus};
use wisp_store::secrets::Secret;
use wisp_store::Store;
use wisp_sync::{
    open_frame, seal_frame, HttpRelay, LabInvite, LabJoinRequest, LabKnowledgeBase,
    LabMemberStatus, LabRejection, LabRole, LabView, LAB_KB_AAD,
};

const TOKEN_SECRET: &str = "lab_relay_token";
const IDENTITY_SECRET: &str = "lab_identity";
/// Project sync's relay token (`project_sync.rs`): the knowledge base is an
/// ordinary synced project, so it is read with this one.
const SYNC_TOKEN_SECRET: &str = "sync_relay_token";
const SYNC_URL_KEY: &str = "sync_relay_url";

/// Everything that makes this computer a member. One secret, written whole,
/// so a crash cannot leave a member key without the lab key it belongs to.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Identity {
    pub(crate) relay_url: String,
    pub(crate) member_key: String,
    /// Hex of the 32-byte lab key.
    lab_key: String,
    pub(crate) name: String,
}

impl Identity {
    pub(crate) fn lab_key(&self) -> Result<[u8; 32], String> {
        hex::decode(&self.lab_key)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| "The stored lab key is damaged. Leave the lab and join again.".into())
    }

    pub(crate) async fn client(&self) -> Result<HttpRelay, String> {
        Ok(
            HttpRelay::new(&self.relay_url, load_secret(TOKEN_SECRET).await)
                .map_err(|error| error.to_string())?
                .with_lab_member(self.member_key.clone()),
        )
    }
}

pub(crate) async fn load_identity() -> Option<Identity> {
    serde_json::from_str(&load_secret(IDENTITY_SECRET).await).ok()
}

async fn write_secret(name: &'static str, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || Secret::set(name, &value))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

async fn forget_identity() -> Result<(), String> {
    tokio::task::spawn_blocking(|| Secret::delete(IDENTITY_SECRET))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

fn same_relay(left: &str, right: &str) -> bool {
    matches!(
        (wisp_sync::relay_base(left), wisp_sync::relay_base(right)),
        (Ok(left), Ok(right)) if left == right
    )
}

/// The member key for a sync transport pointed at the lab's relay. The
/// leader's commits to the knowledge base carry it; no other relay sees it.
pub(crate) async fn member_key_for(relay_url: &str) -> Option<String> {
    load_identity()
        .await
        .filter(|identity| same_relay(&identity.relay_url, relay_url))
        .map(|identity| identity.member_key)
}

/// A relay refusal becomes `lab:<code>`, which the settings pane translates.
/// Anything else is a transport error and is shown as it is.
pub(crate) fn explain(error: anyhow::Error) -> String {
    match error.downcast_ref::<LabRejection>() {
        Some(rejection) if !rejection.code.is_empty() => format!("lab:{}", rejection.code),
        Some(rejection) => format!("The relay refused the request ({}).", rejection.status),
        None => error.to_string(),
    }
}

fn is_rejection(error: &anyhow::Error, code: &str) -> bool {
    error
        .downcast_ref::<LabRejection>()
        .is_some_and(|rejection| rejection.code == code)
}

async fn require_identity() -> Result<Identity, String> {
    load_identity()
        .await
        .ok_or_else(|| "This computer has not joined a lab.".to_string())
}

fn member_info(member: &wisp_sync::LabMember) -> LabMemberInfo {
    LabMemberInfo {
        id: member.id.clone(),
        name: member.name.clone(),
        leader: member.role == LabRole::Leader,
        pending: member.status == LabMemberStatus::Pending,
    }
}

async fn status_from_view(store: &Store, mut status: LabStatus, view: LabView) -> LabStatus {
    let active = view.me.status == LabMemberStatus::Active;
    status.state = if active { "active" } else { "pending" }.into();
    status.lab_name = view.name;
    status.member_id = view.me.id.clone();
    status.leader = active && view.me.role == LabRole::Leader;
    status.members = view.members.iter().map(member_info).collect();
    if let Some(knowledge_base) = view.knowledge_base {
        let local = store
            .get_project(&knowledge_base.project_id)
            .await
            .ok()
            .flatten();
        status.knowledge_base = Some(LabKnowledgeBaseInfo {
            project_id: knowledge_base.project_id,
            local: local.is_some(),
            project_name: local.map(|(name, _)| name).unwrap_or_default(),
        });
    }
    status
}

async fn status(store: &Store) -> LabStatus {
    let mut status = LabStatus {
        state: "none".into(),
        has_token: !load_secret(TOKEN_SECRET).await.is_empty(),
        ..Default::default()
    };
    let Some(identity) = load_identity().await else {
        return status;
    };
    status.relay_url = identity.relay_url.clone();
    status.member_name = identity.name.clone();
    let view = match identity.client().await {
        Ok(client) => client.lab_view().await,
        Err(error) => Err(anyhow::anyhow!(error)),
    };
    match view {
        Ok(view) => status_from_view(store, status, view).await,
        // Removed by the leader, or the relay's lab data is gone.
        Err(error) if is_rejection(&error, "unknown_member") => {
            status.state = "removed".into();
            status
        }
        Err(error) => {
            status.state = "error".into();
            status.detail = explain(error);
            status
        }
    }
}

#[tauri::command]
pub(crate) async fn lab_status(state: State<'_, AppState>) -> Result<LabStatus, String> {
    Ok(status(&state.store).await)
}

/// The knowledge base is read through project sync. A computer that never
/// configured it gets the lab's relay, so **Sync now** works on that project.
/// An existing sync configuration is left alone.
async fn seed_project_sync(store: &Store, relay_url: &str, token: &str) {
    if load_secret(SYNC_TOKEN_SECRET).await.is_empty() {
        let _ = write_secret(SYNC_TOKEN_SECRET, token.to_string()).await;
    }
    if super::get_setting(store, SYNC_URL_KEY)
        .await
        .trim()
        .is_empty()
    {
        let _ = store.set_setting(SYNC_URL_KEY, relay_url).await;
    }
}

/// Found the relay's lab (no invite, first computer) or ask to join it (the
/// leader's invite code). An empty token keeps the stored one.
#[tauri::command]
pub(crate) async fn lab_join(
    state: State<'_, AppState>,
    relay_url: String,
    relay_token: String,
    name: String,
    lab_name: String,
    invite: String,
) -> Result<LabStatus, String> {
    if load_identity().await.is_some() {
        return Err("Leave the current lab before joining another one.".into());
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Enter the name other lab members will see.".into());
    }
    let invite = match invite.trim() {
        "" => None,
        code => Some(LabInvite::parse(code).map_err(|_| "lab:invalid_invite".to_string())?),
    };
    let relay_url = match (relay_url.trim(), &invite) {
        ("", Some(invite)) => invite.relay_url.clone(),
        (typed, _) => typed.to_string(),
    };
    wisp_sync::relay_base(&relay_url).map_err(|error| error.to_string())?;
    let token = match relay_token.trim() {
        "" => load_secret(TOKEN_SECRET).await,
        typed => typed.to_string(),
    };
    if token.is_empty() {
        return Err("Enter the relay access token.".into());
    }
    let lab_key = match &invite {
        Some(invite) => invite.lab_key,
        None => wisp_sync::random_project_key().map_err(|error| error.to_string())?,
    };
    let identity = Identity {
        relay_url: relay_url.clone(),
        member_key: hex::encode(
            wisp_sync::random_project_key().map_err(|error| error.to_string())?,
        ),
        lab_key: hex::encode(lab_key),
        name: name.clone(),
    };
    let client = HttpRelay::new(&relay_url, token.clone())
        .map_err(|error| error.to_string())?
        .with_lab_member(identity.member_key.clone());
    // Keys first: a lab founded with a key this computer then failed to keep
    // could never be read by anyone.
    write_secret(TOKEN_SECRET, token.clone()).await?;
    write_secret(
        IDENTITY_SECRET,
        serde_json::to_string(&identity).map_err(|error| error.to_string())?,
    )
    .await?;
    let request = LabJoinRequest {
        name,
        lab_name: Some(lab_name.trim().to_string()).filter(|name| !name.is_empty()),
        invite: invite.as_ref().map(LabInvite::nonce),
    };
    let view = match client.lab_join(&request).await {
        Ok(view) => view,
        Err(error) => {
            let _ = forget_identity().await;
            return Err(explain(error));
        }
    };
    seed_project_sync(&state.store, &relay_url, &token).await;
    let status = LabStatus {
        relay_url,
        has_token: true,
        member_name: identity.name,
        ..Default::default()
    };
    Ok(status_from_view(&state.store, status, view).await)
}

/// Replace the relay token after the operator rotated it.
#[tauri::command]
pub(crate) async fn lab_set_relay_token(relay_token: String) -> Result<(), String> {
    let token = relay_token.trim().to_string();
    if token.is_empty() {
        return Err("Enter the relay access token.".into());
    }
    write_secret(TOKEN_SECRET, token).await
}

/// Leave the lab and drop its keys from this computer. `forget` skips the
/// relay, for a lab that removed this computer or can no longer be reached.
#[tauri::command]
pub(crate) async fn lab_leave(forget: bool) -> Result<(), String> {
    let Some(identity) = load_identity().await else {
        return Ok(());
    };
    if !forget {
        let client = identity.client().await?;
        match client.lab_view().await {
            Ok(view) => client.lab_remove(&view.me.id).await.map_err(explain)?,
            Err(error) if is_rejection(&error, "unknown_member") => {}
            Err(error) => return Err(explain(error)),
        }
    }
    forget_identity().await
}

/// A new single-use invite code. It carries the lab key: the leader hands it
/// to one person, and gives them the relay token separately.
#[tauri::command]
pub(crate) async fn lab_create_invite() -> Result<String, String> {
    let identity = require_identity().await?;
    let invite = LabInvite::generate(&identity.relay_url, identity.lab_key()?)
        .map_err(|error| error.to_string())?;
    identity
        .client()
        .await?
        .lab_add_invite(&invite)
        .await
        .map_err(explain)?;
    Ok(invite.encode())
}

#[tauri::command]
pub(crate) async fn lab_approve_member(member_id: String) -> Result<(), String> {
    let identity = require_identity().await?;
    identity
        .client()
        .await?
        .lab_approve(&member_id)
        .await
        .map_err(explain)
}

#[tauri::command]
pub(crate) async fn lab_remove_member(member_id: String) -> Result<(), String> {
    let identity = require_identity().await?;
    identity
        .client()
        .await?
        .lab_remove(&member_id)
        .await
        .map_err(explain)
}

/// Make a project the lab's knowledge base. It must already be synchronized
/// through the lab's relay; from now on the relay accepts commits to it from
/// the leader only.
#[tauri::command]
pub(crate) async fn lab_publish_knowledge_base(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<(), String> {
    let identity = require_identity().await?;
    let (relay_url, code) =
        crate::project_sync::relay_device_code(&state.store, &project_id).await?;
    if !same_relay(&relay_url, &identity.relay_url) {
        return Err("lab:knowledge_base_other_relay".into());
    }
    let sealed = seal_frame(&identity.lab_key()?, LAB_KB_AAD, code.as_bytes())
        .map_err(|error| error.to_string())?;
    identity
        .client()
        .await?
        .lab_set_knowledge_base(&LabKnowledgeBase { project_id, sealed })
        .await
        .map_err(explain)
}

/// Download the knowledge base as a project on this computer. `None` when the
/// folder picker was cancelled.
#[tauri::command]
pub(crate) async fn lab_join_knowledge_base(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<crate::ProjectSummary>, String> {
    let identity = require_identity().await?;
    let view = identity.client().await?.lab_view().await.map_err(explain)?;
    let knowledge_base = view
        .knowledge_base
        .ok_or_else(|| "lab:no_knowledge_base".to_string())?;
    let code = open_frame(&identity.lab_key()?, LAB_KB_AAD, &knowledge_base.sealed)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        // Sealed with another key: this invite belonged to a different lab.
        .ok_or_else(|| "lab:knowledge_base_unreadable".to_string())?;
    crate::project_sync::join_synced_project(app, state, code).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_sync::LabMember;

    #[test]
    fn a_refusal_becomes_a_code_and_a_transport_error_stays_readable() {
        let refused = anyhow::Error::from(LabRejection {
            status: 403,
            code: "invite_required".into(),
        });
        assert!(is_rejection(&refused, "invite_required"));
        assert_eq!(explain(refused), "lab:invite_required");
        let bare = anyhow::Error::from(LabRejection {
            status: 502,
            code: String::new(),
        });
        assert_eq!(explain(bare), "The relay refused the request (502).");
        let transport = anyhow::anyhow!("connection refused");
        assert!(!is_rejection(&transport, "invite_required"));
        assert_eq!(explain(transport), "connection refused");
    }

    #[test]
    fn the_member_key_goes_only_to_the_labs_own_relay() {
        assert!(same_relay(
            "https://relay.example.test",
            "https://relay.example.test/"
        ));
        assert!(!same_relay(
            "https://relay.example.test",
            "https://other.example.test"
        ));
        assert!(!same_relay(
            "https://relay.example.test",
            "https://relay.example.test/team"
        ));
        assert!(!same_relay("", ""));
    }

    #[test]
    fn members_are_shown_with_their_role_and_review_state() {
        let info = member_info(&LabMember {
            id: "m1".into(),
            name: "Lin".into(),
            role: LabRole::Member,
            status: LabMemberStatus::Pending,
            joined_at: 1,
        });
        assert_eq!(
            info,
            LabMemberInfo {
                id: "m1".into(),
                name: "Lin".into(),
                leader: false,
                pending: true,
            }
        );
    }

    #[test]
    fn a_damaged_lab_key_is_reported_instead_of_used() {
        let mut identity = Identity {
            relay_url: "https://relay.example.test".into(),
            member_key: "k".into(),
            lab_key: hex::encode([7_u8; 32]),
            name: "Ada".into(),
        };
        assert_eq!(identity.lab_key().unwrap(), [7_u8; 32]);
        identity.lab_key = "abcd".into();
        assert!(identity.lab_key().is_err());
    }
}
