//! Parent-facing delegation tools. `delegate_tasks` resolves and runs a
//! dynamic batch inline; `propose_delegation` remains for v1 UI compatibility.

use crate::{
    delegation_runtime,
    dynamic_workflow::{
        self, AgentApprovalPolicy, DynamicAgentTaskProposal, DynamicAgentWorkflowProposal,
        MAX_CONTEXT_CHARS, MAX_GOAL_CHARS, MAX_INSTRUCTION_CHARS,
    },
    run_context::RunManager,
    specialists, ActiveProject,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use wisp_core::{
    AgentDelegator, CapabilityRegistry, DelegationExecutionResult, DelegationHostPolicy,
    MAX_DELEGATION_TASKS,
};
use wisp_llm::ToolSchema;
use wisp_store::Store;
use wisp_tools::{ConfirmDecision, Tool, ToolEnv, ToolResult};

const INLINE_DATA_BYTES: usize = 4_000;
const INLINE_TEXT_CHARS: usize = 1_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateTasksArgs {
    goal: String,
    #[serde(default)]
    context: String,
    tasks: Vec<DelegateTaskInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateTaskInput {
    id: String,
    instruction: String,
    #[serde(default)]
    depends_on: Vec<String>,
    capabilities: Vec<String>,
    #[serde(default)]
    specialist_id: Option<String>,
    #[serde(default)]
    output_schema: Option<Value>,
    #[serde(default)]
    isolated: bool,
}

pub(crate) async fn delegate_tasks_schema(store: &Store) -> Result<ToolSchema, String> {
    let (registry, host) = delegation_runtime::dynamic_delegation_policy(store).await?;
    let specialists = specialists::ensure(store).await;
    Ok(build_delegate_tasks_schema(&registry, &host, &specialists))
}

fn build_delegate_tasks_schema(
    registry: &CapabilityRegistry,
    host: &DelegationHostPolicy,
    specialists: &[specialists::Specialist],
) -> ToolSchema {
    let capabilities = registry.available_ids(host);
    let capability_help = capabilities
        .iter()
        .filter_map(|id| {
            registry
                .get(id)
                .map(|definition| format!("{id}: {}", definition.description))
        })
        .collect::<Vec<_>>()
        .join("; ");
    let specialist_ids = specialists
        .iter()
        .map(|specialist| specialist.id.clone())
        .collect::<Vec<_>>();
    let specialist_help = specialists
        .iter()
        .map(|specialist| {
            let description = specialist.description.trim();
            if description.is_empty() {
                format!("{}: {}", specialist.id, specialist.name)
            } else {
                format!("{}: {} — {description}", specialist.id, specialist.name)
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    ToolSchema::new(
        "delegate_tasks",
        "Run a bounded batch of temporary Wisp sub-Agents and return their results to this turn. Decompose the work yourself; independent tasks run in parallel, dependencies run after their prerequisites, and you must synthesize the returned evidence into your final answer. Use the smallest useful batch. Do not delegate trivial work or delegate again from a child.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "goal": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_GOAL_CHARS,
                    "description": "Overall delegated outcome that the parent Agent will synthesize"
                },
                "context": {
                    "type": "string",
                    "maxLength": MAX_CONTEXT_CHARS,
                    "description": "Bounded shared constraints and project state needed by every child; do not paste the full transcript"
                },
                "tasks": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_DELEGATION_TASKS,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "id": {
                                "type": "string",
                                "pattern": "^[a-z][a-z0-9_-]{0,30}$",
                                "description": "Unique stable task id used by dependencies and results"
                            },
                            "instruction": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_INSTRUCTION_CHARS,
                                "description": "Self-contained assignment with concrete output and acceptance criteria"
                            },
                            "depends_on": {
                                "type": "array",
                                "uniqueItems": true,
                                "items": {
                                    "type": "string",
                                    "pattern": "^[a-z][a-z0-9_-]{0,30}$"
                                },
                                "description": "Task ids from this same batch that must succeed first"
                            },
                            "capabilities": {
                                "type": "array",
                                "minItems": 1,
                                "uniqueItems": true,
                                "items": {"type": "string", "enum": capabilities},
                                "description": capability_help
                            },
                            "specialist_id": {
                                "type": "string",
                                "enum": specialist_ids,
                                "description": format!("Optional reusable persona. Omit for a temporary generic Agent. {specialist_help}")
                            },
                            "output_schema": {
                                "type": "object",
                                "description": "Optional bounded JSON Schema for this task's data result"
                            },
                            "isolated": {
                                "type": "boolean",
                                "description": "Request an isolated writable workspace; rejected when no eligible isolated executor exists"
                            }
                        },
                        "required": ["id", "instruction", "capabilities"]
                    }
                }
            },
            "required": ["goal", "tasks"]
        }),
    )
}

pub(crate) struct DelegateTasksTool {
    store: Store,
    project: ActiveProject,
    frame_id: String,
    run_manager: RunManager,
    app_data: PathBuf,
    schema: ToolSchema,
    policy_override: Option<(CapabilityRegistry, DelegationHostPolicy)>,
    delegator_override: Option<Arc<dyn AgentDelegator>>,
}

impl DelegateTasksTool {
    pub(crate) async fn new(
        store: Store,
        project: ActiveProject,
        frame_id: impl Into<String>,
        run_manager: RunManager,
        app_data: PathBuf,
    ) -> Result<Self, String> {
        let schema = delegate_tasks_schema(&store).await?;
        Ok(Self {
            store,
            project,
            frame_id: frame_id.into(),
            run_manager,
            app_data,
            schema,
            policy_override: None,
            delegator_override: None,
        })
    }

    #[cfg(test)]
    fn with_runtime(
        store: Store,
        project: ActiveProject,
        frame_id: impl Into<String>,
        policy: (CapabilityRegistry, DelegationHostPolicy),
        delegator: Arc<dyn AgentDelegator>,
    ) -> Self {
        let schema = build_delegate_tasks_schema(&policy.0, &policy.1, &[]);
        let app_data = project.root.clone();
        Self {
            store,
            project,
            frame_id: frame_id.into(),
            run_manager: RunManager::new(),
            app_data,
            schema,
            policy_override: Some(policy),
            delegator_override: Some(delegator),
        }
    }

    async fn policy(&self) -> Result<(CapabilityRegistry, DelegationHostPolicy), String> {
        match &self.policy_override {
            Some(policy) => Ok(policy.clone()),
            None => delegation_runtime::dynamic_delegation_policy(&self.store).await,
        }
    }
}

#[async_trait]
impl Tool for DelegateTasksTool {
    fn name(&self) -> &str {
        "delegate_tasks"
    }

    fn schema(&self) -> ToolSchema {
        self.schema.clone()
    }

    fn preview(&self, args: &Value) -> String {
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let count = args
            .get("tasks")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        format!("{count} tasks · {goal}")
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        match self.run_batch(args, env).await {
            Ok(value) => ToolResult::ok(serde_json::to_string(&value).unwrap_or_else(|_| {
                r#"{"status":"failed","error":"result serialization failed"}"#.into()
            })),
            Err(error) => ToolResult::fail(format!("delegate_tasks error: {error}")),
        }
    }
}

impl DelegateTasksTool {
    async fn run_batch(&self, args: &Value, env: &dyn ToolEnv) -> Result<Value, String> {
        delegation_runtime::require_session_delegation(
            &self.store,
            &self.project.id,
            &self.frame_id,
        )
        .await?;
        let parsed: DelegateTasksArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid task batch: {error}"))?;
        let workflow_id = uuid::Uuid::new_v4().to_string();
        let (registry, host) = self.policy().await?;
        let proposal = DynamicAgentWorkflowProposal {
            goal: parsed.goal,
            context: parsed.context,
            approval_policy: AgentApprovalPolicy::AutoSafe,
            tasks: parsed
                .tasks
                .into_iter()
                .map(|task| DynamicAgentTaskProposal {
                    id: task.id,
                    instruction: task.instruction,
                    depends_on: task.depends_on,
                    capabilities: task.capabilities,
                    specialist_id: task.specialist_id,
                    output_schema: task.output_schema,
                    isolated: task.isolated,
                    model_id: None,
                    executor: None,
                    budget: None,
                })
                .collect(),
        };
        let plan = dynamic_workflow::resolve_proposal(
            &self.store,
            workflow_id.clone(),
            proposal,
            &registry,
            &host,
        )
        .await?;
        let display_ids = plan
            .steps
            .iter()
            .filter_map(|step| {
                step.input
                    .get("task_id")
                    .and_then(Value::as_str)
                    .map(|id| (step.id.clone(), id.to_string()))
            })
            .collect::<HashMap<_, _>>();
        let snapshot = delegation_runtime::persist_dynamic_agent_workflow(
            &self.store,
            &self.project.id,
            &self.project.root,
            self.frame_id.clone(),
            &plan,
            &registry,
            &host,
        )
        .await?;

        if plan.requires_confirmation {
            match env
                .confirm_decision(&approval_prompt(&plan, &display_ids))
                .await
            {
                ConfirmDecision::Approved => {}
                ConfirmDecision::Denied { feedback } => {
                    return Ok(json!({
                        "workflow_id": workflow_id,
                        "status": "denied",
                        "feedback": feedback,
                        "results": [],
                        "message": "No delegated Agent was started. Revise the batch using the user's feedback before trying again."
                    }));
                }
            }
        }
        if env.is_cancelled() {
            return Err("parent turn was cancelled before delegated execution began".into());
        }
        delegation_runtime::approve_created_automatic_workflow(&self.store, snapshot).await?;
        let execution = match &self.delegator_override {
            Some(delegator) => {
                delegation_runtime::execute_agent_workflow_with_delegator(
                    &self.store,
                    &self.project.id,
                    &workflow_id,
                    delegator.clone(),
                    Some((registry, host)),
                )
                .await?
            }
            None => {
                delegation_runtime::execute_inline_agent_workflow(
                    &self.store,
                    self.project.clone(),
                    self.run_manager.clone(),
                    self.app_data.clone(),
                    &workflow_id,
                )
                .await?
            }
        };
        Ok(compact_execution_result(&execution, &display_ids))
    }
}

fn approval_prompt(
    plan: &wisp_core::DelegationPlan,
    display_ids: &HashMap<String, String>,
) -> String {
    let tasks = plan
        .steps
        .iter()
        .map(|step| {
            let id = display_ids
                .get(&step.id)
                .map(String::as_str)
                .unwrap_or(&step.id);
            let reasons = if step.spec.approval_reasons.is_empty() {
                "native read-only".into()
            } else {
                step.spec.approval_reasons.join("; ")
            };
            format!(
                "[ ] {id}: {}\n    capabilities={} · model={} · executor={} · {reasons}",
                step.spec.goal,
                step.spec.capabilities.join(","),
                step.spec.model.as_deref().unwrap_or("active"),
                step.spec
                    .executor
                    .as_ref()
                    .map(|executor| serde_json::to_string(executor).unwrap_or_default())
                    .unwrap_or_else(|| "native".into()),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}Delegated Agent batch: {}\n{}",
        wisp_tools::plan::PLAN_APPROVAL_PREFIX,
        plan.goal,
        tasks
    )
}

fn compact_execution_result(
    execution: &DelegationExecutionResult,
    display_ids: &HashMap<String, String>,
) -> Value {
    let results = execution
        .steps
        .iter()
        .map(|step| {
            let response = &step.response;
            let id = display_ids
                .get(&step.step_id)
                .cloned()
                .unwrap_or_else(|| step.step_id.clone());
            let summary = response
                .output
                .get("summary")
                .and_then(Value::as_str)
                .map(|value| bounded_chars(value, INLINE_TEXT_CHARS))
                .or_else(|| {
                    response
                        .error
                        .as_deref()
                        .map(|value| bounded_chars(value, INLINE_TEXT_CHARS))
                })
                .unwrap_or_default();
            let data = response
                .output
                .get("data")
                .unwrap_or(&response.output);
            json!({
                "id": id,
                "status": response.status,
                "summary": summary,
                "data": compact_value(data, INLINE_DATA_BYTES, &execution.workflow_id, &id),
                "artifacts": response.artifacts,
                "evidence": response.evidence,
                "tests": response.output.get("tests").cloned().unwrap_or_else(|| json!([])),
                "risks": response.output.get("risks").cloned().unwrap_or_else(|| json!([])),
                "usage": response.usage,
                "child_frame_id": response.child_frame_id,
                "error": response.error.as_deref().map(|value| bounded_chars(value, INLINE_TEXT_CHARS)),
                "lookup": {
                    "tool": "get_delegated_result",
                    "mcp_tool": "wisp_get_delegated_result",
                    "workflow_id": execution.workflow_id,
                    "task_id": id,
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "workflow_id": execution.workflow_id,
        "status": execution.status,
        "results": results,
        "message": "Synthesize these ordered delegated results into the final answer. Independent failures are partial evidence; do not hide them."
    })
}

fn compact_value(value: &Value, limit: usize, workflow_id: &str, task_id: &str) -> Value {
    let raw = serde_json::to_string(value).unwrap_or_default();
    if raw.len() <= limit {
        return value.clone();
    }
    json!({
        "truncated": true,
        "preview": bounded_bytes(&raw, limit),
        "lookup": {
            "tool": "get_delegated_result",
            "mcp_tool": "wisp_get_delegated_result",
            "workflow_id": workflow_id,
            "task_id": task_id,
        }
    })
}

fn bounded_bytes(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

fn bounded_chars(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let kept = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{kept}…")
    } else {
        kept
    }
}

pub(crate) fn get_delegated_result_schema() -> ToolSchema {
    ToolSchema::new(
        "get_delegated_result",
        "Read the latest persisted full result for one task from an earlier delegate_tasks batch. Use only when the compact inline result says it was truncated or lacks needed detail.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "workflow_id": {"type": "string"},
                "task_id": {"type": "string"}
            },
            "required": ["workflow_id", "task_id"]
        }),
    )
}

pub(crate) struct GetDelegatedResultTool {
    store: Store,
    project_id: String,
    frame_id: String,
}

impl GetDelegatedResultTool {
    pub(crate) fn new(
        store: Store,
        project_id: impl Into<String>,
        frame_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            project_id: project_id.into(),
            frame_id: frame_id.into(),
        }
    }
}

#[async_trait]
impl Tool for GetDelegatedResultTool {
    fn name(&self) -> &str {
        "get_delegated_result"
    }

    fn schema(&self) -> ToolSchema {
        get_delegated_result_schema()
    }

    fn preview(&self, args: &Value) -> String {
        args.get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match self.read_result(args).await {
            Ok(value) => ToolResult::ok(serde_json::to_string(&value).unwrap_or_default()),
            Err(error) => ToolResult::fail(format!("get_delegated_result error: {error}")),
        }
    }
}

impl GetDelegatedResultTool {
    async fn read_result(&self, args: &Value) -> Result<Value, String> {
        delegation_runtime::require_session_delegation(
            &self.store,
            &self.project_id,
            &self.frame_id,
        )
        .await?;
        let workflow_id = required_arg(args, "workflow_id")?;
        let task_id = required_arg(args, "task_id")?;
        let workflow = self
            .store
            .get_agent_workflow(workflow_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "workflow does not exist".to_string())?;
        if workflow.project_id != self.project_id
            || workflow.frame_id.as_deref() != Some(self.frame_id.as_str())
        {
            return Err("workflow does not belong to this conversation".into());
        }
        let steps = self
            .store
            .list_agent_workflow_steps(workflow_id)
            .await
            .map_err(|error| error.to_string())?;
        let suffix = format!(":{task_id}");
        let step = steps
            .iter()
            .find(|step| step.id == task_id || step.id.ends_with(&suffix))
            .ok_or_else(|| "task does not exist in this workflow".to_string())?;
        let attempt = self
            .store
            .list_agent_workflow_attempts(workflow_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|attempt| attempt.step_id == step.id)
            .max_by_key(|attempt| attempt.attempt)
            .ok_or_else(|| "task has no execution attempt".to_string())?;
        let response = attempt
            .response_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        Ok(json!({
            "workflow_id": workflow_id,
            "task_id": task_id,
            "stored_step_id": step.id,
            "attempt": attempt.attempt,
            "status": attempt.status,
            "response": response,
            "error": attempt.error,
        }))
    }
}

fn required_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("'{name}' is required"))
}

pub(crate) fn propose_delegation_schema() -> ToolSchema {
    ToolSchema::new(
        "propose_delegation",
        "Create a persisted draft plan for controlled sub-Agent delegation when the user's task materially benefits from parallel code, biology, visualization, or independent review. This only proposes a plan: it never approves or runs Agents. After creating it, tell the user to review the Agents panel.",
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "Concrete delegated outcome, including the evidence or artifact the user expects"
                },
                "mode": {
                    "type": "string",
                    "enum": ["manual", "assisted", "automatic"],
                    "description": "Planning mode; defaults to assisted"
                },
                "agents": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["code_execution", "biology_interpreter", "visualization", "reviewer"]
                    },
                    "description": "Required ordered Agent template ids in manual mode; ignored for assisted and automatic planning"
                }
            },
            "required": ["goal"]
        }),
    )
}

pub(crate) struct ProposeDelegationTool {
    store: Store,
    project_id: String,
    project_root: std::path::PathBuf,
    frame_id: String,
}

impl ProposeDelegationTool {
    pub(crate) fn new(store: Store, project: ActiveProject, frame_id: impl Into<String>) -> Self {
        Self::for_project(store, project.id, project.root, frame_id)
    }

    pub(crate) fn for_project(
        store: Store,
        project_id: impl Into<String>,
        project_root: impl Into<std::path::PathBuf>,
        frame_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            project_id: project_id.into(),
            project_root: project_root.into(),
            frame_id: frame_id.into(),
        }
    }
}

#[async_trait]
impl Tool for ProposeDelegationTool {
    fn name(&self) -> &str {
        "propose_delegation"
    }

    fn schema(&self) -> ToolSchema {
        propose_delegation_schema()
    }

    fn preview(&self, args: &Value) -> String {
        args.get("goal")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if goal.is_empty() {
            return ToolResult::fail("propose_delegation error: 'goal' is required");
        }
        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("assisted");
        let template_ids = args
            .get("agents")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        match delegation_runtime::create_agent_workflow_draft(
            &self.store,
            &self.project_id,
            &self.project_root,
            self.frame_id.clone(),
            goal,
            mode,
            &template_ids,
        )
        .await
        {
            Ok(snapshot) => ToolResult::ok(format!(
                "Created draft Agent workflow '{}' with {} controlled steps. No Agent has started. Ask the user to review and approve it in the Agents panel.",
                snapshot.workflow.id,
                snapshot.steps.len(),
            )),
            Err(error) => ToolResult::fail(format!("propose_delegation error: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{HashSet, VecDeque},
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
    };
    use wisp_core::{
        AgentBudget, AgentDelegationResponse, AgentExecutorRef, AgentOutputSchemaSource,
        AgentUsage, ContextPolicy, DelegationStatus, ExecutorFeature, ExecutorProfilePolicy,
        ModelProfilePolicy, NullOutput, PermissionSet, ValidatedAgentDelegationRequest,
    };
    use wisp_llm::{
        Completion, FunctionCall, LlmError, Message, Provider, Role, StreamSink, ToolCall, Usage,
    };

    struct NoEnv(PathBuf);

    #[async_trait]
    impl ToolEnv for NoEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    struct DecisionEnv {
        root: PathBuf,
        decision: ConfirmDecision,
        prompts: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ToolEnv for DecisionEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }

        async fn confirm(&self, _message: &str) -> bool {
            self.decision.approved()
        }

        async fn confirm_decision(&self, message: &str) -> ConfirmDecision {
            self.prompts.lock().unwrap().push(message.into());
            self.decision.clone()
        }

        async fn emit(&self, _event: wisp_tools::ToolEvent) {}
    }

    #[derive(Debug, Clone)]
    struct DelegatorCall {
        task_id: String,
        input: Value,
    }

    struct FakeDelegator {
        active: AtomicUsize,
        max_active: AtomicUsize,
        calls: Mutex<Vec<DelegatorCall>>,
        failed_tasks: HashSet<String>,
        invalid_schema_tasks: HashSet<String>,
    }

    impl FakeDelegator {
        fn new(failed_tasks: &[&str], invalid_schema_tasks: &[&str]) -> Self {
            Self {
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                calls: Mutex::new(vec![]),
                failed_tasks: failed_tasks.iter().map(|value| (*value).into()).collect(),
                invalid_schema_tasks: invalid_schema_tasks
                    .iter()
                    .map(|value| (*value).into())
                    .collect(),
            }
        }

        fn calls(&self) -> Vec<DelegatorCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AgentDelegator for FakeDelegator {
        async fn delegate_validated(
            &self,
            request: ValidatedAgentDelegationRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            let request = request.into_request();
            let task_id = request
                .input
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or(&request.step_id)
                .to_string();
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            self.calls.lock().unwrap().push(DelegatorCall {
                task_id: task_id.clone(),
                input: request.input.clone(),
            });
            tokio::time::sleep(std::time::Duration::from_millis(35)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);

            if self.failed_tasks.contains(&task_id) {
                return Ok(AgentDelegationResponse {
                    request_id: request.request_id,
                    status: DelegationStatus::Failed,
                    output: json!({}),
                    artifact_ids: vec![],
                    artifacts: vec![],
                    evidence: vec![],
                    usage: AgentUsage::default(),
                    agent_session_id: None,
                    child_frame_id: Some(format!("child-{task_id}")),
                    error: Some(format!("{task_id} failed intentionally")),
                });
            }
            let output = if request.spec.output_schema_source == AgentOutputSchemaSource::Task {
                if self.invalid_schema_tasks.contains(&task_id) {
                    json!({"score": "invalid"})
                } else {
                    json!({"score": 7})
                }
            } else {
                json!({
                    "summary": format!("{task_id} complete"),
                    "data": {"task": task_id},
                    "tests": [],
                    "risks": []
                })
            };
            Ok(AgentDelegationResponse {
                request_id: request.request_id,
                status: DelegationStatus::Succeeded,
                output,
                artifact_ids: vec![],
                artifacts: vec![],
                evidence: vec![],
                usage: AgentUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                    ..AgentUsage::default()
                },
                agent_session_id: None,
                child_frame_id: Some(format!("child-{task_id}")),
                error: None,
            })
        }
    }

    struct SequenceProvider {
        completions: Mutex<VecDeque<Completion>>,
        messages: Mutex<Vec<Vec<Message>>>,
        schemas: Mutex<Vec<Vec<String>>>,
    }

    impl SequenceProvider {
        fn new(completions: Vec<Completion>) -> Self {
            Self {
                completions: Mutex::new(completions.into()),
                messages: Mutex::new(vec![]),
                schemas: Mutex::new(vec![]),
            }
        }

        fn pop(&self, messages: &[Message], tools: &[ToolSchema]) -> wisp_llm::Result<Completion> {
            self.messages.lock().unwrap().push(messages.to_vec());
            self.schemas.lock().unwrap().push(
                tools
                    .iter()
                    .map(|schema| schema.function.name.clone())
                    .collect(),
            );
            self.completions
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Config("fake parent ran out of completions".into()))
        }
    }

    #[async_trait]
    impl Provider for SequenceProvider {
        fn name(&self) -> &str {
            "sequence-parent"
        }

        fn model(&self) -> &str {
            "sequence-parent-model"
        }

        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            self.pop(messages, tools)
        }

        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            _sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.pop(messages, tools)
        }
    }

    fn completion(content: &str, tool_calls: Vec<ToolCall>) -> Completion {
        Completion {
            content: content.into(),
            reasoning: None,
            finish_reason: Some(if tool_calls.is_empty() {
                "stop".into()
            } else {
                "tool_calls".into()
            }),
            tool_calls,
            usage: Usage {
                input_tokens: 3,
                output_tokens: 2,
            },
        }
    }

    fn tool_call(args: Value) -> ToolCall {
        ToolCall {
            id: "delegate-call".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: "delegate_tasks".into(),
                arguments: args.to_string(),
            },
        }
    }

    fn test_policy() -> (CapabilityRegistry, DelegationHostPolicy) {
        (
            CapabilityRegistry::builtins(),
            DelegationHostPolicy {
                revision: "delegate-tool-test-v1".into(),
                enabled_capabilities: vec![
                    "reasoning".into(),
                    "project_read".into(),
                    "project_write".into(),
                    "review".into(),
                ],
                models: vec![ModelProfilePolicy {
                    id: "local".into(),
                    features: vec![],
                    external: false,
                    enabled: true,
                }],
                executors: vec![ExecutorProfilePolicy {
                    executor: AgentExecutorRef::Native,
                    features: vec![
                        ExecutorFeature::ProjectRead,
                        ExecutorFeature::ProjectWrite,
                        ExecutorFeature::CodeExecution,
                    ],
                    model_ids: vec!["local".into()],
                    enabled: true,
                }],
                default_model_id: Some("local".into()),
                permission_ceiling: PermissionSet {
                    tools: vec![
                        "read".into(),
                        "search".into(),
                        "grep".into(),
                        "write".into(),
                        "edit".into(),
                    ],
                    paths: vec!["project://**".into()],
                    network: false,
                    write: true,
                    execute: true,
                },
                context_ceiling: ContextPolicy {
                    include_history: false,
                    include_artifacts: true,
                    max_tokens: Some(32_000),
                },
                budget_ceiling: AgentBudget {
                    max_tokens: Some(32_000),
                    max_tool_calls: Some(64),
                    max_cost_microunits: Some(1_000_000),
                },
                default_timeout_secs: Some(5),
                timeout_ceiling_secs: Some(5),
                auto_safe: true,
                ..DelegationHostPolicy::default()
            },
        )
    }

    async fn fixture() -> (Store, ActiveProject, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("wisp_delegation_tool_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("store.sqlite");
        let store = Store::open(&database).await.unwrap();
        store
            .create_project("p", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("f", "p", "OPERON", "wisp")
            .await
            .unwrap();
        let project = ActiveProject {
            id: "p".into(),
            root: root.clone(),
            skills: std::sync::Arc::new(wisp_skills::SkillIndex::load(&[])),
            memory: std::sync::Arc::new(wisp_core::MemoryManager::new(&root)),
        };
        (store, project, root)
    }

    async fn enable_delegation(store: &Store) {
        delegation_runtime::save_session_delegation_enabled(store, "p", "f", true)
            .await
            .unwrap();
    }

    fn parse_tool_result(result: &ToolResult) -> Value {
        assert!(result.success, "{}", result.content);
        serde_json::from_str(&result.content).unwrap()
    }

    #[test]
    fn compact_preview_respects_utf8_byte_limit() {
        let compact = compact_value(&json!({"text": "界界界界界界"}), 10, "workflow", "task");
        let preview = compact["preview"].as_str().unwrap();
        assert!(preview.len() <= 13, "preview was {} bytes", preview.len());
        assert_eq!(compact["lookup"]["mcp_tool"], "wisp_get_delegated_result");
    }

    #[tokio::test]
    async fn parent_agent_delegates_parallel_tasks_and_synthesizes_inline() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let args = json!({
            "goal": "Compare two independent inputs",
            "context": "Return concise evidence for the parent.",
            "tasks": [
                {
                    "id": "left",
                    "instruction": "Analyze the left input.",
                    "capabilities": ["reasoning"]
                },
                {
                    "id": "right",
                    "instruction": "Analyze the right input.",
                    "capabilities": ["reasoning"]
                }
            ]
        });
        let provider = SequenceProvider::new(vec![
            completion("", vec![tool_call(args)]),
            completion("Synthesized left and right evidence.", vec![]),
        ]);
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );
        let mut tools = wisp_tools::Registry::builtins().filtered(&[]);
        tools.add(Box::new(tool));
        let mut context = wisp_core::ContextManager::new(32_000);

        wisp_core::agent_loop(
            &mut context,
            &provider,
            None,
            &tools,
            &root,
            &NullOutput,
            "Compare these inputs and report one conclusion.",
            4,
            None,
        )
        .await
        .unwrap();

        assert_eq!(delegator.max_active.load(Ordering::SeqCst), 2);
        let provider_messages = provider.messages.lock().unwrap();
        assert_eq!(provider_messages.len(), 2);
        let tool_message = provider_messages[1]
            .iter()
            .find(|message| message.role == Role::Tool)
            .expect("delegation result should be returned to the parent model");
        let inline: Value = serde_json::from_str(&tool_message.content.as_text()).unwrap();
        assert_eq!(inline["status"], "succeeded");
        assert_eq!(inline["results"].as_array().unwrap().len(), 2);
        assert!(inline["message"].as_str().unwrap().contains("Synthesize"));
        assert_eq!(
            context.messages.last().unwrap().content.as_text(),
            "Synthesized left and right evidence."
        );
        assert!(provider
            .schemas
            .lock()
            .unwrap()
            .iter()
            .all(|schemas| schemas == &["delegate_tasks"]));

        drop(tools);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn inline_batch_runs_two_tasks_in_parallel_then_supplies_fan_in_results() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );

        let result = tool
            .run(
                &json!({
                    "goal": "Analyze both sources and combine the evidence",
                    "tasks": [
                        {
                            "id": "source_a",
                            "instruction": "Analyze source A.",
                            "capabilities": ["reasoning"]
                        },
                        {
                            "id": "source_b",
                            "instruction": "Analyze source B.",
                            "capabilities": ["reasoning"]
                        },
                        {
                            "id": "combine",
                            "instruction": "Combine both source results.",
                            "depends_on": ["source_a", "source_b"],
                            "capabilities": ["reasoning"]
                        }
                    ]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        let value = parse_tool_result(&result);

        assert_eq!(value["status"], "succeeded");
        assert_eq!(delegator.max_active.load(Ordering::SeqCst), 2);
        let calls = delegator.calls();
        let combine = calls.iter().find(|call| call.task_id == "combine").unwrap();
        let dependencies = combine.input["dependency_results"].as_object().unwrap();
        assert_eq!(dependencies.len(), 2);
        assert!(dependencies
            .values()
            .all(|output| output["summary"].as_str().is_some()));
        let workflow = store.list_agent_workflows("p").await.unwrap().remove(0);
        assert_eq!(workflow.max_parallel, 2);
        assert_eq!(workflow.status, wisp_store::AgentWorkflowStatus::Succeeded);

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn approval_denial_starts_no_children_and_returns_feedback_to_parent() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );
        let env = DecisionEnv {
            root: root.clone(),
            decision: ConfirmDecision::Denied {
                feedback: Some("keep this read-only".into()),
            },
            prompts: Mutex::new(vec![]),
        };

        let result = tool
            .run(
                &json!({
                    "goal": "Edit the report",
                    "tasks": [{
                        "id": "edit_report",
                        "instruction": "Edit report.md.",
                        "capabilities": ["project_write"]
                    }]
                }),
                &env,
            )
            .await;
        let value = parse_tool_result(&result);

        assert_eq!(value["status"], "denied");
        assert_eq!(value["feedback"], "keep this read-only");
        assert!(value["results"].as_array().unwrap().is_empty());
        assert!(delegator.calls().is_empty());
        {
            let prompts = env.prompts.lock().unwrap();
            assert_eq!(prompts.len(), 1);
            assert!(prompts[0].contains(wisp_tools::plan::PLAN_APPROVAL_PREFIX));
            assert!(prompts[0].contains("project_write"));
            assert!(prompts[0].contains("native"));
        }
        let workflow = store.list_agent_workflows("p").await.unwrap().remove(0);
        assert_eq!(workflow.status, wisp_store::AgentWorkflowStatus::Draft);

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn independent_success_survives_failure_while_only_descendants_are_blocked() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&["bad"], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );

        let result = tool
            .run(
                &json!({
                    "goal": "Collect as much independent evidence as possible",
                    "tasks": [
                        {
                            "id": "bad",
                            "instruction": "Run the failing analysis.",
                            "capabilities": ["reasoning"]
                        },
                        {
                            "id": "independent",
                            "instruction": "Run an independent analysis.",
                            "capabilities": ["reasoning"]
                        },
                        {
                            "id": "downstream",
                            "instruction": "Use the failing analysis.",
                            "depends_on": ["bad"],
                            "capabilities": ["reasoning"]
                        }
                    ]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        let value = parse_tool_result(&result);
        let results = value["results"].as_array().unwrap();
        let status = |id: &str| {
            results.iter().find(|item| item["id"] == id).unwrap()["status"]
                .as_str()
                .unwrap()
        };

        assert_eq!(value["status"], "failed");
        assert_eq!(status("bad"), "failed");
        assert_eq!(status("independent"), "succeeded");
        assert_eq!(status("downstream"), "blocked");
        let called = delegator
            .calls()
            .into_iter()
            .map(|call| call.task_id)
            .collect::<HashSet<_>>();
        assert_eq!(called, HashSet::from(["bad".into(), "independent".into()]));

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn task_output_schema_is_enforced_and_full_result_can_be_loaded() {
        let (store, project, root) = fixture().await;
        enable_delegation(&store).await;
        let delegator = Arc::new(FakeDelegator::new(&[], &["invalid"]));
        let tool =
            DelegateTasksTool::with_runtime(store.clone(), project, "f", test_policy(), delegator);
        let score_schema = json!({
            "type": "object",
            "properties": {"score": {"type": "integer", "minimum": 0}},
            "required": ["score"],
            "additionalProperties": false
        });

        let result = tool
            .run(
                &json!({
                    "goal": "Return validated scores",
                    "tasks": [
                        {
                            "id": "valid",
                            "instruction": "Return a valid score.",
                            "capabilities": ["reasoning"],
                            "output_schema": score_schema
                        },
                        {
                            "id": "invalid",
                            "instruction": "Return an invalid score for validation testing.",
                            "capabilities": ["reasoning"],
                            "output_schema": score_schema
                        }
                    ]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        let value = parse_tool_result(&result);
        let workflow_id = value["workflow_id"].as_str().unwrap();
        let results = value["results"].as_array().unwrap();
        assert_eq!(results[0]["status"], "succeeded");
        assert_eq!(results[0]["data"]["score"], 7);
        assert_eq!(results[1]["status"], "failed");
        assert!(results[1]["error"]
            .as_str()
            .unwrap()
            .contains("output_contract"));
        assert_eq!(results[1]["usage"]["input_tokens"], 1);

        let lookup = GetDelegatedResultTool::new(store.clone(), "p", "f")
            .run(
                &json!({"workflow_id": workflow_id, "task_id": "valid"}),
                &NoEnv(root.clone()),
            )
            .await;
        let full = parse_tool_result(&lookup);
        assert_eq!(full["response"]["output"]["data"]["score"], 7);
        assert_eq!(full["response"]["child_frame_id"], "child-valid");

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn disabled_session_cannot_invoke_inline_delegation() {
        let (store, project, root) = fixture().await;
        let delegator = Arc::new(FakeDelegator::new(&[], &[]));
        let tool = DelegateTasksTool::with_runtime(
            store.clone(),
            project,
            "f",
            test_policy(),
            delegator.clone(),
        );

        let result = tool
            .run(
                &json!({
                    "goal": "Analyze one input",
                    "tasks": [{
                        "id": "analysis",
                        "instruction": "Analyze it.",
                        "capabilities": ["reasoning"]
                    }]
                }),
                &NoEnv(root.clone()),
            )
            .await;

        assert!(!result.success);
        assert!(result.content.contains("delegation is off"));
        assert!(delegator.calls().is_empty());
        assert!(store.list_agent_workflows("p").await.unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn disabled_session_cannot_propose_a_workflow() {
        let (store, project, root) = fixture().await;
        let tool = ProposeDelegationTool::new(store.clone(), project, "f");
        let result = tool
            .run(
                &json!({
                    "goal": "analyze code and create a visualization",
                    "mode": "manual",
                    "agents": ["code_execution", "reviewer"]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        assert!(!result.success);
        assert!(result.content.contains("delegation is off"));
        assert!(store.list_agent_workflows("p").await.unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn enabled_session_can_only_create_a_draft() {
        let (store, project, root) = fixture().await;
        delegation_runtime::save_session_delegation_enabled(&store, "p", "f", true)
            .await
            .unwrap();
        let tool = ProposeDelegationTool::new(store.clone(), project, "f");
        let result = tool
            .run(
                &json!({
                    "goal": "analyze code and create a visualization",
                    "mode": "manual",
                    "agents": ["code_execution", "reviewer"]
                }),
                &NoEnv(root.clone()),
            )
            .await;
        assert!(result.success, "{}", result.content);
        let workflows = store.list_agent_workflows("p").await.unwrap();
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].status, wisp_store::AgentWorkflowStatus::Draft);
        assert_eq!(workflows[0].frame_id.as_deref(), Some("f"));
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
