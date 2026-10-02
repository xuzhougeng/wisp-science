//! Explicit-frame review of completed Run workspaces; no hidden-window routing.
use crate::native_settings::Broker;
use serde_json::Value;
use tauri::Manager;
use wisp_dto::{
    native_conversations::{RunReviewOperation as Operation, RunReviewReply, RunReviewRequest},
    native_settings::Request,
};

pub(crate) async fn dispatch(
    broker: &Broker,
    request: &Request,
    project_id: &str,
    session: &str,
) -> Result<Value, String> {
    let args: RunReviewRequest =
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?;
    args.operation.validate()?;
    let state = broker.app.state::<crate::AppState>();
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, session).await?;
    if project.id != project_id || args.session_id != session {
        return Err("Project/session scope mismatch".into());
    }
    let mutation = args.operation.is_mutation();
    crate::native_panels::require_run_scope(&state.store, &scope, &args.run_id, mutation).await?;
    let writable = crate::exploration_commands::require_writable_scope(&state.store, &scope)
        .await
        .is_ok()
        && state
            .store
            .require_unarchived_session(session)
            .await
            .is_ok()
        && state
            .store
            .run_state_scope(&args.run_id)
            .await
            .map_err(|e| e.to_string())?
            .as_ref()
            == Some(&scope);
    if mutation && !writable {
        return Err("Run results are read-only in this conversation".into());
    }
    let run = state
        .store
        .get_run(&args.run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Run not found")?;
    if run.kind != "ssh_direct" || !run.status.is_terminal() {
        return Err("Results review requires a finished SSH-direct Run".into());
    }
    let mut reply = RunReviewReply {
        run_id: args.run_id.clone(),
        read_only: !writable,
        cleaned: run.cleaned_at.is_some(),
        listing: None,
        downloaded: None,
        acknowledged: true,
    };
    let _activity = if mutation {
        Some(state.begin_project_activity(project_id)?)
    } else {
        None
    };
    match args.operation {
        Operation::List {
            path,
            name_filter,
            offset,
        } => {
            reply.listing = Some(if reply.cleaned {
                wisp_dto::WorkspaceListing {
                    entries: vec![],
                    truncated: false,
                }
            } else {
                let value = state
                    .run_manager
                    .list_run_workspace_files(
                        &state.store,
                        &args.run_id,
                        &path,
                        &name_filter,
                        offset,
                        200,
                    )
                    .await?;
                serde_json::from_value(serde_json::to_value(value).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?
            });
        }
        Operation::Download { files, dirs } => {
            reply.downloaded = Some(
                state
                    .run_manager
                    .download_run_files(&state.store, &args.run_id, &files, &dirs)
                    .await?
                    .len(),
            );
        }
        Operation::Delete { paths, .. } => {
            state
                .run_manager
                .delete_run_files(&state.store, &args.run_id, &paths)
                .await?;
        }
        Operation::Cleanup { .. } => {
            let updated = state
                .run_manager
                .cleanup_run_workspace(&state.store, &args.run_id, true)
                .await?;
            reply.cleaned = updated.cleaned_at.is_some();
            if !reply.cleaned {
                return Err("Cleanup did not confirm completion".into());
            }
        }
    }
    serde_json::to_value(reply).map_err(|e| e.to_string())
}
