//! Persisted background delegation completion and parent-conversation delivery.
//!
//! Execution, delivery, and synthesis are separate durable phases. A compact
//! internal result is appended under the owning session lock, while full child
//! responses remain in workflow attempts for `get_delegated_result`.

use crate::{delegation_tool, AgentEvent, AppState, SessionRuntime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State};
use wisp_core::{
    AgentDelegationResponse, AgentUsage, DelegationExecutionResult, DelegationExecutionStatus,
    DelegationStatus, DelegationStepExecution,
};
use wisp_store::{
    AgentWorkflowAttempt, AgentWorkflowAttemptStatus, AgentWorkflowDelivery, AgentWorkflowStatus,
    Store,
};

const COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentCompletionPolicy {
    #[default]
    Inline,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct AgentCompletionSettings {
    #[serde(default)]
    pub(crate) policy: AgentCompletionPolicy,
    #[serde(default)]
    pub(crate) auto_resume: bool,
}

fn completion_setting_key(frame_id: &str) -> String {
    format!("frame_agent_completion:{frame_id}")
}

pub(crate) async fn session_completion_settings(
    store: &Store,
    frame_id: &str,
) -> AgentCompletionSettings {
    let mut settings: AgentCompletionSettings = store
        .get_setting(&completion_setting_key(frame_id))
        .await
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default();
    settings.auto_resume &= settings.policy == AgentCompletionPolicy::Background;
    settings
}

pub(crate) async fn save_session_completion_settings(
    store: &Store,
    project_id: &str,
    frame_id: &str,
    settings: AgentCompletionSettings,
) -> Result<AgentCompletionSettings, String> {
    match store
        .frame_project_id(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
    {
        Some(owner) if owner == project_id => {}
        Some(_) => return Err("Conversation does not belong to the active project.".into()),
        None => return Err("Conversation does not exist.".into()),
    }
    let settings = AgentCompletionSettings {
        policy: settings.policy,
        auto_resume: settings.policy == AgentCompletionPolicy::Background && settings.auto_resume,
    };
    store
        .set_setting(
            &completion_setting_key(frame_id),
            &serde_json::to_string(&settings).map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(settings)
}

#[tauri::command]
pub(crate) async fn get_session_agent_completion(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
) -> Result<AgentCompletionSettings, String> {
    let project = state.active(window.label());
    match state
        .store
        .frame_project_id(&session_id)
        .await
        .map_err(|error| error.to_string())?
        .as_deref()
    {
        Some(owner) if owner == project.id => {
            Ok(session_completion_settings(&state.store, &session_id).await)
        }
        Some(_) => Err("Conversation does not belong to the active project.".into()),
        None => Err("Conversation does not exist.".into()),
    }
}

#[tauri::command]
pub(crate) async fn set_session_agent_completion(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: String,
    policy: AgentCompletionPolicy,
    auto_resume: bool,
) -> Result<AgentCompletionSettings, String> {
    let project = state.active(window.label());
    let settings = save_session_completion_settings(
        &state.store,
        &project.id,
        &session_id,
        AgentCompletionSettings {
            policy,
            auto_resume,
        },
    )
    .await?;
    crate::clear_idle_agents(&state).await;
    Ok(settings)
}

pub(crate) async fn persist_execution_result(
    store: &Store,
    delivery: &AgentWorkflowDelivery,
    execution: &DelegationExecutionResult,
    display_ids: &HashMap<String, String>,
) -> Result<(), String> {
    let compact = delegation_tool::compact_execution_result(execution, display_ids);
    persist_compact_result(store, delivery, compact).await
}

pub(crate) async fn persist_execution_failure(
    store: &Store,
    delivery: &AgentWorkflowDelivery,
    error: &str,
) -> Result<(), String> {
    if let Some(workflow) = store
        .get_agent_workflow(&delivery.workflow_id)
        .await
        .map_err(|cause| cause.to_string())?
    {
        match workflow.status {
            AgentWorkflowStatus::Approved => {
                let _ = store
                    .transition_agent_workflow_status(
                        &workflow.id,
                        AgentWorkflowStatus::Approved,
                        AgentWorkflowStatus::Failed,
                    )
                    .await
                    .map_err(|cause| cause.to_string())?;
            }
            AgentWorkflowStatus::Running => {
                let _ = store
                    .fail_agent_workflow_execution(&workflow.id, error)
                    .await
                    .map_err(|cause| cause.to_string())?;
            }
            _ => {}
        }
    }
    reconstruct_delivery_result(store, delivery).await
}

async fn persist_compact_result(
    store: &Store,
    delivery: &AgentWorkflowDelivery,
    compact: Value,
) -> Result<(), String> {
    let envelope = json!({
        "type": "delegated_batch_completion",
        "workflow_id": delivery.workflow_id,
        "generation": delivery.generation,
        "result": compact,
        "message": "A background delegated batch finished. Synthesize its ordered results for the user; use get_delegated_result only when the compact result is insufficient."
    });
    if !store
        .complete_agent_workflow_delivery(&delivery.id, &envelope.to_string())
        .await
        .map_err(|error| error.to_string())?
    {
        // A duplicate terminal callback is expected after a retry/restart race;
        // the first durable result remains authoritative.
        tracing::debug!(delivery_id = %delivery.id, "Agent completion was already persisted");
    }
    Ok(())
}

fn execution_status(status: AgentWorkflowStatus) -> DelegationExecutionStatus {
    match status {
        AgentWorkflowStatus::Succeeded => DelegationExecutionStatus::Succeeded,
        AgentWorkflowStatus::Cancelled => DelegationExecutionStatus::Cancelled,
        _ => DelegationExecutionStatus::Failed,
    }
}

fn attempt_response(
    attempt: Option<&AgentWorkflowAttempt>,
    step_id: &str,
) -> AgentDelegationResponse {
    if let Some(response) = attempt
        .and_then(|attempt| attempt.response_json.as_deref())
        .and_then(|raw| serde_json::from_str::<AgentDelegationResponse>(raw).ok())
    {
        return response;
    }
    let status = attempt.map_or(DelegationStatus::Failed, |attempt| match attempt.status {
        AgentWorkflowAttemptStatus::Succeeded => DelegationStatus::Succeeded,
        AgentWorkflowAttemptStatus::Cancelled => DelegationStatus::Cancelled,
        AgentWorkflowAttemptStatus::Blocked => DelegationStatus::Blocked,
        _ => DelegationStatus::Failed,
    });
    let output = attempt
        .and_then(|attempt| serde_json::from_str(&attempt.output_json).ok())
        .unwrap_or_else(|| json!({}));
    let artifact_ids = attempt
        .and_then(|attempt| serde_json::from_str(&attempt.artifact_ids_json).ok())
        .unwrap_or_default();
    let evidence = attempt
        .and_then(|attempt| serde_json::from_str(&attempt.evidence_json).ok())
        .unwrap_or_default();
    AgentDelegationResponse {
        request_id: attempt
            .map(|attempt| attempt.request_id.clone())
            .unwrap_or_else(|| format!("interrupted-{step_id}")),
        status,
        output,
        artifact_ids,
        artifacts: vec![],
        evidence,
        usage: attempt.map_or_else(AgentUsage::default, |attempt| AgentUsage {
            input_tokens: attempt.input_tokens.max(0) as u64,
            output_tokens: attempt.output_tokens.max(0) as u64,
            tool_calls: attempt.tool_calls.max(0) as u64,
            cost_microunits: attempt.cost_microunits.max(0) as u64,
        }),
        agent_session_id: attempt.and_then(|attempt| attempt.agent_session_id.clone()),
        child_frame_id: attempt.and_then(|attempt| attempt.child_frame_id.clone()),
        error: attempt
            .and_then(|attempt| attempt.error.clone())
            .or_else(|| Some("The application stopped before this task produced a result.".into())),
    }
}

async fn reconstruct_delivery_result(
    store: &Store,
    delivery: &AgentWorkflowDelivery,
) -> Result<(), String> {
    let workflow = store
        .get_agent_workflow(&delivery.workflow_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Agent workflow disappeared before completion delivery.".to_string())?;
    let plan: wisp_core::DelegationPlan = serde_json::from_str(&workflow.plan_json)
        .map_err(|error| format!("Agent workflow plan is invalid: {error}"))?;
    let attempts = store
        .list_agent_workflow_attempts(&delivery.workflow_id)
        .await
        .map_err(|error| error.to_string())?;
    let by_step = attempts
        .iter()
        .filter(|attempt| attempt.attempt == delivery.generation)
        .map(|attempt| (attempt.step_id.as_str(), attempt))
        .collect::<HashMap<_, _>>();
    let execution = DelegationExecutionResult {
        workflow_id: workflow.id,
        status: execution_status(workflow.status),
        steps: plan
            .steps
            .iter()
            .map(|step| DelegationStepExecution {
                step_id: step.id.clone(),
                response: attempt_response(by_step.get(step.id.as_str()).copied(), &step.id),
            })
            .collect(),
    };
    persist_execution_result(
        store,
        delivery,
        &execution,
        &delegation_tool::display_task_ids(&plan),
    )
    .await
}

async fn repair_incomplete_deliveries(store: &Store) {
    let deliveries = match store.list_incomplete_agent_workflow_deliveries().await {
        Ok(deliveries) => deliveries,
        Err(error) => {
            tracing::warn!(target: "wisp", %error, "failed to inspect Agent completion recovery");
            return;
        }
    };
    for delivery in deliveries {
        if let Err(error) = reconstruct_delivery_result(store, &delivery).await {
            tracing::warn!(target: "wisp", delivery_id = %delivery.id, %error, "failed to recover Agent completion");
        }
    }
}

fn completion_status(result_json: &str) -> String {
    serde_json::from_str::<Value>(result_json)
        .ok()
        .and_then(|value| {
            value
                .get("result")
                .and_then(|result| result.get("status"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "failed".into())
}

pub(crate) fn completion_prompt(deliveries: &[AgentWorkflowDelivery]) -> String {
    let results = deliveries
        .iter()
        .filter_map(|delivery| delivery.result_json.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<background_agent_completions>\n{results}\n</background_agent_completions>\n\
         These background delegated batches have completed. Synthesize the new evidence into one concise user-facing update. Do not claim that still-running work is complete."
    )
}

async fn dispatch_frame(app: AppHandle, frame_id: String) {
    let state = app.state::<AppState>();
    let runtime = {
        let mut sessions = state.sessions.lock().await;
        sessions
            .entry(frame_id.clone())
            .or_insert_with(|| Arc::new(SessionRuntime::new()))
            .clone()
    };
    // The same lock serializes user turns, incremental message persistence,
    // completion insertion, and optional synthesis. A completion that arrives
    // during a turn therefore queues behind it without racing message seqs.
    let workflow_guard = runtime.workflow.clone().lock_owned().await;
    let delivered = match state
        .store
        .deliver_agent_workflow_completions(&frame_id)
        .await
    {
        Ok(delivered) => delivered,
        Err(error) => {
            tracing::warn!(target: "wisp", %frame_id, %error, "failed to deliver Agent completion");
            return;
        }
    };
    if !delivered.is_empty() {
        // Native Agents cache their context. Invalidate it while the shared
        // session lock is held so the delivered internal message is reloaded
        // before either auto-resume or the next user turn.
        *runtime.agent.lock().await = None;
    }
    for delivery in &delivered {
        let result = delivery.result_json.clone().unwrap_or_default();
        let _ = app.emit(
            "agent",
            AgentEvent::DelegationCompleted {
                frame_id: frame_id.clone(),
                workflow_id: delivery.workflow_id.clone(),
                status: completion_status(&result),
                result,
                auto_resume: delivery.auto_resume,
            },
        );
    }
    let claimed = match state
        .store
        .claim_agent_workflow_auto_resumes(&frame_id)
        .await
    {
        Ok(claimed) => claimed,
        Err(error) => {
            tracing::warn!(target: "wisp", %frame_id, %error, "failed to claim Agent auto-resume");
            return;
        }
    };
    if claimed.is_empty() {
        drop(workflow_guard);
        return;
    }
    let ids = claimed
        .iter()
        .map(|delivery| delivery.id.clone())
        .collect::<Vec<_>>();
    let result = crate::send_message_inner(
        state.inner(),
        app.clone(),
        "main",
        Some(frame_id),
        completion_prompt(&claimed),
        None,
        None,
        Some(true),
        None,
        None,
        Some(workflow_guard),
    )
    .await;
    let (success, error) = match &result {
        Ok(_) => (true, None),
        Err(error) => (false, Some(error.as_str())),
    };
    if let Err(error) = state
        .store
        .finish_agent_workflow_auto_resumes(&ids, success, error)
        .await
    {
        tracing::warn!(target: "wisp", %error, "failed to finalize Agent auto-resume");
    }
}

pub(crate) fn start_dispatcher(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<AppState>();
            repair_incomplete_deliveries(&state.store).await;
            let frames = match state
                .store
                .list_ready_agent_workflow_delivery_frames()
                .await
            {
                Ok(frames) => frames,
                Err(error) => {
                    tracing::warn!(target: "wisp", %error, "failed to poll Agent completions");
                    tokio::time::sleep(COMPLETION_POLL_INTERVAL).await;
                    continue;
                }
            };
            for frame_id in frames {
                let mut active = state.completion_dispatches.lock().await;
                if !active.insert(frame_id.clone()) {
                    continue;
                }
                drop(active);
                let app_for_frame = app.clone();
                tauri::async_runtime::spawn(async move {
                    dispatch_frame(app_for_frame.clone(), frame_id.clone()).await;
                    app_for_frame
                        .state::<AppState>()
                        .completion_dispatches
                        .lock()
                        .await
                        .remove(&frame_id);
                });
            }
            tokio::time::sleep(COMPLETION_POLL_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_policy_defaults_inline_and_disables_irrelevant_resume() {
        let default = serde_json::from_str::<AgentCompletionSettings>("{}").unwrap();
        assert_eq!(default.policy, AgentCompletionPolicy::Inline);
        assert!(!default.auto_resume);
        assert_eq!(
            completion_status(r#"{"result":{"status":"succeeded"}}"#),
            "succeeded"
        );
    }

    #[tokio::test]
    async fn busy_parent_queues_delivery_and_idle_parent_claims_resume_once() {
        let path = std::env::temp_dir().join(format!(
            "wisp_background_completion_lock_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store
            .create_frame("f", "p", "Agent", "model")
            .await
            .unwrap();
        store
            .set_setting(
                &completion_setting_key("f"),
                r#"{"policy":"inline","auto_resume":true}"#,
            )
            .await
            .unwrap();
        assert!(!session_completion_settings(&store, "f").await.auto_resume);
        let mut workflow = wisp_store::AgentWorkflow::new("wf", "p", "workspace", "batch").unwrap();
        workflow.frame_id = Some("f".into());
        store.create_agent_workflow(&workflow).await.unwrap();
        assert!(store
            .approve_agent_workflow_plan("wf", workflow.version)
            .await
            .unwrap());
        let delivery = store
            .create_agent_workflow_delivery("wf", true)
            .await
            .unwrap();
        store
            .complete_agent_workflow_delivery(
                &delivery.id,
                r#"{"type":"delegated_batch_completion","result":{"status":"succeeded"}}"#,
            )
            .await
            .unwrap();

        let runtime = Arc::new(SessionRuntime::new());
        let busy_guard = runtime.workflow.clone().lock_owned().await;
        let queued_store = store.clone();
        let queued_runtime = runtime.clone();
        let queued = tokio::spawn(async move {
            let _idle = queued_runtime.workflow.clone().lock_owned().await;
            queued_store
                .deliver_agent_workflow_completions("f")
                .await
                .unwrap()
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(store.message_count("f").await.unwrap(), 0);
        drop(busy_guard);
        assert_eq!(queued.await.unwrap().len(), 1);
        assert_eq!(store.message_count("f").await.unwrap(), 1);

        let claimed = store.claim_agent_workflow_auto_resumes("f").await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(store
            .claim_agent_workflow_auto_resumes("f")
            .await
            .unwrap()
            .is_empty());

        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
