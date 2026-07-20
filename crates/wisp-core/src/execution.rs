//! Dependency-aware execution for validated delegation plans.

use crate::{
    AgentDelegationLineage, AgentDelegationRequest, AgentDelegationResponse, AgentDelegator,
    CapabilityRegistry, DelegationHostPolicy, DelegationPlan, DelegationStatus,
    ValidatedAgentDelegationRequest,
};
use async_trait::async_trait;
use futures_util::{stream::FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

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
    dynamic_policy: Option<(CapabilityRegistry, DelegationHostPolicy)>,
    lineage: Option<AgentDelegationLineage>,
}

impl DelegationExecutor {
    pub fn new(delegator: Arc<dyn AgentDelegator>) -> Self {
        Self {
            delegator,
            observer: Arc::new(NoopDelegationObserver),
            dynamic_policy: None,
            lineage: None,
        }
    }

    pub fn with_observer(mut self, observer: Arc<dyn DelegationExecutionObserver>) -> Self {
        self.observer = observer;
        self
    }

    pub fn with_dynamic_policy(
        mut self,
        registry: CapabilityRegistry,
        host: DelegationHostPolicy,
    ) -> Self {
        self.dynamic_policy = Some((registry, host));
        self
    }

    pub fn with_lineage(mut self, lineage: AgentDelegationLineage) -> Self {
        self.lineage = Some(lineage);
        self
    }

    fn validate_plan(&self, plan: &DelegationPlan) -> anyhow::Result<()> {
        let (registry, host) = self
            .dynamic_policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("dynamic delegation policy is not configured"))?;
        registry.validate_resolved_plan(plan, host)?;
        Ok(())
    }

    fn validate_request(
        &self,
        request: AgentDelegationRequest,
    ) -> anyhow::Result<ValidatedAgentDelegationRequest> {
        let (registry, host) = self
            .dynamic_policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("dynamic delegation policy is not configured"))?;
        ValidatedAgentDelegationRequest::authorize(request, registry, host)
    }

    pub async fn execute(&self, plan: DelegationPlan) -> anyhow::Result<DelegationExecutionResult> {
        self.validate_plan(&plan)?;
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
                        lineage: self.lineage.clone(),
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
        let mut running_mutations = HashSet::<String>::new();
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
                    let request = &requests[step_id];
                    request.spec.dependencies.iter().all(|dependency| {
                        responses
                            .get(dependency)
                            .is_some_and(|response| response.status == DelegationStatus::Succeeded)
                    }) && (!uses_mutation_lane(request) || running_mutations.is_empty())
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
                let request = self.validate_request(request)?;
                self.observer.step_started(request.as_request()).await?;
                running_requests.insert(step_id.clone(), request.as_request().request_id.clone());
                if uses_mutation_lane(request.as_request()) {
                    running_mutations.insert(step_id.clone());
                }
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
                running_mutations.remove(&step_id);
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

fn uses_mutation_lane(request: &AgentDelegationRequest) -> bool {
    !matches!(
        request.spec.workspace_policy,
        Some(crate::AgentWorkspacePolicy::Isolated)
    ) && (request.spec.permissions.write
        || request.spec.permissions.execute
        || matches!(
            request.spec.workspace_policy,
            Some(crate::AgentWorkspacePolicy::SerializedMutation)
        ))
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
    request: ValidatedAgentDelegationRequest,
) -> DelegationFuture {
    Box::pin(async move {
        let raw_request = request.as_request().clone();
        let timeout = raw_request.spec.timeout_secs.map(Duration::from_secs);
        let result = match timeout {
            Some(timeout) => {
                match tokio::time::timeout(timeout, delegator.delegate_authorized(request)).await {
                    Ok(result) => result,
                    Err(_) => {
                        let _ = delegator.cancel(&raw_request.request_id).await;
                        let message = format!(
                            "delegated Agent timed out after {} seconds",
                            timeout.as_secs()
                        );
                        match delegator.status(&raw_request.request_id).await {
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
            None => delegator.delegate_authorized(request).await,
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
                &raw_request.request_id,
                DelegationStatus::Failed,
                format!(
                    "backend returned non-terminal status: {:?}",
                    response.status
                ),
            ),
            Err(error) => failed_response(
                &raw_request.request_id,
                DelegationStatus::Failed,
                error.to_string(),
            ),
        };
        (step_id, raw_request, response)
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
        nested_results: vec![],
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
        AgentBudget, AgentDelegationResponse, AgentExecutorRef, ContextPolicy,
        DelegatedTaskProposal, DelegationMode, ExecutorFeature, ExecutorProfilePolicy,
        ModelProfilePolicy, PermissionSet, ValidatedAgentDelegationRequest,
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
                nested_results: vec![],
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
                nested_results: vec![],
            })
        }
    }

    fn dynamic_policy() -> (CapabilityRegistry, DelegationHostPolicy) {
        (
            CapabilityRegistry::builtins(),
            DelegationHostPolicy {
                revision: "execution-test-v1".into(),
                enabled_capabilities: vec!["reasoning".into()],
                models: vec![ModelProfilePolicy {
                    id: "local".into(),
                    features: vec![],
                    external: false,
                    enabled: true,
                }],
                executors: vec![ExecutorProfilePolicy {
                    executor: AgentExecutorRef::Native,
                    features: vec![],
                    model_ids: vec!["local".into()],
                    enabled: true,
                }],
                default_model_id: Some("local".into()),
                permission_ceiling: PermissionSet::default(),
                context_ceiling: ContextPolicy::default(),
                budget_ceiling: AgentBudget::default(),
                default_timeout_secs: Some(60),
                timeout_ceiling_secs: Some(60),
                auto_safe: true,
                ..DelegationHostPolicy::default()
            },
        )
    }

    fn dynamic_plan() -> DelegationPlan {
        let (registry, host) = dynamic_policy();
        resolve_dynamic_plan(&registry, &host)
    }

    fn resolve_dynamic_plan(
        registry: &CapabilityRegistry,
        host: &DelegationHostPolicy,
    ) -> DelegationPlan {
        registry
            .resolve_plan(
                "reason independently",
                DelegationMode::Automatic,
                1,
                vec![DelegatedTaskProposal {
                    id: "reason".into(),
                    instruction: "Return a concise independent analysis".into(),
                    context_summary: String::new(),
                    depends_on: vec![],
                    capabilities: vec!["reasoning".into()],
                    specialist: None,
                    output_schema: None,
                    isolated: false,
                    model_id: None,
                    executor: None,
                    budget: None,
                    input: serde_json::json!({}),
                }],
                &host,
            )
            .unwrap()
            .into_plan()
    }

    fn fan_in_plan() -> (DelegationPlan, CapabilityRegistry, DelegationHostPolicy) {
        let (registry, host) = dynamic_policy();
        let tasks = [
            ("inspect", vec![]),
            ("research", vec![]),
            ("synthesize", vec!["inspect".into(), "research".into()]),
        ]
        .into_iter()
        .map(|(id, depends_on)| DelegatedTaskProposal {
            id: id.into(),
            instruction: format!("Complete {id}"),
            context_summary: String::new(),
            depends_on,
            capabilities: vec!["reasoning".into()],
            specialist: None,
            output_schema: None,
            isolated: false,
            model_id: None,
            executor: None,
            budget: None,
            input: serde_json::json!({}),
        })
        .collect();
        let plan = registry
            .resolve_plan(
                "parallel analysis with final synthesis",
                DelegationMode::Automatic,
                2,
                tasks,
                &host,
            )
            .unwrap()
            .into_plan();
        (plan, registry, host)
    }

    fn write_plan(isolated: bool) -> (DelegationPlan, CapabilityRegistry, DelegationHostPolicy) {
        let (registry, mut host) = dynamic_policy();
        host.enabled_capabilities.push("project_write".into());
        host.executors[0].features = vec![
            ExecutorFeature::ProjectRead,
            ExecutorFeature::ProjectWrite,
            ExecutorFeature::Isolation,
        ];
        host.permission_ceiling = PermissionSet {
            tools: ["read", "search", "grep", "write", "edit"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            paths: vec!["project://**".into()],
            write: true,
            ..PermissionSet::default()
        };
        let tasks = ["writer-a", "writer-b"]
            .into_iter()
            .map(|id| DelegatedTaskProposal {
                id: id.into(),
                instruction: format!("Update {id}'s independent file"),
                context_summary: String::new(),
                depends_on: vec![],
                capabilities: vec!["project_write".into()],
                specialist: None,
                output_schema: None,
                isolated,
                model_id: None,
                executor: None,
                budget: None,
                input: serde_json::json!({}),
            })
            .collect();
        let plan = registry
            .resolve_plan(
                "write independent files",
                DelegationMode::Manual,
                2,
                tasks,
                &host,
            )
            .unwrap()
            .into_plan();
        (plan, registry, host)
    }

    #[tokio::test]
    async fn scheduler_limits_parallelism_and_runs_fan_in_last() {
        let (plan, registry, host) = fan_in_plan();
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: None,
        });
        let result = DelegationExecutor::new(delegator.clone())
            .with_dynamic_policy(registry, host)
            .execute(plan)
            .await
            .unwrap();
        assert_eq!(result.status, DelegationExecutionStatus::Succeeded);
        assert!(delegator.max_active.load(Ordering::SeqCst) <= 2);
        let calls = delegator.calls.lock().await;
        assert_eq!(calls.last().map(String::as_str), Some("synthesize"));
        assert!(result.steps.last().unwrap().response.output.is_object());
    }

    #[tokio::test]
    async fn scheduler_serializes_shared_workspace_mutations() {
        let (plan, registry, host) = write_plan(false);
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: None,
        });

        let result = DelegationExecutor::new(delegator.clone())
            .with_dynamic_policy(registry, host)
            .execute(plan)
            .await
            .unwrap();

        assert_eq!(result.status, DelegationExecutionStatus::Succeeded);
        assert_eq!(delegator.max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scheduler_runs_isolated_writers_in_parallel() {
        let (plan, registry, host) = write_plan(true);
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: None,
        });

        let result = DelegationExecutor::new(delegator.clone())
            .with_dynamic_policy(registry, host)
            .execute(plan)
            .await
            .unwrap();

        assert_eq!(result.status, DelegationExecutionStatus::Succeeded);
        assert_eq!(delegator.max_active.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn failed_dependency_blocks_fan_in_without_calling_it() {
        let (plan, registry, host) = fan_in_plan();
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: Some("inspect".into()),
        });
        let result = DelegationExecutor::new(delegator.clone())
            .with_dynamic_policy(registry, host)
            .execute(plan)
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
            .any(|step| step == "synthesize"));
    }

    #[tokio::test]
    async fn timeout_preserves_backend_session_provenance() {
        let (registry, mut host) = dynamic_policy();
        host.default_timeout_secs = Some(1);
        host.timeout_ceiling_secs = Some(1);
        let plan = resolve_dynamic_plan(&registry, &host);
        let result = DelegationExecutor::new(Arc::new(TimeoutDelegator))
            .with_dynamic_policy(registry, host)
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
        let (plan, registry, host) = fan_in_plan();
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: None,
        });
        let observer = Arc::new(CancelBeforeStartObserver::default());
        let result = DelegationExecutor::new(delegator.clone())
            .with_dynamic_policy(registry, host)
            .with_observer(observer.clone())
            .execute(plan)
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

    #[tokio::test]
    async fn dynamic_execution_requires_and_uses_explicit_policy_validation() {
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: None,
        });
        let error = DelegationExecutor::new(delegator.clone())
            .execute(dynamic_plan())
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("dynamic delegation policy is not configured"));
        assert!(delegator.calls.lock().await.is_empty());

        let (registry, host) = dynamic_policy();
        let result = DelegationExecutor::new(delegator.clone())
            .with_dynamic_policy(registry, host)
            .execute(dynamic_plan())
            .await
            .unwrap();
        assert_eq!(result.status, DelegationExecutionStatus::Succeeded);
        assert_eq!(delegator.calls.lock().await.as_slice(), ["reason"]);
    }
}
