//! Run cards the user moved to the background. `background_run` records the
//! request on the Run, which ends any `monitor_run` / `wait_for_completion`
//! wait on it so the conversation can go on. A watcher then posts one
//! completion notice to the conversation that owns the Run once it ends, and
//! tells its windows so they can raise a desktop notification.
//!
//! The request is persisted, so a Run that ends while Wisp is closed is still
//! reported: startup resumes watching every Run whose notice is due. Scheduler
//! jobs (`scheduler_job.sh --follow`) are ordinary Runs that live as long as
//! the job, so the same notice covers them.

use crate::{AgentEvent, AppState};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

/// Status reads only; the Run manager's own pollers advance the Run.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Runs with a live watcher in this process, so a repeated request or the
/// startup scan never posts twice from two loops.
fn watching() -> &'static Mutex<HashSet<String>> {
    static WATCHING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    WATCHING.get_or_init(Default::default)
}

#[tauri::command]
pub(crate) async fn background_run(
    app: AppHandle,
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    run_id: String,
) -> Result<(), String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    let run = state
        .store
        .get_run(&run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Run not found".to_string())?;
    if run.project_id != project.id {
        return Err("Run does not belong to the active project".into());
    }
    if state
        .store
        .run_state_scope(&run_id)
        .await
        .map_err(|error| error.to_string())?
        .as_ref()
        != Some(&scope)
    {
        return Err("Run is not visible in the active state scope".into());
    }
    wisp_runs::background::request(&state.store, &run).await?;
    watch(&app, run_id);
    Ok(())
}

/// Startup: resume watching Runs moved to the background in an earlier
/// session, including ones that ended while Wisp was closed.
pub(crate) fn resume_pending(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let pending = app
            .state::<AppState>()
            .store
            .list_pending_background_runs()
            .await;
        match pending {
            Ok(run_ids) => {
                for run_id in run_ids {
                    watch(&app, run_id);
                }
            }
            Err(error) => {
                tracing::warn!(target: "wisp", %error, "failed to list background Runs");
            }
        }
    });
}

fn watch(app: &AppHandle, run_id: String) {
    if !watching()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(run_id.clone())
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        notify_when_ended(&app, &run_id).await;
        watching()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&run_id);
    });
}

async fn notify_when_ended(app: &AppHandle, run_id: &str) {
    let state = app.state::<AppState>();
    let run = match wisp_runs::background::claim_when_ended(&state.store, run_id, POLL_INTERVAL)
        .await
    {
        Ok(Some(run)) => run,
        // Deleted with its project or conversation, or already reported.
        Ok(None) => return,
        Err(error) => {
            // Left unclaimed: the next startup scan retries it.
            tracing::warn!(target: "wisp", %run_id, %error, "failed to claim a background Run notice");
            return;
        }
    };
    let Some(frame_id) = run.frame_id.clone() else {
        return;
    };
    crate::emit_agent_event_in(
        app,
        AgentEvent::RunBackgroundFinished {
            frame_id: frame_id.clone(),
            run_id: run.id.clone(),
            title: run.title.clone(),
            status: run.status.as_str().into(),
        },
        Some(&run.project_id),
    );
    let locale = crate::load_locale(&state.store).await;
    // Waits for a turn running in that conversation to finish first.
    match crate::dispatch::post_reply(
        app,
        &frame_id,
        &wisp_runs::background::completion_notice(locale == "zh", &run),
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => tracing::info!(
            target: "wisp",
            %run_id,
            "background Run notice dropped: the conversation is gone"
        ),
        Err(error) => {
            tracing::warn!(target: "wisp", %run_id, %error, "background Run notice failed")
        }
    }
}
