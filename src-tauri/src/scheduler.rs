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
use std::sync::{atomic::Ordering, Arc};
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
            crate::research_recap::daily_recap_tick(&app).await;
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
    if schedule.replace_previous_turn {
        if let Err(error) = fire_session_timer(&app, &state, schedule, advance).await {
            tracing::warn!(%error, "conversation timer failed");
        }
        return;
    }
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
    window: crate::workspace_surface::WorkspaceSurface,
    name: String,
    prompt: String,
    interval_secs: i64,
    session_id: Option<String>,
    skill: Option<String>,
    start_at: Option<i64>,
    project_id: Option<String>,
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
    // The home Automation page names a project; a project window uses its own.
    let project_id = match project_id.map(|id| id.trim().to_string()) {
        Some(id) if !id.is_empty() => {
            state
                .store
                .get_project(&id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "The project no longer exists.".to_string())?;
            id
        }
        _ => state.require_active(window.label())?.id,
    };
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
        replace_previous_turn: false,
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

struct TimerRunning(Arc<crate::SessionRuntime>);
impl Drop for TimerRunning {
    fn drop(&mut self) {
        self.0.timer_running.store(false, Ordering::SeqCst);
    }
}

/// Busy sessions remain due instead of accumulating queued hourly messages.
async fn fire_session_timer(
    app: &AppHandle,
    state: &AppState,
    scheduled: ScheduleRecord,
    advance: bool,
) -> Result<(), String> {
    let Some(frame_id) = scheduled.frame_id.clone() else {
        return Err("The timer has no target conversation.".into());
    };
    let rt = state.session_runtime(&frame_id).await;
    let Some(workflow) = timer_workflow(&rt) else {
        return if advance {
            Ok(())
        } else {
            Err("This conversation is busy. Try again when its current turn finishes.".into())
        };
    };
    // Re-read after acquiring the workflow: pause/delete/edit can race a tick.
    let schedule = load_schedule(state, &scheduled.id).await?;
    if advance && (!schedule.enabled || schedule.next_run_at != scheduled.next_run_at) {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    if advance
        && !state
            .store
            .claim_schedule_fire(
                &schedule.id,
                schedule.next_run_at,
                next_slot_after(schedule.next_run_at, schedule.interval_secs, now),
                now,
            )
            .await
            .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    rt.timer_running.store(true, Ordering::SeqCst);
    let _running = TimerRunning(rt.clone());
    let result: Result<(), String> = async {
        state
            .store
            .require_unarchived_session(&frame_id)
            .await
            .map_err(|e| e.to_string())?;
        ensure_native_timer(state, &frame_id).await?;
        if let Some(removal) = state
            .store
            .clear_session_timer_turn(&frame_id)
            .await
            .map_err(|e| e.to_string())?
        {
            rt.invalidate_cached_agent();
            *rt.interrupted_turn_start.lock().unwrap() = None;
            rt.sync_last_seq_from_store(&state.store, &frame_id).await?;
            crate::emit_to_session_surfaces(
                app,
                &frame_id,
                Some(&schedule.project_id),
                "session-timer-replaced",
                &removal,
            );
        }
        state
            .store
            .begin_session_timer_turn(&frame_id)
            .await
            .map_err(|e| e.to_string())?;
        let result = send_message_inner(
            state,
            app.clone(),
            "main",
            Some(frame_id.clone()),
            format!("[Timer: {}]\n\n{}", schedule.name, schedule.prompt),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&workflow),
            crate::TurnOrigin::Timer,
        )
        .await;
        // Even errors before the first provider request must close the range.
        state
            .store
            .finish_session_timer_turn(&frame_id)
            .await
            .map_err(|e| e.to_string())?;
        result.map(|_| ())
    }
    .await;
    let run = ScheduleRunRecord {
        id: Uuid::new_v4().to_string(),
        schedule_id: schedule.id.clone(),
        frame_id: Some(frame_id.clone()),
        status: if result.is_ok() { "fired" } else { "failed" }.into(),
        error: result.as_ref().err().cloned(),
        fired_at: now,
    };
    if let Err(error) = state.store.record_schedule_run(&run).await {
        tracing::warn!(%error, "failed to record timer run");
    }
    crate::emit_to_session_surfaces(
        app,
        &frame_id,
        Some(&schedule.project_id),
        "session-timer-updated",
        &frame_id,
    );
    result
}

fn timer_workflow(rt: &Arc<crate::SessionRuntime>) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    let guard = rt.workflow.clone().try_lock_owned().ok()?;
    if rt.draining.load(Ordering::SeqCst)
        || rt.replacing.load(Ordering::SeqCst) > 0
        || rt.deleted.load(Ordering::SeqCst)
        || !rt.queued.lock().unwrap().is_empty()
    {
        return None;
    }
    Some(guard)
}

async fn ensure_native_timer(state: &AppState, frame_id: &str) -> Result<(), String> {
    crate::subagent_tool::require_instruction_source(
        &state.store,
        frame_id,
        crate::TurnOrigin::Timer,
        false,
    )
    .await?;
    if crate::acp::session_agent_id(&state.store, frame_id)
        .await?
        .is_some()
    {
        return Err(
            "Timers require a native Wisp conversation; ACP agents own their remote history."
                .into(),
        );
    }
    Ok(())
}

/// One timer per conversation. Repeating /timer edits its cadence and prompt.
#[tauri::command]
pub(crate) async fn set_session_timer(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: String,
    expression: String,
) -> Result<ScheduleRecord, String> {
    let (interval_secs, prompt) = wisp_dto::parse_timer_expression(&expression)?;
    let project = state.require_active(window.label())?;
    state
        .store
        .require_unarchived_session(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    if state
        .store
        .frame_project_id(&session_id)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some(&project.id)
    {
        return Err("The target conversation belongs to a different project.".into());
    }
    ensure_native_timer(&state, &session_id).await?;
    let now = chrono::Utc::now().timestamp();
    let existing = state
        .store
        .list_schedules(&project.id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.replace_previous_turn && s.frame_id.as_deref() == Some(&session_id));
    if let Some(schedule) = existing {
        let updated = state
            .store
            .update_session_timer(&schedule.id, &prompt, interval_secs, now)
            .await
            .map_err(|e| e.to_string())?;
        if !updated {
            return Err("The timer was cancelled while it was being edited.".into());
        }
        return load_schedule(&state, &schedule.id).await;
    }
    let schedule = ScheduleRecord {
        id: Uuid::new_v4().to_string(),
        project_id: project.id,
        frame_id: Some(session_id),
        replace_previous_turn: true,
        name: prompt.chars().take(MAX_NAME_CHARS).collect(),
        prompt,
        skill: None,
        interval_secs,
        enabled: true,
        next_run_at: now + interval_secs,
        last_run_at: None,
        created_at: now,
        updated_at: now,
    };
    state
        .store
        .create_schedule(&schedule)
        .await
        .map_err(|e| e.to_string())?;
    Ok(schedule)
}

#[tauri::command]
pub(crate) async fn list_schedules(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
) -> Result<Vec<ScheduleRecord>, String> {
    let project_id = state.require_active(window.label())?.id;
    state
        .store
        .list_schedules(&project_id)
        .await
        .map_err(|error| error.to_string())
}

/// Every project's schedules, for the home Automation page. A project whose
/// storage is unavailable is skipped rather than hiding all the others.
#[tauri::command]
pub(crate) async fn list_all_schedules(
    state: State<'_, AppState>,
) -> Result<Vec<ScheduleRecord>, String> {
    let projects = state
        .store
        .list_projects()
        .await
        .map_err(|error| error.to_string())?;
    let mut all = Vec::new();
    for project in projects {
        match state.store.list_schedules(&project.0).await {
            Ok(schedules) => all.extend(schedules),
            Err(error) => {
                tracing::warn!(target: "wisp", %error, project_id = %project.0, "failed to list schedules")
            }
        }
    }
    all.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at).then(a.id.cmp(&b.id)));
    Ok(all)
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
    if schedule.replace_previous_turn {
        return fire_session_timer(&app, &state, schedule, false).await;
    }
    let now = chrono::Utc::now().timestamp();
    tauri::async_runtime::spawn(async move {
        fire_schedule(app, schedule, now, false).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timer_waits_for_normal_workflows_and_never_stacks() {
        let rt = Arc::new(crate::SessionRuntime::new());
        let normal = rt.workflow.clone().lock_owned().await;
        assert!(timer_workflow(&rt).is_none());
        drop(normal);
        let timer = timer_workflow(&rt).unwrap();
        assert!(timer_workflow(&rt).is_none());
        drop(timer);
        rt.draining.store(true, Ordering::SeqCst);
        assert!(timer_workflow(&rt).is_none());
        rt.draining.store(false, Ordering::SeqCst);
        assert!(timer_workflow(&rt).is_some());
    }

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
