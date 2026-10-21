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
    if run.frame_id.is_none() {
        return Err("Only a Run started from a conversation can move to the background".into());
    }
    let requested = state
        .store
        .request_run_background(&run_id)
        .await
        .map_err(|error| error.to_string())?;
    if !requested
        && !state
            .store
            .run_background_requested(&run_id)
            .await
            .map_err(|error| error.to_string())?
    {
        return Err("The Run has already ended".into());
    }
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
    let run = loop {
        match state.store.get_run(run_id).await {
            Ok(Some(run)) if run.status.is_terminal() => break run,
            Ok(Some(_)) => {}
            // Deleted with its project or conversation: nobody to tell.
            Ok(None) => return,
            // A failed read says nothing about the Run; keep watching.
            Err(error) => {
                tracing::debug!(target: "wisp", %run_id, %error, "background Run status read failed")
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    };
    match state.store.claim_run_background_notice(run_id).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            // Left unclaimed: the next startup scan retries it.
            tracing::warn!(target: "wisp", %run_id, %error, "failed to claim a background Run notice");
            return;
        }
    }
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
    match crate::dispatch::post_reply(app, &frame_id, &completion_notice(&locale, &run)).await {
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

fn completion_notice(locale: &str, run: &wisp_store::RunRecord) -> String {
    use wisp_store::RunStatus;
    let zh = locale == "zh";
    let outcome = match (run.status, zh) {
        (RunStatus::Succeeded, true) => "已成功完成",
        (RunStatus::Succeeded, false) => "succeeded",
        (RunStatus::Failed, true) => "失败",
        (RunStatus::Failed, false) => "failed",
        (RunStatus::Cancelled, true) => "已取消",
        (RunStatus::Cancelled, false) => "was cancelled",
        (RunStatus::TimedOut, true) => "超时",
        (RunStatus::TimedOut, false) => "timed out",
        (_, true) => "失去连接",
        (_, false) => "lost its connection",
    };
    let mut facts = Vec::new();
    if let Some(code) = run.exit_code {
        facts.push(if zh {
            format!("退出码 {code}")
        } else {
            format!("exit code {code}")
        });
    }
    if let (Some(started), Some(ended)) = (run.started_at, run.ended_at) {
        let elapsed = elapsed_label(ended.saturating_sub(started), zh);
        facts.push(if zh {
            format!("用时 {elapsed}")
        } else {
            format!("took {elapsed}")
        });
    }
    let facts = if facts.is_empty() {
        String::new()
    } else if zh {
        format!("（{}）", facts.join("，"))
    } else {
        format!(" ({})", facts.join(", "))
    };
    if zh {
        format!(
            "后台任务《{}》{outcome}{facts}。\n\nRun `{}` · {}",
            run.title, run.id, run.context_id
        )
    } else {
        format!(
            "Background Run “{}” {outcome}{facts}.\n\nRun `{}` · {}",
            run.title, run.id, run.context_id
        )
    }
}

fn elapsed_label(seconds: i64, zh: bool) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    match (hours, minutes, zh) {
        (0, 0, true) => format!("{seconds} 秒"),
        (0, 0, false) => format!("{seconds}s"),
        (0, _, true) => format!("{minutes} 分 {seconds} 秒"),
        (0, _, false) => format!("{minutes}m {seconds}s"),
        (_, _, true) => format!("{hours} 小时 {minutes} 分"),
        (_, _, false) => format!("{hours}h {minutes}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(status: wisp_store::RunStatus) -> wisp_store::RunRecord {
        let mut run =
            wisp_store::RunRecord::new("run-1", "p", "ssh:gpu", "Align reads", "ssh_direct");
        run.status = status;
        run.started_at = Some(100);
        run.ended_at = Some(100 + 3 * 3600 + 25 * 60 + 7);
        run.exit_code = Some(0);
        run
    }

    #[test]
    fn notice_names_the_run_its_outcome_and_how_to_find_it() {
        let succeeded = run(wisp_store::RunStatus::Succeeded);
        assert_eq!(
            completion_notice("zh", &succeeded),
            "后台任务《Align reads》已成功完成（退出码 0，用时 3 小时 25 分）。\n\nRun `run-1` · ssh:gpu"
        );
        assert_eq!(
            completion_notice("en", &succeeded),
            "Background Run “Align reads” succeeded (exit code 0, took 3h 25m).\n\nRun `run-1` · ssh:gpu"
        );
        let mut failed = run(wisp_store::RunStatus::Failed);
        failed.exit_code = Some(137);
        failed.ended_at = Some(100 + 65);
        assert!(completion_notice("zh", &failed).contains("失败（退出码 137，用时 1 分 5 秒）"));
        let mut lost = run(wisp_store::RunStatus::Lost);
        lost.exit_code = None;
        lost.started_at = None;
        assert_eq!(
            completion_notice("en", &lost),
            "Background Run “Align reads” lost its connection.\n\nRun `run-1` · ssh:gpu"
        );
    }

    #[test]
    fn elapsed_label_keeps_the_two_largest_units() {
        assert_eq!(elapsed_label(42, false), "42s");
        assert_eq!(elapsed_label(61, true), "1 分 1 秒");
        assert_eq!(elapsed_label(7200 + 59, false), "2h 0m");
    }
}
