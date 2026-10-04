//! Dispatch: one conversation's agent hands work to another conversation,
//! is acknowledged once that work starts, and later receives one reviewed
//! result without keeping its own turn alive.
//!
//! [`Dispatcher`] is the dispatching side of that contract. The research
//! assistant (`research_assistant::AssistantDispatcher`) and in-conversation
//! subagents (`subagent_tool::SubagentDispatcher`, #1061) implement it.
use crate::{channels, AppState, SessionRuntime, TurnOrigin};
use async_trait::async_trait;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, oneshot, OwnedMutexGuard};
use wisp_store::Store;

/// Where dispatched work reports back to, and how its turn is framed, gated
/// and supervised.
#[async_trait]
pub(crate) trait Dispatcher: Send + Sync + 'static {
    /// Conversation that receives the completion report.
    fn parent_frame(&self) -> &str;
    /// Origin of the dispatched turn; it decides that turn's approval policy.
    fn origin(&self) -> TurnOrigin;
    /// The instruction as the target conversation receives it.
    fn brief(&self, instruction: &str) -> String;
    /// System prompt for reviewing the returned evidence before delivery.
    fn review_prompt(&self) -> &'static str;
    /// Whether the outcome of work in `project_id` may still be delivered.
    async fn may_report(&self, _store: &Store, _project_id: &str) -> Result<bool, String> {
        Ok(true)
    }
    /// An approval the dispatched turn is waiting on. The target
    /// conversation's own approval card stays available either way.
    async fn supervise(
        &self,
        _app: &AppHandle,
        _instruction: &str,
        _request: crate::ConfirmRequest,
    ) {
    }
    /// The report is in the parent conversation.
    fn delivered(&self, _project_id: &str, _report: &str) {}
}

async fn runtime(state: &AppState, session: &str) -> Arc<SessionRuntime> {
    state
        .sessions
        .lock()
        .await
        .entry(session.into())
        .or_insert_with(|| Arc::new(SessionRuntime::new()))
        .clone()
}

pub(crate) async fn reserve(
    state: &AppState,
    session: &str,
) -> Result<OwnedMutexGuard<()>, String> {
    runtime(state, session).await.workflow.clone().try_lock_owned()
        .map_err(|_| "This conversation is already working. Wait for it to finish before dispatching another task or changing its server.".into())
}

pub(crate) async fn bind_context(
    store: &Store,
    session: &str,
    context: &str,
) -> Result<(), String> {
    let found = store
        .get_execution_context(context)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Unknown execution context '{context}'."))?;
    let value = if found.kind == wisp_store::ExecutionContextKind::Local {
        crate::ssh_hosts::SessionDefaultExecutionContext::Local
    } else {
        crate::ssh_hosts::SessionDefaultExecutionContext::Remote(found.id)
    };
    crate::ssh_hosts::persist_session_default_execution_context(store, session, value).await?;
    Ok(())
}

pub(crate) async fn start(
    app: &AppHandle,
    dispatcher: Arc<dyn Dispatcher>,
    session: &str,
    instruction: &str,
    context: Option<&str>,
    reserved: Option<OwnedMutexGuard<()>>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let guard = match reserved {
        Some(guard) => guard,
        None => reserve(&state, session).await?,
    };
    let (project, scope) =
        crate::exploration_commands::working_project_for_frame(&state, session).await?;
    let _activity = state.begin_project_activity(&project.id)?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    if let Some(context) = context {
        bind_context(&state.store, session, context).await?;
        crate::mcp_connections::host()
            .reconcile(&state.store, Some(&project.id))
            .await;
        // Existing runtimes must rebuild their compute wiring for the new server.
        crate::clear_session_agent(&state, session).await;
    }
    let (started_tx, started_rx) = oneshot::channel();
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let observer = channels::prepare_progress_observer(progress_tx);
    let app = app.clone();
    let session = session.to_string();
    let instruction = instruction.to_string();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let turn = crate::send_message_inner(
            state.inner(),
            app.clone(),
            "main",
            Some(session.clone()),
            dispatcher.brief(&instruction),
            None,
            None,
            None,
            None,
            Some(observer.id()),
            None,
            None,
            Some(guard),
            dispatcher.origin(),
        );
        let mut approvals = tokio::task::JoinSet::new();
        let (outcome, stop_reason, answer) =
            observe_turn(turn, progress_rx, started_tx, |request| {
                let app = app.clone();
                let instruction = instruction.clone();
                let dispatcher = dispatcher.clone();
                approvals.spawn(async move {
                    dispatcher.supervise(&app, &instruction, request).await;
                });
            })
            .await;
        approvals.abort_all();
        while approvals.join_next().await.is_some() {}
        drop(observer);
        if let Err(error) = report(
            &app,
            dispatcher.as_ref(),
            &session,
            &instruction,
            outcome,
            stop_reason,
            answer,
        )
        .await
        {
            tracing::warn!(%error, %session, "dispatch result delivery failed");
        }
    });
    started_rx
        .await
        .map_err(|_| "Dispatched task stopped before acknowledging startup.".to_string())?
}

/// Await only the first accepted-turn event for the dispatch tool. Continue
/// observing completion in this background future; no model-driven polling.
async fn observe_turn<F, A>(
    turn: F,
    mut progress: mpsc::UnboundedReceiver<channels::ProgressEvent>,
    started: oneshot::Sender<Result<(), String>>,
    mut approval: A,
) -> (Result<String, String>, Option<String>, String)
where
    F: std::future::Future<Output = Result<String, String>>,
    A: FnMut(crate::ConfirmRequest),
{
    tokio::pin!(turn);
    let mut started = Some(started);
    let mut stop_reason = None;
    let mut answer = String::new();
    let mut tool_preview = None;
    let outcome = loop {
        tokio::select! {
            biased;
            Some(event) = progress.recv() => {
                match event {
                    channels::ProgressEvent::TurnFinished { stop_reason: reason } => stop_reason = reason,
                    channels::ProgressEvent::TurnAnswer(text) => answer = text,
                    channels::ProgressEvent::ToolStarted { name, preview } => tool_preview = Some((name, preview)),
                    channels::ProgressEvent::ApprovalRequested(mut request) => {
                        if request.preview.trim().is_empty() {
                            if let Some((name, preview)) = &tool_preview {
                                if name == &request.tool { request.preview = preview.clone(); }
                            }
                        }
                        approval(request);
                    },
                    _ => if let Some(started) = started.take() { let _ = started.send(Ok(())); },
                }
            }
            outcome = &mut turn => break outcome,
        }
    };
    // The future can publish its terminal event and return in the same poll.
    while let Ok(event) = progress.try_recv() {
        match event {
            channels::ProgressEvent::TurnFinished {
                stop_reason: reason,
            } => stop_reason = reason,
            channels::ProgressEvent::TurnAnswer(text) => answer = text,
            _ => {}
        }
    }
    if let Some(started) = started {
        let _ = started.send(outcome.as_ref().map(|_| ()).map_err(Clone::clone));
    }
    (outcome, stop_reason, answer)
}

fn completion_status(outcome: &Result<String, String>, stop_reason: Option<&str>) -> String {
    match outcome {
        Err(error) => format!("failed: {error}"),
        Ok(_) => match stop_reason {
            None | Some("end_turn") => "turn completed (verify the result; this alone does not prove the research task succeeded)".into(),
            Some(reason) => format!("stopped: {reason}; task completion is not confirmed"),
        }
    }
}

async fn report(
    app: &AppHandle,
    dispatcher: &dyn Dispatcher,
    session: &str,
    instruction: &str,
    outcome: Result<String, String>,
    stop_reason: Option<String>,
    answer: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let parent = dispatcher.parent_frame();
    let reference = state
        .store
        .get_session_reference(session)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Dispatched conversation no longer exists.".to_string())?;
    if !dispatcher
        .may_report(&state.store, &reference.project_id)
        .await?
    {
        return Ok(());
    }
    let status = completion_status(&outcome, stop_reason.as_deref());
    let evidence = serde_json::json!({
        "project": reference.project_name, "conversation": reference.title,
        "session_id": session, "instruction": instruction, "status": status,
        "result": answer.chars().take(12000).collect::<String>(),
    })
    .to_string();
    let summary = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::turn_hooks::side_complete(
            &state,
            parent,
            "dispatch_result",
            crate::turn_hooks::SideModel::Session { max_tokens: 1000 },
            dispatcher.review_prompt(),
            &evidence,
            None,
        ),
    )
    .await;
    let text = match summary {
        Ok(Ok(summary)) if !summary.text.trim().is_empty() => summary.text,
        _ => format!(
            "任务已返回 / Task returned\n{} · {}\n状态 / Status: {status}\n{}",
            reference.project_name,
            reference.title,
            answer.chars().take(3000).collect::<String>()
        ),
    };
    // Wait for the parent's startup acknowledgement (or another user turn) to
    // finish, then append a separate assistant reply under its workflow lock.
    let rt = runtime(&state, parent).await;
    let _guard = rt.workflow.lock().await;
    if !dispatcher
        .may_report(&state.store, &reference.project_id)
        .await?
    {
        return Ok(());
    }
    // The parent conversation may have been deleted while the work ran.
    let Some(parent_project) = state
        .store
        .frame_project_id(parent)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let mut agent = rt.agent.lock().await;
    let events = persist_report(&state.store, parent, &text).await?;
    *agent = None;
    rt.sync_last_seq_from_store(&state.store, parent).await?;
    for event in events {
        crate::emit_agent_event_in(app, event, Some(&parent_project));
    }
    dispatcher.delivered(&reference.project_id, &text);
    Ok(())
}

async fn persist_report(
    store: &Store,
    frame: &str,
    text: &str,
) -> Result<Vec<crate::AgentEvent>, String> {
    use crate::AgentEvent;
    let seq = store
        .max_message_seq(frame)
        .await
        .map_err(|e| e.to_string())?
        + 1;
    store
        .append_message(frame, seq, &wisp_llm::Message::assistant(text))
        .await
        .map_err(|e| e.to_string())?;
    let events = vec![
        AgentEvent::BackgroundReply(wisp_dto::BackgroundReply {
            frame_id: frame.into(),
            text: text.into(),
        }),
        AgentEvent::MessageBoundary {
            frame_id: frame.into(),
            seq,
        },
    ];
    let mut ui_seq = store
        .next_session_ui_event_seq(frame)
        .await
        .map_err(|e| e.to_string())?;
    for event in &events {
        store
            .append_session_ui_event(
                frame,
                ui_seq,
                &serde_json::to_string(event).map_err(|e| e.to_string())?,
            )
            .await
            .map_err(|e| e.to_string())?;
        ui_seq += 1;
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research_assistant;

    #[tokio::test]
    async fn dispatch_acknowledges_start_without_waiting_for_completion() {
        let (events, progress) = mpsc::unbounded_channel();
        let (started, acknowledgement) = oneshot::channel();
        let (finish, finished) = oneshot::channel();
        let worker = tokio::spawn(observe_turn(
            async move {
                events.send(channels::ProgressEvent::Activity).unwrap();
                finished.await.unwrap();
                events
                    .send(channels::ProgressEvent::TurnAnswer(
                        "CPU2 produced results.tsv".into(),
                    ))
                    .unwrap();
                events
                    .send(channels::ProgressEvent::TurnFinished { stop_reason: None })
                    .unwrap();
                Ok("project-session".into())
            },
            progress,
            started,
            |_| {},
        ));
        assert_eq!(acknowledgement.await.unwrap(), Ok(()));
        assert!(
            !worker.is_finished(),
            "startup acknowledgement must not wait for the task"
        );
        finish.send(()).unwrap();
        let (outcome, reason, answer) = worker.await.unwrap();
        assert_eq!(outcome.unwrap(), "project-session");
        assert_eq!(reason, None);
        assert_eq!(answer, "CPU2 produced results.tsv");
    }

    #[tokio::test]
    async fn startup_failure_and_cancellation_do_not_claim_task_success() {
        let (_events, progress) = mpsc::unbounded_channel();
        let (started, acknowledgement) = oneshot::channel();
        let (outcome, _, answer) = observe_turn(
            async { Err("No model configured".into()) },
            progress,
            started,
            |_| {},
        )
        .await;
        assert!(acknowledgement.await.unwrap().is_err());
        assert!(answer.is_empty());
        assert!(completion_status(&outcome, None).contains("failed"));
        assert!(completion_status(&Ok("s".into()), Some("cancelled"))
            .contains("completion is not confirmed"));
        assert!(completion_status(&Ok("s".into()), Some("max_iterations")).contains("stopped"));
    }

    #[tokio::test]
    async fn project_approval_is_forwarded_with_its_exact_identity_and_operation() {
        let (events, progress) = mpsc::unbounded_channel();
        let (started, acknowledgement) = oneshot::channel();
        let (review_tx, mut reviews) = mpsc::unbounded_channel();
        let (decision, decided) = oneshot::channel::<bool>();
        let request =
            crate::ConfirmRequest::new("child", "Run tool 'shell'?".into(), "shell", String::new());
        let approval_id = request.approval_id.clone();
        let worker = tokio::spawn(observe_turn(
            async move {
                events.send(channels::ProgressEvent::Activity).unwrap();
                events
                    .send(channels::ProgressEvent::ToolStarted {
                        name: "shell".into(),
                        preview: "ls /data/RNA".into(),
                    })
                    .unwrap();
                events
                    .send(channels::ProgressEvent::ApprovalRequested(request))
                    .unwrap();
                assert!(decided.await.unwrap());
                Ok("child".into())
            },
            progress,
            started,
            move |request| {
                review_tx.send(request).unwrap();
            },
        ));
        acknowledgement.await.unwrap().unwrap();
        let review = reviews.recv().await.unwrap();
        assert_eq!(review.frame_id, "child");
        assert_eq!(review.approval_id, approval_id);
        assert_eq!(review.preview, "ls /data/RNA");
        assert!(!worker.is_finished());
        decision.send(true).unwrap();
        assert!(worker.await.unwrap().0.is_ok());
    }

    #[tokio::test]
    async fn binds_existing_conversation_to_cpu2_and_persists_result_in_assistant_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("wisp.sqlite")).await.unwrap();
        store.create_project("p", "RNA", "").await.unwrap();
        store.create_frame("s", "p", "OPERON", "m").await.unwrap();
        store
            .upsert_execution_context(
                &wisp_store::ExecutionContext::new("ssh:CPU2", "CPU2").unwrap(),
            )
            .await
            .unwrap();
        bind_context(&store, "s", "ssh:CPU2").await.unwrap();
        assert_eq!(
            store
                .session_default_execution_context("s")
                .await
                .unwrap()
                .as_deref(),
            Some("ssh:CPU2")
        );
        assert!(store
            .session_execution_context_enabled("s", "ssh:CPU2")
            .await
            .unwrap());
        assert!(bind_context(&store, "s", "missing").await.is_err());
        assert_eq!(
            store
                .session_default_execution_context("s")
                .await
                .unwrap()
                .as_deref(),
            Some("ssh:CPU2")
        );
        research_assistant::ensure(&store, dir.path())
            .await
            .unwrap();
        let events = persist_report(
            &store,
            research_assistant::ASSISTANT_FRAME_ID,
            "CPU2 已完成，输出 results.tsv",
        )
        .await
        .unwrap();
        assert_eq!(events.len(), 2);
        let wire = serde_json::to_value(&events[0]).unwrap();
        assert!(
            matches!(serde_json::from_value::<wisp_dto::AgentEvent>(wire).unwrap(), wisp_dto::AgentEvent::BackgroundReply(reply) if reply.text.contains("results.tsv"))
        );
        let mut replay = vec![crate::AgentEvent::Text {
            frame_id: research_assistant::ASSISTANT_FRAME_ID.into(),
            delta: "Task started".into(),
        }];
        replay.extend(events);
        let (items, _) = crate::events_to_items(&replay);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "Task started");
        assert!(items[1].text.contains("results.tsv"));
        let rows = store
            .load_messages(research_assistant::ASSISTANT_FRAME_ID)
            .await
            .unwrap();
        assert_eq!(rows.last().unwrap().role, wisp_llm::Role::Assistant);
        assert!(rows
            .last()
            .unwrap()
            .content
            .as_text()
            .contains("results.tsv"));
        assert_eq!(
            store
                .load_session_ui_events(research_assistant::ASSISTANT_FRAME_ID)
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(store.load_messages("s").await.unwrap().is_empty());
    }
}
