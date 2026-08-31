//! Dependency-aware execution for validated delegation plans.

use crate::{
    AgentDelegationLineage, AgentDelegationRequest, AgentDelegationResponse, AgentDelegator,
    CapabilityRegistry, DelegationHostPolicy, DelegationPlan, DelegationStatus, RunActivitySpec,
    ValidatedAgentDelegationRequest, WorkflowTaskKind,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRunActivityRequest {
    pub request_id: String,
    pub workflow_id: String,
    pub step_id: String,
    pub activity: RunActivitySpec,
    pub input: Value,
}

#[async_trait]
pub trait WorkflowRunActivityDriver: Send + Sync {
    async fn execute(
        &self,
        request: WorkflowRunActivityRequest,
    ) -> anyhow::Result<AgentDelegationResponse>;

    async fn cancel(&self, request_id: &str) -> anyhow::Result<bool>;
}

#[async_trait]
pub trait DelegationExecutionObserver: Send + Sync {
    async fn workflow_started(&self, _plan: &DelegationPlan) -> anyhow::Result<()> {
        Ok(())
    }

    /// Resume a persisted running Workflow after every supplied prior step has
    /// already reached a durable terminal state.
    async fn workflow_resumed(&self, _plan: &DelegationPlan) -> anyhow::Result<()> {
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
    run_activity_driver: Option<Arc<dyn WorkflowRunActivityDriver>>,
}

impl DelegationExecutor {
    pub fn new(delegator: Arc<dyn AgentDelegator>) -> Self {
        Self {
            delegator,
            observer: Arc::new(NoopDelegationObserver),
            dynamic_policy: None,
            lineage: None,
            run_activity_driver: None,
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

    pub fn with_run_activity_driver(mut self, driver: Arc<dyn WorkflowRunActivityDriver>) -> Self {
        self.run_activity_driver = Some(driver);
        self
    }

    fn validate_plan(&self, plan: &DelegationPlan) -> anyhow::Result<()> {
        let (registry, host) = self
            .dynamic_policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("dynamic delegation policy is not configured"))?;
        registry.validate_resolved_plan(plan, host)?;
        if plan
            .steps
            .iter()
            .any(|step| step.task_kind == WorkflowTaskKind::RunActivity)
            && self.run_activity_driver.is_none()
        {
            anyhow::bail!("Workflow Run activity driver is not configured");
        }
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
        self.execute_inner(plan, HashMap::new(), false).await
    }

    pub async fn execute_with_completed_steps(
        &self,
        plan: DelegationPlan,
        completed_steps: Vec<DelegationStepExecution>,
    ) -> anyhow::Result<DelegationExecutionResult> {
        let mut completed = HashMap::new();
        for step in completed_steps {
            if step.response.status != DelegationStatus::Succeeded
                || completed
                    .insert(step.step_id.clone(), step.response)
                    .is_some()
            {
                anyhow::bail!("completed Workflow steps must be unique and successful");
            }
        }
        for response in completed.values() {
            response.validate()?;
        }
        self.execute_inner(plan, completed, false).await
    }

    pub async fn resume(
        &self,
        plan: DelegationPlan,
        prior_steps: Vec<DelegationStepExecution>,
    ) -> anyhow::Result<DelegationExecutionResult> {
        let mut prior = HashMap::new();
        for step in prior_steps {
            if !matches!(
                step.response.status,
                DelegationStatus::Succeeded
                    | DelegationStatus::Failed
                    | DelegationStatus::Cancelled
                    | DelegationStatus::Blocked
            ) || prior.insert(step.step_id.clone(), step.response).is_some()
            {
                anyhow::bail!("resumed Workflow prior steps must be unique and terminal");
            }
        }
        self.execute_inner(plan, prior, true).await
    }

    async fn execute_inner(
        &self,
        plan: DelegationPlan,
        mut responses: HashMap<String, AgentDelegationResponse>,
        resumed: bool,
    ) -> anyhow::Result<DelegationExecutionResult> {
        self.validate_plan(&plan)?;
        if responses
            .keys()
            .any(|step_id| !plan.steps.iter().any(|step| step.id == *step_id))
        {
            anyhow::bail!("resumed Workflow contains an unknown prior step");
        }
        if resumed {
            self.observer.workflow_resumed(&plan).await?;
        } else {
            self.observer.workflow_started(&plan).await?;
        }

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
        let step_kinds = plan
            .steps
            .iter()
            .map(|step| (step.id.clone(), step.task_kind))
            .collect::<HashMap<_, _>>();
        let run_activities = plan
            .steps
            .iter()
            .filter_map(|step| {
                step.run_activity
                    .clone()
                    .map(|activity| (step.id.clone(), activity))
            })
            .collect::<HashMap<_, _>>();
        let mut pending = plan
            .steps
            .iter()
            .filter(|step| !responses.contains_key(&step.id))
            .map(|step| step.id.clone())
            .collect::<Vec<_>>();
        let mut running = FuturesUnordered::new();
        let mut running_requests = HashMap::<String, String>::new();
        let mut running_kinds = HashMap::<String, WorkflowTaskKind>::new();
        let mut running_agents = 0usize;
        let mut running_mutations = HashSet::<String>::new();
        let mut cancellation_applied = false;

        while !pending.is_empty() || !running.is_empty() {
            if !cancellation_applied && self.observer.workflow_cancel_requested(&plan).await? {
                cancellation_applied = true;
                for (step_id, request_id) in &running_requests {
                    match running_kinds
                        .get(step_id)
                        .copied()
                        .unwrap_or(WorkflowTaskKind::Agent)
                    {
                        WorkflowTaskKind::Agent => {
                            let _ = self.delegator.cancel(request_id).await;
                        }
                        WorkflowTaskKind::RunActivity => {
                            if let Some(driver) = &self.run_activity_driver {
                                let _ = driver.cancel(request_id).await;
                            }
                        }
                    }
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

            loop {
                let Some(index) = pending.iter().position(|step_id| {
                    let request = &requests[step_id];
                    let kind = step_kinds
                        .get(step_id)
                        .copied()
                        .unwrap_or(WorkflowTaskKind::Agent);
                    request.spec.dependencies.iter().all(|dependency| {
                        responses
                            .get(dependency)
                            .is_some_and(|response| response.status == DelegationStatus::Succeeded)
                    }) && (kind == WorkflowTaskKind::RunActivity
                        || (running_agents < plan.max_parallel
                            && (!uses_mutation_lane(request) || running_mutations.is_empty())))
                }) else {
                    break;
                };
                let step_id = pending.remove(index);
                let mut request = requests[&step_id].clone();
                attach_dependency_results(
                    &mut request.input,
                    &request.spec.dependencies,
                    &responses,
                    &requests,
                );
                let kind = step_kinds
                    .get(&step_id)
                    .copied()
                    .unwrap_or(WorkflowTaskKind::Agent);
                self.observer.step_started(&request).await?;
                running_requests.insert(step_id.clone(), request.request_id.clone());
                running_kinds.insert(step_id.clone(), kind);
                match kind {
                    WorkflowTaskKind::Agent => {
                        let request = self.validate_request(request)?;
                        if uses_mutation_lane(request.as_request()) {
                            running_mutations.insert(step_id.clone());
                        }
                        running_agents += 1;
                        running.push(run_agent_request(self.delegator.clone(), step_id, request));
                    }
                    WorkflowTaskKind::RunActivity => {
                        let activity = run_activities
                            .get(&step_id)
                            .cloned()
                            .ok_or_else(|| anyhow::anyhow!("Run activity metadata is missing"))?;
                        let driver =
                            self.run_activity_driver.as_ref().cloned().ok_or_else(|| {
                                anyhow::anyhow!("Workflow Run activity driver is not configured")
                            })?;
                        running.push(run_activity_request(driver, step_id, request, activity));
                    }
                }
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
            if let Some((step_id, request, response, kind)) = next {
                running_requests.remove(&step_id);
                running_kinds.remove(&step_id);
                if kind == WorkflowTaskKind::Agent {
                    running_agents = running_agents.saturating_sub(1);
                }
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
        dyn Future<
                Output = (
                    String,
                    AgentDelegationRequest,
                    AgentDelegationResponse,
                    WorkflowTaskKind,
                ),
            > + Send
            + 'static,
    >,
>;

fn run_agent_request(
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
        (step_id, raw_request, response, WorkflowTaskKind::Agent)
    })
}

fn run_activity_request(
    driver: Arc<dyn WorkflowRunActivityDriver>,
    step_id: String,
    request: AgentDelegationRequest,
    activity: RunActivitySpec,
) -> DelegationFuture {
    Box::pin(async move {
        let activity_request = WorkflowRunActivityRequest {
            request_id: request.request_id.clone(),
            workflow_id: request.workflow_id.clone(),
            step_id: request.step_id.clone(),
            activity,
            input: request.input.clone(),
        };
        let response = match driver.execute(activity_request).await {
            Ok(response)
                if matches!(
                    response.status,
                    DelegationStatus::Succeeded
                        | DelegationStatus::Failed
                        | DelegationStatus::Cancelled
                ) =>
            {
                response
            }
            Ok(response) => failed_response(
                &request.request_id,
                DelegationStatus::Failed,
                format!(
                    "Run activity returned non-terminal status: {:?}",
                    response.status
                ),
            ),
            Err(error) => failed_response(
                &request.request_id,
                DelegationStatus::Failed,
                error.to_string(),
            ),
        };
        (step_id, request, response, WorkflowTaskKind::RunActivity)
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
    requests: &HashMap<String, AgentDelegationRequest>,
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
    let dependency_skill_sources = dependencies
        .iter()
        .filter_map(|dependency| {
            requests.get(dependency).map(|request| {
                (
                    dependency.clone(),
                    serde_json::to_value(&request.spec.skill_bindings)
                        .unwrap_or_else(|_| Value::Array(vec![])),
                )
            })
        })
        .collect::<Map<_, _>>();
    input.insert(
        "dependency_skill_sources".into(),
        Value::Object(dependency_skill_sources),
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
    use tokio::sync::{Mutex, Notify};

    struct RecordingDelegator {
        active: AtomicUsize,
        max_active: AtomicUsize,
        calls: Mutex<Vec<String>>,
        fail: Option<String>,
    }

    struct TimeoutDelegator;

    struct RunActivityConcurrencyDelegator {
        independent_started: Arc<Notify>,
    }

    struct WaitingRunActivityDriver {
        independent_started: Arc<Notify>,
    }

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
    impl AgentDelegator for RunActivityConcurrencyDelegator {
        async fn delegate_validated(
            &self,
            request: ValidatedAgentDelegationRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            let request = request.as_request();
            if request.step_id == "independent" {
                self.independent_started.notify_one();
            }
            Ok(AgentDelegationResponse {
                request_id: request.request_id.clone(),
                status: DelegationStatus::Succeeded,
                output: if request.step_id == "prep" {
                    serde_json::json!({"method_search_spec_artifact_version_id":"spec-v1"})
                } else {
                    serde_json::json!({"summary":"done"})
                },
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

        async fn status(
            &self,
            _request_id: &str,
        ) -> anyhow::Result<Option<AgentDelegationResponse>> {
            Ok(None)
        }

        async fn cancel(&self, _request_id: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    #[async_trait]
    impl WorkflowRunActivityDriver for WaitingRunActivityDriver {
        async fn execute(
            &self,
            request: WorkflowRunActivityRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            tokio::time::timeout(Duration::from_secs(2), self.independent_started.notified())
                .await
                .map_err(|_| anyhow::anyhow!("Agent capacity was not released while Run waited"))?;
            Ok(AgentDelegationResponse {
                request_id: request.request_id,
                status: DelegationStatus::Succeeded,
                output: serde_json::json!({"run_id":"method-run","run_status":"succeeded"}),
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
                    skill_bindings: vec![],
                    specialist: None,
                    output_schema: None,
                    isolated: false,
                    model_id: None,
                    executor: None,
                    budget: None,
                    timeout_secs: None,
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
            skill_bindings: vec![],
            specialist: None,
            output_schema: None,
            isolated: false,
            model_id: None,
            executor: None,
            budget: None,
            timeout_secs: None,
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
                skill_bindings: vec![],
                specialist: None,
                output_schema: None,
                isolated,
                model_id: None,
                executor: None,
                budget: None,
                timeout_secs: None,
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

    fn run_activity_plan() -> (DelegationPlan, CapabilityRegistry, DelegationHostPolicy) {
        let (registry, host) = dynamic_policy();
        let mut plan = registry
            .resolve_plan(
                "prepare, search, and continue independent work",
                DelegationMode::Manual,
                1,
                vec![
                    DelegatedTaskProposal {
                        id: "prep".into(),
                        instruction: "Prepare an audited method-search spec".into(),
                        context_summary: String::new(),
                        depends_on: vec![],
                        capabilities: vec!["reasoning".into()],
                        skill_bindings: vec![],
                        specialist: None,
                        output_schema: Some(serde_json::json!({
                            "type":"object",
                            "required":["method_search_spec_artifact_version_id"],
                            "properties":{"method_search_spec_artifact_version_id":{"type":"string"}}
                        })),
                        isolated: false,
                        model_id: None,
                        executor: None,
                        budget: None,
                        timeout_secs: None,
                        input: serde_json::json!({}),
                    },
                    DelegatedTaskProposal {
                        id: "independent".into(),
                        instruction: "Continue independent analysis".into(),
                        context_summary: String::new(),
                        depends_on: vec!["prep".into()],
                        capabilities: vec!["reasoning".into()],
                        skill_bindings: vec![],
                        specialist: None,
                        output_schema: None,
                        isolated: false,
                        model_id: None,
                        executor: None,
                        budget: None,
                        timeout_secs: None,
                        input: serde_json::json!({}),
                    },
                ],
                &host,
            )
            .unwrap()
            .into_plan();
        let mut activity = RunActivitySpec {
            activity: "method_search".into(),
            context_id: "local".into(),
            context_revision: "a".repeat(64),
            input_task_id: "prep".into(),
            spec_output_pointer: "method_search_spec_artifact_version_id".into(),
            max_candidates: 20,
            max_wall_seconds: 3_600,
            max_evaluator_seconds: 60,
            max_cost_microunits: 1_000_000,
            provider_profile_id: None,
            model_profile_id: Some("local".into()),
            approval_reasons: vec!["Execute bounded method search".into()],
            integrity_hash: String::new(),
        };
        activity.seal().unwrap();
        let mut activity_spec = plan.steps[0].spec.clone();
        activity_spec.agent_id = "activity".into();
        activity_spec.name = "activity".into();
        activity_spec.goal = "Run the method search".into();
        activity_spec.dependencies = vec!["prep".into()];
        activity_spec.capabilities.clear();
        activity_spec.skill_bindings.clear();
        activity_spec.executor = None;
        activity_spec.request_preferences = None;
        activity_spec.authorization = None;
        plan.steps.insert(
            1,
            crate::DelegationPlanStep {
                id: "activity".into(),
                spec: activity_spec,
                input: serde_json::json!({}),
                task_kind: WorkflowTaskKind::RunActivity,
                run_activity: Some(activity),
            },
        );
        plan.requires_confirmation = true;
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
    async fn resume_keeps_succeeded_steps_and_only_runs_the_remaining_dag() {
        let (plan, registry, host) = fan_in_plan();
        let delegator = Arc::new(RecordingDelegator {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
            fail: None,
        });
        let prior = DelegationStepExecution {
            step_id: "inspect".into(),
            response: AgentDelegationResponse {
                request_id: "persisted-inspect".into(),
                status: DelegationStatus::Succeeded,
                output: serde_json::json!({"step":"inspect","persisted":true}),
                artifact_ids: vec![],
                artifacts: vec![],
                evidence: vec![],
                usage: Default::default(),
                agent_session_id: None,
                child_frame_id: None,
                error: None,
                nested_results: vec![],
            },
        };
        let result = DelegationExecutor::new(delegator.clone())
            .with_dynamic_policy(registry, host)
            .resume(plan, vec![prior])
            .await
            .unwrap();
        assert_eq!(result.status, DelegationExecutionStatus::Succeeded);
        let calls = delegator.calls.lock().await;
        assert!(!calls.iter().any(|step| step == "inspect"));
        assert!(calls.iter().any(|step| step == "research"));
        assert_eq!(calls.last().map(String::as_str), Some("synthesize"));
        assert_eq!(result.steps[0].response.request_id, "persisted-inspect");
    }

    #[tokio::test]
    async fn waiting_run_activity_does_not_consume_agent_parallelism() {
        let (plan, registry, host) = run_activity_plan();
        let independent_started = Arc::new(Notify::new());
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            DelegationExecutor::new(Arc::new(RunActivityConcurrencyDelegator {
                independent_started: independent_started.clone(),
            }))
            .with_dynamic_policy(registry, host)
            .with_run_activity_driver(Arc::new(WaitingRunActivityDriver {
                independent_started,
            }))
            .execute(plan),
        )
        .await
        .expect("Run activity should release Agent capacity")
        .unwrap();
        assert_eq!(result.status, DelegationExecutionStatus::Succeeded);
        assert_eq!(result.steps.len(), 3);
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
