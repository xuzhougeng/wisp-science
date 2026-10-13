//! Live snapshots survive switching conversations; only start/final snapshots
//! are persisted, so a thousand-item batch does not write a thousand full lists.
use crate::{AgentEvent, AppState};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{Manager, State};
use wisp_dto::{SwarmPhase, SwarmProgress, SwarmTaskStatus, SWARM_PRESENTATION};

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
    if !refs.iter().any(|r| {
        matches!(
            r,
            wisp_dto::ComposerReferenceArg::Project { .. }
                | wisp_dto::ComposerReferenceArg::Session { .. }
        )
    }) {
        return Ok(None);
    }
    let progress = wisp_core::swarm::SwarmReporter::new("reader", question, "", |update| {
        app.state::<AppState>()
            .swarm_progress
            .update(frame, update.clone());
        crate::emit_agent_event_in(app, progress_event(frame, &update), None);
    });
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
    }
    if let Some(snapshot) = app.state::<AppState>().swarm_progress.get(frame) {
        if let Ok(mut seq) = store.next_session_ui_event_seq(frame).await {
            crate::append_ui_event(store, frame, &mut seq, progress_event(frame, &snapshot)).await;
        }
    }
    result
}

#[tauri::command]
pub(crate) async fn get_swarm_progress(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    session_id: String,
) -> Result<Option<SwarmProgress>, String> {
    let project = state.require_active(window.label())?;
    if state
        .store
        .frame_project_id(&session_id)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some(project.id.as_str())
    {
        return Err("Session not found in the active project.".into());
    }
    if let Some(progress) = state.swarm_progress.get(&session_id) {
        return Ok(Some(progress));
    }
    let event = state
        .store
        .load_latest_session_presentation(&session_id, SWARM_PRESENTATION)
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
}
