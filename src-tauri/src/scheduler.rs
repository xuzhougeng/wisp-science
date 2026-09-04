//! Scheduled automations: fire a prompt (optionally bound to a skill) into a
//! chat session on an interval while the app is running.
//!
//! There is no system-level daemon: schedules only fire in-process, and slots
//! missed while the app was closed collapse into a single catch-up fire on
//! the next launch. Each fire is a normal agent turn routed through
//! `send_message_inner` (like channel and delegation-resume turns), so it
//! persists to the session transcript and streams to the UI through the
//! existing turn events.

use crate::{create_session_frame, send_message_inner, AppState, ComposerReferenceArg};
use std::time::Duration;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;
use wisp_store::{next_slot_after, ScheduleRecord, ScheduleRunRecord};

/// Due schedules are picked up within one poll interval of their slot.
const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Let windows/projects restore before the first catch-up scan.
const START_DELAY: Duration = Duration::from_secs(5);
/// Sub-minute intervals would turn a missed-slot catch-up into a busy loop.
const MIN_INTERVAL_SECS: i64 = 60;
const MAX_NAME_CHARS: usize = 80;

pub(crate) fn start_scheduler(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        loop {
            tick.tick().await;
            fire_due_schedules(&app).await;
        }
    });
}

async fn fire_due_schedules(app: &AppHandle) {
    let state = app.state::<AppState>();
    let now = chrono::Utc::now().timestamp();
    let due = match state.store.due_schedules(now).await {
        Ok(due) => due,
        Err(error) => {
            tracing::warn!(target: "wisp", %error, "failed to poll due schedules");
            return;
        }
    };
    for schedule in due {
        // A fired turn can run for minutes; never let one schedule delay the
        // others (or the next poll) behind it.
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            fire_schedule(app, schedule, now, true).await;
        });
    }
}

/// Fire one schedule. With `advance`, the slot is first claimed atomically so
/// a schedule can never double-fire; manual runs skip the claim and leave the
/// cadence untouched.
async fn fire_schedule(app: AppHandle, schedule: ScheduleRecord, now: i64, advance: bool) {
    let state = app.state::<AppState>();
    if advance {
        let new_next = next_slot_after(schedule.next_run_at, schedule.interval_secs, now);
        match state
            .store
            .claim_schedule_fire(&schedule.id, schedule.next_run_at, new_next, now)
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::warn!(target: "wisp", %error, schedule_id = %schedule.id, "failed to claim schedule");
                return;
            }
        }
    }
    let result = run_scheduled_turn(&app, &state, &schedule).await;
    let (status, error) = match &result {
        Ok(_) => ("fired", None),
        Err(error) => ("failed", Some(error.clone())),
    };
    let run = ScheduleRunRecord {
        id: Uuid::new_v4().to_string(),
        schedule_id: schedule.id.clone(),
        frame_id: result.ok().flatten().or_else(|| schedule.frame_id.clone()),
        status: status.into(),
        error,
        fired_at: now,
    };
    if let Err(error) = state.store.record_schedule_run(&run).await {
        tracing::warn!(target: "wisp", %error, schedule_id = %schedule.id, "failed to record schedule run");
    }
}

/// Returns the frame the turn ran in. A schedule bound to a session fires
/// into it; an unbound schedule gets a fresh session per fire.
async fn run_scheduled_turn(
    app: &AppHandle,
    state: &State<'_, AppState>,
    schedule: &ScheduleRecord,
) -> Result<Option<String>, String> {
    let frame_id = match schedule.frame_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => create_session_frame(&state.store, &schedule.project_id).await?,
    };
    let references = schedule
        .skill
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            vec![ComposerReferenceArg::Skill {
                name: name.to_string(),
            }]
        });
    // Provenance for both the user and the model: this turn was not typed.
    let message = format!("[Scheduled task: {}]\n\n{}", schedule.name, schedule.prompt);
    send_message_inner(
        state.inner(),
        app.clone(),
        "main",
        Some(frame_id.clone()),
        message,
        None,
        references,
        None,
        None,
        None,
        None,
        None,
        None,
        crate::TurnOrigin::Desktop,
    )
    .await
    .map(Some)
}

struct ScheduleArgs {
    name: String,
    prompt: String,
    skill: Option<String>,
    frame_id: Option<String>,
    interval_secs: i64,
    next_run_at: i64,
}

fn normalize_schedule_args(
    name: &str,
    prompt: &str,
    interval_secs: i64,
    skill: Option<String>,
    session_id: Option<String>,
    start_at: Option<i64>,
    now: i64,
) -> Result<ScheduleArgs, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Schedule prompt cannot be empty.".into());
    }
    let name = {
        let name = name.trim();
        if name.is_empty() {
            prompt
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(MAX_NAME_CHARS)
                .collect()
        } else {
            name.chars().take(MAX_NAME_CHARS).collect()
        }
    };
    let skill = skill
        .map(|skill| skill.trim().to_string())
        .filter(|skill| !skill.is_empty());
    let frame_id = session_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    // A past start date means "catch up now"; an absent one means the first
    // fire happens one interval from creation.
    let next_run_at = start_at.unwrap_or(now + interval_secs.max(MIN_INTERVAL_SECS));
    Ok(ScheduleArgs {
        name,
        prompt: prompt.to_string(),
        skill,
        frame_id,
        interval_secs: interval_secs.max(MIN_INTERVAL_SECS),
        next_run_at,
    })
}

async fn load_schedule(state: &AppState, id: &str) -> Result<ScheduleRecord, String> {
    state
        .store
        .get_schedule(id.trim())
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The schedule no longer exists.".to_string())
}

#[tauri::command]
pub(crate) async fn create_schedule(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    prompt: String,
    interval_secs: i64,
    session_id: Option<String>,
    skill: Option<String>,
    start_at: Option<i64>,
) -> Result<ScheduleRecord, String> {
    let now = chrono::Utc::now().timestamp();
    let args = normalize_schedule_args(
        &name,
        &prompt,
        interval_secs,
        skill,
        session_id,
        start_at,
        now,
    )?;
    let project_id = state.require_active(window.label())?.id;
    if let Some(frame_id) = args.frame_id.as_deref() {
        let owner = state
            .store
            .frame_project_id(frame_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The target session no longer exists.".to_string())?;
        if owner != project_id {
            return Err("The target session belongs to a different project.".into());
        }
    }
    let schedule = ScheduleRecord {
        id: Uuid::new_v4().to_string(),
        project_id,
        frame_id: args.frame_id,
        name: args.name,
        prompt: args.prompt,
        skill: args.skill,
        interval_secs: args.interval_secs,
        enabled: true,
        next_run_at: args.next_run_at,
        last_run_at: None,
        created_at: now,
        updated_at: now,
    };
    state
        .store
        .create_schedule(&schedule)
        .await
        .map_err(|error| error.to_string())?;
    Ok(schedule)
}

#[tauri::command]
pub(crate) async fn list_schedules(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<ScheduleRecord>, String> {
    let project_id = state.require_active(window.label())?.id;
    state
        .store
        .list_schedules(&project_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn list_schedule_runs(
    state: State<'_, AppState>,
    id: String,
    limit: Option<usize>,
) -> Result<Vec<ScheduleRunRecord>, String> {
    state
        .store
        .list_schedule_runs(id.trim(), limit.unwrap_or(50))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn set_schedule_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    if state
        .store
        .set_schedule_enabled(id.trim(), enabled, now)
        .await
        .map_err(|error| error.to_string())?
    {
        Ok(())
    } else {
        Err("The schedule no longer exists.".into())
    }
}

#[tauri::command]
pub(crate) async fn delete_schedule(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .store
        .delete_schedule(id.trim())
        .await
        .map_err(|error| error.to_string())
}

/// Fire immediately, out of cadence: the slot schedule is left untouched.
#[tauri::command]
pub(crate) async fn run_schedule_now(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> Result<(), String> {
    let schedule = load_schedule(&state, &id).await?;
    let now = chrono::Utc::now().timestamp();
    tauri::async_runtime::spawn(async move {
        fire_schedule(app, schedule, now, false).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_args_validate_and_default() {
        assert!(normalize_schedule_args("x", "  ", 3600, None, None, None, 1_000).is_err());

        let args = normalize_schedule_args(
            "",
            "Summarize new papers.\nDetails follow.",
            86_400,
            Some(" literature-review ".into()),
            Some(" frame-1 ".into()),
            None,
            1_000,
        )
        .unwrap();
        assert_eq!(args.name, "Summarize new papers.");
        assert_eq!(args.skill.as_deref(), Some("literature-review"));
        assert_eq!(args.frame_id.as_deref(), Some("frame-1"));
        // No explicit start: first fire is one interval out.
        assert_eq!(args.next_run_at, 1_000 + 86_400);
    }

    #[test]
    fn schedule_args_clamp_interval_and_keep_past_start_for_catch_up() {
        let args = normalize_schedule_args("daily", "go", 5, None, None, Some(500), 1_000).unwrap();
        assert_eq!(args.interval_secs, MIN_INTERVAL_SECS);
        assert_eq!(args.next_run_at, 500, "past start stays due for catch-up");

        let future =
            normalize_schedule_args("daily", "go", 86_400, None, None, Some(9_999), 1_000).unwrap();
        assert_eq!(future.next_run_at, 9_999);
    }
}
