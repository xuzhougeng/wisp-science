//! An authoritative projection of the shared WebView/native turn queue.
use crate::{native_settings::Broker, QueuedItem, SessionRuntime};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::{atomic::Ordering, Mutex},
};
use tauri::Manager;
use wisp_dto::native_queue::{
    QueueAction, QueueActionRequest, QueueItem, QueueOutcome, QueueSnapshot,
};

#[derive(Default)]
pub(crate) struct QueueEvents(Mutex<HashMap<String, VecDeque<QueueOutcome>>>);
impl QueueEvents {
    pub(crate) fn record(&self, session: &str, id: u64, state: &str) {
        if !matches!(
            state,
            "started" | "completed" | "cancelled" | "superseded" | "failed"
        ) {
            return;
        }
        let mut sessions = self.0.lock().unwrap();
        if sessions.len() >= 512 && !sessions.contains_key(session) {
            return;
        }
        let rows = sessions.entry(session.into()).or_default();
        let id = id.to_string();
        rows.retain(|row| row.id != id);
        rows.push_back(QueueOutcome {
            id,
            state: state.into(),
        });
        while rows.len() > 256 {
            rows.pop_front();
        }
    }
    fn outcomes(&self, session: &str) -> Vec<QueueOutcome> {
        self.0
            .lock()
            .unwrap()
            .get(session)
            .map(|rows| rows.iter().cloned().collect())
            .unwrap_or_default()
    }
}

fn digest(item: &QueuedItem) -> String {
    wisp_sync::sha256_hex(
        &serde_json::to_vec(&(&item.message, &item.attachments, &item.references))
            .expect("queue payload serialization"),
    )
}
fn row(item: &QueuedItem, state: &str) -> QueueItem {
    QueueItem {
        id: item.id.to_string(),
        digest: digest(item),
        state: state.into(),
        message: item.message.clone(),
        attachments: item.attachments.clone(),
        references: item.references.clone(),
    }
}
fn items(runtime: &SessionRuntime) -> Vec<QueueItem> {
    let queued = runtime.queued.lock().unwrap();
    let cutins = runtime.queued_cutins.lock().unwrap();
    cutins
        .iter()
        .map(|(_, item)| row(item, "cutin_pending"))
        .chain(queued.iter().map(|item| row(item, "queued")))
        .collect()
}
pub(crate) async fn snapshot(broker: &Broker, session: &str, acp: bool) -> QueueSnapshot {
    let runtime = broker
        .app
        .state::<crate::AppState>()
        .sessions
        .lock()
        .await
        .get(session)
        .cloned();
    QueueSnapshot {
        items: runtime.as_ref().map(|rt| items(rt)).unwrap_or_default(),
        outcomes: broker.conversations.queue_events.outcomes(session),
        can_cut_in: !acp
            && runtime
                .as_ref()
                .is_none_or(|rt| !rt.timer_running.load(Ordering::SeqCst)),
    }
}

/// Mutate under the queue lock. A row that already started or changed must not
/// cause any cancellation or overwrite; identity includes attachments/references.
fn apply(
    runtime: &SessionRuntime,
    request: &QueueActionRequest,
    can_cut_in: bool,
) -> Result<Option<&'static str>, String> {
    let id = request.queue_id()?;
    let mut queued = runtime.queued.lock().unwrap();
    let index = queued
        .iter()
        .position(|item| item.id == id)
        .ok_or("This queued turn has already started or changed; refresh the queue")?;
    if digest(&queued[index]) != request.digest {
        return Err("This queued turn was edited elsewhere; refresh before continuing".into());
    }
    match &request.action {
        QueueAction::Edit { message } => {
            let text = wisp_dto::native_conversations::message_with_attachments(
                message,
                &queued[index].attachments,
            );
            if text.trim().is_empty() && queued[index].references.is_empty() {
                return Err("A queued turn needs text or a reference".into());
            }
            queued[index].message = text;
        }
        QueueAction::Cancel => {
            queued.remove(index);
            return Ok(Some("cancelled"));
        }
        QueueAction::MoveUp => crate::agent_turn::swap_queued_toward(&mut queued, id, true),
        QueueAction::MoveDown => crate::agent_turn::swap_queued_toward(&mut queued, id, false),
        QueueAction::Replace => {
            let item = queued.remove(index);
            queued.insert(0, item);
        }
        QueueAction::CutIn => {
            if !can_cut_in || runtime.timer_running.load(Ordering::SeqCst) {
                return Err("This session does not support inserting into the current turn".into());
            }
            let item = queued.remove(index);
            let mut cutins = runtime.queued_cutins.lock().unwrap();
            let guidance_id = runtime.guidance_seq.fetch_add(1, Ordering::Relaxed);
            runtime
                .pending_guidance
                .lock()
                .unwrap()
                .push((guidance_id, item.message.clone()));
            cutins.push((guidance_id, item));
        }
    }
    Ok(None)
}

pub(crate) async fn dispatch(
    broker: &Broker,
    project: &str,
    request: QueueActionRequest,
    acp: bool,
) -> Result<Value, String> {
    let state = broker.app.state::<crate::AppState>();
    crate::native_conversations::require_owner(&state.store, project, &request.session_id).await?;
    state
        .store
        .require_unarchived_session(&request.session_id)
        .await
        .map_err(|e| e.to_string())?;
    let scope = state
        .store
        .frame_state_scope(&request.session_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Session scope missing")?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    if matches!(
        state
            .store
            .session_branch_state(&request.session_id)
            .await
            .map_err(|e| e.to_string())?,
        Some("merged" | "orphaned")
    ) {
        return Err("Frozen conversation queues cannot be changed".into());
    }
    let runtime = state
        .sessions
        .lock()
        .await
        .get(&request.session_id)
        .cloned()
        .ok_or("The queue is no longer available")?;
    let replacing = matches!(request.action, QueueAction::Replace)
        .then(|| crate::agent_turn::ReplacementReservation::new(runtime.clone()));
    if let Some(outcome) = apply(&runtime, &request, !acp)? {
        crate::agent_turn::emit_queued_turn_state(
            &broker.app,
            &request.session_id,
            request.queue_id()?,
            outcome,
        );
    }
    if matches!(request.action, QueueAction::CutIn) {
        crate::agent_turn::emit_queued_turn_state(
            &broker.app,
            &request.session_id,
            request.queue_id()?,
            "cutin_pending",
        );
    }
    if replacing.is_some() {
        crate::agent_turn::stop_agent(state.clone(), Some(request.session_id.clone())).await?;
    }
    let spawn = {
        let queued = runtime.queued.lock().unwrap();
        (!queued.is_empty() || !runtime.queued_cutins.lock().unwrap().is_empty())
            && !runtime.draining.swap(true, Ordering::SeqCst)
    };
    if spawn {
        crate::agent_turn::spawn_queue_driver(
            broker.app.clone(),
            runtime.clone(),
            request.session_id.clone(),
            "native-follow-up".into(),
        );
    }
    drop(replacing);
    Ok(json!({"session_id":request.session_id,"id":request.id}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seed(rt: &SessionRuntime, id: u64, file: &str) {
        rt.queued.lock().unwrap().push(QueuedItem {
            id,
            message: "same".into(),
            attachments: vec![file.into()],
            references: vec![],
        });
    }
    fn request(rt: &SessionRuntime, id: u64, action: QueueAction) -> QueueActionRequest {
        let item = items(rt)
            .into_iter()
            .find(|item| item.id == id.to_string())
            .unwrap();
        QueueActionRequest {
            session_id: "s".into(),
            id: item.id,
            digest: item.digest,
            action,
        }
    }
    #[test]
    fn native_queue_actions_preserve_identity_payload_and_order() {
        let rt = SessionRuntime::new();
        seed(&rt, 7, "a.csv");
        seed(&rt, 8, "b.csv");
        let stale = request(&rt, 7, QueueAction::Cancel);
        apply(
            &rt,
            &request(
                &rt,
                7,
                QueueAction::Edit {
                    message: "changed".into(),
                },
            ),
            true,
        )
        .unwrap();
        assert!(apply(&rt, &stale, true).is_err());
        assert_eq!(items(&rt)[0].attachments, ["a.csv"]);
        assert_eq!(items(&rt)[1].message, "same");
        apply(&rt, &request(&rt, 8, QueueAction::MoveUp), true).unwrap();
        assert_eq!(items(&rt)[0].id, "8");
        apply(&rt, &request(&rt, 7, QueueAction::Replace), true).unwrap();
        assert_eq!(items(&rt)[0].id, "7");
        assert!(apply(&rt, &request(&rt, 7, QueueAction::CutIn), false).is_err());
        apply(&rt, &request(&rt, 7, QueueAction::CutIn), true).unwrap();
        assert_eq!(items(&rt)[0].state, "cutin_pending");
        assert_eq!(items(&rt)[0].attachments, ["a.csv"]);
        assert!(apply(&rt, &stale, true).is_err());
        apply(&rt, &request(&rt, 8, QueueAction::Cancel), true).unwrap();
        assert_eq!(items(&rt).len(), 1);
    }
    #[test]
    fn native_queue_outcomes_are_bounded_and_session_owned() {
        let events = QueueEvents::default();
        for id in 0..300 {
            events.record("s", id, "started");
        }
        assert_eq!(events.outcomes("s").len(), 256);
        events.record("s", 299, "failed");
        assert_eq!(events.outcomes("s").last().unwrap().state, "failed");
        assert!(events.outcomes("other").is_empty());
    }
    #[test]
    fn native_queue_accepts_distinct_payloads_and_reconciles_cutin_handoff() {
        let rt = SessionRuntime::new();
        for (id, file) in [(1, "a.csv"), (2, "b.csv")] {
            crate::agent_turn::queue_follow_up_with_limit(
                true,
                &rt,
                id,
                "same",
                &[file.into()],
                &[],
                64,
            )
            .unwrap();
        }
        assert_eq!(items(&rt).len(), 2);
        assert!(crate::agent_turn::queue_follow_up_with_limit(
            true,
            &rt,
            1,
            "same",
            &["other.csv".into()],
            &[],
            64
        )
        .is_err());
        let cutin = request(&rt, 2, QueueAction::CutIn);
        apply(&rt, &cutin, true).unwrap();
        let next = crate::agent_turn::take_next_queued_turn(&rt).unwrap();
        assert_eq!(next.id, 2);
        assert_eq!(next.attachments, ["b.csv"]);
        assert!(apply(&rt, &cutin, true).is_err());
        assert_eq!(crate::agent_turn::take_next_queued_turn(&rt).unwrap().id, 1);
    }
}
