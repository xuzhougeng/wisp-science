//! Live snapshots survive switching conversations; only start/final snapshots
//! are persisted, so a thousand-item batch does not write a thousand full lists.
use crate::{AgentEvent, AppState};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{Manager, State};
use wisp_dto::{SwarmPhase, SwarmProgress, SwarmTaskStatus, SWARM_PRESENTATION};
use wisp_store::FRAME_READER_RETRY_PREFIX;

#[derive(Default)]
pub(crate) struct SwarmCache(Mutex<HashMap<String, SwarmProgress>>);

impl SwarmCache {
    pub fn update(&self, frame: &str, progress: SwarmProgress) {
        let mut cache = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(current) = cache.get_mut(frame) {
            current.merge(progress);
        } else {
            // Keep active batches, evict old terminal views as needed.
            if cache.len() >= 64 {
                let oldest = cache
                    .iter()
                    .filter(|(_, p)| p.phase.terminal())
                    .min_by_key(|(_, p)| p.started_at_ms)
                    .map(|(id, _)| id.clone());
                if let Some(id) = oldest {
                    cache.remove(&id);
                }
            }
            cache.insert(frame.into(), progress);
        }
    }

    fn get(&self, frame: &str) -> Option<SwarmProgress> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(frame)
            .cloned()
    }
}

pub(crate) fn progress_event(frame: &str, progress: &SwarmProgress) -> AgentEvent {
    AgentEvent::ToolPresentation {
        frame_id: frame.into(),
        presentation_id: progress.id.clone(),
        presentation_kind: SWARM_PRESENTATION.into(),
        payload: serde_json::to_value(progress).unwrap_or_default(),
    }
}

/// Reader runs before the normal turn output exists. Give it the same event
/// contract immediately, then persist its final snapshot before returning.
pub(crate) async fn read_references(
    app: &tauri::AppHandle,
    store: &wisp_store::Store,
    refs: &[wisp_dto::ComposerReferenceArg],
    frame: &str,
    question: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Option<String>, String> {
    let (mut projects, mut sessions) = (Vec::new(), Vec::new());
    for reference in refs {
        match reference {
            wisp_dto::ComposerReferenceArg::Project { id } if !projects.contains(id) => {
                projects.push(id.clone())
            }
            wisp_dto::ComposerReferenceArg::Session { id } if !sessions.contains(id) => {
                sessions.push(id.clone())
            }
            _ => {}
        }
    }
    // Resolved before the retry evidence is taken: a failed lookup aborts
    // this turn, and the evidence must still reach the next one.
    let scope = crate::research_assistant::take_project_scope(store, frame, &mut projects).await?;
    // Evidence recovered by a manual retry rides along with the next turn,
    // whether or not that turn references the project again.
    let pending = take_pending_retry(store, frame).await;
    let join = |parts: [Option<String>; 3]| {
        let parts = parts.into_iter().flatten().collect::<Vec<_>>();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    };
    if projects.is_empty() && sessions.is_empty() {
        return Ok(join([scope, None, pending]));
    }
    let progress = wisp_core::swarm::SwarmReporter::new("reader", question, "", |update| {
        app.state::<AppState>()
            .swarm_progress
            .update(frame, update.clone());
        crate::emit_agent_event_in(app, progress_event(frame, &update), None);
    });
    let result = crate::project_reader::read_references_with_progress(
        store,
        &projects,
        &sessions,
        frame,
        question,
        cancel,
        Some(&progress),
    )
    .await;
    if let Err(error) = &result {
        progress.phase(
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                SwarmPhase::Cancelled
            } else {
                SwarmPhase::Failed
            },
            error,
        );
        // This turn will not reach the model; keep the recovered evidence for
        // the next one instead of dropping it.
        if let Some(pending) = &pending {
            store_pending_retry(store, frame, pending).await;
        }
    }
    persist_snapshot(app, store, frame).await;
    result.map(|injection| join([scope, injection, pending]))
}

async fn persist_snapshot(app: &tauri::AppHandle, store: &wisp_store::Store, frame: &str) {
    if let Some(snapshot) = app.state::<AppState>().swarm_progress.get(frame) {
        if let Ok(mut seq) = store.next_session_ui_event_seq(frame).await {
            crate::append_ui_event(store, frame, &mut seq, progress_event(frame, &snapshot)).await;
        }
    }
}

fn pending_retry_key(frame: &str) -> String {
    format!("{FRAME_READER_RETRY_PREFIX}{frame}")
}

/// Queue recovered evidence for the frame's next turn. Several retries before
/// that turn accumulate in order.
pub(crate) async fn store_pending_retry(store: &wisp_store::Store, frame: &str, injection: &str) {
    let key = pending_retry_key(frame);
    let value = match store.get_setting(&key).await.ok().flatten() {
        Some(existing) if !existing.is_empty() => format!("{existing}\n\n{injection}"),
        _ => injection.to_string(),
    };
    if let Err(error) = store.set_setting(&key, &value).await {
        tracing::warn!("persist Reader retry evidence failed: {error}");
    }
}

pub(crate) async fn take_pending_retry(store: &wisp_store::Store, frame: &str) -> Option<String> {
    let key = pending_retry_key(frame);
    let value = store
        .get_setting(&key)
        .await
        .ok()
        .flatten()
        .filter(|value| !value.is_empty())?;
    if let Err(error) = store.delete_setting(&key).await {
        tracing::warn!("clear Reader retry evidence failed: {error}");
    }
    Some(value)
}

async fn latest_progress(
    state: &AppState,
    window_label: &str,
    session_id: &str,
) -> Result<Option<SwarmProgress>, String> {
    let project = state.require_active(window_label)?;
    if state
        .store
        .frame_project_id(session_id)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some(project.id.as_str())
    {
        return Err("Session not found in the active project.".into());
    }
    if let Some(progress) = state.swarm_progress.get(session_id) {
        return Ok(Some(progress));
    }
    let event = state
        .store
        .load_latest_session_presentation(session_id, SWARM_PRESENTATION)
        .await
        .map_err(|e| e.to_string())?;
    let mut progress = event
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| serde_json::from_value::<SwarmProgress>(value["payload"].clone()).ok());
    if let Some(progress) = &mut progress {
        if !progress.phase.terminal() {
            progress.phase = SwarmPhase::Paused;
            for task in &mut progress.tasks {
                if matches!(
                    task.status,
                    SwarmTaskStatus::Running | SwarmTaskStatus::Queued
                ) {
                    task.status = SwarmTaskStatus::Paused;
                }
            }
        }
    }
    Ok(progress)
}

#[tauri::command]
pub(crate) async fn get_swarm_progress(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: String,
) -> Result<Option<SwarmProgress>, String> {
    latest_progress(&state, window.label(), &session_id).await
}

/// Decide what a retry request may re-run, without touching any runtime.
fn plan_retry(
    progress: &SwarmProgress,
    task_ids: Option<&[String]>,
) -> Result<(Vec<String>, Vec<crate::project_reader::RetryTarget>, String), String> {
    if !progress.phase.terminal() {
        return Err("This batch is still running.".into());
    }
    if progress.source != "reader" {
        return Err(
            "Only saved-session retrieval can be retried here. Ask the assistant to resume this batch instead."
                .into(),
        );
    }
    let mut ids = progress.retryable_task_ids();
    if let Some(filter) = task_ids {
        ids.retain(|id| filter.contains(id));
    }
    if ids.is_empty() {
        return Err("No failed tasks to retry.".into());
    }
    let targets = progress
        .tasks
        .iter()
        .filter(|task| ids.contains(&task.id))
        .map(|task| crate::project_reader::RetryTarget {
            session_id: task.id.clone(),
            failed_chunks: task.failed_unit_indexes.clone(),
            total_units: task.total_units,
        })
        .collect::<Vec<_>>();
    // Task instructions carry the (clipped) question the batch answered.
    let question = progress
        .tasks
        .iter()
        .find(|task| ids.contains(&task.id))
        .map(|task| task.instruction.trim_end_matches('…').to_string())
        .unwrap_or_default();
    Ok((ids, targets, question))
}

/// Re-run the failed tasks of the latest finished Reader batch in this
/// conversation. Returns as soon as the retry has started; progress flows
/// through the same `agent_swarm` presentation events, and **Stop this turn**
/// cancels it. Recovered evidence is handed to the next turn.
#[tauri::command]
pub(crate) async fn retry_swarm_tasks(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: String,
    task_ids: Option<Vec<String>>,
) -> Result<(), String> {
    let progress = latest_progress(&state, window.label(), &session_id)
        .await?
        .ok_or_else(|| "No parallel batch to retry.".to_string())?;
    let (ids, targets, question) = plan_retry(&progress, task_ids.as_deref())?;
    if state.running_turns.lock().await.contains(&session_id) {
        return Err("Wait for the current turn to finish before retrying.".into());
    }
    if !state
        .swarm_retries
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(session_id.clone())
    {
        return Err("A retry is already running in this conversation.".into());
    }
    let runtime = state.session_runtime(&session_id).await;
    runtime.cancel.store(false, Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        run_retry(
            &app,
            &session_id,
            progress,
            &ids,
            &targets,
            &question,
            &runtime.cancel,
        )
        .await;
        app.state::<AppState>()
            .swarm_retries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&session_id);
    });
    Ok(())
}

async fn run_retry(
    app: &tauri::AppHandle,
    frame: &str,
    progress: SwarmProgress,
    ids: &[String],
    targets: &[crate::project_reader::RetryTarget],
    question: &str,
    cancel: &std::sync::atomic::AtomicBool,
) {
    let state = app.state::<AppState>();
    let store = &state.store;
    let reporter = wisp_core::swarm::SwarmReporter::resume(progress, ids, |update| {
        state.swarm_progress.update(frame, update.clone());
        crate::emit_agent_event_in(app, progress_event(frame, &update), None);
    });
    match crate::project_reader::retry_failed_sessions(
        store, targets, frame, question, cancel, &reporter,
    )
    .await
    {
        Ok(injection) => store_pending_retry(store, frame, &injection).await,
        Err(error) => reporter.phase(
            if cancel.load(Ordering::SeqCst) {
                SwarmPhase::Cancelled
            } else {
                SwarmPhase::Failed
            },
            &error,
        ),
    }
    drop(reporter);
    persist_snapshot(app, store, frame).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_dto::SwarmTask;

    #[test]
    fn cache_is_frame_scoped_and_only_full_boundaries_are_persisted() {
        let cache = SwarmCache::default();
        let first = SwarmProgress {
            id: "batch".into(),
            source: "reader".into(),
            title: "Find evidence".into(),
            model: "fake".into(),
            started_at_ms: 10,
            revision: 1,
            phase: SwarmPhase::Starting,
            total: 1,
            tasks: vec![SwarmTask::queued(
                "a".into(),
                "Notes".into(),
                "Read".into(),
                1,
            )],
            summary: String::new(),
        };
        assert!(crate::should_persist_ui_event(&progress_event("f", &first)));
        cache.update("f", first.clone());
        let mut running = first.clone();
        running.revision = 2;
        running.phase = SwarmPhase::Running;
        running.tasks[0].revision = 2;
        running.tasks[0].status = SwarmTaskStatus::Running;
        assert!(!crate::should_persist_ui_event(&progress_event(
            "f", &running
        )));
        cache.update("f", running);
        assert!(cache.get("other").is_none());
        assert_eq!(
            cache.get("f").unwrap().tasks[0].status,
            SwarmTaskStatus::Running
        );
        let mut finished = first;
        finished.phase = SwarmPhase::Failed;
        finished.revision = 3;
        finished.tasks[0].status = SwarmTaskStatus::Failed;
        finished.tasks[0].revision = 3;
        assert!(crate::should_persist_ui_event(&progress_event(
            "f", &finished
        )));
        cache.update("f", finished);
        assert_eq!(cache.get("f").unwrap().phase, SwarmPhase::Failed);
    }

    fn partial_reader() -> SwarmProgress {
        let mut done = SwarmTask::queued("done".into(), "Done".into(), "find it…".into(), 1);
        done.status = SwarmTaskStatus::Completed;
        done.completed_units = 1;
        let mut bad = SwarmTask::queued("bad".into(), "Bad".into(), "find it…".into(), 4);
        bad.status = SwarmTaskStatus::Failed;
        bad.completed_units = 4;
        bad.failed_units = 1;
        bad.failed_unit_indexes = vec![2];
        let mut worse = SwarmTask::queued("worse".into(), "Worse".into(), "find it…".into(), 2);
        worse.status = SwarmTaskStatus::Failed;
        worse.completed_units = 2;
        worse.failed_units = 2;
        worse.failed_unit_indexes = vec![0, 1];
        SwarmProgress {
            id: "batch".into(),
            source: "reader".into(),
            title: "find it".into(),
            model: "fake".into(),
            started_at_ms: 10,
            revision: 9,
            phase: SwarmPhase::Partial,
            total: 3,
            tasks: vec![done, bad, worse],
            summary: String::new(),
        }
    }

    #[test]
    fn retry_plan_targets_failed_reader_tasks_only() {
        let progress = partial_reader();
        let (ids, targets, question) = plan_retry(&progress, None).unwrap();
        assert_eq!(ids, vec!["bad".to_string(), "worse".to_string()]);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].session_id, "bad");
        assert_eq!(targets[0].failed_chunks, vec![2]);
        assert_eq!(targets[0].total_units, 4);
        assert_eq!(
            question, "find it",
            "the clip marker is not part of the question"
        );

        let (ids, targets, _) = plan_retry(&progress, Some(&["worse".into()])).unwrap();
        assert_eq!(ids, vec!["worse".to_string()]);
        assert_eq!(targets[0].failed_chunks, vec![0, 1]);
        assert!(plan_retry(&progress, Some(&["done".into()]))
            .unwrap_err()
            .contains("No failed tasks"));

        let mut running = progress.clone();
        running.phase = SwarmPhase::Running;
        assert!(plan_retry(&running, None)
            .unwrap_err()
            .contains("still running"));
        let mut batch = progress.clone();
        batch.source = "map_items".into();
        assert!(plan_retry(&batch, None).unwrap_err().contains("resume"));
        let mut clean = progress;
        clean.tasks.truncate(1);
        clean.phase = SwarmPhase::Completed;
        assert!(plan_retry(&clean, None).is_err());
    }

    #[tokio::test]
    async fn pending_retry_evidence_is_handed_to_exactly_one_later_turn() {
        let database = std::env::temp_dir().join(format!(
            "wisp_swarm_retry_pending_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&database).await.unwrap();
        store.create_project("p", "P", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        assert!(take_pending_retry(&store, "f").await.is_none());
        store_pending_retry(&store, "f", "## Reader retry\nfirst").await;
        store_pending_retry(&store, "f", "## Reader retry\nsecond").await;
        assert!(take_pending_retry(&store, "other").await.is_none());
        let pending = take_pending_retry(&store, "f").await.unwrap();
        assert_eq!(pending, "## Reader retry\nfirst\n\n## Reader retry\nsecond");
        assert!(take_pending_retry(&store, "f").await.is_none());
        drop(store);
        let _ = std::fs::remove_file(database);
    }
}
