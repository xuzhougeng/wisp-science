use super::{save_approval_grants, AppState, ApprovalGrantKey, ConfirmRequest, PendingConfirm};
use serde::Serialize;
use std::collections::HashMap;
use tauri::State;

const MIN_REMOTE_APPROVAL_PREFIX: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteConfirmationSource {
    Native,
    Acp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteConfirmationResolution {
    pub(crate) approval_id: String,
    pub(crate) frame_id: String,
    pub(crate) source: RemoteConfirmationSource,
}

async fn ensure_project_frame(
    state: &AppState,
    project_id: &str,
    frame_id: &str,
) -> Result<(), String> {
    match state
        .store
        .frame_project_id(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
    {
        Some(owner) if owner == project_id => Ok(()),
        Some(_) => Err("Conversation does not belong to the active project.".into()),
        None => Err("Conversation does not exist.".into()),
    }
}

pub(crate) fn session_full_permission(state: &AppState, session_id: &str) -> bool {
    state
        .full_permission_sessions
        .read()
        .map(|sessions| sessions.contains(session_id))
        .unwrap_or(false)
}

pub(super) fn cancel_pending_confirmation(state: &AppState, session_id: &str) {
    let pending = state.confirms.lock().unwrap().remove(session_id);
    if let Some(pending) = pending {
        let _ = pending
            .tx
            .send(wisp_tools::ConfirmDecision::Denied { feedback: None });
    }
    state.awaiting_confirm.lock().unwrap().remove(session_id);
    state.device_hub.resolve_needs_user(session_id);
}

pub(crate) fn pending_confirmation_requests(state: &AppState) -> Vec<ConfirmRequest> {
    let mut requests = state
        .confirms
        .lock()
        .unwrap()
        .values()
        .map(|pending| pending.request.clone())
        .collect::<Vec<_>>();
    requests.sort_by(|left, right| left.approval_id.cmp(&right.approval_id));
    requests
}

fn remote_confirmation_session(
    pending: &HashMap<String, PendingConfirm>,
    selector: &str,
) -> Result<String, String> {
    let selector = selector.trim().to_ascii_lowercase();
    if selector.len() < MIN_REMOTE_APPROVAL_PREFIX
        || !selector.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err(format!(
            "审批编号至少需要 {MIN_REMOTE_APPROVAL_PREFIX} 位十六进制字符。"
        ));
    }

    let matches = pending
        .iter()
        .filter(|(_, value)| value.request.approval_id.starts_with(&selector))
        .map(|(session_id, _)| session_id.clone())
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err("未找到该待审批请求；它可能已经处理或失效。".into()),
        [session_id] => Ok(session_id.clone()),
        _ => Err("审批编号前缀不唯一，请输入更多位。".into()),
    }
}

fn take_remote_confirmation(
    state: &AppState,
    selector: &str,
) -> Result<(String, PendingConfirm), String> {
    let mut pending = state.confirms.lock().unwrap();
    let session_id = remote_confirmation_session(&pending, selector)?;
    let value = pending
        .remove(&session_id)
        .expect("matched pending confirmation must still exist");
    Ok((session_id, value))
}

async fn settle_confirmation(
    state: &AppState,
    session_id: &str,
    pending: PendingConfirm,
    decision: wisp_tools::ConfirmDecision,
    scope: Option<&str>,
) -> Result<(), String> {
    state.awaiting_confirm.lock().unwrap().remove(session_id);
    state.device_hub.resolve_needs_user(session_id);
    if decision.approved() {
        let scope = scope.unwrap_or("once");
        if matches!(scope, "session" | "project" | "global") {
            if let Some(grant) = pending.grant.clone() {
                let snapshot = {
                    let mut grants = state.approval_grants.lock().unwrap();
                    grants.grant(scope, session_id, &pending.project_id, grant);
                    grants.clone()
                };
                if scope != "session" {
                    save_approval_grants(&state.store, &snapshot).await?;
                }
            }
        }
    }
    let _ = pending.tx.send(decision);
    Ok(())
}

pub(crate) async fn respond_remote_confirmation(
    state: &AppState,
    selector: &str,
    approved: bool,
    feedback: Option<String>,
) -> Result<RemoteConfirmationResolution, String> {
    let (session_id, pending) = take_remote_confirmation(state, selector)?;
    let resolution = RemoteConfirmationResolution {
        approval_id: pending.request.approval_id.clone(),
        frame_id: session_id.clone(),
        source: RemoteConfirmationSource::Native,
    };
    let decision = if approved {
        wisp_tools::ConfirmDecision::Approved
    } else {
        wisp_tools::ConfirmDecision::Denied {
            feedback: feedback
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }
    };
    settle_confirmation(state, &session_id, pending, decision, Some("once")).await?;
    Ok(resolution)
}

#[tauri::command]
pub(super) async fn get_session_full_permission(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<bool, String> {
    let project = state.require_active(window.label())?;
    ensure_project_frame(&state, &project.id, &session_id).await?;
    Ok(session_full_permission(&state, &session_id))
}

#[tauri::command]
pub(super) async fn set_session_full_permission(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
    enabled: bool,
) -> Result<bool, String> {
    let project = state.require_active(window.label())?;
    ensure_project_frame(&state, &project.id, &session_id).await?;
    {
        let mut sessions = state
            .full_permission_sessions
            .write()
            .map_err(|_| "Full Permission state is unavailable.".to_string())?;
        if enabled {
            sessions.insert(session_id.clone());
        } else {
            sessions.remove(&session_id);
        }
    }

    // If the user enables the mode while an ordinary approval is already
    // waiting, settle that approval immediately. Later confirmation sites read
    // the shared mode live and never enqueue a card.
    if enabled {
        let pending = state.confirms.lock().unwrap().remove(&session_id);
        if let Some(pending) = pending {
            let _ = pending.tx.send(wisp_tools::ConfirmDecision::Approved);
            state.awaiting_confirm.lock().unwrap().remove(&session_id);
            state.device_hub.resolve_needs_user(&session_id);
        }
    }
    Ok(enabled)
}

#[tauri::command]
pub(super) async fn confirm_response(
    state: State<'_, AppState>,
    session_id: String,
    approved: bool,
    feedback: Option<String>,
    scope: Option<String>,
) -> Result<(), String> {
    let decision = if approved {
        wisp_tools::ConfirmDecision::Approved
    } else {
        wisp_tools::ConfirmDecision::Denied {
            feedback: feedback
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        }
    };
    let pending = state.confirms.lock().unwrap().remove(&session_id);
    if let Some(pending) = pending {
        settle_confirmation(&state, &session_id, pending, decision, scope.as_deref()).await
    } else {
        Err("no pending confirmation".into())
    }
}

#[derive(Serialize, Clone)]
pub(super) struct ApprovalGrantInfo {
    scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    kind: String,
    target: String,
    label: String,
}

fn approval_grant_label(key: &ApprovalGrantKey) -> String {
    match key.target.as_str() {
        "shell" => "Shell commands".into(),
        other => other.to_string(),
    }
}

#[tauri::command]
pub(super) fn list_approval_grants(state: State<'_, AppState>) -> Vec<ApprovalGrantInfo> {
    let grants = state.approval_grants.lock().unwrap().clone();
    let mut out = vec![];
    for (session_id, keys) in grants.session {
        for key in keys {
            out.push(ApprovalGrantInfo {
                scope: "session".into(),
                session_id: Some(session_id.clone()),
                project_id: None,
                label: approval_grant_label(&key),
                kind: key.kind,
                target: key.target,
            });
        }
    }
    for (project_id, keys) in grants.project {
        for key in keys {
            out.push(ApprovalGrantInfo {
                scope: "project".into(),
                session_id: None,
                project_id: Some(project_id.clone()),
                label: approval_grant_label(&key),
                kind: key.kind,
                target: key.target,
            });
        }
    }
    for key in grants.global {
        out.push(ApprovalGrantInfo {
            scope: "global".into(),
            session_id: None,
            project_id: None,
            label: approval_grant_label(&key),
            kind: key.kind,
            target: key.target,
        });
    }
    out.sort_by(|a, b| {
        a.scope
            .cmp(&b.scope)
            .then(a.label.cmp(&b.label))
            .then(a.target.cmp(&b.target))
    });
    out
}

#[tauri::command]
pub(super) async fn revoke_approval_grant(
    state: State<'_, AppState>,
    scope: String,
    kind: String,
    target: String,
    session_id: Option<String>,
    project_id: Option<String>,
) -> Result<(), String> {
    let key = ApprovalGrantKey { kind, target };
    let snapshot = {
        let mut grants = state.approval_grants.lock().unwrap();
        grants.revoke(&scope, session_id.as_deref(), project_id.as_deref(), &key);
        grants.clone()
    };
    save_approval_grants(&state.store, &snapshot).await
}

#[tauri::command]
pub(super) async fn revoke_all_approval_grants(state: State<'_, AppState>) -> Result<(), String> {
    let snapshot = {
        let mut grants = state.approval_grants.lock().unwrap();
        grants.clear();
        grants.clone()
    };
    save_approval_grants(&state.store, &snapshot).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(approval_id: &str) -> PendingConfirm {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut request = ConfirmRequest::new(
            "session",
            "Dangerous command detected".into(),
            "shell",
            "rm output.tmp".into(),
        );
        request.approval_id = approval_id.into();
        PendingConfirm {
            tx,
            grant: None,
            project_id: "project".into(),
            request,
        }
    }

    #[test]
    fn remote_approval_selector_is_case_insensitive_and_requires_safe_prefix() {
        let requests = HashMap::from([(
            "session-a".into(),
            pending("abcdef1234567890abcdef1234567890"),
        )]);
        assert_eq!(
            remote_confirmation_session(&requests, "ABCDEF12").as_deref(),
            Ok("session-a")
        );
        assert!(remote_confirmation_session(&requests, "abc").is_err());
        assert!(remote_confirmation_session(&requests, "not-hex").is_err());
        assert!(remote_confirmation_session(&requests, "ffffff").is_err());
    }

    #[test]
    fn remote_approval_selector_rejects_ambiguous_prefixes() {
        let requests = HashMap::from([
            (
                "session-a".into(),
                pending("abcdef1234567890abcdef1234567890"),
            ),
            (
                "session-b".into(),
                pending("abcdef9876543210abcdef9876543210"),
            ),
        ]);
        let error = remote_confirmation_session(&requests, "abcdef").unwrap_err();
        assert!(error.contains("不唯一"));
        assert_eq!(
            remote_confirmation_session(&requests, "abcdef12").as_deref(),
            Ok("session-a")
        );
    }
}
