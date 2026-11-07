//! Native history actions retain explicit ownership and reuse desktop commands.
use crate::native_settings::{invoke_command, Broker};
use serde_json::{json, Value};
use tauri::Manager;
use wisp_dto::native_history::{HistoryAction, HistoryRequest, HistoryState, TurnIdentity};

fn rewind_guard(
    runtime: &crate::SessionRuntime,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    let guard = runtime
        .workflow
        .clone()
        .try_lock_owned()
        .map_err(|_| "Wait for this conversation to finish before rewinding")?;
    if runtime.has_queued_turns() {
        return Err("Remove queued messages before rewinding this conversation".into());
    }
    Ok(guard)
}

pub(crate) fn identities(outline: &[(i64, String, i64, Option<i64>)]) -> Vec<TurnIdentity> {
    outline
        .iter()
        .enumerate()
        .map(|(user_index, (seq, text, _, _))| TurnIdentity {
            user_index,
            user_seq: *seq,
            digest: wisp_sync::sha256_hex(text.as_bytes()),
        })
        .collect()
}

pub(crate) async fn snapshot(
    state: &crate::AppState,
    project: &str,
    session: &str,
    outline: &[(i64, String, i64, Option<i64>)],
) -> Result<HistoryState, String> {
    let branch = state
        .store
        .session_branch_state(session)
        .await
        .map_err(|e| e.to_string())?;
    let scope = state
        .store
        .frame_state_scope(session)
        .await
        .map_err(|e| e.to_string())?;
    Ok(HistoryState {
        revision: wisp_sync::sha256_hex(&serde_json::to_vec(outline).map_err(|e| e.to_string())?),
        can_branch: !wisp_store::is_assistant_project_id(project)
            && branch.is_none()
            && !matches!(scope, Some(wisp_store::StateScope::Exploration { .. })),
        reviewing: state.reviewing.lock().unwrap().contains(session),
        turns: identities(outline),
    })
}

pub(crate) async fn dispatch(
    broker: &Broker,
    project: &str,
    request: HistoryRequest,
    native_running: bool,
) -> Result<Value, String> {
    let state = broker.app.state::<crate::AppState>();
    let session = &request.session_id;
    crate::native_conversations::require_owner(&state.store, project, session).await?;
    crate::native_conversations::require_mutable_session(&state.store, session).await?;
    // Claim the workflow lock before reading the confirmation revision so a
    // WebView send or queue driver cannot append a new turn between
    // validation and truncation.
    let _rewind_workflow = if matches!(request.action, HistoryAction::Rewind) {
        Some(rewind_guard(&*state.session_runtime(session).await)?)
    } else {
        None
    };
    let outline = state
        .store
        .load_session_user_messages(session)
        .await
        .map_err(|e| e.to_string())?;
    let current = snapshot(&state, project, session, &outline).await?;
    let running = native_running || state.running_turns.lock().await.contains(session);
    let acp = state
        .store
        .get_acp_session(session)
        .await
        .map_err(|e| e.to_string())?
        .is_some();
    request.validate(&current, running, acp)?;
    let index = request.target.user_index;
    let mut args = json!({"sessionId":session, "userIndex":index});
    let command = match &request.action {
        HistoryAction::Branch { checkpoint } => {
            args["checkpointKind"] = serde_json::to_value(checkpoint).map_err(|e| e.to_string())?;
            args["title"] = json!(outline[index].1);
            "branch_session"
        }
        HistoryAction::Rewind => {
            // The WebView command would wait on the workflow lock held above.
            crate::session_commands::rewind_locked(&state, session, index).await?;
            return Ok(json!({"session_id":session,"target":request.target,"result":null}));
        }
        HistoryAction::UndoPreview => "preview_turn_undo",
        HistoryAction::Undo => "undo_turn",
        HistoryAction::Review => "review_session",
        HistoryAction::ProposeMemory => {
            args = json!({"sessionId":session, "turnIndex":index});
            "propose_turn_memory"
        }
        HistoryAction::ConfirmMemory {
            scope,
            content,
            replace_id,
        } => {
            args = json!({"sessionId":session,"turnIndex":index,"scope":scope,"content":content,"replaceId":replace_id});
            "confirm_turn_memory"
        }
    };
    let result = invoke_command(broker, Some(project.into()), command, args).await?;
    Ok(json!({"session_id":session,"target":request.target,"result":result}))
}

pub(crate) async fn excerpt(
    broker: &Broker,
    project: &str,
    request: wisp_dto::native_history::ExcerptRequest,
) -> Result<Value, String> {
    let state = broker.app.state::<crate::AppState>();
    let session = &request.session_id;
    crate::native_conversations::require_owner(&state.store, project, session).await?;
    // Branches and private explorations are valid read sources. Do not apply
    // mutation guards or wait on a later running turn's workflow lock.
    let _activity = state.begin_project_activity(project)?;
    let runtime = state.session_runtime(session).await;
    let workflow = runtime.workflow.clone().try_lock_owned().ok();
    let before = identities(
        &state
            .store
            .load_session_user_messages(session)
            .await
            .map_err(|e| e.to_string())?,
    );
    request.validate(
        &before,
        workflow.is_none() || state.running_turns.lock().await.contains(session),
    )?;
    let excerpt = crate::session_commands::render_session_excerpt(
        broker.app.state(),
        session.clone(),
        request.start.user_index,
        request.end.user_index,
    )
    .await?;
    let after = identities(
        &state
            .store
            .load_session_user_messages(session)
            .await
            .map_err(|e| e.to_string())?,
    );
    request.validate(
        &after,
        workflow.is_none() || state.running_turns.lock().await.contains(session),
    )?;
    let range = request.start.user_index..=request.end.user_index;
    if before[range.clone()] != after[range] {
        return Err("The selected turns changed while quoting; reload before continuing".into());
    }
    crate::native_conversations::require_owner(&state.store, project, session).await?;
    serde_json::to_value(wisp_dto::native_history::ConversationExcerpt {
        project_id: project.into(),
        start: request.start,
        end: request.end,
        excerpt,
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_rewind_cannot_overlap_the_source_workflow() {
        let runtime = crate::SessionRuntime::new();
        let running = runtime.workflow.clone().lock_owned().await;
        assert!(rewind_guard(&runtime).is_err());
        drop(running);
        runtime.queued_cutins.lock().unwrap().push((
            1,
            crate::QueuedItem {
                id: 9,
                message: "unconsumed cut-in".into(),
                attachments: vec![],
                references: vec![],
                unattended: false,
            },
        ));
        assert!(rewind_guard(&runtime).is_err());
        runtime.queued_cutins.lock().unwrap().clear();
        let rewinding = rewind_guard(&runtime).unwrap();
        assert!(runtime.workflow.try_lock().is_err());
        drop(rewinding);
        assert!(runtime.workflow.try_lock().is_ok());
    }
    #[test]
    fn native_history_identity_detects_replaced_rows_at_the_same_index() {
        let before = identities(&[(35, "question".into(), 1, Some(2))]);
        assert_ne!(before, identities(&[(36, "question".into(), 1, Some(2))]));
        assert_ne!(
            before,
            identities(&[(35, "edited question".into(), 1, Some(2))])
        );
        assert_eq!(before, identities(&[(35, "question".into(), 1, Some(3))]));
    }
}
