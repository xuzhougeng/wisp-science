use super::{save_approval_grants, AppState, ApprovalGrantKey};
use serde::Serialize;
use tauri::State;

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
        if approved {
            let scope = scope.unwrap_or_else(|| "once".into());
            if matches!(scope.as_str(), "session" | "project" | "global") {
                if let Some(grant) = pending.grant.clone() {
                    let snapshot = {
                        let mut grants = state.approval_grants.lock().unwrap();
                        grants.grant(&scope, &session_id, &pending.project_id, grant);
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
