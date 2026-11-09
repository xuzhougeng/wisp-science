//! Explicitly scoped exploration review/actions for the native workbench.
use crate::native_settings::{invoke_command, Broker};
use serde_json::{json, Value};
use tauri::Manager;
use wisp_dto::native_conversations as dto;

pub(crate) async fn dispatch(
    broker: &Broker,
    project: &str,
    command: &str,
    value: &Value,
) -> Result<Value, String> {
    if command == "native_conversation_exploration_abandon" {
        let args: dto::SessionRequest =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        let state = broker.app.state::<crate::AppState>();
        crate::native_conversations::require_owner(&state.store, project, &args.session_id).await?;
        if !crate::native_conversations::native_explorations(&state.store, project)
            .await?
            .iter()
            .any(|row| row.source_frame_id == args.session_id)
        {
            return Err("Exploration round is no longer active; refresh before continuing".into());
        }
        invoke_command(
            broker,
            Some(project.into()),
            "abandon_exploration_round",
            json!({"sourceFrameId":args.session_id}),
        )
        .await?;
        return serde_json::to_value(dto::ExplorationActionResult {
            project_id: project.into(),
            session_id: args.session_id.clone(),
            source_session_id: args.session_id,
            exploration_id: String::new(),
            action: "abandon".into(),
            completed: true,
        })
        .map_err(|e| e.to_string());
    }
    if command == "native_conversation_exploration_start" {
        let args: dto::ExplorationStartRequest =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        if args.name.trim().is_empty() {
            return Err("An exploration name is required".into());
        }
        let state = broker.app.state::<crate::AppState>();
        crate::native_conversations::require_owner(&state.store, project, &args.session_id).await?;
        state
            .store
            .require_unarchived_session(&args.session_id)
            .await
            .map_err(|e| e.to_string())?;
        // Hold the source workflow across identity validation and checkpointing;
        // a send/rewind cannot replace the selected turn at the same index.
        let runtime = state.session_runtime(&args.session_id).await;
        let _workflow = runtime
            .workflow
            .clone()
            .try_lock_owned()
            .map_err(|_| "Wait for the source conversation to finish")?;
        let outline = state
            .store
            .load_session_user_messages(&args.session_id)
            .await
            .map_err(|e| e.to_string())?;
        if crate::native_history::identities(&outline).get(args.target.user_index)
            != Some(&args.target)
        {
            return Err("This historical turn changed; reload it before continuing".into());
        }
        let index =
            i64::try_from(args.target.user_index).map_err(|_| "Invalid exploration turn")?;
        let exploration = crate::exploration_commands::start_from_source(
            &state,
            &broker.app.state(),
            project,
            &args.session_id,
            Some(index),
            &args.name,
        )
        .await?;
        let summary = crate::native_conversations::native_explorations(&state.store, project)
            .await?
            .into_iter()
            .find(|row| {
                row.exploration.id == exploration.id
                    && row.source_frame_id == args.session_id
                    && row.checkpoint_user_index == args.target.user_index
            })
            .ok_or("Created exploration could not be confirmed; inspect the exploration list")?;
        return serde_json::to_value(dto::ExplorationStarted {
            project_id: project.into(),
            source_session_id: args.session_id,
            target: args.target,
            summary,
        })
        .map_err(|e| e.to_string());
    }
    let (session, id, guard) = if command == "native_conversation_exploration_promote" {
        let args: dto::ExplorationPromoteRequest =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        if args.expected_guard_hash.trim().is_empty() {
            return Err("A promotion preview guard is required".into());
        }
        (
            args.session_id,
            args.exploration_id,
            Some(args.expected_guard_hash),
        )
    } else {
        let args: dto::ExplorationOpenRequest =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        (args.session_id, args.exploration_id, None)
    };
    let state = broker.app.state::<crate::AppState>();
    crate::native_conversations::require_owner(&state.store, project, &session).await?;
    let summary = crate::native_conversations::native_explorations(&state.store, project)
        .await?
        .into_iter()
        .find(|row| row.exploration.id == id && row.exploration.frame_id == session)
        .ok_or("Exploration no longer belongs to the current round")?;
    crate::native_conversations::require_owner(&state.store, project, &summary.source_frame_id)
        .await?;
    let action = command
        .strip_prefix("native_conversation_exploration_")
        .ok_or("Invalid exploration command")?;
    if action == "preview" {
        let value = invoke_command(
            broker,
            Some(project.into()),
            "preview_exploration_promotion",
            json!({"explorationId":id}),
        )
        .await?;
        let preview: wisp_dto::ExplorationPromotionPreview =
            serde_json::from_value(value).map_err(|e| e.to_string())?;
        if preview.exploration.id != id
            || preview.exploration.frame_id != session
            || preview.diff.exploration_id != id
        {
            return Err("Exploration preview scope mismatch".into());
        }
        return serde_json::to_value(dto::ExplorationPreview {
            project_id: project.into(),
            session_id: session,
            source_session_id: summary.source_frame_id,
            preview,
        })
        .map_err(|e| e.to_string());
    }
    match action {
        "promote" => {
            let value = invoke_command(
                broker,
                Some(project.into()),
                "promote_exploration",
                json!({"explorationId":id,"expectedGuardHash":guard}),
            )
            .await?;
            let result: wisp_dto::ExplorationPromotionResult =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            if result.mainline_frame_id != summary.source_frame_id {
                return Err("Promotion result scope mismatch".into());
            }
        }
        "discard" => {
            invoke_command(
                broker,
                Some(project.into()),
                "discard_exploration",
                json!({"explorationId":id}),
            )
            .await?;
        }
        "manual_open" => {
            invoke_command(
                broker,
                Some(project.into()),
                "open_exploration_manual_resolution",
                json!({"explorationId":id}),
            )
            .await?;
        }
        "manual_finish" => {
            let value = invoke_command(
                broker,
                Some(project.into()),
                "preview_exploration_promotion",
                json!({"explorationId":id}),
            )
            .await?;
            let preview: wisp_dto::ExplorationPromotionPreview =
                serde_json::from_value(value).map_err(|e| e.to_string())?;
            if !preview.eligibility.manual_resolution_available {
                return Err("Manual resolution is no longer available".into());
            }
            // Same WebView completion: keep the files the user resolved in main,
            // then abandon the whole exploration round through its existing guards.
            invoke_command(
                broker,
                Some(project.into()),
                "abandon_exploration_round",
                json!({"sourceFrameId":summary.source_frame_id}),
            )
            .await?;
        }
        _ => return Err("Unsupported exploration action".into()),
    }
    serde_json::to_value(dto::ExplorationActionResult {
        project_id: project.into(),
        session_id: session,
        source_session_id: summary.source_frame_id,
        exploration_id: id,
        action: action.into(),
        completed: true,
    })
    .map_err(|e| e.to_string())
}
