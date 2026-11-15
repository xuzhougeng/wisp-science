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
use std::path::PathBuf;
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
    AgentWorkflowAttemptStatus, AgentWorkflowStatus, AgentWorkflowStep, Store,
    MAX_ROOT_AGENT_TASKS,
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskArgs {
    id: String,
    instruction: String,
    #[serde(default)]
    depends_on: Vec<String>,
    capabilities: Vec<String>,
    #[serde(default)]
    output_schema: Option<Value>,
    #[serde(default)]
    model_tier: Option<ModelTier>,
    #[serde(default)]
    budget: Option<BudgetArgs>,
    #[serde(default)]
    timeout_secs: Option<u64>,
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
    /// queued, running, succeeded, failed, blocked or cancelled.
    pub status: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    /// The conversation the task ran in.
    pub conversation: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tool_calls: i64,
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
        if task.capabilities.is_empty() {
            return Err(format!("task {} requires at least one capability", task.id));
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
) -> Result<DelegationPlan, String> {
    validate(&batch)?;
    let context = batch.context.trim().to_owned();
    let mut steps = Vec::with_capacity(batch.tasks.len());
    for task in batch.tasks {
        let stored = stored_task_id(&workflow, &task.id);
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
                    specialist: None,
                    output_schema: task.output_schema,
                    isolated: false,
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
    let requires_confirmation = !host.auto_safe
        || steps
            .iter()
            .any(|step| !step.spec.approval_reasons.is_empty());
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
        max_depth: 1,
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
            stored.task_kind = "agent".into();
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
            "run_in_context" | "get_run" | "cancel_run" | "python" | "r" => {
                spec.permissions.execute && !reviewer
            }
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
    let delegation = "Do not delegate further.";
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
fn json_candidates(raw: &str) -> Vec<Value> {
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
}

impl Recorder {
    async fn closed_attempt(
        &self,
        request: &AgentDelegationRequest,
        status: AgentWorkflowAttemptStatus,
        reason: &str,
    ) -> anyhow::Result<()> {
        let number = self
            .store
            .next_agent_workflow_attempt_number(&request.step_id)
            .await?;
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
        let number = self
            .store
            .next_agent_workflow_attempt_number(&request.step_id)
            .await?;
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
        Ok((CapabilityRegistry::builtins(), host_policy(models, python)))
    }
}

/// What this host offers delegated tasks, given its models (the worker
/// first) and whether a Python runtime is installed.
fn host_policy(models: Vec<ModelProfilePolicy>, python: bool) -> DelegationHostPolicy {
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
    ];
    let mut capabilities: Vec<String> = [
        "reasoning",
        "project_read",
        "project_write",
        "code_run",
        "review",
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
            self.task_turn(delegator, request, &child).await
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
                &delegator.root,
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
            PathBuf::from(&delegator.root),
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
        let (registry, host) = self.delegation_policy(parent).await?;
        let workflow = uuid::Uuid::new_v4().to_string();
        let plan = resolve(workflow.clone(), batch, &registry, &host)?;
        if env.project_write_locked()
            && plan
                .steps
                .iter()
                .any(|step| step.spec.permissions.write || step.spec.permissions.execute)
        {
            return Err("The project is frozen while an exploration runs; only read-only tasks can be delegated.".into());
        }
        let display = display_task_ids(&plan);
        let (record, steps) =
            records(&plan, project, root, parent, &host).map_err(|error| error.to_string())?;
        store
            .create_agent_workflow_plan(&record, &steps)
            .await
            .map_err(|error| error.to_string())?;
        if plan.requires_confirmation && !env.approval_bypass() {
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
        let delegator = Arc::new(Delegator {
            server: self.clone(),
            project: project.into(),
            root: root.into(),
            parent: parent.into(),
            active: Mutex::new(HashMap::new()),
        });
        let recorder = Arc::new(Recorder {
            store: store.clone(),
            attempts: Default::default(),
            claimed: AtomicBool::new(false),
        });
        let executor = DelegationExecutor::new(delegator.clone())
            .with_observer(recorder.clone())
            .with_lineage(AgentDelegationLineage {
                root_workflow_id: workflow.clone(),
                parent_attempt_id: None,
                depth: 1,
            })
            .with_dynamic_policy(registry, host);
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
            if workflow.frame_id.as_deref() != Some(session) || workflow.depth > 0 {
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
                        status: latest
                            .map(|attempt| attempt_status_name(attempt.status))
                            .unwrap_or_else(|| "queued".into()),
                        summary,
                        error: latest.and_then(|attempt| attempt.error.clone()),
                        conversation: latest.and_then(|attempt| attempt.child_frame_id.clone()),
                        input_tokens: latest.map_or(0, |attempt| attempt.input_tokens),
                        output_tokens: latest.map_or(0, |attempt| attempt.output_tokens),
                        tool_calls: latest.map_or(0, |attempt| attempt.tool_calls),
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
    pub(crate) fn delegation_tools(
        &self,
        project: &str,
        root: &str,
        parent: &str,
    ) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(DelegateTasksTool {
                server: self.clone(),
                project: project.into(),
                root: root.into(),
                parent: parent.into(),
            }),
            Box::new(DelegatedResultTool {
                server: self.clone(),
                parent: parent.into(),
            }),
        ]
    }
}

fn delegate_schema() -> ToolSchema {
    let capabilities = [
        "reasoning: think and write without tools",
        "project_read: read, search and grep the project",
        "project_write: also write and edit project files",
        "code_run: run commands in an execution context and follow the Runs",
        "review: an independent read-only reviewer that returns findings",
        "image_inspection: look at images, when a vision model is configured",
        "visualization: write files and run Python or R to make figures, when a runtime is installed",
    ]
    .join("; ");
    ToolSchema::new(
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

struct DelegateTasksTool {
    server: Server,
    project: String,
    root: String,
    parent: String,
}

#[async_trait]
impl Tool for DelegateTasksTool {
    fn name(&self) -> &str {
        DELEGATE
    }

    fn schema(&self) -> ToolSchema {
        delegate_schema()
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
            "tasks": [{"id": "a", "instruction": "A", "capabilities": ["reasoning"], "isolated": true}],
        }))
        .is_err());
    }

    fn worker(external: bool) -> ModelProfilePolicy {
        ModelProfilePolicy {
            id: WORKER.into(),
            features: vec![],
            external,
            enabled: true,
        }
    }

    fn plan(tasks: Value, external: bool) -> Result<DelegationPlan, String> {
        resolve(
            "wf".into(),
            batch(tasks),
            &CapabilityRegistry::builtins(),
            &host_policy(vec![worker(external)], false),
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
        for tool in ["run_in_context", "get_run", "cancel_run"] {
            assert!(tools[1].iter().any(|kept| kept == tool), "{tools:?}");
        }
        // This host has no method search, and a reviewer changes nothing.
        assert!(!tools[1].iter().any(|kept| kept == "prepare_method_search"));
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
        let limits = root_limits(&local, &host_policy(vec![worker(false)], false));
        assert_eq!(limits.wall_time_secs, None);
        assert_eq!(limits.max_tokens, 0);
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
}
