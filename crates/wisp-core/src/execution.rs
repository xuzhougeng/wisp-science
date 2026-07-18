//! Dependency-aware execution for validated delegation plans.

use crate::{
    AgentDelegationRequest, AgentDelegationResponse, AgentDelegator, AgentTemplateRegistry,
    DelegationPlan, DelegationStatus,
};
use async_trait::async_trait;
use futures_util::{stream::FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationExecutionStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationStepExecution {
    pub step_id: String,
    pub response: AgentDelegationResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationExecutionResult {
    pub workflow_id: String,
    pub status: DelegationExecutionStatus,
    pub steps: Vec<DelegationStepExecution>,
}

#[async_trait]
pub trait DelegationExecutionObserver: Send + Sync {
    async fn workflow_started(&self, _plan: &DelegationPlan) -> anyhow::Result<()> {
        Ok(())
    }

    async fn step_started(&self, _request: &AgentDelegationRequest) -> anyhow::Result<()> {
        Ok(())
    }

    async fn step_finished(
        &self,
        _request: &AgentDelegationRequest,
        _response: &AgentDelegationResponse,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn step_blocked(
        &self,
        _request: &AgentDelegationRequest,
        _reason: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn step_cancelled(
        &self,
        request: &AgentDelegationRequest,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.step_blocked(request, reason).await
    }

    async fn workflow_cancel_requested(&self, _plan: &DelegationPlan) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn workflow_finished(
        &self,
        _plan: &DelegationPlan,
        _status: DelegationExecutionStatus,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct NoopDelegationObserver;

#[async_trait]
impl DelegationExecutionObserver for NoopDelegationObserver {}

pub struct DelegationExecutor {
    delegator: Arc<dyn AgentDelegator>,
    observer: Arc<dyn DelegationExecutionObserver>,
    templates: AgentTemplateRegistry,
}

impl DelegationExecutor {
    pub fn new(delegator: Arc<dyn AgentDelegator>) -> Self {
        Self {
            delegator,
            observer: Arc::new(NoopDelegationObserver),
            templates: AgentTemplateRegistry::builtins(),
        }
    }

    pub fn with_observer(mut self, observer: Arc<dyn DelegationExecutionObserver>) -> Self {
        self.observer = observer;
        self
    }

    pub async fn execute(&self, plan: DelegationPlan) -> anyhow::Result<DelegationExecutionResult> {
        plan.validate(&self.templates)?;
        self.observer.workflow_started(&plan).await?;

        let requests = plan
            .steps
            .iter()
            .map(|step| {
                (
                    step.id.clone(),
                    AgentDelegationRequest {
                        request_id: uuid::Uuid::new_v4().to_string(),
                        workflow_id: plan.id.clone(),
                        step_id: step.id.clone(),
                        spec: step.spec.clone(),
                        input: step.input.clone(),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let mut pending = plan
            .steps
            .iter()
            .map(|step| step.id.clone())
            .collect::<Vec<_>>();
        let mut responses = HashMap::<String, AgentDelegationResponse>::new();
        let mut running = FuturesUnordered::new();
        let mut running_requests = HashMap::<String, String>::new();
        let mut cancellation_applied = false;

        while !pending.is_empty() || !running.is_empty() {
            if !cancellation_applied && self.observer.workflow_cancel_requested(&plan).await? {
                cancellation_applied = true;
                for request_id in running_requests.values() {
                    let _ = self.delegator.cancel(request_id).await;
                }
                for step_id in pending.drain(..) {
                    let request = &requests[&step_id];
                    let reason = "Agent workflow cancellation was requested".to_string();
                    self.observer.step_cancelled(request, &reason).await?;
                    responses.insert(
                        step_id,
                        failed_response(&request.request_id, DelegationStatus::Cancelled, reason),
                    );
                }
            }
            let mut index = 0;
            while index < pending.len() {
                let step_id = &pending[index];
                let request = &requests[step_id];
                let blocked_by = request.spec.dependencies.iter().find(|dependency| {
                    responses
                        .get(*dependency)
                        .is_some_and(|response| response.status != DelegationStatus::Succeeded)
                });
                if let Some(dependency) = blocked_by {
                    let request = requests[step_id].clone();
                    let reason = format!("dependency {dependency} did not succeed");
                    let response = failed_response(
                        &request.request_id,
                        DelegationStatus::Blocked,
                        reason.clone(),
                    );
                    self.observer.step_blocked(&request, &reason).await?;
                    responses.insert(step_id.clone(), response);
                    pending.remove(index);
                } else {
                    index += 1;
                }
            }

            while running.len() < plan.max_parallel {
                let Some(index) = pending.iter().position(|step_id| {
                    requests[step_id]
                        .spec
                        .dependencies
                        .iter()
                        .all(|dependency| {
                            responses.get(dependency).is_some_and(|response| {
                                response.status == DelegationStatus::Succeeded
                            })
                        })
                }) else {
                    break;
                };
                let step_id = pending.remove(index);
                let mut request = requests[&step_id].clone();
                attach_dependency_results(
                    &mut request.input,
                    &request.spec.dependencies,
                    &responses,
                );
                self.observer.step_started(&request).await?;
                running_requests.insert(step_id.clone(), request.request_id.clone());
                running.push(run_request(self.delegator.clone(), step_id, request));
            }

            if running.is_empty() {
                if pending.is_empty() {
                    break;
                }
                anyhow::bail!("delegation plan scheduler made no progress");
            }

            let next = tokio::select! {
                value = running.next() => value,
                _ = tokio::time::sleep(Duration::from_millis(100)), if !cancellation_applied => continue,
            };
            if let Some((step_id, request, response)) = next {
                running_requests.remove(&step_id);
                self.observer.step_finished(&request, &response).await?;
                responses.insert(step_id, response);
            }
        }

        let status = if responses
            .values()
            .all(|response| response.status == DelegationStatus::Succeeded)
        {
            DelegationExecutionStatus::Succeeded
        } else if responses
            .values()
            .any(|response| response.status == DelegationStatus::Cancelled)
        {
            DelegationExecutionStatus::Cancelled
        } else {
            DelegationExecutionStatus::Failed
        };
        self.observer.workflow_finished(&plan, status).await?;
        Ok(DelegationExecutionResult {
            workflow_id: plan.id.clone(),
            status,
            steps: plan
                .steps
                .iter()
                .map(|step| DelegationStepExecution {
                    step_id: step.id.clone(),
                    response: responses
                        .remove(&step.id)
                        .expect("validated plan step must have a terminal response"),
                })
                .collect(),
        })
    }
}

type DelegationFuture = Pin<
    Box<
        dyn Future<Output = (String, AgentDelegationRequest, AgentDelegationResponse)>
            + Send
            + 'static,
    >,
>;

fn run_request(
    delegator: Arc<dyn AgentDelegator>,
    step_id: String,
    request: AgentDelegationRequest,
) -> DelegationFuture {
    Box::pin(async move {
        let timeout = request.spec.timeout_secs.map(Duration::from_secs);
        let result = match timeout {
            Some(timeout) => {
                match tokio::time::timeout(timeout, delegator.delegate(request.clone())).await {
                    Ok(result) => result,
                    Err(_) => {
                        let _ = delegator.cancel(&request.request_id).await;
                        let message = format!(
                            "delegated Agent timed out after {} seconds",
                            timeout.as_secs()
                        );
                        match delegator.status(&request.request_id).await {
                            Ok(Some(mut response)) => {
                                response.status = DelegationStatus::Failed;
                                response.output = Value::Object(Map::new());
                                response.error = Some(message);
                                Ok(response)
                            }
                            _ => Err(anyhow::anyhow!(message)),
                        }
                    }
                }
            }
            None => delegator.delegate(request.clone()).await,
        };
        let response = match result {
            Ok(response)
                if matches!(
                    response.status,
                    DelegationStatus::Succeeded
                        | DelegationStatus::Failed
                        | DelegationStatus::Cancelled
                        | DelegationStatus::Blocked
                ) =>
            {
                response
            }
            Ok(response) => failed_response(
                &request.request_id,
                DelegationStatus::Failed,
                format!(
                    "backend returned non-terminal status: {:?}",
                    response.status
                ),
            ),
            Err(error) => failed_response(
                &request.request_id,
                DelegationStatus::Failed,
                error.to_string(),
            ),
        };
        (step_id, request, response)
    })
}

fn failed_response(
    request_id: &str,
    status: DelegationStatus,
    error: String,
) -> AgentDelegationResponse {
    AgentDelegationResponse {
        request_id: request_id.into(),
        status,
        output: Value::Object(Map::new()),
        artifact_ids: vec![],
        artifacts: vec![],
        evidence: vec![],
        usage: Default::default(),
        agent_session_id: None,
        child_frame_id: None,
        error: Some(error),
    }
}

fn attach_dependency_results(
    input: &mut Value,
    dependencies: &[String],
    responses: &HashMap<String, AgentDelegationResponse>,
) {
    if dependencies.is_empty() {
        return;
    }
    let Some(input) = input.as_object_mut() else {
        return;
    };
    let dependency_results = dependencies
        .iter()
        .filter_map(|dependency| {
            responses
                .get(dependency)
                .map(|response| (dependency.clone(), response.output.clone()))
        })
        .collect::<Map<_, _>>();
    input.insert(
        "dependency_results".into(),
        Value::Object(dependency_results),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentDelegationResponse, DelegationMode, DelegationPlanner, ValidatedAgentDelegationRequest,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    struct RecordingDelegator {
        active: AtomicUsize,
        max_active: AtomicUsize,
        calls: Mutex<Vec<String>>,
        fail: Option<String>,
    }

    struct TimeoutDelegator;

    #[derive(Default)]
    struct CancelBeforeStartObserver {
        cancelled_steps: AtomicUsize,
    }

    #[async_trait]
    impl DelegationExecutionObserver for CancelBeforeStartObserver {
        async fn workflow_cancel_requested(&self, _plan: &DelegationPlan) -> anyhow::Result<bool> {
            Ok(true)
        }

        async fn step_cancelled(
            &self,
            _request: &AgentDelegationRequest,
            _reason: &str,
        ) -> anyhow::Result<()> {
            self.cancelled_steps.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait]
    impl AgentDelegator for TimeoutDelegator {
        async fn delegate_validated(
            &self,
            _request: ValidatedAgentDelegationRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            std::future::pending().await
        }

        async fn status(
            &self,
            request_id: &str,
        ) -> anyhow::Result<Option<AgentDelegationResponse>> {
            Ok(Some(AgentDelegationResponse {
                request_id: request_id.into(),
                status: DelegationStatus::Running,
                output: serde_json::json!({}),
                artifact_ids: vec![],
                artifacts: vec![],
                evidence: vec![],
                usage: Default::default(),
                agent_session_id: Some("session".into()),
                child_frame_id: Some("frame".into()),
                error: None,
            }))
        }

        async fn cancel(&self, _request_id: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    #[async_trait]
    impl AgentDelegator for RecordingDelegator {
        async fn delegate_validated(
            &self,
            request: ValidatedAgentDelegationRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            let request = request.into_request();
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            self.calls.lock().await.push(request.step_id.clone());
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            if self.fail.as_deref() == Some(request.step_id.as_str()) {
                anyhow::bail!("intentional backend failure");
            }
            Ok(AgentDelegationResponse {
                request_id: request.request_id,
                status: DelegationStatus::Succeeded,
                output: serde_json::json!({"step":request.step_id}),
                artifact_ids: vec![],
                artifacts: vec![],
                evidence: vec![],
                usage: Default::default(),
                agent_session_id: None,
                child_frame_id: None,
                error: None,
            })
        }
    }

    fn parallel_plan() -> DelegationPlan {
        DelegationPlanner
            .suggest(
                "analyze biology genes and create a visualization figure",
                DelegationMode::Automatic,
                "context",
                &[],
                &[],
                &AgentTemplateRegistry::builtins(),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn scheduler_limits_parallelism_and_runs_reviewer_last() {
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: None,
        });
        let result = DelegationExecutor::new(delegator.clone())
            .execute(parallel_plan())
            .await
            .unwrap();
        assert_eq!(result.status, DelegationExecutionStatus::Succeeded);
        assert!(delegator.max_active.load(Ordering::SeqCst) <= 2);
        let calls = delegator.calls.lock().await;
        assert_eq!(calls.last().map(String::as_str), Some("reviewer"));
        let reviewer = result.steps.last().unwrap();
        assert!(reviewer.response.output.is_object());
    }

    #[tokio::test]
    async fn failed_dependency_blocks_reviewer_without_calling_it() {
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: Some("biology_interpreter".into()),
        });
        let result = DelegationExecutor::new(delegator.clone())
            .execute(parallel_plan())
            .await
            .unwrap();
        assert_eq!(result.status, DelegationExecutionStatus::Failed);
        assert_eq!(
            result.steps.last().unwrap().response.status,
            DelegationStatus::Blocked
        );
        assert!(!delegator
            .calls
            .lock()
            .await
            .iter()
            .any(|step| step == "reviewer"));
    }

    #[tokio::test]
    async fn timeout_preserves_backend_session_provenance() {
        let mut plan = parallel_plan();
        plan.steps.truncate(1);
        plan.steps[0].spec.dependencies.clear();
        plan.steps[0].spec.timeout_secs = Some(1);
        let result = DelegationExecutor::new(Arc::new(TimeoutDelegator))
            .execute(plan)
            .await
            .unwrap();
        let response = &result.steps[0].response;
        assert_eq!(response.status, DelegationStatus::Failed);
        assert!(response.error.as_deref().unwrap().contains("timed out"));
        assert_eq!(response.agent_session_id.as_deref(), Some("session"));
        assert_eq!(response.child_frame_id.as_deref(), Some("frame"));
    }

    #[tokio::test]
    async fn persisted_cancellation_prevents_pending_steps_from_starting() {
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: None,
        });
        let observer = Arc::new(CancelBeforeStartObserver::default());
        let result = DelegationExecutor::new(delegator.clone())
            .with_observer(observer.clone())
            .execute(parallel_plan())
            .await
            .unwrap();
        assert_eq!(result.status, DelegationExecutionStatus::Cancelled);
        assert!(delegator.calls.lock().await.is_empty());
        assert_eq!(
            observer.cancelled_steps.load(Ordering::SeqCst),
            result.steps.len()
        );
        assert!(result
            .steps
            .iter()
            .all(|step| step.response.status == DelegationStatus::Cancelled));
    }
}
