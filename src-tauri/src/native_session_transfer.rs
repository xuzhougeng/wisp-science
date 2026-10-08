//! Native transfer previews reuse the shared file plan and transfer lock boundary.
use crate::native_settings::Broker;
use serde_json::{json, Value};
use tauri::Manager;
use wisp_dto::native_session_transfer as dto;

pub(crate) async fn source(
    store: &wisp_store::Store,
    project: &str,
    session: &str,
) -> Result<(String, usize, String), String> {
    crate::native_conversations::require_owner(store, project, session).await?;
    let reference = store
        .get_session_reference(session)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Saved conversation was not found")?;
    let messages = store
        .load_messages_all_epochs(session)
        .await
        .map_err(|e| e.to_string())?;
    let events = store
        .load_session_ui_events(session)
        .await
        .map_err(|e| e.to_string())?;
    let head = store
        .frame_head_epoch(session)
        .await
        .map_err(|e| e.to_string())?;
    let epochs: Vec<_> = store.context_epochs(session).await.map_err(|e| e.to_string())?.into_iter().map(|epoch| json!({
        "epoch":epoch.epoch,"parent_epoch":epoch.parent_epoch,"strategy":epoch.strategy,"kind":epoch.kind,
        "before_tokens":epoch.before_tokens,"after_tokens":epoch.after_tokens,"first_seq":epoch.first_seq,
        "initial_head_seq":epoch.initial_head_seq,"checkpoint_seq":epoch.checkpoint_seq,
        "first_kept_seq":epoch.first_kept_seq,"archive_ref":epoch.archive_ref,
        "ui_event_seq":epoch.ui_event_seq,"created_at":epoch.created_at,
    })).collect();
    let archive = store
        .research_archive(session)
        .await
        .map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(&(
        project,
        session,
        &reference.title,
        &messages,
        events,
        head,
        epochs,
        archive,
    ))
    .map_err(|e| e.to_string())?;
    Ok((
        reference.title,
        messages.len(),
        wisp_sync::sha256_hex(&bytes),
    ))
}

pub(crate) fn require_no_queue(runtime: &crate::SessionRuntime) -> Result<(), String> {
    if runtime.has_queued_turns() {
        return Err("Remove queued messages before transferring this conversation".into());
    }
    Ok(())
}

pub(crate) async fn require_revision(
    store: &wisp_store::Store,
    project: &str,
    session: &str,
    expected: &str,
) -> Result<(), String> {
    if source(store, project, session).await?.2 != expected {
        return Err("Conversation changed after the transfer preview; preview it again".into());
    }
    Ok(())
}

pub(crate) async fn preview(
    broker: &Broker,
    project: &str,
    args: dto::PreviewRequest,
    native_running: bool,
) -> Result<Value, String> {
    args.validate(project)?;
    let state = broker.app.state::<crate::AppState>();
    let _source_activity = state.begin_project_activity(project)?;
    let _target_activity = state.begin_project_activity(&args.target_project_id)?;
    if state
        .store
        .get_project(&args.target_project_id)
        .await
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Err("Target project not found".into());
    }
    if matches!(
        state
            .store
            .frame_state_scope(&args.session_id)
            .await
            .map_err(|e| e.to_string())?,
        Some(wisp_store::StateScope::Exploration { .. })
    ) {
        return Err("Exploration conversations cannot be transferred to another project".into());
    }
    if args.mode == dto::Mode::Move {
        crate::native_conversations::require_mutable_session(&state.store, &args.session_id)
            .await?;
    }
    let runtime = state.session_runtime(&args.session_id).await;
    let _workflow = runtime
        .workflow
        .clone()
        .try_lock_owned()
        .map_err(|_| "Wait for this conversation to finish before transferring it")?;
    require_no_queue(&runtime)?;
    if native_running
        || state.running_turns.lock().await.contains(&args.session_id)
        || state
            .awaiting_confirm
            .lock()
            .unwrap()
            .contains(&args.session_id)
        || state.reviewing.lock().unwrap().contains(&args.session_id)
    {
        return Err("Wait for the conversation to finish its turn, approval, or review".into());
    }
    crate::session_commands::flush_session_events(&runtime.ui_event_writer).await?;
    let (title, message_count, revision) = source(&state.store, project, &args.session_id).await?;
    let (artifacts, artifact_error) = if args.mode == dto::Mode::Move {
        match state
            .store
            .preview_session_artifacts(&args.session_id, project, Some(&args.target_project_id))
            .await
        {
            Ok(preview) => (Some(preview), None),
            Err(error) => (None, Some(error.to_string())),
        }
    } else {
        (None, None)
    };
    serde_json::to_value(dto::Preview {
        schema: dto::SCHEMA.into(),
        project_id: project.into(),
        session_id: args.session_id,
        target_project_id: args.target_project_id,
        mode: args.mode,
        title,
        message_count,
        revision,
        artifacts,
        artifact_error,
    })
    .map_err(|e| e.to_string())
}

pub(crate) async fn transfer(
    broker: &Broker,
    project: &str,
    args: dto::TransferRequest,
    native_running: bool,
) -> Result<Value, String> {
    args.validate(project)?;
    if native_running {
        return Err("Wait for this conversation to finish before transferring it".into());
    }
    if args.mode == dto::Mode::Move {
        crate::native_conversations::require_mutable_session(
            &broker.app.state::<crate::AppState>().store,
            &args.session_id,
        )
        .await?;
    }
    let result = crate::native_conversations::call(broker, project, "transfer_session_to_project", json!({
        "id":args.session_id,"targetProjectId":args.target_project_id,"mode":args.mode.as_str(),
        "includeArtifacts":args.include_artifacts,"artifactFingerprint":args.artifact_fingerprint,
        "expectedRevision":args.revision,
    })).await?;
    let frame_id = result
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Invalid transfer result")?;
    crate::native_conversations::require_owner(
        &broker.app.state::<crate::AppState>().store,
        &args.target_project_id,
        frame_id,
    )
    .await?;
    serde_json::to_value(dto::TransferResult {
        schema: dto::SCHEMA.into(),
        project_id: project.into(),
        session_id: args.session_id,
        target_project_id: args.target_project_id,
        mode: args.mode,
        include_artifacts: args.include_artifacts,
        frame_id: frame_id.into(),
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_transfer_revision_binds_owner_title_all_epochs_and_saved_events() {
        let store = wisp_store::Store::open(std::path::Path::new(":memory:"))
            .await
            .unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store
            .create_frame("s", "p", "OPERON", "test-model")
            .await
            .unwrap();
        store
            .append_message("s", 0, &wisp_llm::Message::user("Original"))
            .await
            .unwrap();
        let before = source(&store, "p", "s").await.unwrap();
        assert_eq!(before.1, 1);
        require_revision(&store, "p", "s", &before.2).await.unwrap();
        assert!(source(&store, "other", "s").await.is_err());
        store
            .append_message("s", 1, &wisp_llm::Message::assistant("Answer"))
            .await
            .unwrap();
        let after = source(&store, "p", "s").await.unwrap();
        assert_ne!(before.2, after.2);
        assert!(require_revision(&store, "p", "s", &before.2).await.is_err());
        store.rename_session("s", "p", "New title").await.unwrap();
        let renamed = source(&store, "p", "s").await.unwrap();
        assert_ne!(after.2, renamed.2);
        store
            .append_session_ui_event("s", 0, r#"{"kind":"Status","text":"Saved"}"#)
            .await
            .unwrap();
        let event = source(&store, "p", "s").await.unwrap();
        assert_ne!(renamed.2, event.2);
        store
            .open_context_epoch(
                "s",
                wisp_store::OpenContextEpoch {
                    messages: &[wisp_llm::Message::user(
                        "[context summary checkpoint] Summary",
                    )],
                    strategy: "manual",
                    kind: "semantic",
                    before_tokens: 20,
                    after_tokens: 4,
                    checkpoint_index: Some(0),
                    first_kept_seq: None,
                    archive_ref: None,
                    ui_event_seq: None,
                },
            )
            .await
            .unwrap();
        let compacted = source(&store, "p", "s").await.unwrap();
        assert_eq!(compacted.1, 3, "The preview counts every persisted epoch");
        assert_ne!(event.2, compacted.2);
    }
    #[test]
    fn native_transfer_refuses_unconsumed_queue_entries() {
        let runtime = crate::SessionRuntime::new();
        require_no_queue(&runtime).unwrap();
        runtime.queued_cutins.lock().unwrap().push((
            1,
            crate::QueuedItem {
                id: 3,
                message: "Next".into(),
                attachments: vec![],
                references: vec![],
                unattended: false,
            },
        ));
        assert!(require_no_queue(&runtime).is_err());
    }
}
