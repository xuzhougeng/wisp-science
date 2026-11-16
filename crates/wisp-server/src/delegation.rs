//! Delegated task batches: a conversation's agent splits bounded work into
//! a dependency-ordered set of tasks, each done by a temporary agent, and
//! gets their results back in the same turn. A port of the WebView's
//! `src-tauri/src/delegation_tool.rs`, `dynamic_workflow.rs` and the native
//! path of `delegation_runtime.rs`.
//!
//! Each task runs as a conversation of its own under the one that started
//! the batch: the researcher can open and watch it, not write to it. Its
//! tools are the ones its capabilities grant and nothing else, and it works
//! without asking: the batch as a whole is approved first when any task
//! can change files, run code or use a remote model, unless the
//! conversation has full access. Batches, tasks and attempts are kept in
//! the store's agent-workflow tables, the same records the WebView keeps.
//! Not here yet: background batches, nested delegation, specialists,
//! isolated worktrees and retrying a batch.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use wisp_core::{
    AgentArtifact, AgentBudget, AgentDelegationLineage, AgentDelegationRequest,
    AgentDelegationResponse, AgentDelegator, AgentEvidence, AgentExecutorRef,
    AgentOutputSchemaSource, AgentSpec, AgentUsage, CapabilityRegistry, ContextPolicy,
    DelegatedTaskProposal, DelegationExecutionObserver, DelegationExecutionResult,
    DelegationExecutionStatus, DelegationExecutor, DelegationHostPolicy, DelegationMode,
    DelegationPlan, DelegationPlanStep, DelegationStatus, ExecutorFeature, ExecutorProfilePolicy,
    ModelFeature, ModelProfilePolicy, ModelTier, PermissionSet, ValidatedAgentDelegationRequest,
    WorkflowTaskKind, DYNAMIC_DELEGATION_SCHEMA_VERSION, MAX_DELEGATION_TASKS,
};
use wisp_llm::{Role, ToolSchema};
use wisp_store::{
    AgentDelegationRootLimits, AgentWorkflow, AgentWorkflowAttempt, AgentWorkflowAttemptStart,
    AgentWorkflowAttemptStatus, AgentWorkflowDelivery, AgentWorkflowStatus, AgentWorkflowStep,
    Store, MAX_ROOT_AGENT_TASKS,
};
use wisp_tools::{ConfirmDecision, Tool, ToolEnv, ToolResult};

use crate::Server;

pub(crate) const DELEGATE: &str = "delegate_tasks";
pub(crate) const RESULT: &str = "get_delegated_result";
/// The task's model: the configured subagent model, or the conversation's.
const WORKER: &str = "worker";
const CONVERSATION: &str = "conversation";
const MAX_PARALLEL: usize = 2;
const MAX_GOAL_CHARS: usize = 2_000;
const MAX_CONTEXT_CHARS: usize = 12_000;
const MAX_INSTRUCTION_CHARS: usize = 8_000;
const MAX_TASK_ID_BYTES: usize = 31;
const MAX_TITLE_CHARS: usize = 80;
const INLINE_DATA_BYTES: usize = 4_000;
const INLINE_TEXT_CHARS: usize = 1_000;
const POLL: Duration = Duration::from_millis(200);
/// The setting that marks a conversation as a delegated task, holding the
/// batch it belongs to.
const TASK_KEY: &str = "delegated_task:";

const RESULT_INSTRUCTIONS: &str = "Return one JSON object and no Markdown fence. Include summary (string), files_changed (array), diff_summary (string), artifacts (array), evidence (array), tests (array), and risks (array).";

const DESCRIPTION: &str = "Execute a bounded task graph with known dependencies and deliverables using temporary Wisp sub-Agents, and wait for their results in this turn. Independent tasks run in parallel (two at a time), dependencies run after their prerequisites succeed, and you must synthesize the returned evidence into your final answer. Each task runs in a watch-only conversation of its own with only the tools its capabilities grant. Prefer dispatch_subagent for sustained exploration that revises scripts, waits for compute Runs and needs follow-up instructions. Use the smallest useful graph; task count alone does not choose the mechanism. Keep trivial work in the current conversation.";

/// The setting that marks `session` as a task of a delegated batch.
pub(crate) fn task_key(session: &str) -> String {
    format!("{TASK_KEY}{session}")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchArgs {
    goal: String,
    #[serde(default)]
    context: String,
    tasks: Vec<TaskArgs>,
    /// Return a handle now; the result is delivered to the conversation
    /// when the batch ends, and its agent resumed to sum it up.
    #[serde(default)]
    background: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskArgs {
    id: String,
    instruction: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    /// `agent` (the default) or `run_activity`: a step the host runs
    /// itself, a method search over the contract a dependency prepared.
    #[serde(default)]
    task_kind: Option<String>,
    #[serde(default)]
    run_activity: Option<RunActivityArgs>,
    #[serde(default)]
    output_schema: Option<Value>,
    /// One of the researcher's experts to do the task as.
    #[serde(default)]
    specialist_id: Option<String>,
    #[serde(default)]
    model_tier: Option<ModelTier>,
    #[serde(default)]
    budget: Option<BudgetArgs>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// Work in a temporary Git worktree, merged back when the task succeeds.
    #[serde(default)]
    isolated: bool,
}

/// The pointer a method search reads its frozen spec's id from, in what
/// the preparing task returned.
const SPEC_POINTER: &str = "method_search_spec_artifact_version_id";

/// What a batch the researcher launched (a workflow run as agents) is
/// recorded as, apart from the batches agents delegate.
const LAUNCHED: &str = "Workflow run as agents";

/// A method search step, as the WebView's `RunActivityProposal`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunActivityArgs {
    activity: String,
    #[serde(default = "local_context")]
    context_id: String,
    /// The task whose result names the frozen spec; a direct dependency.
    input_task_id: String,
    #[serde(default = "spec_pointer")]
    spec_output_pointer: String,
    max_candidates: u32,
    max_wall_seconds: u64,
    max_evaluator_seconds: u64,
    max_cost_microunits: u64,
}

fn local_context() -> String {
    wisp_runtime::LOCAL_CONTEXT_ID.into()
}

fn spec_pointer() -> String {
    SPEC_POINTER.into()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetArgs {
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    max_tool_calls: Option<u32>,
    #[serde(default)]
    max_cost_microunits: Option<u64>,
}

/// One task of a batch as the window lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegatedTask {
    /// The id the agent gave it.
    pub id: String,
    pub instruction: String,
    pub depends_on: Vec<String>,
    pub capabilities: Vec<String>,
    /// The expert it was done as, if any.
    pub expert: Option<String>,
    /// queued, running, succeeded, failed, blocked or cancelled.
    pub status: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    /// The conversation the task ran in.
    pub conversation: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tool_calls: i64,
    /// The most tokens it may use; none (or zero) for no limit.
    pub max_tokens: Option<u32>,
    /// It works in a Git worktree of its own.
    pub isolated: bool,
}

/// A delegated batch as the window lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegatedBatch {
    pub id: String,
    pub goal: String,
    /// draft, approved, running, succeeded, failed or cancelled.
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub tasks: Vec<DelegatedTask>,
    /// Started by a task of another batch: it runs again only with that
    /// task.
    pub nested: bool,
    /// A workflow the researcher ran as agents, rather than a batch an
    /// agent delegated: while a draft, its card approves or discards it.
    pub launched: bool,
}

impl DelegatedBatch {
    pub fn running(&self) -> bool {
        matches!(self.status.as_str(), "approved" | "running")
    }
}

fn valid_task_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_TASK_ID_BYTES
        && bytes[0].is_ascii_lowercase()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn stored_task_id(workflow: &str, task: &str) -> String {
    format!("{workflow}:{task}")
}

/// The same checks as the WebView's `dynamic_workflow::validate_proposal`.
fn validate(batch: &BatchArgs) -> Result<(), String> {
    let goal = batch.goal.trim();
    if goal.is_empty() || goal.chars().count() > MAX_GOAL_CHARS {
        return Err(format!(
            "goal must contain 1 to {MAX_GOAL_CHARS} characters"
        ));
    }
    if batch.context.chars().count() > MAX_CONTEXT_CHARS {
        return Err(format!(
            "shared context cannot exceed {MAX_CONTEXT_CHARS} characters"
        ));
    }
    if batch.tasks.is_empty() || batch.tasks.len() > MAX_DELEGATION_TASKS {
        return Err(format!(
            "a batch requires 1 to {MAX_DELEGATION_TASKS} tasks"
        ));
    }
    let mut ids = HashSet::new();
    for task in &batch.tasks {
        if !valid_task_id(&task.id) {
            return Err(format!(
                "invalid task id '{}'; use a lowercase letter followed by at most 30 lowercase letters, digits, '_' or '-'",
                task.id
            ));
        }
        if !ids.insert(task.id.as_str()) {
            return Err(format!("duplicate task id: {}", task.id));
        }
        let instruction = task.instruction.trim();
        if instruction.is_empty() || instruction.chars().count() > MAX_INSTRUCTION_CHARS {
            return Err(format!(
                "task {} instruction must contain 1 to {MAX_INSTRUCTION_CHARS} characters",
                task.id
            ));
        }
        match task.task_kind.as_deref() {
            None | Some("agent") => {
                if task.run_activity.is_some() {
                    return Err(format!(
                        "task {} has run_activity but is not a run_activity task",
                        task.id
                    ));
                }
                if task.capabilities.is_empty() {
                    return Err(format!("task {} requires at least one capability", task.id));
                }
            }
            Some("run_activity") => {
                let activity = task.run_activity.as_ref().ok_or_else(|| {
                    format!("Run activity task {} requires run_activity", task.id)
                })?;
                if !task.capabilities.is_empty()
                    || task.specialist_id.is_some()
                    || task.output_schema.is_some()
                    || task.isolated
                    || task.model_tier.is_some()
                    || task.budget.is_some()
                    || task.timeout_secs.is_some()
                {
                    return Err(format!(
                        "Run activity task {} cannot include Agent-only fields",
                        task.id
                    ));
                }
                if activity.activity != "method_search"
                    || activity.context_id != wisp_runtime::LOCAL_CONTEXT_ID
                    || activity.spec_output_pointer != SPEC_POINTER
                    || !(1..=50).contains(&activity.max_candidates)
                    || activity.max_wall_seconds == 0
                    || activity.max_wall_seconds > 7 * 24 * 60 * 60
                    || activity.max_evaluator_seconds == 0
                    || activity.max_evaluator_seconds > 300
                    || activity.max_cost_microunits == 0
                {
                    return Err(format!(
                        "Run activity task {} has an invalid activity or budget (a local method_search, 1-50 candidates, at most a week, evaluator at most 300 s)",
                        task.id
                    ));
                }
                if !task.depends_on.contains(&activity.input_task_id) {
                    return Err(format!(
                        "Run activity task {} input_task_id must be a direct dependency",
                        task.id
                    ));
                }
            }
            Some(other) => {
                return Err(format!("task {} has unknown task_kind {other}", task.id));
            }
        }
        if task.output_schema.as_ref().is_some_and(|schema| {
            !schema.is_object()
                || serde_json::to_vec(schema)
                    .is_ok_and(|bytes| bytes.len() > wisp_core::MAX_AGENT_OUTPUT_SCHEMA_BYTES)
        }) {
            return Err(format!(
                "task {} output_schema must be an object no larger than {} bytes",
                task.id,
                wisp_core::MAX_AGENT_OUTPUT_SCHEMA_BYTES
            ));
        }
    }
    for task in &batch.tasks {
        let mut seen = HashSet::new();
        for dependency in &task.depends_on {
            if !seen.insert(dependency.as_str()) {
                return Err(format!(
                    "task {} contains duplicate dependency {dependency}",
                    task.id
                ));
            }
            if dependency == &task.id {
                return Err(format!("task {} cannot depend on itself", task.id));
            }
            if !ids.contains(dependency.as_str()) {
                return Err(format!(
                    "task {} depends on unknown task {dependency}",
                    task.id
                ));
            }
        }
        // The preparing task must promise the spec's id in its result.
        if let Some(activity) = task.run_activity.as_ref() {
            let input = batch
                .tasks
                .iter()
                .find(|candidate| candidate.id == activity.input_task_id)
                .ok_or_else(|| format!("Run activity task {} input_task_id is unknown", task.id))?;
            let pointer = activity.spec_output_pointer.as_str();
            let declares = input.output_schema.as_ref().is_some_and(|schema| {
                schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|properties| properties.contains_key(pointer))
                    && schema
                        .get("required")
                        .and_then(Value::as_array)
                        .is_some_and(|required| {
                            required.iter().any(|value| value.as_str() == Some(pointer))
                        })
            });
            if !declares {
                return Err(format!(
                    "Run activity task {} requires dependency {} to declare required output {pointer} in its output_schema",
                    task.id, input.id
                ));
            }
        }
    }
    let mut done = HashSet::new();
    while done.len() < batch.tasks.len() {
        let before = done.len();
        for task in &batch.tasks {
            if !done.contains(task.id.as_str())
                && task
                    .depends_on
                    .iter()
                    .all(|dependency| done.contains(dependency.as_str()))
            {
                done.insert(task.id.as_str());
            }
        }
        if done.len() == before {
            return Err("task dependencies contain a cycle".into());
        }
    }
    Ok(())
}

/// The batch as a plan the capability policy has resolved.
fn resolve(
    workflow: String,
    batch: BatchArgs,
    registry: &CapabilityRegistry,
    host: &DelegationHostPolicy,
    experts: &[crate::experts::Expert],
    local_context: Option<&str>,
) -> Result<DelegationPlan, String> {
    validate(&batch)?;
    let context = batch.context.trim().to_owned();
    let mut steps = Vec::with_capacity(batch.tasks.len());
    for task in batch.tasks {
        let stored = stored_task_id(&workflow, &task.id);
        if let Some(activity) = task.run_activity {
            steps.push(run_activity_step(
                &workflow,
                stored,
                task.id,
                task.instruction,
                task.depends_on,
                &context,
                activity,
                host,
                local_context,
            )?);
            continue;
        }
        // The expert's name and instructions go with the task; this host
        // runs it on the task's own model.
        let specialist = match task.specialist_id.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(id) => {
                let expert = experts
                    .iter()
                    .find(|expert| expert.id == id)
                    .ok_or_else(|| format!("unknown expert: {id}"))?;
                Some(wisp_core::SpecialistSnapshot {
                    id: expert.id.clone(),
                    name: expert.name.clone(),
                    instructions: expert.instructions.clone(),
                    model_id: None,
                    skills: None,
                    connectors: None,
                })
            }
        };
        let resolved = registry
            .resolve_task(
                DelegatedTaskProposal {
                    id: stored.clone(),
                    instruction: task.instruction.trim().into(),
                    context_summary: context.clone(),
                    depends_on: task
                        .depends_on
                        .iter()
                        .map(|dependency| stored_task_id(&workflow, dependency))
                        .collect(),
                    capabilities: task.capabilities,
                    skill_bindings: vec![],
                    specialist,
                    output_schema: task.output_schema,
                    isolated: task.isolated,
                    model_id: None,
                    model_tier: task.model_tier,
                    executor: None,
                    budget: task.budget.map(|budget| AgentBudget {
                        max_tokens: budget.max_tokens,
                        max_tool_calls: budget.max_tool_calls,
                        max_cost_microunits: budget.max_cost_microunits,
                    }),
                    timeout_secs: task.timeout_secs,
                    input: json!({"task_id": task.id}),
                },
                host,
            )
            .map_err(|error| error.to_string())?;
        steps.push(DelegationPlanStep {
            id: stored,
            spec: resolved.spec().clone(),
            input: resolved.input().clone(),
            task_kind: WorkflowTaskKind::Agent,
            run_activity: None,
        });
    }
    // A method search spends time and money on its own, so it is always
    // approved first, as in the WebView.
    let requires_confirmation = !host.auto_safe
        || steps.iter().any(|step| {
            step.task_kind == WorkflowTaskKind::RunActivity
                || !step.spec.approval_reasons.is_empty()
        });
    let plan = DelegationPlan {
        schema_version: DYNAMIC_DELEGATION_SCHEMA_VERSION,
        id: workflow,
        goal: batch.goal.trim().into(),
        mode: DelegationMode::Automatic,
        requires_confirmation,
        max_parallel: MAX_PARALLEL,
        steps,
    };
    plan.validate().map_err(|error| error.to_string())?;
    Ok(plan)
}

/// A method search step, resolved and sealed as the WebView's
/// `dynamic_workflow` does: on this computer's context as it is now, with
/// the host's default model to propose candidates, and the budget the
/// contract must have been frozen with.
#[allow(clippy::too_many_arguments)]
fn run_activity_step(
    workflow: &str,
    stored: String,
    display: String,
    instruction: String,
    depends_on: Vec<String>,
    context: &str,
    activity: RunActivityArgs,
    host: &DelegationHostPolicy,
    local_context: Option<&str>,
) -> Result<DelegationPlanStep, String> {
    let context_revision = local_context
        .ok_or("This computer's execution context is not available for a method search")?;
    let mut resolved = wisp_core::RunActivitySpec {
        activity: activity.activity,
        context_id: activity.context_id,
        context_revision: context_revision.to_owned(),
        input_task_id: stored_task_id(workflow, &activity.input_task_id),
        spec_output_pointer: activity.spec_output_pointer,
        max_candidates: activity.max_candidates,
        max_wall_seconds: activity.max_wall_seconds,
        max_evaluator_seconds: activity.max_evaluator_seconds,
        max_cost_microunits: activity.max_cost_microunits,
        provider_profile_id: None,
        model_profile_id: host.default_model_id.clone(),
        approval_reasons: vec![
            "Runs bounded candidate code generation and evaluation.".into(),
            "Consumes the explicitly approved method-search time and cost budget.".into(),
        ],
        integrity_hash: String::new(),
    };
    resolved.seal().map_err(|error| error.to_string())?;
    let spec = AgentSpec {
        agent_id: stored.clone(),
        name: display.clone(),
        goal: instruction.trim().into(),
        context_summary: context.into(),
        inputs: vec![],
        acceptance_criteria: vec![],
        dependencies: depends_on
            .iter()
            .map(|dependency| stored_task_id(workflow, dependency))
            .collect(),
        role: wisp_core::AgentRole::Custom("run_activity".into()),
        backend: wisp_core::AgentBackend::Local,
        model: None,
        prompt_template: "Host-managed Workflow Run activity.".into(),
        input_contract: json!({"type": "object"}),
        output_contract: json!({"type": "object"}),
        permissions: PermissionSet::default(),
        context_policy: ContextPolicy::default(),
        budget: AgentBudget::default(),
        timeout_secs: Some(resolved.max_wall_seconds),
        requires_review: true,
        session_policy: wisp_core::AgentSessionPolicy::New,
        allow_delegation: false,
        origin: wisp_core::AgentOrigin::Temporary,
        capabilities: vec![],
        skill_bindings: vec![],
        executor: None,
        request_preferences: None,
        workspace_policy: None,
        output_schema_source: AgentOutputSchemaSource::Standard,
        approval_reasons: resolved.approval_reasons.clone(),
        authorization: None,
    };
    Ok(DelegationPlanStep {
        id: stored,
        spec,
        input: json!({"task_id": display}),
        task_kind: WorkflowTaskKind::RunActivity,
        run_activity: Some(resolved),
    })
}

/// Gives tasks that did not finish the token budget the researcher set, by
/// the agent's task id; zero is no limit. Nothing else of the plan changes.
/// Returns whether anything did.
fn revise_budgets(
    plan: &mut DelegationPlan,
    display: &HashMap<String, String>,
    finished: &HashSet<String>,
    budgets: &HashMap<String, u32>,
    registry: &CapabilityRegistry,
    host: &DelegationHostPolicy,
) -> Result<bool, String> {
    let mut changed = false;
    for (task, &tokens) in budgets {
        let step = plan
            .steps
            .iter_mut()
            .find(|step| display.get(&step.id).unwrap_or(&step.id) == task)
            .ok_or_else(|| format!("The batch has no task '{task}'"))?;
        if finished.contains(&step.id) {
            return Err(format!(
                "Task '{task}' finished; only a task that did not can be given another budget"
            ));
        }
        // Resolved again as if the task had asked for it, so the plan's own
        // checks still hold.
        let mut budget = step
            .spec
            .request_preferences
            .as_ref()
            .and_then(|preferences| preferences.budget.clone())
            .unwrap_or_default();
        budget.max_tokens = Some(tokens);
        let revised = registry
            .resolve_with_budget(&step.spec, step.input.clone(), budget, host)
            .map_err(|error| format!("Task '{task}' cannot have that budget: {error}"))?;
        if revised != step.spec {
            step.spec = revised;
            changed = true;
        }
    }
    Ok(changed)
}

fn display_task_ids(plan: &DelegationPlan) -> HashMap<String, String> {
    plan.steps
        .iter()
        .filter_map(|step| {
            step.input
                .get("task_id")
                .and_then(Value::as_str)
                .map(|id| (step.id.clone(), id.to_owned()))
        })
        .collect()
}

/// The question the plan card shows: one checklist row per task.
fn approval_prompt(plan: &DelegationPlan, display: &HashMap<String, String>) -> String {
    let tasks = plan
        .steps
        .iter()
        .map(|step| {
            let id = display.get(&step.id).unwrap_or(&step.id);
            let reasons = if step.spec.approval_reasons.is_empty() {
                "read-only".into()
            } else {
                step.spec.approval_reasons.join("; ")
            };
            format!(
                "[ ] {id}: {}\n    capabilities={} · {reasons}",
                step.spec.goal,
                step.spec.capabilities.join(","),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}Delegated Agent batch: {}\n{tasks}",
        wisp_tools::plan::PLAN_APPROVAL_PREFIX,
        plan.goal
    )
}

fn root_limits(plan: &DelegationPlan, host: &DelegationHostPolicy) -> AgentDelegationRootLimits {
    let multiply = |value: Option<u64>| {
        value
            .unwrap_or_default()
            .saturating_mul(u64::from(MAX_ROOT_AGENT_TASKS))
    };
    AgentDelegationRootLimits {
        // A task granted delegation may start one batch of its own.
        max_depth: if plan.steps.iter().any(|step| step.spec.allow_delegation) {
            2
        } else {
            1
        },
        max_tasks: MAX_ROOT_AGENT_TASKS,
        max_parallel: u32::try_from(plan.max_parallel).unwrap_or(2).clamp(1, 2),
        // Zero is the persisted unlimited value.
        max_tokens: multiply(host.budget_ceiling.max_tokens.map(u64::from)),
        max_tool_calls: multiply(host.budget_ceiling.max_tool_calls.map(u64::from)),
        max_cost_microunits: multiply(host.budget_ceiling.max_cost_microunits),
        // The sum of the tasks' limits, or none when any task has none.
        wall_time_secs: plan.steps.iter().try_fold(0_u64, |total, step| {
            step.spec
                .timeout_secs
                .map(|seconds| total.saturating_add(seconds))
        }),
    }
}

/// The batch's records, as the WebView's `workflow_records` writes them.
fn records(
    plan: &DelegationPlan,
    project: &str,
    root: &str,
    frame: &str,
    host: &DelegationHostPolicy,
) -> anyhow::Result<(AgentWorkflow, Vec<AgentWorkflowStep>)> {
    let name = if plan.goal.chars().count() > 72 {
        format!("{}…", plan.goal.chars().take(71).collect::<String>())
    } else {
        plan.goal.clone()
    };
    let mut workflow = AgentWorkflow::new(&plan.id, project, root, name)?;
    workflow.frame_id = Some(frame.into());
    workflow.description = "Inline dynamic Agent batch".into();
    workflow.goal.clone_from(&plan.goal);
    workflow.mode = "automatic".into();
    workflow.max_parallel = i64::try_from(plan.max_parallel)?;
    workflow.requires_confirmation = plan.requires_confirmation;
    workflow.plan_json = serde_json::to_string(plan)?;
    workflow.root_limits_json = serde_json::to_string(&root_limits(plan, host))?;
    let steps = plan
        .steps
        .iter()
        .enumerate()
        .map(|(position, step)| {
            let spec = &step.spec;
            let mut stored = AgentWorkflowStep::new(
                &step.id,
                &plan.id,
                i64::try_from(position)?,
                &spec.agent_id,
                spec.role.as_str(),
                spec.backend.as_str(),
                &spec.prompt_template,
            )?;
            stored.model.clone_from(&spec.model);
            stored.input_schema_json = serde_json::to_string(&spec.input_contract)?;
            stored.output_schema_json = serde_json::to_string(&spec.output_contract)?;
            stored
                .input_contract_json
                .clone_from(&stored.input_schema_json);
            stored
                .output_contract_json
                .clone_from(&stored.output_schema_json);
            stored.permissions_json = serde_json::to_string(&spec.permissions)?;
            stored.context_policy_json = serde_json::to_string(&spec.context_policy)?;
            stored.budget_json = serde_json::to_string(&spec.budget)?;
            stored.spec_json = serde_json::to_string(spec)?;
            stored.task_kind = match step.task_kind {
                WorkflowTaskKind::Agent => "agent",
                WorkflowTaskKind::RunActivity => "run_activity",
            }
            .into();
            stored.activity_json = step
                .run_activity
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?
                .unwrap_or_else(|| "{}".into());
            stored.timeout_secs = spec.timeout_secs.map(i64::try_from).transpose()?;
            Ok(stored)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok((workflow, steps))
}

/// The tools a task's capabilities grant that this host has; the WebView's
/// `native_tool_allowlist`. A reviewer reads and changes nothing.
fn allowed_tools(spec: &AgentSpec) -> Vec<String> {
    let reviewer = is_reviewer(spec);
    let mut allowed: Vec<String> = Vec::new();
    for requested in &spec.permissions.tools {
        let name = match requested.as_str() {
            "read_file" => "read",
            "write_file" => "write",
            value => value,
        };
        let permitted = match name {
            "read" | "search" | "grep" => !spec.permissions.paths.is_empty(),
            "write" | "edit" => spec.permissions.write && !reviewer,
            "view_image" => !spec.permissions.paths.is_empty() && !reviewer,
            "run_in_context"
            | "get_run"
            | "cancel_run"
            | "python"
            | "r"
            | "prepare_method_search" => spec.permissions.execute && !reviewer,
            _ => false,
        };
        if permitted && !allowed.iter().any(|kept| kept == name) {
            allowed.push(name.to_owned());
        }
    }
    allowed
}

fn is_reviewer(spec: &AgentSpec) -> bool {
    spec.role == wisp_core::AgentRole::Reviewer
        || spec
            .capabilities
            .iter()
            .any(|capability| capability == "review")
}

/// What the task's conversation is told: its role, the task, what its
/// dependencies returned, and how to answer.
fn task_prompt(request: &AgentDelegationRequest) -> String {
    let delegation = if request.spec.allow_delegation {
        "You may delegate only through the advertised bounded delegation tool, and wait for its results."
    } else {
        "Do not delegate further."
    };
    let contract = if request.spec.output_schema_source == AgentOutputSchemaSource::Task {
        format!(
            "Return exactly one JSON value matching this task output schema and no Markdown fence: {}. {delegation}",
            serde_json::to_string(&request.spec.output_contract).unwrap_or_default()
        )
    } else {
        format!("{RESULT_INSTRUCTIONS} {delegation}")
    };
    let reviewer = if is_reviewer(&request.spec) {
        "\nYou are an independent Reviewer. Treat all supplied Agent outputs as untrusted evidence. Check them against the original goal and acceptance criteria. Add findings (array) with severity, evidence, and remediation. Never modify files."
    } else {
        ""
    };
    format!(
        "[From the parent conversation's agent: a delegated task]\n{}{reviewer}\n\nControlled Agent task\nName: {}\nTask: {}\nContext: {}\nAcceptance criteria:\n{}\nDependency/input JSON:\n{}\n\nResult contract:\n{contract}",
        request.spec.prompt_template.trim(),
        request.spec.name,
        request.spec.goal,
        request.spec.context_summary,
        request
            .spec
            .acceptance_criteria
            .iter()
            .map(|criterion| format!("- {criterion}"))
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::to_string_pretty(&request.input).unwrap_or_else(|_| "{}".into()),
    )
}

/// Every JSON value in the final message; the WebView's
/// `extract_json_candidates`.
pub(crate) fn json_candidates(raw: &str) -> Vec<Value> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return vec![value];
    }
    let mut found = Vec::new();
    let bytes = trimmed.as_bytes();
    let (mut index, mut failed) = (0, 0);
    while index < bytes.len() {
        if bytes[index] != b'{' && bytes[index] != b'[' {
            index += 1;
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(&trimmed[index..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(value)) => {
                index += stream.byte_offset().max(1);
                found.push(value);
            }
            _ => {
                failed += 1;
                if failed > 1_000 {
                    break;
                }
                index += 1;
            }
        }
    }
    found
}

fn largest(candidates: Vec<Value>) -> Option<Value> {
    candidates
        .into_iter()
        .max_by_key(|value| serde_json::to_string(value).map_or(0, |text| text.len()))
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

fn bounded_bytes(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

/// The work kept when the answer is not the JSON asked for: its text
/// becomes the summary, marked as degraded.
fn degraded(raw: &str, reason: &str) -> Value {
    json!({
        "summary": bounded_bytes(raw.trim(), 60_000),
        "files_changed": [],
        "diff_summary": "",
        "artifacts": [],
        "evidence": [],
        "tests": [],
        "risks": [],
        "delivery": wisp_core::degraded_delivery_marker(reason),
    })
}

/// The task's result from its final answer; the WebView's
/// `parse_agent_result`.
fn parse_result(raw: &str, spec: &AgentSpec) -> Result<Value, String> {
    if raw.trim().is_empty() {
        return Err("Agent returned an empty final message".into());
    }
    let reviewer = is_reviewer(spec);
    let candidates = json_candidates(raw);
    if spec.output_schema_source == AgentOutputSchemaSource::Task {
        if let Some(value) = largest(candidates) {
            return Ok(value);
        }
        if reviewer {
            return Err("Reviewer returned no parseable JSON task result".into());
        }
        return Ok(degraded(
            raw,
            "The final message contained no parseable JSON value; the raw text is preserved in summary.",
        ));
    }
    let envelope = largest(
        candidates
            .into_iter()
            .filter(|value| value.get("summary").is_some_and(Value::is_string))
            .collect(),
    );
    if let Some(Value::Object(mut object)) = envelope {
        if !object.get("diff_summary").is_some_and(Value::is_string) {
            object.insert("diff_summary".into(), json!(""));
        }
        for field in ["files_changed", "artifacts", "evidence", "tests", "risks"] {
            if !object.get(field).is_some_and(Value::is_array) {
                object.insert(field.into(), json!([]));
            }
        }
        if reviewer
            && spec.output_schema_source == AgentOutputSchemaSource::Standard
            && !object.get("findings").is_some_and(Value::is_array)
        {
            return Err("Reviewer result is missing the findings array".into());
        }
        return Ok(Value::Object(object));
    }
    if reviewer {
        return Err("Reviewer returned no JSON result object with a summary string".into());
    }
    Ok(degraded(
        raw,
        "The final message contained no JSON object with a summary string; the raw text is preserved in summary.",
    ))
}

fn evidence_of(output: &Value) -> Vec<AgentEvidence> {
    output
        .get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|value| match value {
            Value::Object(value) => AgentEvidence {
                kind: value
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("agent")
                    .into(),
                summary: value
                    .get("summary")
                    .or_else(|| value.get("evidence"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                reference: value
                    .get("reference")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            value => AgentEvidence {
                kind: "agent".into(),
                summary: value.as_str().unwrap_or_default().into(),
                reference: None,
            },
        })
        .collect()
}

fn declared_artifacts(output: &Value) -> Vec<AgentArtifact> {
    output
        .get("artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|value| {
            let path = value.get("path").and_then(Value::as_str).map(str::to_owned);
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| {
                    path.as_deref()
                        .and_then(|path| std::path::Path::new(path).file_name()?.to_str())
                })
                .unwrap_or_default()
                .to_owned();
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| path.as_ref().map(|path| format!("declared:{path}")))
                .or_else(|| (!name.is_empty()).then(|| format!("declared:{name}")))?;
            Some(AgentArtifact {
                id,
                name,
                kind: value
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("file")
                    .into(),
                path,
            })
        })
        .collect()
}

fn ended(
    request: &str,
    status: DelegationStatus,
    error: Option<String>,
    child: Option<String>,
    usage: AgentUsage,
) -> AgentDelegationResponse {
    AgentDelegationResponse {
        request_id: request.into(),
        status,
        output: json!({}),
        artifact_ids: vec![],
        artifacts: vec![],
        evidence: vec![],
        usage,
        agent_session_id: None,
        child_frame_id: child,
        error,
        nested_results: vec![],
    }
}

fn failed(
    request: &str,
    error: String,
    child: Option<String>,
    usage: AgentUsage,
) -> AgentDelegationResponse {
    ended(request, DelegationStatus::Failed, Some(error), child, usage)
}

/// What the parent's turn gets back: each task's status and summary, its
/// data when small, and how to read the rest.
fn compact(execution: &DelegationExecutionResult, display: &HashMap<String, String>) -> Value {
    let lookup =
        |task: &str| json!({"tool": RESULT, "workflow_id": execution.workflow_id, "task_id": task});
    let results = execution
        .steps
        .iter()
        .map(|step| {
            let response = &step.response;
            let id = display
                .get(&step.step_id)
                .cloned()
                .unwrap_or_else(|| step.step_id.clone());
            let summary = response
                .output
                .get("summary")
                .and_then(Value::as_str)
                .or(response.error.as_deref())
                .map(|text| bounded_chars(text, INLINE_TEXT_CHARS))
                .unwrap_or_default();
            let data = response.output.get("data").unwrap_or(&response.output);
            let raw = serde_json::to_string(data).unwrap_or_default();
            let data = if raw.len() <= INLINE_DATA_BYTES {
                data.clone()
            } else {
                json!({
                    "truncated": true,
                    "preview": bounded_bytes(&raw, INLINE_DATA_BYTES),
                    "lookup": lookup(&id),
                })
            };
            let mut entry = json!({
                "id": id,
                "status": response.status,
                "summary": summary,
                "data": data,
                "artifacts": response.artifacts,
                "evidence": response.evidence,
                "tests": response.output.get("tests").cloned().unwrap_or_else(|| json!([])),
                "risks": response.output.get("risks").cloned().unwrap_or_else(|| json!([])),
                "usage": response.usage,
                "child_frame_id": response.child_frame_id,
                "error": response.error.as_deref().map(|text| bounded_chars(text, INLINE_TEXT_CHARS)),
                "lookup": lookup(&id),
            });
            if wisp_core::is_degraded_delivery(&response.output) {
                entry["delivery"] = response.output["delivery"].clone();
            }
            entry
        })
        .collect::<Vec<_>>();
    let message = if execution.status == DelegationExecutionStatus::Succeeded {
        "Synthesize these ordered delegated results into the final answer. Independent failures are partial evidence; do not hide them."
    } else {
        "Report the failure reason and workflow_id, and what the tasks that finished found. Inspect the cause before delegating again; do not silently replace the batch."
    };
    json!({
        "workflow_id": execution.workflow_id,
        "status": execution.status,
        "results": results,
        "message": message,
    })
}

fn status_name(status: AgentWorkflowStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn attempt_status_name(status: AgentWorkflowAttemptStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

/// Records each task's attempts as the batch runs; the WebView's
/// `StoreDelegationObserver` without retries.
struct Recorder {
    store: Store,
    attempts: tokio::sync::Mutex<HashMap<String, String>>,
    claimed: AtomicBool,
    /// A rerun numbers its attempts by its delivery's generation, so its
    /// result is read from its own attempts.
    generation: Option<i64>,
}

impl Recorder {
    async fn attempt_number(&self, step: &str) -> anyhow::Result<i64> {
        match self.generation {
            Some(generation) => Ok(generation),
            None => self.store.next_agent_workflow_attempt_number(step).await,
        }
    }

    async fn closed_attempt(
        &self,
        request: &AgentDelegationRequest,
        status: AgentWorkflowAttemptStatus,
        reason: &str,
    ) -> anyhow::Result<()> {
        let number = self.attempt_number(&request.step_id).await?;
        let mut attempt = AgentWorkflowAttempt::queued(
            uuid::Uuid::new_v4().to_string(),
            &request.workflow_id,
            &request.step_id,
            number,
            &request.request_id,
            request.spec.backend.as_str(),
            serde_json::to_string(&request.input)?,
        )?;
        lineage(&mut attempt, request);
        self.store.create_agent_workflow_attempt(&attempt).await?;
        attempt.status = status;
        attempt.error = Some(reason.into());
        attempt.cancel_requested = status == AgentWorkflowAttemptStatus::Cancelled;
        attempt.finished_at = Some(chrono::Utc::now().timestamp());
        anyhow::ensure!(
            self.store
                .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::Queued)
                .await?,
            "the task's attempt changed while it was being recorded"
        );
        Ok(())
    }
}

fn lineage(attempt: &mut AgentWorkflowAttempt, request: &AgentDelegationRequest) {
    if let Some(lineage) = &request.lineage {
        attempt
            .root_workflow_id
            .clone_from(&lineage.root_workflow_id);
        attempt
            .parent_attempt_id
            .clone_from(&lineage.parent_attempt_id);
        attempt.depth = i64::from(lineage.depth);
    }
    attempt.allow_delegation = request.spec.allow_delegation;
}

#[async_trait]
impl DelegationExecutionObserver for Recorder {
    async fn workflow_started(&self, plan: &DelegationPlan) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.store
                .transition_agent_workflow_status(
                    &plan.id,
                    AgentWorkflowStatus::Approved,
                    AgentWorkflowStatus::Running,
                )
                .await?,
            "Agent workflow is not approved or is already running"
        );
        self.claimed.store(true, Ordering::Release);
        Ok(())
    }

    async fn step_started(&self, request: &AgentDelegationRequest) -> anyhow::Result<()> {
        let number = self.attempt_number(&request.step_id).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut attempt = AgentWorkflowAttempt::queued(
            &id,
            &request.workflow_id,
            &request.step_id,
            number,
            &request.request_id,
            request.spec.backend.as_str(),
            serde_json::to_string(&request.input)?,
        )?;
        lineage(&mut attempt, request);
        loop {
            match self
                .store
                .try_create_started_agent_workflow_attempt(attempt.clone())
                .await?
            {
                AgentWorkflowAttemptStart::Started(_) => break,
                AgentWorkflowAttemptStart::Busy => {
                    anyhow::ensure!(
                        !self
                            .store
                            .agent_workflow_cancel_requested(&request.workflow_id)
                            .await?,
                        "The batch was cancelled while waiting for capacity"
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                AgentWorkflowAttemptStart::Stopped(reason) => anyhow::bail!(reason),
            }
        }
        self.attempts
            .lock()
            .await
            .insert(request.request_id.clone(), id);
        Ok(())
    }

    async fn step_finished(
        &self,
        request: &AgentDelegationRequest,
        response: &AgentDelegationResponse,
    ) -> anyhow::Result<()> {
        let id = self
            .attempts
            .lock()
            .await
            .get(&request.request_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Agent attempt is not persisted"))?;
        let mut attempt = self
            .store
            .get_agent_workflow_attempt(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent attempt disappeared"))?;
        let expected = attempt.status;
        anyhow::ensure!(
            matches!(
                expected,
                AgentWorkflowAttemptStatus::Running | AgentWorkflowAttemptStatus::WaitingRun
            ),
            "Workflow attempt is no longer active"
        );
        attempt.status = match response.status {
            DelegationStatus::Succeeded => AgentWorkflowAttemptStatus::Succeeded,
            DelegationStatus::Cancelled => AgentWorkflowAttemptStatus::Cancelled,
            _ => AgentWorkflowAttemptStatus::Failed,
        };
        attempt.response_json = Some(serde_json::to_string(response)?);
        attempt.output_json = serde_json::to_string(&response.output)?;
        attempt.artifact_ids_json = serde_json::to_string(&response.artifact_ids)?;
        attempt.evidence_json = serde_json::to_string(&response.evidence)?;
        attempt.error = response.error.clone();
        attempt.child_frame_id = response.child_frame_id.clone();
        attempt.input_tokens = i64::try_from(response.usage.input_tokens).unwrap_or(i64::MAX);
        attempt.output_tokens = i64::try_from(response.usage.output_tokens).unwrap_or(i64::MAX);
        attempt.tool_calls = i64::try_from(response.usage.tool_calls).unwrap_or(i64::MAX);
        attempt.cost_microunits = i64::try_from(response.usage.cost_microunits).unwrap_or(i64::MAX);
        attempt.finished_at = Some(chrono::Utc::now().timestamp());
        anyhow::ensure!(
            self.store
                .update_agent_workflow_attempt(&attempt, expected)
                .await?,
            "Agent attempt terminal state lost a concurrent update"
        );
        Ok(())
    }

    async fn step_blocked(
        &self,
        request: &AgentDelegationRequest,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.closed_attempt(request, AgentWorkflowAttemptStatus::Blocked, reason)
            .await
    }

    async fn step_cancelled(
        &self,
        request: &AgentDelegationRequest,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.closed_attempt(request, AgentWorkflowAttemptStatus::Cancelled, reason)
            .await
    }

    async fn workflow_cancellation_reason(
        &self,
        plan: &DelegationPlan,
    ) -> anyhow::Result<Option<String>> {
        self.store.agent_workflow_stop_reason(&plan.id).await
    }

    async fn workflow_finished(
        &self,
        plan: &DelegationPlan,
        status: DelegationExecutionStatus,
    ) -> anyhow::Result<()> {
        let status = match status {
            DelegationExecutionStatus::Succeeded => AgentWorkflowStatus::Succeeded,
            DelegationExecutionStatus::Failed => AgentWorkflowStatus::Failed,
            DelegationExecutionStatus::Cancelled => AgentWorkflowStatus::Cancelled,
        };
        anyhow::ensure!(
            self.store
                .transition_agent_workflow_status(&plan.id, AgentWorkflowStatus::Running, status)
                .await?,
            "Agent workflow terminal state lost a concurrent update"
        );
        Ok(())
    }
}

/// Runs each task in a conversation of its own under the parent.
struct Delegator {
    server: Server,
    project: String,
    root: String,
    parent: String,
    /// The conversation each running task works in, by request.
    active: Mutex<HashMap<String, String>>,
}

impl Delegator {
    fn stop_all(&self) {
        let running: Vec<String> = self.active.lock().unwrap().values().cloned().collect();
        for child in running {
            self.server.session(&child).stop();
        }
    }
}

#[async_trait]
impl AgentDelegator for Delegator {
    async fn delegate_validated(
        &self,
        request: ValidatedAgentDelegationRequest,
    ) -> anyhow::Result<AgentDelegationResponse> {
        let request = request.into_request();
        Ok(self.server.run_task(self, &request).await)
    }

    async fn status(&self, _request: &str) -> anyhow::Result<Option<AgentDelegationResponse>> {
        Ok(None)
    }

    async fn cancel(&self, request: &str) -> anyhow::Result<bool> {
        let child = self.active.lock().unwrap().get(request).cloned();
        Ok(match child {
            Some(child) => {
                self.server.session(&child).stop();
                true
            }
            None => false,
        })
    }
}

impl Server {
    /// What a batch started from `parent` may ask for: the capabilities,
    /// tools and models this host has.
    pub(crate) async fn delegation_policy(
        &self,
        parent: &str,
    ) -> Result<(CapabilityRegistry, DelegationHostPolicy), String> {
        let store = &self.0.store;
        let e = |error: anyhow::Error| error.to_string();
        let own = store
            .get_setting(&crate::desktop::model_choice_key(parent))
            .await
            .map_err(e)?
            .unwrap_or_default();
        let configured = store
            .get_setting(crate::desktop::SUBAGENT_MODEL)
            .await
            .map_err(e)?
            .unwrap_or_default();
        // The same choice as a subagent's: the configured subagent model,
        // or with `inherit` the conversation's own.
        let worker = if configured.trim().is_empty() {
            own.clone()
        } else {
            configured
        };
        let profile = |id: &str, choice: &str| {
            let picked = self.0.model.read().unwrap().pick(choice);
            match picked {
                Ok(model) => ModelProfilePolicy {
                    id: id.into(),
                    features: model
                        .supports_vision
                        .then_some(ModelFeature::Vision)
                        .into_iter()
                        .collect(),
                    external: url::Url::parse(&model.provider.base_url)
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
                        .is_none_or(|host| {
                            !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]")
                        }),
                    enabled: true,
                },
                Err(_) => ModelProfilePolicy {
                    id: id.into(),
                    features: vec![],
                    external: true,
                    enabled: false,
                },
            }
        };
        let mut models = vec![profile(WORKER, &worker)];
        if worker != own {
            models.push(profile(CONVERSATION, &own));
        }
        let python = std::env::var_os("WISP_KERNEL_WORKER")
            .map(PathBuf::from)
            .or_else(wisp_runtime::bundled_worker_path)
            .is_some_and(|worker| worker.is_file());
        // A task can work in a worktree of its own only in a Git project.
        let git = match store.frame_project_id(parent).await.map_err(e)? {
            Some(project) => match self.project_root(&project).await {
                Ok(root) => {
                    crate::delegation_isolation::git_worktree_available(Path::new(&root)).await
                }
                Err(_) => false,
            },
            None => false,
        };
        Ok((
            CapabilityRegistry::builtins(),
            host_policy(models, python, git),
        ))
    }
}

/// What a task's conversation delegates under: the task's running attempt,
/// the task as it was resolved, the root batch's limits and the task's id.
struct Nested {
    attempt: AgentWorkflowAttempt,
    parent_spec: AgentSpec,
    limits: AgentDelegationRootLimits,
    parent_display_id: String,
}

/// The policy a task's own batch is resolved under: within what the task
/// was granted (its capabilities, model, permissions, context and budget),
/// with delegation dropped at the last level; the WebView's
/// `nested_delegation_policy`.
fn nested_policy(
    registry: &CapabilityRegistry,
    host: &DelegationHostPolicy,
    parent: &AgentSpec,
    parent_depth: u8,
    limits: &AgentDelegationRootLimits,
) -> Result<(CapabilityRegistry, DelegationHostPolicy), String> {
    if !parent.allow_delegation || parent_depth >= limits.max_depth {
        return Err("This task may not delegate at its depth; do the work itself.".into());
    }
    let child_depth = parent_depth.saturating_add(1);
    let enabled_capabilities: Vec<String> = parent
        .capabilities
        .iter()
        .filter(|capability| capability.as_str() != "delegation" || child_depth < limits.max_depth)
        .filter(|capability| host.enabled_capabilities.contains(capability))
        .cloned()
        .collect();
    if enabled_capabilities.is_empty() {
        return Err("This task has no capability left to give a task of its own.".into());
    }
    let executor = parent
        .executor
        .as_ref()
        .ok_or("The delegating task has no executor.")?;
    let executors: Vec<ExecutorProfilePolicy> = host
        .executors
        .iter()
        .filter(|profile| profile.enabled && &profile.executor == executor)
        .cloned()
        .collect();
    if executors.len() != 1 {
        return Err("The delegating task's executor is no longer available.".into());
    }
    let model = parent
        .model
        .as_ref()
        .ok_or("The delegating task has no model.")?;
    let models: Vec<ModelProfilePolicy> = host
        .models
        .iter()
        .filter(|profile| profile.enabled && &profile.id == model)
        .cloned()
        .collect();
    if models.len() != 1 {
        return Err("The delegating task's model is no longer available.".into());
    }
    let timeout = parent.timeout_secs.map(|seconds| {
        seconds
            .min(host.timeout_ceiling_secs.unwrap_or(seconds))
            .min(limits.wall_time_secs.unwrap_or(seconds))
    });
    let source = serde_json::to_vec(&(
        &host.revision,
        parent
            .authorization
            .as_ref()
            .map(|authorization| &authorization.integrity_hash),
        parent_depth,
        limits,
    ))
    .map_err(|error| error.to_string())?;
    let hash = source
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    Ok((
        registry.clone(),
        DelegationHostPolicy {
            revision: format!("nested-agent-policy-v1:{hash:016x}"),
            enabled_capabilities,
            models,
            executors,
            default_model_id: Some(model.clone()),
            // Pinned to the delegating task's one model.
            inherit_model_id: None,
            permission_ceiling: parent.permissions.intersect(&host.permission_ceiling),
            context_ceiling: parent.context_policy.restrict(&host.context_ceiling),
            budget_ceiling: parent.budget.restrict(&host.budget_ceiling),
            default_timeout_secs: timeout,
            timeout_ceiling_secs: timeout,
            auto_safe: true,
            ..host.clone()
        },
    ))
}

/// Takes the delegating task's place back once its own tasks are done,
/// waiting for room; a task that stopped meanwhile has no place to take.
async fn reacquire_slot(store: &Store, attempt: &str) -> Result<(), String> {
    loop {
        if store
            .set_agent_workflow_attempt_delegation_slot_yielded(attempt, false)
            .await
            .map_err(|error| error.to_string())?
        {
            return Ok(());
        }
        let running = store
            .get_agent_workflow_attempt(attempt)
            .await
            .map_err(|error| error.to_string())?
            .is_some_and(|attempt| attempt.status == AgentWorkflowAttemptStatus::Running);
        if !running {
            return Err("The delegating task stopped while its own tasks ran.".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Removes an isolated task's worktree when the task is dropped before it
/// is settled; the WebView's `IsolatedExecutionGuard`.
struct WorktreeGuard {
    isolation: crate::delegation_isolation::GitWorktreeIsolation,
    workspace: Option<crate::delegation_isolation::IsolatedWorkspace>,
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        let Some(workspace) = self.workspace.take() else {
            return;
        };
        let isolation = self.isolation.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = isolation.abort(&workspace).await {
                    tracing::warn!(%error, "Could not remove an abandoned task worktree");
                }
            });
        }
    }
}

/// Copies the snapshots an isolated task recorded in its worktree into the
/// project's own store; they are named by their content, so the paths kept
/// with its artifacts stay right, and one already there is left alone.
async fn keep_snapshots(worktree: &Path, project: &Path) -> std::io::Result<()> {
    let from = worktree.join(".wisp").join("artifacts").join("sha256");
    let to = project.join(".wisp").join("artifacts").join("sha256");
    tokio::task::spawn_blocking(move || {
        let Ok(shards) = std::fs::read_dir(&from) else {
            // It recorded nothing.
            return Ok(());
        };
        for shard in shards {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            let target = to.join(shard.file_name());
            std::fs::create_dir_all(&target)?;
            for file in std::fs::read_dir(shard.path())? {
                let file = file?;
                if !file.file_type()?.is_file() {
                    continue;
                }
                let kept = target.join(file.file_name());
                if !kept.exists() {
                    std::fs::copy(file.path(), kept)?;
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

/// A few of the files a merge changed, for its evidence line.
fn summarize_files(files: &[String]) -> String {
    let mut shown = files.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    if files.len() > 5 {
        shown.push_str(&format!(" and {} more", files.len() - 5));
    }
    shown
}

/// A task's own tasks read as `parent/child`, so results name both.
fn namespace_display_ids(plan: &mut DelegationPlan, parent: &str) {
    for step in &mut plan.steps {
        if let Some(Value::String(id)) = step.input.get_mut("task_id") {
            *id = format!("{parent}/{id}");
        }
    }
}

/// What this host offers delegated tasks, given its models (the worker
/// first) and whether a Python runtime is installed.
fn host_policy(models: Vec<ModelProfilePolicy>, python: bool, git: bool) -> DelegationHostPolicy {
    let enabled: Vec<String> = models
        .iter()
        .filter(|model| model.enabled)
        .map(|model| model.id.clone())
        .collect();
    let vision = models
        .iter()
        .any(|model| model.enabled && model.features.contains(&ModelFeature::Vision));
    let mut features = vec![
        ExecutorFeature::ProjectRead,
        ExecutorFeature::ProjectWrite,
        ExecutorFeature::CodeExecution,
        ExecutorFeature::Delegation,
    ];
    if git {
        features.push(ExecutorFeature::Isolation);
    }
    let mut capabilities: Vec<String> = [
        "reasoning",
        "project_read",
        "project_write",
        "code_run",
        "review",
        "delegation",
    ]
    .map(String::from)
    .to_vec();
    let mut tools: Vec<String> = [
        "read",
        "search",
        "grep",
        "write",
        "edit",
        "run_in_context",
        "get_run",
        "cancel_run",
        "prepare_method_search",
        DELEGATE,
        RESULT,
    ]
    .map(String::from)
    .to_vec();
    if vision {
        features.push(ExecutorFeature::Vision);
        capabilities.push("image_inspection".into());
        tools.push("view_image".into());
    }
    if python {
        capabilities.push("visualization".into());
        tools.extend(["python".into(), "r".into()]);
    }
    let inherit = enabled
        .iter()
        .any(|id| id == CONVERSATION)
        .then(|| CONVERSATION.to_owned());
    DelegationHostPolicy {
        revision: format!(
            "gpui-delegation-v1:{}",
            serde_json::to_string(&(&models, &capabilities)).unwrap_or_default()
        ),
        enabled_capabilities: capabilities,
        executors: vec![ExecutorProfilePolicy {
            executor: AgentExecutorRef::Native,
            features,
            model_ids: enabled.clone(),
            enabled: !enabled.is_empty(),
        }],
        default_model_id: enabled.first().cloned(),
        inherit_model_id: inherit,
        models,
        permission_ceiling: PermissionSet {
            tools,
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
        // No caps unless a task asks for its own.
        budget_ceiling: AgentBudget::default(),
        default_timeout_secs: None,
        timeout_ceiling_secs: None,
        auto_safe: true,
        ..DelegationHostPolicy::default()
    }
}

/// What the agent gets back at once for a background batch.
fn background_handle(
    workflow: &str,
    plan: &DelegationPlan,
    display: &HashMap<String, String>,
) -> Value {
    let tasks: Vec<Value> = plan
        .steps
        .iter()
        .map(|step| json!({"id": display.get(&step.id).unwrap_or(&step.id), "status": "queued"}))
        .collect();
    json!({
        "workflow_id": workflow,
        "status": "running",
        "completion_policy": "background",
        "auto_resume": true,
        "tasks": tasks,
        "lookup": {"tool": RESULT, "workflow_id": workflow},
        "message": "The delegated batch is running in the background. Do not poll or claim completion; one result will be delivered to this conversation and you will be resumed to synthesize it.",
    })
}

/// Keeps a background batch's result for delivery.
async fn complete_delivery(
    store: &Store,
    delivery: &AgentWorkflowDelivery,
    result: Value,
) -> Result<(), String> {
    let envelope = json!({
        "type": "delegated_batch_completion",
        "workflow_id": delivery.workflow_id,
        "generation": delivery.generation,
        "result": result,
        "message": "A background delegated batch finished. Synthesize its ordered results for the user; use get_delegated_result only when the compact result is insufficient.",
    });
    // A second result for the same delivery is ignored: the first stands.
    store
        .complete_agent_workflow_delivery(&delivery.id, &envelope.to_string())
        .await
        .map(drop)
        .map_err(|error| error.to_string())
}

/// Ends a background batch that failed to run and keeps what its tasks
/// left as its result.
async fn settle_failed(
    store: &Store,
    delivery: &AgentWorkflowDelivery,
    error: &str,
) -> Result<(), String> {
    let e = |error: anyhow::Error| error.to_string();
    if let Some(workflow) = store
        .get_agent_workflow(&delivery.workflow_id)
        .await
        .map_err(e)?
    {
        match workflow.status {
            AgentWorkflowStatus::Approved => {
                store
                    .transition_agent_workflow_status(
                        &workflow.id,
                        AgentWorkflowStatus::Approved,
                        AgentWorkflowStatus::Failed,
                    )
                    .await
                    .map_err(e)?;
            }
            AgentWorkflowStatus::Running => {
                store
                    .fail_agent_workflow_execution(&workflow.id, error)
                    .await
                    .map_err(e)?;
            }
            _ => {}
        }
    }
    reconstruct(store, delivery).await
}

/// A task's response as its latest attempt kept it, or a failure when it
/// never produced one.
fn attempt_response(attempt: Option<&AgentWorkflowAttempt>, step: &str) -> AgentDelegationResponse {
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
    AgentDelegationResponse {
        request_id: attempt
            .map(|attempt| attempt.request_id.clone())
            .unwrap_or_else(|| format!("interrupted-{step}")),
        status,
        output: attempt
            .and_then(|attempt| serde_json::from_str(&attempt.output_json).ok())
            .unwrap_or_else(|| json!({})),
        artifact_ids: attempt
            .and_then(|attempt| serde_json::from_str(&attempt.artifact_ids_json).ok())
            .unwrap_or_default(),
        artifacts: vec![],
        evidence: attempt
            .and_then(|attempt| serde_json::from_str(&attempt.evidence_json).ok())
            .unwrap_or_default(),
        usage: attempt.map_or_else(AgentUsage::default, |attempt| AgentUsage {
            input_tokens: attempt.input_tokens.max(0) as u64,
            output_tokens: attempt.output_tokens.max(0) as u64,
            tool_calls: attempt.tool_calls.max(0) as u64,
            cost_microunits: attempt.cost_microunits.max(0) as u64,
        }),
        agent_session_id: None,
        child_frame_id: attempt.and_then(|attempt| attempt.child_frame_id.clone()),
        error: attempt
            .and_then(|attempt| attempt.error.clone())
            .or_else(|| Some("The app stopped before this task produced a result.".into())),
        nested_results: vec![],
    }
}

/// The result of a batch that ended without one, from what its tasks
/// left: each task's newest attempt of this delivery's run.
async fn reconstruct(store: &Store, delivery: &AgentWorkflowDelivery) -> Result<(), String> {
    let e = |error: anyhow::Error| error.to_string();
    let workflow = store
        .get_agent_workflow(&delivery.workflow_id)
        .await
        .map_err(e)?
        .ok_or("The batch disappeared before its result was delivered.")?;
    let plan: DelegationPlan = serde_json::from_str(&workflow.plan_json)
        .map_err(|error| format!("The batch's plan is invalid: {error}"))?;
    let attempts = store
        .list_agent_workflow_attempts(&delivery.workflow_id)
        .await
        .map_err(e)?;
    let execution = DelegationExecutionResult {
        workflow_id: workflow.id.clone(),
        status: match workflow.status {
            AgentWorkflowStatus::Succeeded => DelegationExecutionStatus::Succeeded,
            AgentWorkflowStatus::Cancelled => DelegationExecutionStatus::Cancelled,
            _ => DelegationExecutionStatus::Failed,
        },
        steps: plan
            .steps
            .iter()
            .map(|step| {
                let latest = attempts
                    .iter()
                    .filter(|attempt| {
                        attempt.step_id == step.id && attempt.attempt <= delivery.generation
                    })
                    .max_by_key(|attempt| attempt.attempt);
                wisp_core::DelegationStepExecution {
                    step_id: step.id.clone(),
                    response: attempt_response(latest, &step.id),
                }
            })
            .collect(),
    };
    complete_delivery(
        store,
        delivery,
        compact(&execution, &display_task_ids(&plan)),
    )
    .await
}

/// The tasks of a batch whose latest attempt succeeded, with what they
/// returned, for a rerun to keep.
async fn successful_steps(
    store: &Store,
    workflow: &str,
) -> anyhow::Result<Vec<wisp_core::DelegationStepExecution>> {
    let mut latest = HashMap::<String, AgentWorkflowAttempt>::new();
    for attempt in store.list_agent_workflow_attempts(workflow).await? {
        if latest
            .get(&attempt.step_id)
            .is_none_or(|kept| kept.attempt < attempt.attempt)
        {
            latest.insert(attempt.step_id.clone(), attempt);
        }
    }
    Ok(latest
        .into_values()
        .filter(|attempt| attempt.status == AgentWorkflowAttemptStatus::Succeeded)
        .filter_map(|attempt| {
            let response: AgentDelegationResponse =
                serde_json::from_str(attempt.response_json.as_deref()?).ok()?;
            (response.status == DelegationStatus::Succeeded).then(|| {
                wisp_core::DelegationStepExecution {
                    step_id: attempt.step_id,
                    response,
                }
            })
        })
        .collect())
}

/// What the resumed agent is told about the batches that came back.
fn completion_prompt(deliveries: &[AgentWorkflowDelivery]) -> String {
    let results = deliveries
        .iter()
        .filter_map(|delivery| delivery.result_json.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "<background_agent_completions>\n{results}\n</background_agent_completions>\nThese background delegated batches have completed. Synthesize the new evidence into one concise user-facing update. Do not claim that still-running work is complete."
    )
}

impl Server {
    /// Adds the results of `frame`'s finished background batches to it once
    /// it is idle, and resumes its agent to sum them up.
    pub(crate) async fn deliver_batches(&self, frame: &str) {
        // A few rounds at most: results that arrive while one is delivered
        // are taken by the next.
        for _ in 0..4 {
            if !self.0.delivering.lock().unwrap().insert(frame.to_owned()) {
                return;
            }
            let delivered = self.deliver_batches_once(frame).await;
            self.0.delivering.lock().unwrap().remove(frame);
            if let Err(error) = &delivered {
                tracing::warn!(%error, frame, "Could not deliver a background batch");
                return;
            }
            let again = self
                .0
                .store
                .list_ready_agent_workflow_delivery_frames()
                .await
                .is_ok_and(|frames| frames.iter().any(|ready| ready == frame));
            if !again {
                return;
            }
        }
    }

    async fn deliver_batches_once(&self, frame: &str) -> anyhow::Result<()> {
        let store = &self.0.store;
        let session = self.session(frame);
        // Added once whatever the conversation is doing has ended.
        loop {
            if self.0.shutting_down.load(Ordering::Relaxed) {
                return Ok(());
            }
            {
                let _accept = session.accept.lock().await;
                if !session.live.lock().unwrap().running {
                    // Not the timer's to take away with its turn.
                    self.seal_timer_turn(&session, frame).await?;
                    if !store
                        .deliver_agent_workflow_completions(frame)
                        .await?
                        .is_empty()
                    {
                        let mut live = session.live.lock().unwrap();
                        live.generation = live.generation.wrapping_add(1);
                    }
                    break;
                }
            }
            tokio::time::sleep(POLL).await;
        }
        let claimed = store.claim_agent_workflow_auto_resumes(frame).await?;
        if claimed.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = claimed.iter().map(|delivery| delivery.id.clone()).collect();
        let started = async {
            let project = store
                .frame_project_id(frame)
                .await?
                .ok_or_else(|| anyhow::anyhow!("The conversation no longer exists"))?;
            let root = self.project_root(&project).await?;
            self.0.subagent_resumes.lock().unwrap().insert(
                frame.to_owned(),
                (completion_prompt(&claimed), false, String::new()),
            );
            let request = wisp_dto::native_conversations::SendRequest {
                session_id: frame.to_owned(),
                request_id: uuid::Uuid::new_v4().to_string(),
                message: String::new(),
                resume: true,
                resume_after: None,
                attachments: Vec::new(),
                references: Vec::new(),
                acp_agent_id: None,
            };
            let started = self
                .start_turn(
                    &project,
                    &root,
                    request,
                    crate::channels::TurnOrigin::Desktop,
                )
                .await;
            if started.is_err() {
                self.0.subagent_resumes.lock().unwrap().remove(frame);
            }
            started.map(drop)
        }
        .await;
        let error = started.as_ref().err().map(|error| format!("{error:#}"));
        store
            .finish_agent_workflow_auto_resumes(&ids, started.is_ok(), error.as_deref())
            .await?;
        Ok(())
    }

    /// Settles background batches the app closed on and delivers results
    /// that were not delivered, now and every half minute. The desktop
    /// starts this once.
    pub fn start_batch_deliveries(&self) {
        if self.0.sweeping_deliveries.swap(true, Ordering::SeqCst) {
            return;
        }
        let server = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                if server.0.shutting_down.load(Ordering::Relaxed) {
                    break;
                }
                server.sweep_deliveries().await;
            }
        });
    }

    pub(crate) async fn sweep_deliveries(&self) {
        let store = &self.0.store;
        for delivery in store
            .list_incomplete_agent_workflow_deliveries()
            .await
            .unwrap_or_default()
        {
            if let Err(error) = reconstruct(store, &delivery).await {
                tracing::warn!(%error, "Could not settle an interrupted background batch");
            }
        }
        for frame in store
            .list_ready_agent_workflow_delivery_frames()
            .await
            .unwrap_or_default()
        {
            let server = self.clone();
            tokio::spawn(async move { server.deliver_batches(&frame).await });
        }
    }
}

impl Server {
    /// The batch a conversation is a task of, if it is one.
    pub(crate) async fn delegated_batch_of(&self, session: &str) -> Option<String> {
        self.0
            .store
            .get_setting(&task_key(session))
            .await
            .ok()
            .flatten()
            .filter(|batch| !batch.is_empty())
    }

    /// The authority a task's conversation delegates with: none unless its
    /// task was granted delegation and the root batch still has room.
    async fn nested_context(&self, frame: &str) -> Result<Option<Nested>, String> {
        let store = &self.0.store;
        let e = |error: anyhow::Error| error.to_string();
        let Some(attempt) = store
            .running_agent_workflow_attempt_for_child_frame(frame)
            .await
            .map_err(e)?
        else {
            return Ok(None);
        };
        if !attempt.allow_delegation
            || !store
                .agent_workflow_attempt_has_delegation_capacity(&attempt.id)
                .await
                .map_err(e)?
        {
            return Ok(None);
        }
        let step = store
            .get_agent_workflow_step(&attempt.step_id)
            .await
            .map_err(e)?
            .ok_or("The delegating task no longer exists.")?;
        let parent_spec: AgentSpec = serde_json::from_str(&step.spec_json)
            .map_err(|error| format!("The delegating task's record is invalid: {error}"))?;
        if !parent_spec.allow_delegation {
            return Ok(None);
        }
        let root = store
            .get_agent_workflow(&attempt.root_workflow_id)
            .await
            .map_err(e)?
            .ok_or("The batch this task belongs to no longer exists.")?;
        let limits: AgentDelegationRootLimits = serde_json::from_str(&root.root_limits_json)
            .map_err(|error| format!("The batch's limits are invalid: {error}"))?;
        let parent_display_id = store
            .get_agent_workflow(&attempt.workflow_id)
            .await
            .map_err(e)?
            .and_then(|workflow| serde_json::from_str::<DelegationPlan>(&workflow.plan_json).ok())
            .and_then(|plan| display_task_ids(&plan).remove(&attempt.step_id))
            .unwrap_or_else(|| attempt.step_id.clone());
        Ok(Some(Nested {
            attempt,
            parent_spec,
            limits,
            parent_display_id,
        }))
    }

    /// Makes the conversation a task works in, under its parent.
    async fn task_conversation(
        &self,
        delegator: &Delegator,
        request: &AgentDelegationRequest,
    ) -> anyhow::Result<String> {
        let store = &self.0.store;
        let task = request
            .input
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or(&request.step_id);
        let child = self.create_conversation(&delegator.project).await?;
        store
            .set_session_dispatched_from(&child, &delegator.parent)
            .await?;
        store
            .set_setting(&task_key(&child), &request.workflow_id)
            .await?;
        let first = request.spec.goal.lines().next().unwrap_or_default();
        let title: String = format!("{task} · {first}")
            .chars()
            .take(MAX_TITLE_CHARS)
            .collect();
        store
            .rename_session(&child, &delegator.project, &title)
            .await?;
        Ok(child)
    }

    /// Does one task in its own conversation and reports how it went. A
    /// task that cannot start fails; it never fails the batch itself.
    async fn run_task(
        &self,
        delegator: &Delegator,
        request: &AgentDelegationRequest,
    ) -> AgentDelegationResponse {
        if request.spec.workspace_policy == Some(wisp_core::AgentWorkspacePolicy::Isolated) {
            return self.run_isolated(delegator, request).await;
        }
        self.run_task_in(delegator, request, &delegator.root).await
    }

    /// An isolated task: in a Git worktree of its own, whose changes are
    /// cherry-picked into the project when it succeeds and the preflight
    /// finds no conflict, and are otherwise kept as a patch artifact; the
    /// WebView's `delegate_isolated`.
    async fn run_isolated(
        &self,
        delegator: &Delegator,
        request: &AgentDelegationRequest,
    ) -> AgentDelegationResponse {
        use crate::delegation_isolation::{
            GitWorktreeIsolation, IsolationDisposition, IsolationFinish,
        };
        let id = request.request_id.as_str();
        let isolation = GitWorktreeIsolation::new(self.0.config.data.join("agent-worktrees"));
        let workspace = match isolation.create(Path::new(&delegator.root)).await {
            Ok(workspace) => workspace,
            Err(error) => {
                return failed(
                    id,
                    format!("The task could not get a worktree of its own: {error:#}"),
                    None,
                    AgentUsage::default(),
                )
            }
        };
        let root = workspace.project_root.to_string_lossy().into_owned();
        // A task cut off midway (its batch dropped) still has its worktree
        // removed.
        let mut guard = WorktreeGuard {
            isolation: isolation.clone(),
            workspace: Some(workspace.clone()),
        };
        let mut response = self.run_task_in(delegator, request, &root).await;
        if let Some(child) = response.child_frame_id.clone() {
            if let Err(error) = self
                .settle_isolated(&delegator.project, &child, response.status)
                .await
            {
                response.status = DelegationStatus::Failed;
                response.output = json!({});
                response.error = Some(error);
            }
        }
        // What it recorded was snapshotted in the worktree; the snapshots
        // are named by their content, so they move to the project as they
        // are.
        if let Err(error) =
            keep_snapshots(&workspace.project_root, Path::new(&delegator.root)).await
        {
            response.evidence.push(AgentEvidence {
                kind: "artifact_retention_warning".into(),
                summary: format!("Some of the task's artifacts could not be kept: {error}"),
                reference: None,
            });
            if response.status == DelegationStatus::Succeeded {
                response.status = DelegationStatus::Failed;
                response.output = json!({});
                response.error = Some(
                    "The task's artifacts could not be kept, so its changes were not merged".into(),
                );
            }
        }
        let finish = if response.status == DelegationStatus::Succeeded {
            IsolationFinish::Merge
        } else {
            IsolationFinish::Preserve {
                reason: format!(
                    "the task ended as {}",
                    format!("{:?}", response.status).to_lowercase()
                ),
            }
        };
        guard.workspace = None;
        let result = match isolation.finish(&workspace, finish).await {
            Ok(result) => result,
            Err(error) => {
                response.status = DelegationStatus::Failed;
                response.output = json!({});
                response.error = Some(format!(
                    "The task's worktree could not be settled: {error:#}"
                ));
                return response;
            }
        };
        let changed = result.changed_files.len();
        match &result.disposition {
            IsolationDisposition::NoChanges => response.evidence.push(AgentEvidence {
                kind: "workspace_isolation".into(),
                summary: "The task ran in a temporary Git worktree and changed nothing in the project."
                    .into(),
                reference: None,
            }),
            IsolationDisposition::Applied { commit } => response.evidence.push(AgentEvidence {
                kind: "workspace_merge".into(),
                summary: format!(
                    "No conflict was found and its {changed} change(s) were cherry-picked into the project: {}",
                    summarize_files(&result.changed_files)
                ),
                reference: Some(commit.clone()),
            }),
            IsolationDisposition::Preserved { reason } => {
                if !result.patch.is_empty() {
                    self.keep_patch(request, &mut response, &result.patch, reason)
                        .await;
                }
                response.evidence.push(AgentEvidence {
                    kind: "workspace_isolation".into(),
                    summary: format!(
                        "The project was left as it was; the task's {changed} change(s) were kept because {reason}."
                    ),
                    reference: None,
                });
            }
            IsolationDisposition::Rejected { reason } => {
                if !result.patch.is_empty() {
                    self.keep_patch(request, &mut response, &result.patch, reason)
                        .await;
                }
                response.status = DelegationStatus::Failed;
                response.output = json!({});
                response.error = Some(reason.clone());
                response.evidence.push(AgentEvidence {
                    kind: "workspace_merge_rejected".into(),
                    summary: format!(
                        "The project was left as it was and the task's {changed} change(s) were kept as a patch artifact."
                    ),
                    reference: None,
                });
            }
        }
        if let Some(warning) = result.cleanup_warning {
            response.evidence.push(AgentEvidence {
                kind: "workspace_cleanup_warning".into(),
                summary: warning,
                reference: None,
            });
        }
        response
    }

    /// Ends what an isolated task left running: its interpreters always,
    /// its Runs when it did not succeed. A task that succeeded waits for its
    /// Runs, and fails if one did not succeed; the WebView's
    /// `settle_isolated_resources`.
    async fn settle_isolated(
        &self,
        project: &str,
        child: &str,
        status: DelegationStatus,
    ) -> Result<(), String> {
        let store = &self.0.store;
        crate::runtime::session_runtimes(
            &self.0.config,
            &self.session(child),
            self.service_keys().await,
        )
        .shutdown_all()
        .await;
        let cancel = status != DelegationStatus::Succeeded;
        let mut sent = HashSet::new();
        loop {
            let active: Vec<_> = store
                .list_active_runs()
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|run| run.project_id == project && run.frame_id.as_deref() == Some(child))
                .collect();
            if active.is_empty() {
                break;
            }
            if cancel {
                for run in active {
                    if sent.insert(run.id.clone()) {
                        let _ = self.0.runs.cancel(store, &run.id).await;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if status == DelegationStatus::Succeeded {
            let unfinished = store
                .list_runs_by_project(project)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|run| run.frame_id.as_deref() == Some(child))
                .find(|run| run.status != wisp_store::RunStatus::Succeeded);
            if let Some(run) = unfinished {
                return Err(format!(
                    "The task's Run {} ended as {}, so its changes were not merged",
                    run.id,
                    run.status.as_str()
                ));
            }
        }
        Ok(())
    }

    /// Keeps the changes of an isolated task that were not merged as a
    /// patch artifact of its conversation.
    async fn keep_patch(
        &self,
        request: &AgentDelegationRequest,
        response: &mut AgentDelegationResponse,
        patch: &[u8],
        reason: &str,
    ) {
        let kept = async {
            let directory = self.0.config.data.join("agent-patches");
            tokio::fs::create_dir_all(&directory).await?;
            let artifact = uuid::Uuid::new_v4().to_string();
            let name = format!("isolated-agent-{artifact}.patch");
            let path = directory.join(&name);
            tokio::fs::write(&path, patch).await?;
            let frame = match response.child_frame_id.clone() {
                Some(frame) => frame,
                None => self
                    .0
                    .store
                    .get_agent_workflow(&request.workflow_id)
                    .await?
                    .and_then(|workflow| workflow.frame_id)
                    .ok_or_else(|| anyhow::anyhow!("the patch has no conversation to belong to"))?,
            };
            let project = self
                .0
                .store
                .frame_project_id(&frame)
                .await?
                .ok_or_else(|| anyhow::anyhow!("the patch's conversation is gone"))?;
            let storage = path.to_string_lossy().into_owned();
            self.0
                .store
                .save_artifact(&artifact, &project, &frame, &name, "text/x-diff", &storage)
                .await?;
            anyhow::Ok((artifact, name, storage))
        }
        .await;
        match kept {
            Ok((artifact, name, storage)) => {
                response.artifact_ids.push(artifact.clone());
                response.artifacts.push(AgentArtifact {
                    id: artifact.clone(),
                    name,
                    kind: "text/x-diff".into(),
                    path: Some(storage),
                });
                response.evidence.push(AgentEvidence {
                    kind: "workspace_patch".into(),
                    summary: format!("The task's changes were kept as a patch because {reason}."),
                    reference: Some(artifact),
                });
            }
            Err(error) => response.evidence.push(AgentEvidence {
                kind: "workspace_patch_warning".into(),
                summary: format!("The task's changes could not be kept as a patch: {error:#}"),
                reference: None,
            }),
        }
    }

    /// One task in its own conversation, working in `root`.
    async fn run_task_in(
        &self,
        delegator: &Delegator,
        request: &AgentDelegationRequest,
        root: &str,
    ) -> AgentDelegationResponse {
        let id = request.request_id.as_str();
        let store = &self.0.store;
        let child = match self.task_conversation(delegator, request).await {
            Ok(child) => child,
            Err(error) => return failed(id, format!("{error:#}"), None, AgentUsage::default()),
        };
        if let Err(error) = store
            .set_running_agent_workflow_attempt_provenance(id, None, &child)
            .await
        {
            tracing::warn!(%error, "Could not record the task's conversation");
        }
        delegator
            .active
            .lock()
            .unwrap()
            .insert(id.to_owned(), child.clone());
        let cancelled = store
            .agent_workflow_cancel_requested(&request.workflow_id)
            .await
            .unwrap_or(false);
        let outcome = if cancelled {
            Err(String::new())
        } else {
            self.task_turn(delegator, request, &child, root).await
        };
        delegator.active.lock().unwrap().remove(id);
        let session = self.session(&child);
        let usage = {
            let live = session.live.lock().unwrap();
            let used = live.usage.clone().unwrap_or_default();
            AgentUsage {
                input_tokens: used.input,
                output_tokens: used.output,
                ..AgentUsage::default()
            }
        };
        let messages = store.load_messages(&child).await.unwrap_or_default();
        let usage = AgentUsage {
            tool_calls: messages
                .iter()
                .filter(|message| message.role == Role::Tool)
                .count() as u64,
            ..usage
        };
        let stopped = session.cancel.load(Ordering::Relaxed)
            || store
                .agent_workflow_cancel_requested(&request.workflow_id)
                .await
                .unwrap_or(false);
        if stopped {
            return ended(id, DelegationStatus::Cancelled, None, Some(child), usage);
        }
        if let Err(error) = outcome {
            return failed(id, error, Some(child), usage);
        }
        let answer = crate::channels::final_answer(&messages).unwrap_or_default();
        let output = match parse_result(&answer, &request.spec) {
            Ok(output) => output,
            Err(error) => return failed(id, error, Some(child), usage),
        };
        let mut artifacts: Vec<AgentArtifact> = store
            .list_artifacts(&child)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(id, name, kind, path, _, _)| AgentArtifact {
                id,
                name,
                kind,
                path: Some(path),
            })
            .collect();
        for artifact in declared_artifacts(&output) {
            if !artifacts.iter().any(|kept| kept.id == artifact.id) {
                artifacts.push(artifact);
            }
        }
        AgentDelegationResponse {
            request_id: id.into(),
            status: DelegationStatus::Succeeded,
            artifact_ids: artifacts
                .iter()
                .map(|artifact| artifact.id.clone())
                .collect(),
            artifacts,
            evidence: evidence_of(&output),
            output,
            usage,
            agent_session_id: None,
            child_frame_id: Some(child),
            error: None,
            nested_results: vec![],
        }
    }

    /// The task's one turn: its model and the tools it was granted, full
    /// access within them, and its wall-time limit.
    async fn task_turn(
        &self,
        delegator: &Delegator,
        request: &AgentDelegationRequest,
        child: &str,
        root: &str,
    ) -> Result<(), String> {
        let e = |error: anyhow::Error| format!("{error:#}");
        let store = &self.0.store;
        let inherit = request.spec.model.as_deref() == Some(CONVERSATION);
        self.prepare_child(&delegator.parent, child, inherit)
            .await?;
        let options = self.stored_options(child).await.map_err(e)?;
        let mut model = self
            .0
            .model
            .read()
            .unwrap()
            .pick(&options.model)
            .map_err(e)?;
        self.fresh_subscription(&mut model, &self.provider_of(&options.model))
            .await
            .map_err(e)?;
        let session = self.session(child);
        let send = wisp_dto::native_conversations::SendRequest {
            session_id: child.to_owned(),
            request_id: uuid::Uuid::new_v4().to_string(),
            message: task_prompt(request),
            resume: false,
            resume_after: None,
            attachments: Vec::new(),
            references: Vec::new(),
            acp_agent_id: None,
        };
        let mut agent = self
            .wisp_agent(
                &delegator.project,
                root,
                &send,
                &session,
                &options,
                &model,
                false,
                Vec::new(),
                None,
            )
            .await
            .map_err(e)?;
        let tools = std::mem::replace(&mut agent.tools, wisp_tools::Registry::builtins());
        agent.tools = tools.filtered(&allowed_tools(&request.spec));
        // A task granted delegation may start one bounded batch of its own.
        if request.spec.allow_delegation {
            for tool in self.delegation_tools(&delegator.project, root, child).await {
                agent.add_tool(tool);
            }
        }
        let page = store
            .load_session_transcript_page(child, None, 20)
            .await
            .map_err(e)?;
        let saved = serde_json::to_string(&send).map_err(|error| error.to_string())?;
        store
            .accept_server_turn(child, &send.request_id, &saved)
            .await
            .map_err(e)?;
        self.begin_turn(&session, page);
        let mut task = tokio::spawn(crate::runtime::run(
            store.clone(),
            agent,
            session.clone(),
            delegator.project.clone(),
            send.clone(),
            true,
            false,
            PathBuf::from(root),
            crate::runtime::TurnScope {
                lab_mail: false,
                write_locked: false,
                isolation: None,
                state: wisp_store::StateScope::mainline(delegator.project.clone()),
            },
        ));
        let limit = request
            .spec
            .timeout_secs
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs);
        let mut timed_out = false;
        let joined = match limit {
            Some(limit) => match tokio::time::timeout(limit, &mut task).await {
                Ok(joined) => joined,
                Err(_) => {
                    timed_out = true;
                    session.stop();
                    task.await
                }
            },
            None => task.await,
        };
        let mut error = match joined {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(format!("{error:#}")),
            Err(_) => Some("The task stopped unexpectedly".into()),
        };
        if timed_out {
            // Not a stop the researcher asked for: the task failed.
            session.cancel.store(false, Ordering::Relaxed);
            error = Some(format!(
                "The task ran out of time after {} seconds",
                limit.map_or(0, |limit| limit.as_secs())
            ));
        }
        if let Err(failure) = store
            .finish_server_turn(child, &send.request_id, error.as_deref())
            .await
        {
            tracing::warn!(error = %failure, "Could not save the task's turn");
        }
        {
            // As a desktop turn ends.
            let mut live = session.live.lock().unwrap();
            live.running = false;
            live.guidable = false;
            live.generation = live.generation.wrapping_add(1);
            live.approvals.clear();
            live.messages.clear();
            live.draft.clear();
            live.reasoning.clear();
            live.error = error.clone();
        }
        match error {
            None => Ok(()),
            Some(error) => Err(error),
        }
    }

    /// Runs a batch from `parent`'s turn and returns what its agent gets.
    async fn delegate(
        &self,
        project: &str,
        root: &str,
        parent: &str,
        args: &Value,
        env: &dyn ToolEnv,
    ) -> Result<Value, String> {
        let store = &self.0.store;
        let batch: BatchArgs = serde_json::from_value(args.clone())
            .map_err(|error| format!("invalid task batch: {error}"))?;
        let (_, scope) =
            crate::explorations::service::working_project_in(store, &self.0.config.data, parent)
                .await?;
        if matches!(scope, wisp_store::StateScope::Exploration { .. }) {
            return Err("Delegated task batches are not available inside an exploration; do the work in this conversation.".into());
        }
        // A task's conversation delegates only with what its task was
        // granted, and waits for the results.
        let nested = match self.delegated_batch_of(parent).await {
            Some(_) => Some(self.nested_context(parent).await?.ok_or(
                "This task may not delegate further, or its batch has no room left; do the work itself.",
            )?),
            None => None,
        };
        let background = batch.background;
        if background && nested.is_some() {
            return Err("A delegated task cannot start a background batch; omit background to wait for the results.".into());
        }
        let (mut registry, mut host) = self.delegation_policy(parent).await?;
        if let Some(context) = &nested {
            let depth = u8::try_from(context.attempt.depth)
                .map_err(|_| "The delegating task's depth is invalid.".to_string())?;
            (registry, host) = nested_policy(
                &registry,
                &host,
                &context.parent_spec,
                depth,
                &context.limits,
            )?;
        }
        let workflow = uuid::Uuid::new_v4().to_string();
        let experts = crate::experts::list(store).await;
        let local = self.local_context_revision().await;
        let mut plan = resolve(
            workflow.clone(),
            batch,
            &registry,
            &host,
            &experts,
            local.as_deref(),
        )?;
        if let Some(context) = &nested {
            namespace_display_ids(&mut plan, &context.parent_display_id);
        }
        if env.project_write_locked()
            && plan
                .steps
                .iter()
                .any(|step| step.spec.permissions.write || step.spec.permissions.execute)
        {
            return Err("The project is frozen while an exploration runs; only read-only tasks can be delegated.".into());
        }
        let display = display_task_ids(&plan);
        let (mut record, steps) =
            records(&plan, project, root, parent, &host).map_err(|error| error.to_string())?;
        let mut lineage = None;
        if let Some(context) = &nested {
            // Recorded under the root batch, one level below the task.
            let root_batch = store
                .get_agent_workflow(&context.attempt.root_workflow_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or("The batch this task belongs to no longer exists.")?;
            record.workspace_id = root_batch.workspace_id;
            record
                .root_workflow_id
                .clone_from(&context.attempt.root_workflow_id);
            record.parent_attempt_id = Some(context.attempt.id.clone());
            record.depth = context.attempt.depth;
            record.root_limits_json =
                serde_json::to_string(&context.limits).map_err(|error| error.to_string())?;
            record.requires_confirmation = false;
            lineage = Some(AgentDelegationLineage {
                root_workflow_id: context.attempt.root_workflow_id.clone(),
                parent_attempt_id: Some(context.attempt.id.clone()),
                depth: u8::try_from(context.attempt.depth + 1)
                    .map_err(|_| "The nested batch is too deep.".to_string())?,
            });
        }
        store
            .create_agent_workflow_plan(&record, &steps)
            .await
            .map_err(|error| error.to_string())?;
        // The researcher approved the task that delegates; its own batch
        // asks no one.
        if plan.requires_confirmation && nested.is_none() && !env.approval_bypass() {
            if let ConfirmDecision::Denied { feedback } = env
                .confirm_decision(&approval_prompt(&plan, &display))
                .await
            {
                // Nothing ran, so nothing is kept.
                let _ = store.delete_agent_workflow(&workflow).await;
                return Ok(json!({
                    "workflow_id": workflow,
                    "status": "denied",
                    "feedback": feedback,
                    "results": [],
                    "message": "No delegated Agent was started. Revise the batch using the user's feedback before trying again.",
                }));
            }
        }
        if !store
            .approve_agent_workflow_plan(&workflow, record.version)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("The batch changed before it could start.".into());
        }
        if env.is_cancelled() {
            let _ = store
                .transition_agent_workflow_status(
                    &workflow,
                    AgentWorkflowStatus::Approved,
                    AgentWorkflowStatus::Cancelled,
                )
                .await;
            return Ok(json!({
                "workflow_id": workflow,
                "status": "cancelled",
                "results": [],
                "message": "The turn was stopped before any delegated Agent started.",
            }));
        }
        let (delegator, recorder, executor) = self.batch_executor(
            project, root, parent, &workflow, registry, host, None, lineage,
        );
        if background {
            // The batch outlives the turn: stopping the turn leaves it
            // running, and its card's Stop ends it.
            let delivery = store
                .create_agent_workflow_delivery(&workflow, true)
                .await
                .map_err(|error| error.to_string())?;
            let handle = background_handle(&workflow, &plan, &display);
            self.run_in_background(parent, delivery, display, executor, plan, None);
            return Ok(handle);
        }
        // The delegating task gives up its place among the batch's running
        // tasks while its own run, and takes it back after.
        if let Some(context) = &nested {
            if !store
                .set_agent_workflow_attempt_delegation_slot_yielded(&context.attempt.id, true)
                .await
                .map_err(|error| error.to_string())?
            {
                let _ = store
                    .transition_agent_workflow_status(
                        &workflow,
                        AgentWorkflowStatus::Approved,
                        AgentWorkflowStatus::Cancelled,
                    )
                    .await;
                return Err("The delegating task could not make room for its own tasks.".into());
            }
        }
        let execution = executor.execute(plan);
        tokio::pin!(execution);
        // Stopping the turn stops the batch and every task in it.
        let mut stopping = false;
        let result = loop {
            tokio::select! {
                result = &mut execution => break result,
                () = tokio::time::sleep(POLL) => {
                    if env.is_cancelled() && !stopping {
                        stopping = true;
                        let _ = store.request_agent_workflow_cancel(&workflow).await;
                        delegator.stop_all();
                    }
                }
            }
        };
        if let Some(context) = &nested {
            reacquire_slot(store, &context.attempt.id).await?;
        }
        match result {
            Ok(execution) => Ok(compact(&execution, &display)),
            Err(error) => {
                if recorder.claimed.load(Ordering::Acquire) {
                    let _ = store
                        .fail_agent_workflow_execution(&workflow, &error.to_string())
                        .await;
                }
                Err(error.to_string())
            }
        }
    }

    /// What runs a batch of `parent`: its tasks' delegator, the recorder of
    /// their attempts, and the executor over both.
    #[allow(clippy::too_many_arguments)]
    fn batch_executor(
        &self,
        project: &str,
        root: &str,
        parent: &str,
        workflow: &str,
        registry: CapabilityRegistry,
        host: DelegationHostPolicy,
        generation: Option<i64>,
        lineage: Option<AgentDelegationLineage>,
    ) -> (Arc<Delegator>, Arc<Recorder>, DelegationExecutor) {
        let delegator = Arc::new(Delegator {
            server: self.clone(),
            project: project.into(),
            root: root.into(),
            parent: parent.into(),
            active: Mutex::new(HashMap::new()),
        });
        let recorder = Arc::new(Recorder {
            store: self.0.store.clone(),
            attempts: Default::default(),
            claimed: AtomicBool::new(false),
            generation,
        });
        // A root batch's tasks are its first level; a task's own batch
        // carries the root's lineage one level down.
        let lineage = lineage.unwrap_or_else(|| AgentDelegationLineage {
            root_workflow_id: workflow.into(),
            parent_attempt_id: None,
            depth: 1,
        });
        // A method search step is run by the host itself.
        let driver = crate::method_search::coordinator::StoreWorkflowRunActivityDriver::new(
            self.0.store.clone(),
            project.into(),
            PathBuf::from(root),
            Arc::new(crate::method_search::control::HostGenerators {
                server: self.clone(),
                frame: Some(parent.into()),
            }),
            self.method_search_evaluator(),
        );
        let executor = DelegationExecutor::new(delegator.clone())
            .with_observer(recorder.clone())
            .with_lineage(lineage)
            .with_run_activity_driver(Arc::new(driver))
            .with_dynamic_policy(registry, host);
        (delegator, recorder, executor)
    }

    /// Runs a batch after the turn; its result is delivered to `parent`.
    /// With `completed`, those tasks keep their earlier results.
    fn run_in_background(
        &self,
        parent: &str,
        delivery: AgentWorkflowDelivery,
        display: HashMap<String, String>,
        executor: DelegationExecutor,
        plan: DelegationPlan,
        completed: Option<Vec<wisp_core::DelegationStepExecution>>,
    ) {
        let server = self.clone();
        let parent = parent.to_owned();
        tokio::spawn(async move {
            let store = server.0.store.clone();
            let executed = match completed {
                Some(completed) => executor.execute_with_completed_steps(plan, completed).await,
                None => executor.execute(plan).await,
            };
            let settled = match executed {
                Ok(execution) => {
                    complete_delivery(&store, &delivery, compact(&execution, &display)).await
                }
                Err(error) => settle_failed(&store, &delivery, &error.to_string()).await,
            };
            if let Err(error) = settled {
                tracing::warn!(%error, batch = %delivery.workflow_id, "Could not keep a background batch's result");
            }
            server.deliver_batches(&parent).await;
        });
    }

    /// Carries on a running batch whose steps have all ended, from what
    /// they recorded: one whose method search was resumed after the app
    /// restarted, so nothing in this run of the app was waiting on it. A
    /// background batch's result is then delivered as usual. The WebView's
    /// `resume_inline_agent_workflow`.
    pub(crate) async fn continue_delegated_batch(&self, batch: &str) -> anyhow::Result<()> {
        let store = &self.0.store;
        let workflow = store
            .get_agent_workflow(batch)
            .await?
            .ok_or_else(|| anyhow::anyhow!("The batch no longer exists"))?;
        anyhow::ensure!(
            workflow.status == AgentWorkflowStatus::Running && workflow.depth == 0,
            "Only a running batch carries on after its method search"
        );
        let session = workflow
            .frame_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("The batch has no conversation"))?;
        let plan: DelegationPlan = serde_json::from_str(&workflow.plan_json)?;
        let (registry, host) = self
            .delegation_policy(&session)
            .await
            .map_err(anyhow::Error::msg)?;
        let mut latest = HashMap::<String, AgentWorkflowAttempt>::new();
        for attempt in store.list_agent_workflow_attempts(batch).await? {
            if latest
                .get(&attempt.step_id)
                .is_none_or(|kept| kept.attempt < attempt.attempt)
            {
                latest.insert(attempt.step_id.clone(), attempt);
            }
        }
        let mut prior = Vec::new();
        for attempt in latest.into_values() {
            anyhow::ensure!(
                attempt.status.is_terminal(),
                "A step of the batch is still running"
            );
            prior.push(wisp_core::DelegationStepExecution {
                response: attempt_response(Some(&attempt), &attempt.step_id),
                step_id: attempt.step_id,
            });
        }
        let root = self.project_root(&workflow.project_id).await?;
        let display = display_task_ids(&plan);
        let (_, _, executor) = self.batch_executor(
            &workflow.project_id,
            &root,
            &session,
            batch,
            registry,
            host,
            None,
            None,
        );
        let executed = executor.resume(plan, prior).await;
        let pending = store
            .list_agent_workflow_deliveries(batch)
            .await?
            .into_iter()
            .filter(|delivery| delivery.result_json.is_none())
            .last();
        if let Some(delivery) = pending {
            let settled = match &executed {
                Ok(execution) => {
                    complete_delivery(store, &delivery, compact(execution, &display)).await
                }
                Err(error) => settle_failed(store, &delivery, &error.to_string()).await,
            };
            if let Err(error) = settled {
                tracing::warn!(%error, %batch, "Could not keep a carried-on batch's result");
            }
            self.deliver_batches(&session).await;
        }
        executed.map(drop)
    }

    /// Runs a failed or stopped batch of `session` again, in the
    /// background: tasks that succeeded keep their results, the others run
    /// anew, and the result is delivered to the conversation, whose agent
    /// is resumed to sum it up.
    pub async fn retry_delegated_batch(
        &self,
        session: &str,
        batch: &str,
        budgets: &HashMap<String, u32>,
    ) -> anyhow::Result<()> {
        let store = &self.0.store;
        let workflow = store
            .get_agent_workflow(batch)
            .await?
            .ok_or_else(|| anyhow::anyhow!("The batch no longer exists"))?;
        anyhow::ensure!(
            workflow.frame_id.as_deref() == Some(session),
            "The batch was not started from this conversation"
        );
        anyhow::ensure!(
            matches!(
                workflow.status,
                AgentWorkflowStatus::Failed | AgentWorkflowStatus::Cancelled
            ),
            "Only a batch that failed or was stopped can be run again"
        );
        anyhow::ensure!(
            workflow.depth == 0,
            "A task's own batch runs again with the task that started it"
        );
        let mut plan: DelegationPlan = serde_json::from_str(&workflow.plan_json)?;
        let (registry, host) = self
            .delegation_policy(session)
            .await
            .map_err(anyhow::Error::msg)?;
        let completed = successful_steps(store, batch).await?;
        let finished: HashSet<String> = completed.iter().map(|step| step.step_id.clone()).collect();
        let display = display_task_ids(&plan);
        let revised = revise_budgets(&mut plan, &display, &finished, budgets, &registry, &host)
            .map_err(anyhow::Error::msg)?;
        registry.validate_resolved_plan(&plan, &host).map_err(|_| {
            anyhow::anyhow!(
                "The models or tools changed since this batch was planned; ask the agent to delegate it again"
            )
        })?;
        let root = self.project_root(&workflow.project_id).await?;
        let approved = if revised {
            // Only the budgets change; the tasks keep their ids, so the
            // finished ones are still kept.
            let (mut replacement, steps) =
                records(&plan, &workflow.project_id, &root, session, &host)?;
            replacement.workspace_id.clone_from(&workflow.workspace_id);
            replacement
                .root_workflow_id
                .clone_from(&workflow.root_workflow_id);
            replacement
                .parent_attempt_id
                .clone_from(&workflow.parent_attempt_id);
            replacement.depth = workflow.depth;
            replacement.description.clone_from(&workflow.description);
            replacement.version = workflow.version;
            replacement.enabled = workflow.enabled;
            replacement.created_at = workflow.created_at;
            store
                .replace_agent_workflow_plan_for_retry(
                    &replacement,
                    &steps,
                    workflow.status,
                    workflow.version,
                )
                .await?
        } else {
            store
                .transition_agent_workflow_status(
                    batch,
                    workflow.status,
                    AgentWorkflowStatus::Approved,
                )
                .await?
        };
        anyhow::ensure!(approved, "The batch changed meanwhile; try again");
        let delivery = store.create_agent_workflow_delivery(batch, true).await?;
        let (_, _, executor) = self.batch_executor(
            &workflow.project_id,
            &root,
            session,
            batch,
            registry,
            host,
            Some(delivery.generation),
            None,
        );
        self.run_in_background(session, delivery, display, executor, plan, Some(completed));
        Ok(())
    }

    /// Plans a batch the researcher launched in `session` (a workflow run
    /// as agents) and keeps it in Draft for them to approve, as the
    /// WebView's `create_agent_workflow` does. Without `review`, a batch
    /// that asks for no approval starts at once. Returns the batch and
    /// whether it started.
    pub(crate) async fn draft_batch(
        &self,
        project: &str,
        session: &str,
        args: Value,
        review: bool,
    ) -> anyhow::Result<(String, bool)> {
        let store = &self.0.store;
        let batch: BatchArgs = serde_json::from_value(args)
            .map_err(|error| anyhow::anyhow!("The workflow's tasks: {error}"))?;
        validate(&batch).map_err(anyhow::Error::msg)?;
        let (registry, host) = self
            .delegation_policy(session)
            .await
            .map_err(anyhow::Error::msg)?;
        let workflow = uuid::Uuid::new_v4().to_string();
        let experts = crate::experts::list(store).await;
        let local = self.local_context_revision().await;
        let plan = resolve(
            workflow.clone(),
            batch,
            &registry,
            &host,
            &experts,
            local.as_deref(),
        )
        .map_err(anyhow::Error::msg)?;
        let root = self.project_root(project).await?;
        let (mut record, steps) = records(&plan, project, &root, session, &host)?;
        record.requires_confirmation = review || plan.requires_confirmation;
        record.description = LAUNCHED.into();
        store.create_agent_workflow_plan(&record, &steps).await?;
        if record.requires_confirmation {
            return Ok((workflow, false));
        }
        self.approve_delegated_batch(session, &workflow).await?;
        Ok((workflow, true))
    }

    /// Approves a drafted batch of `session` and runs it in the
    /// background; its result is delivered to the conversation.
    pub async fn approve_delegated_batch(&self, session: &str, batch: &str) -> anyhow::Result<()> {
        let store = &self.0.store;
        let workflow = self.drafted_batch(session, batch).await?;
        let plan: DelegationPlan = serde_json::from_str(&workflow.plan_json)?;
        let (registry, host) = self
            .delegation_policy(session)
            .await
            .map_err(anyhow::Error::msg)?;
        registry.validate_resolved_plan(&plan, &host).map_err(|_| {
            anyhow::anyhow!(
                "The models or tools changed since this batch was drafted; draft it again"
            )
        })?;
        anyhow::ensure!(
            store
                .approve_agent_workflow_plan(batch, workflow.version)
                .await?,
            "The batch changed meanwhile; try again"
        );
        let root = self.project_root(&workflow.project_id).await?;
        let display = display_task_ids(&plan);
        let (_, _, executor) = self.batch_executor(
            &workflow.project_id,
            &root,
            session,
            batch,
            registry,
            host,
            None,
            None,
        );
        let delivery = store.create_agent_workflow_delivery(batch, true).await?;
        self.run_in_background(session, delivery, display, executor, plan, None);
        Ok(())
    }

    /// Throws away a drafted batch of `session`, which never ran.
    pub async fn discard_delegated_batch(&self, session: &str, batch: &str) -> anyhow::Result<()> {
        self.drafted_batch(session, batch).await?;
        anyhow::ensure!(
            self.0.store.delete_agent_workflow(batch).await?,
            "The batch is no longer there"
        );
        Ok(())
    }

    /// A batch of `session` that waits in Draft.
    async fn drafted_batch(
        &self,
        session: &str,
        batch: &str,
    ) -> anyhow::Result<wisp_store::AgentWorkflow> {
        let workflow = self
            .0
            .store
            .get_agent_workflow(batch)
            .await?
            .ok_or_else(|| anyhow::anyhow!("The batch no longer exists"))?;
        anyhow::ensure!(
            workflow.frame_id.as_deref() == Some(session),
            "The batch was not started from this conversation"
        );
        // An agent's own batch is approved on its plan card instead.
        anyhow::ensure!(
            workflow.status == AgentWorkflowStatus::Draft
                && workflow.depth == 0
                && workflow.description == LAUNCHED,
            "Only a workflow that waits for approval can be approved or discarded"
        );
        Ok(workflow)
    }

    /// The latest full result of one task of a batch `parent` started.
    pub(crate) async fn delegated_result(
        &self,
        parent: &str,
        args: &Value,
    ) -> Result<Value, String> {
        let required = |name: &str| {
            args.get(name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("'{name}' is required"))
        };
        let workflow_id = required("workflow_id")?;
        let task_id = required("task_id")?;
        let store = &self.0.store;
        let e = |error: anyhow::Error| error.to_string();
        let workflow = store
            .get_agent_workflow(workflow_id)
            .await
            .map_err(e)?
            .ok_or("workflow does not exist")?;
        if workflow.frame_id.as_deref() != Some(parent) {
            return Err("workflow does not belong to this conversation".into());
        }
        let stored = stored_task_id(workflow_id, task_id);
        let attempt = store
            .list_agent_workflow_attempts(workflow_id)
            .await
            .map_err(e)?
            .into_iter()
            .filter(|attempt| attempt.step_id == stored)
            .max_by_key(|attempt| attempt.attempt)
            .ok_or("task does not exist in this workflow or has not run")?;
        Ok(json!({
            "workflow_id": workflow_id,
            "task_id": task_id,
            "stored_step_id": stored,
            "attempt": attempt.attempt,
            "status": attempt.status,
            "response": attempt
                .response_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok()),
            "error": attempt.error,
        }))
    }

    /// The latest work over many items the conversation ran, as it went.
    pub fn swarm_progress(&self, session: &str) -> Option<wisp_dto::SwarmProgress> {
        self.session(session).live.lock().unwrap().swarm.clone()
    }

    /// The batches a conversation started, newest first, with each task's
    /// latest attempt.
    pub async fn delegated_batches(
        &self,
        project: &str,
        session: &str,
    ) -> anyhow::Result<Vec<DelegatedBatch>> {
        let store = &self.0.store;
        let mut batches = Vec::new();
        for workflow in store.list_agent_workflows(project).await? {
            if workflow.frame_id.as_deref() != Some(session) {
                continue;
            }
            let Ok(plan) = serde_json::from_str::<DelegationPlan>(&workflow.plan_json) else {
                continue;
            };
            let attempts = store.list_agent_workflow_attempts(&workflow.id).await?;
            let display = display_task_ids(&plan);
            let tasks = plan
                .steps
                .iter()
                .map(|step| {
                    let latest = attempts
                        .iter()
                        .filter(|attempt| attempt.step_id == step.id)
                        .max_by_key(|attempt| attempt.attempt);
                    let summary = latest
                        .and_then(|attempt| {
                            serde_json::from_str::<Value>(&attempt.output_json).ok()
                        })
                        .and_then(|output| {
                            output
                                .get("summary")
                                .and_then(Value::as_str)
                                .map(|text| bounded_chars(text, INLINE_TEXT_CHARS))
                        });
                    DelegatedTask {
                        id: display
                            .get(&step.id)
                            .cloned()
                            .unwrap_or_else(|| step.id.clone()),
                        instruction: step.spec.goal.clone(),
                        depends_on: step
                            .spec
                            .dependencies
                            .iter()
                            .map(|dependency| {
                                display
                                    .get(dependency)
                                    .cloned()
                                    .unwrap_or_else(|| dependency.clone())
                            })
                            .collect(),
                        capabilities: step.spec.capabilities.clone(),
                        expert: match &step.spec.origin {
                            wisp_core::AgentOrigin::Specialist(expert) => Some(expert.name.clone()),
                            _ => None,
                        },
                        status: latest
                            .map(|attempt| attempt_status_name(attempt.status))
                            .unwrap_or_else(|| "queued".into()),
                        summary,
                        error: latest.and_then(|attempt| attempt.error.clone()),
                        conversation: latest.and_then(|attempt| attempt.child_frame_id.clone()),
                        input_tokens: latest.map_or(0, |attempt| attempt.input_tokens),
                        output_tokens: latest.map_or(0, |attempt| attempt.output_tokens),
                        tool_calls: latest.map_or(0, |attempt| attempt.tool_calls),
                        max_tokens: step.spec.budget.max_tokens,
                        isolated: step.spec.workspace_policy
                            == Some(wisp_core::AgentWorkspacePolicy::Isolated),
                    }
                })
                .collect();
            batches.push(DelegatedBatch {
                id: workflow.id.clone(),
                goal: workflow.goal.clone(),
                status: status_name(workflow.status),
                created_at: workflow.created_at,
                updated_at: workflow.updated_at,
                tasks,
                nested: workflow.depth > 0,
                launched: workflow.description == LAUNCHED,
            });
        }
        batches.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(batches)
    }

    /// Stops a running batch of `session` and every task in it.
    pub async fn cancel_delegated_batch(&self, session: &str, batch: &str) -> anyhow::Result<()> {
        let store = &self.0.store;
        let workflow = store
            .get_agent_workflow(batch)
            .await?
            .ok_or_else(|| anyhow::anyhow!("The batch no longer exists"))?;
        anyhow::ensure!(
            workflow.frame_id.as_deref() == Some(session),
            "The batch was not started from this conversation"
        );
        store.request_agent_workflow_cancel(batch).await?;
        for attempt in store.list_agent_workflow_attempts(batch).await? {
            if attempt.status == AgentWorkflowAttemptStatus::Running {
                if let Some(child) = attempt.child_frame_id {
                    self.session(&child).stop();
                }
            }
        }
        Ok(())
    }

    /// The tools a conversation delegates task batches with.
    pub(crate) async fn delegation_tools(
        &self,
        project: &str,
        root: &str,
        parent: &str,
    ) -> Vec<Box<dyn Tool>> {
        let experts = crate::experts::list(&self.0.store).await;
        vec![
            Box::new(DelegateTasksTool {
                server: self.clone(),
                project: project.into(),
                root: root.into(),
                parent: parent.into(),
                schema: delegate_schema(&experts),
            }),
            Box::new(DelegatedResultTool {
                server: self.clone(),
                parent: parent.into(),
            }),
        ]
    }
}

fn delegate_schema(experts: &[crate::experts::Expert]) -> ToolSchema {
    let capabilities = [
        "reasoning: think and write without tools",
        "project_read: read, search and grep the project",
        "project_write: also write and edit project files",
        "code_run: run commands in an execution context and follow the Runs",
        "review: an independent read-only reviewer that returns findings",
        "image_inspection: look at images, when a vision model is configured",
        "visualization: write files and run Python or R to make figures, when a runtime is installed",
        "delegation: may hand part of its task to a batch of its own, one level down, within this batch's limits",
    ]
    .join("; ");
    let mut schema = ToolSchema::new(
        DELEGATE,
        DESCRIPTION,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "goal": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_GOAL_CHARS,
                    "description": "Overall delegated outcome that you will synthesize"
                },
                "context": {
                    "type": "string",
                    "maxLength": MAX_CONTEXT_CHARS,
                    "description": "Bounded shared constraints and project state every task needs; do not paste the transcript"
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
                                "items": {"type": "string", "pattern": "^[a-z][a-z0-9_-]{0,30}$"},
                                "description": "Task ids from this batch that must succeed first"
                            },
                            "capabilities": {
                                "type": "array",
                                "minItems": 1,
                                "uniqueItems": true,
                                "items": {"type": "string"},
                                "description": capabilities
                            },
                            "output_schema": {
                                "type": "object",
                                "description": "Optional bounded JSON Schema for this task's data result"
                            },
                            "model_tier": {
                                "type": "string",
                                "enum": ["fast", "inherit"],
                                "description": "fast (default): the configured subagent model. inherit: this conversation's own model, only for a task that needs its full capability."
                            },
                            "budget": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "max_tokens": {"type": "integer", "minimum": 0},
                                    "max_tool_calls": {"type": "integer", "minimum": 0},
                                    "max_cost_microunits": {"type": "integer", "minimum": 0}
                                },
                                "description": "Optional per-task limits; omit or set 0 for unlimited"
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "Optional wall-clock limit in seconds; omit or 0 for none"
                            },
                            "isolated": {
                                "type": "boolean",
                                "description": "Work in a temporary Git worktree (Git projects only): its changes are cherry-picked into the project when it succeeds without conflict, otherwise kept as a patch artifact"
                            },
                            "task_kind": {
                                "type": "string",
                                "enum": ["agent", "run_activity"],
                                "description": "agent (default), or run_activity: a computational method search the host runs over the contract a direct dependency froze with prepare_method_search. A run_activity task has no capabilities or other Agent fields; the batch is approved first and the researcher starts the search from its Run after reading the contract. Run such a batch in the background."
                            },
                            "run_activity": {
                                "type": "object",
                                "additionalProperties": false,
                                "description": "For a run_activity task only. Its budget must equal the one the contract was frozen with.",
                                "properties": {
                                    "activity": {"type": "string", "enum": ["method_search"]},
                                    "context_id": {"type": "string", "enum": ["local"]},
                                    "input_task_id": {"type": "string", "description": "The dependency whose output_schema requires method_search_spec_artifact_version_id"},
                                    "spec_output_pointer": {"type": "string", "enum": ["method_search_spec_artifact_version_id"]},
                                    "max_candidates": {"type": "integer", "minimum": 1, "maximum": 50},
                                    "max_wall_seconds": {"type": "integer", "minimum": 1, "maximum": 604800},
                                    "max_evaluator_seconds": {"type": "integer", "minimum": 1, "maximum": 300},
                                    "max_cost_microunits": {"type": "integer", "minimum": 1}
                                },
                                "required": ["activity", "input_task_id", "max_candidates", "max_wall_seconds", "max_evaluator_seconds", "max_cost_microunits"]
                            }
                        },
                        "required": ["id", "instruction"]
                    }
                }
            },
            "required": ["goal", "tasks"]
        }),
    );
    // The researcher's experts, when there are any, as personas for a task.
    if !experts.is_empty() {
        let help = experts
            .iter()
            .map(|expert| match expert.description.trim() {
                "" => format!("{}: {}", expert.id, expert.name),
                description => format!("{}: {} — {description}", expert.id, expert.name),
            })
            .collect::<Vec<_>>()
            .join("; ");
        schema.function.parameters["properties"]["tasks"]["items"]["properties"]["specialist_id"] = json!({
            "type": "string",
            "enum": experts.iter().map(|expert| expert.id.clone()).collect::<Vec<_>>(),
            "description": format!("Optional: one of the researcher's experts to do the task as, with its instructions. Omit for a temporary generic Agent. {help}"),
        });
    }
    schema.function.parameters["properties"]["background"] = json!({
        "type": "boolean",
        "description": "Run the batch without blocking this turn, only for long work such as analyses expected to take many minutes. The call returns a handle at once; the result is delivered to this conversation when the batch ends and you are resumed to synthesize it. Do not poll it or claim it finished. Omit to wait for the results in this turn."
    });
    schema
}

struct DelegateTasksTool {
    server: Server,
    project: String,
    root: String,
    parent: String,
    schema: ToolSchema,
}

#[async_trait]
impl Tool for DelegateTasksTool {
    fn name(&self) -> &str {
        DELEGATE
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
        match self
            .server
            .delegate(&self.project, &self.root, &self.parent, args, env)
            .await
        {
            Ok(value) => {
                let denied = value.get("status").and_then(Value::as_str) == Some("denied");
                let result = ToolResult::ok(value.to_string());
                if denied {
                    result.stop_batch()
                } else {
                    result
                }
            }
            Err(error) => ToolResult::fail(format!("delegate_tasks error: {error}")),
        }
    }
}

struct DelegatedResultTool {
    server: Server,
    parent: String,
}

#[async_trait]
impl Tool for DelegatedResultTool {
    fn name(&self) -> &str {
        RESULT
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            RESULT,
            "Read the latest full result of one task from an earlier delegate_tasks batch of this conversation. Use only when the compact inline result says it was truncated or lacks needed detail.",
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

    fn read_only(&self) -> bool {
        true
    }

    fn preview(&self, args: &Value) -> String {
        args.get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match self.server.delegated_result(&self.parent, args).await {
            Ok(value) => ToolResult::ok(value.to_string()),
            Err(error) => ToolResult::fail(format!("get_delegated_result error: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(tasks: Value) -> BatchArgs {
        serde_json::from_value(json!({"goal": "Compare two methods", "tasks": tasks})).unwrap()
    }

    #[test]
    fn a_batch_needs_known_acyclic_dependencies_and_valid_ids() {
        let ok = batch(json!([
            {"id": "read", "instruction": "Read", "capabilities": ["project_read"]},
            {"id": "sum", "instruction": "Sum up", "capabilities": ["reasoning"], "depends_on": ["read"]},
        ]));
        assert_eq!(validate(&ok), Ok(()));
        let cycle = batch(json!([
            {"id": "a", "instruction": "A", "capabilities": ["reasoning"], "depends_on": ["b"]},
            {"id": "b", "instruction": "B", "capabilities": ["reasoning"], "depends_on": ["a"]},
        ]));
        assert_eq!(
            validate(&cycle),
            Err("task dependencies contain a cycle".into())
        );
        let unknown = batch(json!([
            {"id": "a", "instruction": "A", "capabilities": ["reasoning"], "depends_on": ["z"]},
        ]));
        assert!(validate(&unknown).unwrap_err().contains("unknown task z"));
        let bad = batch(json!([{"id": "Bad", "instruction": "A", "capabilities": ["reasoning"]}]));
        assert!(validate(&bad).unwrap_err().contains("invalid task id"));
        let none = batch(json!([{"id": "a", "instruction": "A", "capabilities": []}]));
        assert!(validate(&none)
            .unwrap_err()
            .contains("at least one capability"));
        // A field this host does not honour is refused, not ignored.
        assert!(serde_json::from_value::<BatchArgs>(json!({
            "goal": "g",
            "tasks": [{"id": "a", "instruction": "A", "capabilities": ["reasoning"], "skill_ids": ["x"]}],
        }))
        .is_err());
        // Working in a worktree of its own needs a Git project.
        let isolated = json!([{"id": "a", "instruction": "A", "capabilities": ["project_write"], "isolated": true}]);
        let refused = plan(isolated.clone(), false).unwrap_err();
        assert!(!refused.is_empty());
        let git = resolve(
            "wf".into(),
            batch(isolated),
            &CapabilityRegistry::builtins(),
            &host_policy(vec![worker(false)], false, true),
            &[],
            None,
        )
        .unwrap();
        assert_eq!(
            git.steps[0].spec.workspace_policy,
            Some(wisp_core::AgentWorkspacePolicy::Isolated)
        );
    }

    #[test]
    fn a_method_search_step_needs_a_preparing_task_and_a_sealed_local_budget() {
        let prep = json!({
            "id": "prep",
            "instruction": "Freeze the contract",
            "capabilities": ["code_run"],
            "output_schema": {
                "type": "object",
                "properties": {"method_search_spec_artifact_version_id": {"type": "string"}},
                "required": ["method_search_spec_artifact_version_id"],
            },
        });
        let search = |activity: Value| {
            json!({
                "id": "search",
                "instruction": "Search",
                "task_kind": "run_activity",
                "depends_on": ["prep"],
                "run_activity": activity,
            })
        };
        let budget = json!({
            "activity": "method_search",
            "input_task_id": "prep",
            "max_candidates": 3,
            "max_wall_seconds": 600,
            "max_evaluator_seconds": 30,
            "max_cost_microunits": 1000,
        });
        let resolved = plan(json!([prep.clone(), search(budget.clone())]), false).unwrap();
        let step = &resolved.steps[1];
        assert_eq!(step.task_kind, WorkflowTaskKind::RunActivity);
        let activity = step.run_activity.as_ref().unwrap();
        assert_eq!(activity.context_id, wisp_runtime::LOCAL_CONTEXT_ID);
        assert_eq!(activity.context_revision, LOCAL_REVISION);
        assert_eq!(activity.input_task_id, "wf:prep");
        assert_eq!(activity.spec_output_pointer, SPEC_POINTER);
        assert!(!activity.integrity_hash.is_empty());
        assert_eq!(step.spec.dependencies, vec!["wf:prep".to_string()]);
        assert!(resolved.requires_confirmation);

        // The preparing task must promise the spec's id.
        let mut silent = prep.clone();
        silent.as_object_mut().unwrap().remove("output_schema");
        assert!(validate(&batch(json!([silent, search(budget.clone())])))
            .unwrap_err()
            .contains("declare required output"));
        // It must be a direct dependency, and the budget must be bounded.
        let mut undepended = search(budget.clone());
        undepended["depends_on"] = json!([]);
        assert!(validate(&batch(json!([prep.clone(), undepended])))
            .unwrap_err()
            .contains("direct dependency"));
        let mut unbounded = budget.clone();
        unbounded["max_candidates"] = json!(0);
        assert!(validate(&batch(json!([prep.clone(), search(unbounded)])))
            .unwrap_err()
            .contains("invalid activity or budget"));
        // An agent-only field on the step is refused rather than ignored.
        let mut mixed = search(budget.clone());
        mixed["capabilities"] = json!(["code_run"]);
        assert!(validate(&batch(json!([prep.clone(), mixed])))
            .unwrap_err()
            .contains("Agent-only fields"));
        // Without this computer's context there is nothing to seal it to.
        let unsealed = resolve(
            "wf".into(),
            batch(json!([prep, search(budget)])),
            &CapabilityRegistry::builtins(),
            &host_policy(vec![worker(false)], false, false),
            &[],
            None,
        );
        assert!(unsealed.is_err());
    }

    fn worker(external: bool) -> ModelProfilePolicy {
        ModelProfilePolicy {
            id: WORKER.into(),
            features: vec![],
            external,
            enabled: true,
        }
    }

    /// A canonical SHA-256, as the host seals this computer's context with.
    const LOCAL_REVISION: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn plan(tasks: Value, external: bool) -> Result<DelegationPlan, String> {
        resolve(
            "wf".into(),
            batch(tasks),
            &CapabilityRegistry::builtins(),
            &host_policy(vec![worker(external)], false, false),
            &[],
            Some(LOCAL_REVISION),
        )
    }

    #[test]
    fn a_task_gets_only_the_tools_its_capabilities_grant_and_changes_are_approved_first() {
        let local = plan(
            json!([
                {"id": "read", "instruction": "Read", "capabilities": ["project_read"]},
                {"id": "run", "instruction": "Run it", "capabilities": ["code_run"], "depends_on": ["read"]},
                {"id": "check", "instruction": "Check", "capabilities": ["review"], "depends_on": ["run"]},
            ]),
            false,
        )
        .unwrap();
        let tools: Vec<_> = local
            .steps
            .iter()
            .map(|step| allowed_tools(&step.spec))
            .collect();
        assert_eq!(tools[0], ["read", "search", "grep"]);
        // Running code includes freezing a method search's contract.
        for tool in [
            "run_in_context",
            "get_run",
            "cancel_run",
            "prepare_method_search",
        ] {
            assert!(tools[1].iter().any(|kept| kept == tool), "{tools:?}");
        }
        // A reviewer changes nothing.
        assert!(tools[2]
            .iter()
            .all(|tool| matches!(tool.as_str(), "read" | "search" | "grep")));
        // Running code needs the researcher's yes; only reading does not,
        // unless the model is a remote one.
        assert!(local.requires_confirmation);
        let reading =
            json!([{"id": "read", "instruction": "Read", "capabilities": ["project_read"]}]);
        assert!(!plan(reading.clone(), false).unwrap().requires_confirmation);
        assert!(plan(reading, true).unwrap().requires_confirmation);
        // Dependencies are kept under the batch; the agent's ids are shown.
        let display = display_task_ids(&local);
        assert_eq!(display["wf:read"], "read");
        assert_eq!(local.steps[1].spec.dependencies, ["wf:read"]);
        let prompt = approval_prompt(&local, &display);
        assert!(prompt.starts_with(wisp_tools::plan::PLAN_APPROVAL_PREFIX));
        assert!(prompt.contains("[ ] run: Run it"));
        // What this host does not have is refused.
        assert!(plan(
            json!([{"id": "web", "instruction": "Search", "capabilities": ["external_research"]}]),
            false,
        )
        .is_err());
        let limits = root_limits(&local, &host_policy(vec![worker(false)], false, false));
        assert_eq!(limits.wall_time_secs, None);
        assert_eq!(limits.max_tokens, 0);
    }

    #[test]
    fn a_task_can_be_done_as_one_of_the_researchers_experts() {
        let expert = crate::experts::Expert {
            id: "statistician".into(),
            name: "Statistician".into(),
            description: "Checks the tests".into(),
            instructions: "Report effect sizes with intervals.".into(),
            builtin: false,
            model: String::new(),
            skills: None,
            connectors: None,
        };
        let experts = [expert];
        let resolved = resolve(
            "wf".into(),
            batch(json!([
                {"id": "check", "instruction": "Check the tests", "capabilities": ["reasoning"], "specialist_id": "statistician"},
            ])),
            &CapabilityRegistry::builtins(),
            &host_policy(vec![worker(false)], false, false),
            &experts,
            None,
        )
        .unwrap();
        let spec = &resolved.steps[0].spec;
        assert!(spec
            .prompt_template
            .contains("Report effect sizes with intervals."));
        assert!(
            matches!(&spec.origin, wisp_core::AgentOrigin::Specialist(found) if found.id == "statistician")
        );
        // An expert that is not there is refused, not ignored.
        assert!(resolve(
            "wf".into(),
            batch(json!([
                {"id": "check", "instruction": "Check", "capabilities": ["reasoning"], "specialist_id": "ghost"},
            ])),
            &CapabilityRegistry::builtins(),
            &host_policy(vec![worker(false)], false, false),
            &experts,
            None,
        )
        .unwrap_err()
        .contains("unknown expert"));
        // The schema offers them by id, and offers none when there are none.
        let schema = delegate_schema(&experts);
        let items = &schema.function.parameters["properties"]["tasks"]["items"]["properties"];
        assert_eq!(items["specialist_id"]["enum"], json!(["statistician"]));
        let bare = delegate_schema(&[]);
        assert!(
            bare.function.parameters["properties"]["tasks"]["items"]["properties"]
                .get("specialist_id")
                .is_none()
        );
    }

    #[test]
    fn a_result_is_read_from_the_json_in_the_final_answer_or_kept_as_text() {
        let resolved = plan(
            json!([
                {"id": "read", "instruction": "Read", "capabilities": ["project_read"]},
                {"id": "check", "instruction": "Check", "capabilities": ["review"]},
            ]),
            false,
        )
        .unwrap();
        let spec = &resolved.steps[0].spec;
        let fenced =
            "Done.\n```json\n{\"summary\": \"two methods agree\", \"risks\": [\"small n\"]}\n```";
        let output = parse_result(fenced, spec).unwrap();
        assert_eq!(output["summary"], "two methods agree");
        assert_eq!(output["risks"], json!(["small n"]));
        assert_eq!(output["files_changed"], json!([]));
        let plain = parse_result("The methods agree.", spec).unwrap();
        assert_eq!(plain["summary"], "The methods agree.");
        assert!(wisp_core::is_degraded_delivery(&plain));
        assert!(parse_result("  ", spec).is_err());
        // A reviewer's answer must be the findings it was asked for.
        let reviewer = &resolved.steps[1].spec;
        assert!(parse_result("Looks fine.", reviewer).is_err());
        assert!(parse_result("{\"summary\": \"ok\"}", reviewer).is_err());
        assert!(parse_result("{\"summary\": \"ok\", \"findings\": []}", reviewer).is_ok());
    }

    #[test]
    fn a_task_delegates_within_what_it_was_granted_and_only_one_level_down() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy(vec![worker(false)], false, false);
        let mut resolved = plan(
            json!([
                {"id": "lead", "instruction": "Lead", "capabilities": ["reasoning", "delegation"]},
                {"id": "read", "instruction": "Read", "capabilities": ["project_read"]},
            ]),
            false,
        )
        .unwrap();
        let lead = resolved.steps[0].spec.clone();
        assert!(lead.allow_delegation);
        assert!(!resolved.steps[1].spec.allow_delegation);
        // Granting delegation is approved first and allows one more level.
        assert!(resolved.requires_confirmation);
        let limits = root_limits(&resolved, &host);
        assert_eq!(limits.max_depth, 2);
        let (_, nested) = nested_policy(&registry, &host, &lead, 1, &limits).unwrap();
        // Its own tasks get what it had, without delegating again.
        assert_eq!(nested.enabled_capabilities, ["reasoning"]);
        assert_eq!(nested.default_model_id, lead.model);
        assert_eq!(nested.models.len(), 1);
        assert!(nested_policy(&registry, &host, &lead, 2, &limits).is_err());
        let reader = resolved.steps[1].spec.clone();
        assert!(nested_policy(&registry, &host, &reader, 1, &limits).is_err());
        // A batch of reading tasks only stays at one level.
        let flat = plan(
            json!([{"id": "read", "instruction": "Read", "capabilities": ["project_read"]}]),
            false,
        )
        .unwrap();
        assert_eq!(root_limits(&flat, &host).max_depth, 1);
        // Its tasks are named after the task that started them.
        namespace_display_ids(&mut resolved, "lead");
        let display = display_task_ids(&resolved);
        let mut names: Vec<_> = display.values().cloned().collect();
        names.sort();
        assert_eq!(names, ["lead/lead", "lead/read"]);
    }

    #[test]
    fn only_a_task_that_did_not_finish_is_given_another_budget() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy(vec![worker(false)], false, false);
        let mut resolved = plan(
            json!([
                {"id": "read", "instruction": "Read", "capabilities": ["project_read"]},
                {"id": "check", "instruction": "Check", "capabilities": ["review"],
                 "depends_on": ["read"]},
            ]),
            false,
        )
        .unwrap();
        let display = display_task_ids(&resolved);
        let step = |plan: &DelegationPlan, task: &str| {
            plan.steps
                .iter()
                .find(|step| display[&step.id] == task)
                .unwrap()
                .clone()
        };
        let read = step(&resolved, "read").id;
        let finished = HashSet::from([read]);
        let budgets = |task: &str, tokens| HashMap::from([(task.to_owned(), tokens)]);
        let revise = |plan: &mut DelegationPlan, budgets: &HashMap<String, u32>| {
            revise_budgets(plan, &display, &finished, budgets, &registry, &host)
        };
        let refused = revise(&mut resolved, &budgets("read", 9_000)).unwrap_err();
        assert!(refused.contains("finished"), "{refused}");
        assert!(revise(&mut resolved, &budgets("gone", 9_000)).is_err());
        assert!(revise(&mut resolved, &budgets("check", 9_000)).unwrap());
        assert_eq!(step(&resolved, "check").spec.budget.max_tokens, Some(9_000));
        // The revised plan still passes the checks a rerun makes.
        registry.validate_resolved_plan(&resolved, &host).unwrap();
        // The same budget again changes nothing; zero is no limit.
        assert!(!revise(&mut resolved, &budgets("check", 9_000)).unwrap());
        assert!(revise(&mut resolved, &budgets("check", 0)).unwrap());
        assert_eq!(step(&resolved, "check").spec.budget.max_tokens, None);
        registry.validate_resolved_plan(&resolved, &host).unwrap();
        assert!(!revise(&mut resolved, &HashMap::new()).unwrap());
    }

    #[tokio::test]
    async fn a_batch_the_app_closed_on_is_settled_delivered_and_run_again() {
        let temp = tempfile::tempdir().unwrap();
        let server = Server::open_with_models(
            crate::tests::config(temp.path()),
            crate::tests::models(Default::default(), false),
        )
        .await
        .unwrap();
        let parent = server.create_conversation(crate::PROJECT).await.unwrap();
        let root = server.project_root(crate::PROJECT).await.unwrap();
        let (registry, host) = server.delegation_policy(&parent).await.unwrap();
        let plan = resolve(
            "wf-1".into(),
            batch(json!([{"id": "read", "instruction": "Read", "capabilities": ["project_read"]}])),
            &registry,
            &host,
            &[],
            None,
        )
        .unwrap();
        let (record, steps) = records(&plan, crate::PROJECT, &root, &parent, &host).unwrap();
        let store = &server.0.store;
        store
            .create_agent_workflow_plan(&record, &steps)
            .await
            .unwrap();
        assert!(store
            .approve_agent_workflow_plan("wf-1", record.version)
            .await
            .unwrap());
        store
            .create_agent_workflow_delivery("wf-1", true)
            .await
            .unwrap();
        assert!(store
            .transition_agent_workflow_status(
                "wf-1",
                AgentWorkflowStatus::Approved,
                AgentWorkflowStatus::Running,
            )
            .await
            .unwrap());
        // The app closes mid-batch and opens again.
        store.recover_interrupted_agent_workflows().await.unwrap();
        server.sweep_deliveries().await;
        let messages = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let messages = store.load_messages(&parent).await.unwrap();
                if messages
                    .iter()
                    .any(|message| message.content.as_text() == "Saved result")
                    && !server.session(&parent).live.lock().unwrap().running
                {
                    return messages;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the settled batch must be delivered and the agent resumed");
        let delivered = messages
            .iter()
            .find(|message| {
                message.tool_name.as_deref() == Some(wisp_store::AGENT_WORKFLOW_COMPLETION_TOOL)
            })
            .unwrap()
            .content
            .as_text();
        assert!(delivered.contains("\"status\":\"failed\""), "{delivered}");
        assert!(delivered.contains("The app stopped before this task produced a result."));
        let batches = server
            .delegated_batches(crate::PROJECT, &parent)
            .await
            .unwrap();
        assert_eq!(batches[0].status, "failed");
        // Delivered once: a second sweep adds nothing.
        server.sweep_deliveries().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let again = store.load_messages(&parent).await.unwrap();
        let count = |messages: &[wisp_llm::Message]| {
            messages
                .iter()
                .filter(|message| {
                    message.tool_name.as_deref() == Some(wisp_store::AGENT_WORKFLOW_COMPLETION_TOOL)
                })
                .count()
        };
        assert_eq!(count(&again), 1);

        // Run again with a larger budget, it does the task and delivers a
        // second result.
        let none = HashMap::new();
        assert!(server
            .retry_delegated_batch("other", "wf-1", &none)
            .await
            .is_err());
        let unknown = HashMap::from([("nope".to_owned(), 10)]);
        let refused = server
            .retry_delegated_batch(&parent, "wf-1", &unknown)
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("no task 'nope'"), "{refused}");
        let larger = HashMap::from([("read".to_owned(), 50_000)]);
        server
            .retry_delegated_batch(&parent, "wf-1", &larger)
            .await
            .unwrap();
        let batches = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let batches = server
                    .delegated_batches(crate::PROJECT, &parent)
                    .await
                    .unwrap();
                let delivered = count(&store.load_messages(&parent).await.unwrap());
                if batches[0].status == "succeeded"
                    && delivered == 2
                    && !server.session(&parent).live.lock().unwrap().running
                {
                    return batches;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the rerun must succeed and be delivered");
        let task = &batches[0].tasks[0];
        assert_eq!(task.status, "succeeded");
        assert_eq!(task.summary.as_deref(), Some("Saved result"));
        assert!(task.conversation.is_some());
        assert_eq!(task.max_tokens, Some(50_000));
        // Only what failed or was stopped runs again.
        let refused = server
            .retry_delegated_batch(&parent, "wf-1", &none)
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("failed or was stopped"), "{refused}");
        server.shutdown().await;
    }
}
