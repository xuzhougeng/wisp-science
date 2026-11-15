//! Wisp Lab: this computer's membership in the research group a
//! `wisp-relay` started with `WISP_LAB=1` hosts (docs/wisp-lab.md). A
//! port of the WebView's `src-tauri/src/channels/lab.rs`.
//!
//! The relay knows who belongs to the lab. The lab key stays on the
//! members' computers: it comes inside the leader's invite code and seals
//! every mail before it leaves. The identity is one keychain secret under
//! the WebView's name, written whole, so a member key never outlives the
//! lab key it belongs to. Not ported: the knowledge base, which is a
//! project shared through project sync; its name is shown when the leader
//! published one.

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use wisp_dto::{LabKnowledgeBaseInfo, LabMemberInfo, LabStatus};
use wisp_sync::{
    HttpRelay, LabInvite, LabJoinRequest, LabMemberStatus, LabRejection, LabRole, LabView,
};

use super::get_setting;
use crate::Server;

const TOKEN_SECRET: &str = "lab_relay_token";
const IDENTITY_SECRET: &str = "lab_identity";
/// Project sync's relay, which the WebView reads the knowledge base with.
const SYNC_TOKEN_SECRET: &str = "sync_relay_token";
const SYNC_URL_KEY: &str = "sync_relay_url";
/// "1" while this computer holds a lab identity; read on every turn, so
/// the keychain is opened only when the lab is used.
pub(crate) const MEMBER_KEY: &str = "lab_member";
const KB_PROJECT_KEY: &str = "lab_knowledge_base_project";
/// The project whose conversations answer other members' agents.
pub(crate) const INBOX_PROJECT_KEY: &str = "lab_inbox_project";

/// Everything that makes this computer a member.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Identity {
    pub(crate) relay_url: String,
    pub(crate) member_key: String,
    /// Hex of the 32-byte lab key.
    lab_key: String,
    pub(crate) name: String,
}

impl Identity {
    pub(crate) fn lab_key(&self) -> Result<[u8; 32]> {
        hex::decode(&self.lab_key)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| anyhow!("The stored lab key is damaged. Leave the lab and join again."))
    }
}

/// What a relay's refusal code means.
fn refusal(code: &str) -> Option<&'static str> {
    Some(match code {
        "lab_disabled" => {
            "This relay does not host a lab. Its operator has to start it with WISP_LAB=1."
        }
        "unauthorized" => "The relay rejected the access token.",
        "unknown_member" => "The relay does not know this computer as a lab member.",
        "invite_required" => "This lab already has a leader. Ask them for an invite code.",
        "invalid_invite" => {
            "This invite code is not valid. It may have been used already or have expired."
        }
        "invalid_name" => "Enter a name of at most 64 characters.",
        "not_active" => "The leader has not approved this computer yet.",
        "not_leader" => "Only the lab leader can do this.",
        "leader_cannot_leave" => "The leader cannot leave the lab.",
        "lab_full" => "The lab has reached its member limit.",
        "too_many_invites" => {
            "Too many unused invite codes. Wait for some to be used or to expire."
        }
        "member_not_found" => "That member is no longer in the lab.",
        "storage_error" => "The relay could not save the change. Check its data directory.",
        _ => return None,
    })
}

/// A relay's refusal as what it means; a transport error as it is.
pub(crate) fn explain(error: anyhow::Error) -> anyhow::Error {
    match error.downcast_ref::<LabRejection>() {
        Some(rejection) => match refusal(&rejection.code) {
            Some(meaning) => anyhow!(meaning),
            None if rejection.code.is_empty() => {
                anyhow!("The relay refused the request ({}).", rejection.status)
            }
            None => anyhow!("The relay refused the request ({}).", rejection.code),
        },
        None => error,
    }
}

pub(crate) fn is_rejection(error: &anyhow::Error, code: &str) -> bool {
    error
        .downcast_ref::<LabRejection>()
        .is_some_and(|rejection| rejection.code == code)
}

fn member_info(member: &wisp_sync::LabMember) -> LabMemberInfo {
    LabMemberInfo {
        id: member.id.clone(),
        name: member.name.clone(),
        leader: member.role == LabRole::Leader,
        pending: member.status == LabMemberStatus::Pending,
    }
}

impl Server {
    pub(crate) fn lab_identity(&self) -> Option<Identity> {
        serde_json::from_str(&self.0.secrets.get(IDENTITY_SECRET)?).ok()
    }

    pub(crate) fn lab_client(&self, identity: &Identity) -> Result<HttpRelay> {
        let token = self.0.secrets.get(TOKEN_SECRET).unwrap_or_default();
        Ok(
            HttpRelay::new(&identity.relay_url, token)?
                .with_lab_member(identity.member_key.clone()),
        )
    }

    fn require_lab(&self) -> Result<(Identity, HttpRelay)> {
        let identity = self
            .lab_identity()
            .context("This computer has not joined a lab.")?;
        let client = self.lab_client(&identity)?;
        Ok((identity, client))
    }

    pub(crate) async fn is_lab_member(&self) -> bool {
        get_setting(&self.0.store, MEMBER_KEY).await == "1"
    }

    async fn set_lab_member(&self, member: bool) {
        let store = &self.0.store;
        let _ = store
            .set_setting(MEMBER_KEY, if member { "1" } else { "" })
            .await;
        if !member {
            let _ = store.set_setting(KB_PROJECT_KEY, "").await;
        }
    }

    async fn status_from_view(&self, mut status: LabStatus, view: LabView) -> LabStatus {
        let store = &self.0.store;
        let active = view.me.status == LabMemberStatus::Active;
        status.state = if active { "active" } else { "pending" }.into();
        status.lab_name = view.name;
        status.member_id = view.me.id.clone();
        status.leader = active && view.me.role == LabRole::Leader;
        status.members = view.members.iter().map(member_info).collect();
        let _ = store
            .set_setting(
                KB_PROJECT_KEY,
                view.knowledge_base
                    .as_ref()
                    .map_or("", |knowledge_base| knowledge_base.project_id.as_str()),
            )
            .await;
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

    /// The lab as it stands, asked of its relay.
    pub async fn lab_status(&self) -> LabStatus {
        let mut status = LabStatus {
            state: "none".into(),
            has_token: !self
                .0
                .secrets
                .get(TOKEN_SECRET)
                .unwrap_or_default()
                .is_empty(),
            ..Default::default()
        };
        let Some(identity) = self.lab_identity() else {
            self.set_lab_member(false).await;
            return status;
        };
        // The keychain decides when settings and keychain drift apart.
        self.set_lab_member(true).await;
        status.inbox_project_id = get_setting(&self.0.store, INBOX_PROJECT_KEY).await;
        status.relay_url = identity.relay_url.clone();
        status.member_name = identity.name.clone();
        let view = match self.lab_client(&identity) {
            Ok(client) => client.lab_view().await,
            Err(error) => Err(error),
        };
        match view {
            Ok(view) => self.status_from_view(status, view).await,
            // Removed by the leader, or the relay's lab is gone.
            Err(error) if is_rejection(&error, "unknown_member") => {
                status.state = "removed".into();
                status
            }
            Err(error) => {
                status.state = "error".into();
                status.detail = explain(error).to_string();
                status
            }
        }
    }

    /// Founds the relay's lab (no invite: the first computer leads it) or
    /// asks to join it with the leader's invite code. No token keeps the
    /// stored one; with an invite the relay's address may be left empty.
    pub async fn join_lab(
        &self,
        relay_url: &str,
        relay_token: Option<String>,
        name: &str,
        lab_name: &str,
        invite: &str,
    ) -> Result<LabStatus> {
        ensure!(
            self.lab_identity().is_none(),
            "Leave the current lab before joining another one."
        );
        let name = name.trim().to_owned();
        ensure!(
            !name.is_empty(),
            "Enter the name other lab members will see."
        );
        let invite = match invite.trim() {
            "" => None,
            code => Some(
                LabInvite::parse(code)
                    .map_err(|_| anyhow!(refusal("invalid_invite").unwrap_or_default()))?,
            ),
        };
        let relay_url = match (relay_url.trim(), &invite) {
            ("", Some(invite)) => invite.relay_url.clone(),
            (typed, _) => typed.to_owned(),
        };
        wisp_sync::relay_base(&relay_url)?;
        let token = match relay_token.as_deref().map(str::trim).unwrap_or_default() {
            "" => self.0.secrets.get(TOKEN_SECRET).unwrap_or_default(),
            typed => typed.to_owned(),
        };
        ensure!(!token.is_empty(), "Enter the relay access token.");
        let lab_key = match &invite {
            Some(invite) => invite.lab_key,
            None => wisp_sync::random_project_key()?,
        };
        let identity = Identity {
            relay_url: relay_url.clone(),
            member_key: hex::encode(wisp_sync::random_project_key()?),
            lab_key: hex::encode(lab_key),
            name: name.clone(),
        };
        // The keys first: a lab founded with a key this computer then
        // failed to keep could never be read by anyone.
        self.0.secrets.set(TOKEN_SECRET, &token)?;
        self.0
            .secrets
            .set(IDENTITY_SECRET, &serde_json::to_string(&identity)?)?;
        let client = self.lab_client(&identity)?;
        let request = LabJoinRequest {
            name,
            lab_name: Some(lab_name.trim().to_owned()).filter(|name| !name.is_empty()),
            invite: invite.as_ref().map(LabInvite::nonce),
        };
        let view = match client.lab_join(&request).await {
            Ok(view) => view,
            Err(error) => {
                let _ = self.0.secrets.set(IDENTITY_SECRET, "");
                return Err(explain(error));
            }
        };
        // The WebView reads the knowledge base through project sync; a
        // computer that never set that up gets the lab's relay for it.
        if self
            .0
            .secrets
            .get(SYNC_TOKEN_SECRET)
            .unwrap_or_default()
            .is_empty()
        {
            let _ = self.0.secrets.set(SYNC_TOKEN_SECRET, &token);
        }
        if get_setting(&self.0.store, SYNC_URL_KEY)
            .await
            .trim()
            .is_empty()
        {
            let _ = self.0.store.set_setting(SYNC_URL_KEY, &relay_url).await;
        }
        self.set_lab_member(true).await;
        self.start_lab();
        let status = LabStatus {
            inbox_project_id: get_setting(&self.0.store, INBOX_PROJECT_KEY).await,
            relay_url,
            has_token: true,
            member_name: identity.name,
            ..Default::default()
        };
        Ok(self.status_from_view(status, view).await)
    }

    /// Replaces the relay token after its operator changed it.
    pub async fn set_lab_relay_token(&self, token: &str) -> Result<()> {
        let token = token.trim();
        ensure!(!token.is_empty(), "Enter the relay access token.");
        self.0.secrets.set(TOKEN_SECRET, token)?;
        // The mailbox poller holds the old one.
        self.start_lab();
        Ok(())
    }

    /// Leaves the lab and drops its keys from this computer. `forget`
    /// skips the relay, for a lab that removed this computer or cannot be
    /// reached.
    pub async fn leave_lab(&self, forget: bool) -> Result<()> {
        let Some(identity) = self.lab_identity() else {
            return Ok(());
        };
        if !forget {
            let client = self.lab_client(&identity)?;
            match client.lab_view().await {
                Ok(view) => client.lab_remove(&view.me.id).await.map_err(explain)?,
                Err(error) if is_rejection(&error, "unknown_member") => {}
                Err(error) => return Err(explain(error)),
            }
        }
        // The keychain has no delete here; empty reads as none.
        self.0.secrets.set(IDENTITY_SECRET, "")?;
        self.0.channels.lab.stop();
        self.set_lab_member(false).await;
        Ok(())
    }

    /// The project whose conversations answer other members' agents; none
    /// stops this computer from taking lab mail (it waits on the relay).
    pub async fn set_lab_inbox_project(&self, project: &str) -> Result<()> {
        let project = project.trim();
        if !project.is_empty()
            && (wisp_store::is_assistant_project_id(project)
                || self.0.store.get_project(project).await?.is_none())
        {
            bail!("Project not found.");
        }
        self.0.store.set_setting(INBOX_PROJECT_KEY, project).await
    }

    /// A new single-use invite code. It carries the lab key: the leader
    /// gives it to one person, and the relay token separately.
    pub async fn create_lab_invite(&self) -> Result<String> {
        let (identity, client) = self.require_lab()?;
        let invite = LabInvite::generate(&identity.relay_url, identity.lab_key()?)?;
        client.lab_add_invite(&invite).await.map_err(explain)?;
        Ok(invite.encode())
    }

    pub async fn approve_lab_member(&self, member: &str) -> Result<()> {
        let (_, client) = self.require_lab()?;
        client.lab_approve(member).await.map_err(explain)
    }

    pub async fn remove_lab_member(&self, member: &str) -> Result<()> {
        let (_, client) = self.require_lab()?;
        client.lab_remove(member).await.map_err(explain)
    }

    /// Fetches lab mail in place of any running poller; nothing on a
    /// computer without a lab.
    pub(crate) fn start_lab(&self) {
        let worker = &self.0.channels.lab;
        let Some(identity) = self.lab_identity() else {
            worker.stop();
            return;
        };
        let Ok(client) = self.lab_client(&identity) else {
            worker.stop();
            return;
        };
        let stop = worker.begin();
        tokio::spawn(super::lab_mail::run(self.clone(), identity, client, stop));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_reads_as_what_it_means_and_a_transport_error_stays() {
        let refused = anyhow::Error::from(LabRejection {
            status: 403,
            code: "invite_required".into(),
        });
        assert!(is_rejection(&refused, "invite_required"));
        assert_eq!(
            explain(refused).to_string(),
            "This lab already has a leader. Ask them for an invite code."
        );
        let bare = anyhow::Error::from(LabRejection {
            status: 502,
            code: String::new(),
        });
        assert_eq!(
            explain(bare).to_string(),
            "The relay refused the request (502)."
        );
        let transport = anyhow!("connection refused");
        assert!(!is_rejection(&transport, "invite_required"));
        assert_eq!(explain(transport).to_string(), "connection refused");
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
