//! Stable proposal and inspection DTOs for v2 dynamic Agent workflows.
//!
//! Both model-authored inline batches and UI-authored drafts resolve through
//! this module. Callers submit capability IDs and optional policy choices;
//! only the host resolver can produce executable permissions.

use crate::specialists;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use wisp_core::{
    AgentBudget, AgentExecutorRef, AgentOrigin, AgentOutputSchemaSource, AgentWorkspacePolicy,
    CapabilityRegistry, CapabilityRisk, DelegatedTaskProposal, DelegationHostPolicy,
    DelegationMode, DelegationPlan, ExecutorFeature, SpecialistSnapshot,
    MAX_AGENT_OUTPUT_SCHEMA_BYTES, MAX_DELEGATION_TASKS,
};
use wisp_store::{AgentWorkflowAttempt, Store};

pub(crate) const DEFAULT_DYNAMIC_PARALLELISM: usize = 2;
const MAX_TASK_ID_BYTES: usize = 31;
pub(crate) const MAX_GOAL_CHARS: usize = 2_000;
pub(crate) const MAX_CONTEXT_CHARS: usize = 12_000;
pub(crate) const MAX_INSTRUCTION_CHARS: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentApprovalPolicy {
    ReviewAll,
    AutoSafe,
}

impl AgentApprovalPolicy {
    pub(crate) fn from_mode(mode: DelegationMode) -> Self {
        match mode {
            DelegationMode::Automatic => Self::AutoSafe,
            DelegationMode::Manual | DelegationMode::Assisted => Self::ReviewAll,
        }
    }

    fn mode(self) -> DelegationMode {
        match self {
            Self::ReviewAll => DelegationMode::Manual,
            Self::AutoSafe => DelegationMode::Automatic,
        }
    }

    pub(crate) fn from_workflow_mode(mode: &str) -> Self {
        if mode == "automatic" {
            Self::AutoSafe
        } else {
            Self::ReviewAll
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentExecutorSelection {
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) profile_id: Option<String>,
}

impl AgentExecutorSelection {
    fn into_ref(self) -> Result<AgentExecutorRef, String> {
        let profile_id = || {
            self.profile_id
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{} executor requires profile_id", self.kind))
        };
        match self.kind.as_str() {
            "native" if self.profile_id.is_none() => Ok(AgentExecutorRef::Native),
            "native" => Err("native executor cannot include profile_id".into()),
            "acp" => Ok(AgentExecutorRef::Acp {
                profile_id: profile_id()?,
            }),
            "external" => Ok(AgentExecutorRef::External {
                profile_id: profile_id()?,
            }),
            _ => Err(format!("unknown executor kind: {}", self.kind)),
        }
    }

    pub(crate) fn from_ref(executor: &AgentExecutorRef) -> Self {
        match executor {
            AgentExecutorRef::Native => Self {
                kind: "native".into(),
                profile_id: None,
            },
            AgentExecutorRef::Acp { profile_id } => Self {
                kind: "acp".into(),
                profile_id: Some(profile_id.clone()),
            },
            AgentExecutorRef::External { profile_id } => Self {
                kind: "external".into(),
                profile_id: Some(profile_id.clone()),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AgentCapabilityOption {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) description: String,
    pub(crate) risk: CapabilityRisk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AgentModelOption {
    pub(crate) id: String,
    pub(crate) external: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExecutorProfileSummary {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) profile_id: Option<String>,
    pub(crate) display_name: String,
    pub(crate) available: bool,
    pub(crate) supported_features: Vec<ExecutorFeature>,
}

impl ExecutorProfileSummary {
    fn from_policy(policy: &wisp_core::ExecutorProfilePolicy) -> Self {
        let selection = AgentExecutorSelection::from_ref(&policy.executor);
        let id = selection
            .profile_id
            .as_ref()
            .map(|profile_id| format!("{}:{profile_id}", selection.kind))
            .unwrap_or_else(|| selection.kind.clone());
        let display_name = selection
            .profile_id
            .clone()
            .unwrap_or_else(|| "Native".into());
        Self {
            id,
            kind: selection.kind,
            profile_id: selection.profile_id,
            display_name,
            available: policy.enabled,
            supported_features: policy.features.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DynamicAgentEditorOptions {
    pub(crate) capabilities: Vec<AgentCapabilityOption>,
    pub(crate) models: Vec<AgentModelOption>,
    pub(crate) executors: Vec<ExecutorProfileSummary>,
}

pub(crate) fn editor_options(
    registry: &CapabilityRegistry,
    host: &DelegationHostPolicy,
) -> DynamicAgentEditorOptions {
    let capabilities = registry
        .available_ids(host)
        .into_iter()
        .filter_map(|id| {
            let definition = registry.get(&id)?;
            Some(AgentCapabilityOption {
                id,
                display_name: definition.display_name.clone(),
                description: definition.description.clone(),
                risk: definition.risk,
            })
        })
        .collect();
    let models = host
        .models
        .iter()
        .filter(|model| model.enabled)
        .map(|model| AgentModelOption {
            id: model.id.clone(),
            external: model.external,
        })
        .collect();
    let executors = host
        .executors
        .iter()
        .map(ExecutorProfileSummary::from_policy)
        .collect();
    DynamicAgentEditorOptions {
        capabilities,
        models,
        executors,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentBudgetProposal {
    #[serde(default)]
    pub(crate) max_tokens: Option<u32>,
    #[serde(default)]
    pub(crate) max_tool_calls: Option<u32>,
    #[serde(default)]
    pub(crate) max_cost_microunits: Option<u64>,
}

impl From<AgentBudgetProposal> for AgentBudget {
    fn from(value: AgentBudgetProposal) -> Self {
        Self {
            max_tokens: value.max_tokens,
            max_tool_calls: value.max_tool_calls,
            max_cost_microunits: value.max_cost_microunits,
        }
    }
}

impl From<&AgentBudget> for AgentBudgetProposal {
    fn from(value: &AgentBudget) -> Self {
        Self {
            max_tokens: value.max_tokens,
            max_tool_calls: value.max_tool_calls,
            max_cost_microunits: value.max_cost_microunits,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DynamicAgentTaskProposal {
    pub(crate) id: String,
    pub(crate) instruction: String,
    #[serde(default)]
    pub(crate) depends_on: Vec<String>,
    pub(crate) capabilities: Vec<String>,
    #[serde(default)]
    pub(crate) specialist_id: Option<String>,
    #[serde(default)]
    pub(crate) output_schema: Option<Value>,
    #[serde(default)]
    pub(crate) isolated: bool,
    #[serde(default)]
    pub(crate) model_id: Option<String>,
    #[serde(default)]
    pub(crate) executor: Option<AgentExecutorSelection>,
    #[serde(default)]
    pub(crate) budget: Option<AgentBudgetProposal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DynamicAgentWorkflowProposal {
    pub(crate) goal: String,
    #[serde(default)]
    pub(crate) context: String,
    pub(crate) approval_policy: AgentApprovalPolicy,
    pub(crate) tasks: Vec<DynamicAgentTaskProposal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AgentExecutorSummary {
    pub(crate) kind: String,
    pub(crate) profile_id: Option<String>,
    pub(crate) model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AgentApprovalReasonSummary {
    pub(crate) task_id: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AgentResultSummary {
    pub(crate) status: String,
    pub(crate) summary: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) child_frame_id: Option<String>,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) tool_calls: i64,
    pub(crate) cost_microunits: i64,
    pub(crate) duration_secs: Option<i64>,
    pub(crate) full_result_available: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ResolvedAgentTaskSummary {
    pub(crate) id: String,
    pub(crate) stored_step_id: String,
    pub(crate) instruction: String,
    pub(crate) depends_on: Vec<String>,
    pub(crate) capabilities: Vec<String>,
    pub(crate) specialist_id: Option<String>,
    pub(crate) specialist_name: Option<String>,
    pub(crate) executor: AgentExecutorSummary,
    pub(crate) workspace_policy: String,
    pub(crate) tools: Vec<String>,
    pub(crate) can_write: bool,
    pub(crate) can_execute: bool,
    pub(crate) can_access_network: bool,
    pub(crate) budget: AgentBudgetProposal,
    pub(crate) timeout_secs: Option<u64>,
    pub(crate) approval_reasons: Vec<String>,
    pub(crate) output_schema: Option<Value>,
    pub(crate) result: Option<AgentResultSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DynamicAgentWorkflowSummary {
    pub(crate) schema_version: u32,
    pub(crate) approval_policy: AgentApprovalPolicy,
    pub(crate) editable_proposal: DynamicAgentWorkflowProposal,
    pub(crate) tasks: Vec<ResolvedAgentTaskSummary>,
    pub(crate) approval_reasons: Vec<AgentApprovalReasonSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AgentWorkflowVersionConflict {
    pub(crate) workflow_id: String,
    pub(crate) expected_version: i64,
    pub(crate) actual_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DynamicWorkflowCommandError {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) version_conflict: Option<AgentWorkflowVersionConflict>,
}

impl DynamicWorkflowCommandError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            version_conflict: None,
        }
    }

    pub(crate) fn conflict(
        workflow_id: impl Into<String>,
        expected_version: i64,
        actual_version: i64,
    ) -> Self {
        let workflow_id = workflow_id.into();
        Self {
            code: "version_conflict".into(),
            message: "Agent plan changed in another window; refresh and try again.".into(),
            version_conflict: Some(AgentWorkflowVersionConflict {
                workflow_id,
                expected_version,
                actual_version,
            }),
        }
    }
}

pub(crate) async fn resolve_proposal(
    store: &Store,
    workflow_id: String,
    proposal: DynamicAgentWorkflowProposal,
    registry: &CapabilityRegistry,
    host: &DelegationHostPolicy,
) -> Result<DelegationPlan, String> {
    validate_proposal(&proposal)?;
    let mut tasks = Vec::with_capacity(proposal.tasks.len());
    for task in proposal.tasks {
        let specialist = specialist_snapshot(store, task.specialist_id.as_deref()).await?;
        if specialist
            .as_ref()
            .is_some_and(|value| value.id == "reviewer")
            && (task.capabilities.iter().any(|capability| {
                !matches!(capability.as_str(), "reasoning" | "project_read" | "review")
            }) || !task
                .capabilities
                .iter()
                .any(|capability| capability == "review"))
        {
            return Err(
                "the built-in Reviewer requires the review capability and cannot receive write or execute capabilities"
                    .into(),
            );
        }
        let display_id = task.id;
        tasks.push(DelegatedTaskProposal {
            id: stored_task_id(&workflow_id, &display_id),
            instruction: task.instruction.trim().into(),
            context_summary: proposal.context.trim().into(),
            depends_on: task
                .depends_on
                .iter()
                .map(|dependency| stored_task_id(&workflow_id, dependency))
                .collect(),
            capabilities: task.capabilities,
            specialist,
            output_schema: task.output_schema,
            isolated: task.isolated,
            model_id: task.model_id.filter(|value| !value.trim().is_empty()),
            executor: task
                .executor
                .map(AgentExecutorSelection::into_ref)
                .transpose()?,
            budget: task.budget.map(AgentBudget::from),
            input: json!({"task_id": display_id}),
        });
    }
    registry
        .resolve_plan_with_id(
            workflow_id,
            proposal.goal.trim().into(),
            proposal.approval_policy.mode(),
            DEFAULT_DYNAMIC_PARALLELISM,
            tasks,
            host,
        )
        .map(|resolved| resolved.into_plan())
        .map_err(|error| error.to_string())
}

pub(crate) fn summarize(
    plan: &DelegationPlan,
    attempts: &[AgentWorkflowAttempt],
) -> Result<DynamicAgentWorkflowSummary, String> {
    if plan.schema_version != wisp_core::DYNAMIC_DELEGATION_SCHEMA_VERSION {
        return Err("dynamic workflow summary requires a v2 plan".into());
    }
    let ids = plan
        .steps
        .iter()
        .map(|step| (step.id.as_str(), display_task_id(plan, step)))
        .collect::<HashMap<_, _>>();
    let approval_policy = AgentApprovalPolicy::from_mode(plan.mode);
    let context = plan
        .steps
        .first()
        .map(|step| step.spec.context_summary.clone())
        .unwrap_or_default();
    let mut proposal_tasks = Vec::with_capacity(plan.steps.len());
    let mut tasks = Vec::with_capacity(plan.steps.len());
    let mut approval_reasons = Vec::new();
    for step in &plan.steps {
        let id = ids
            .get(step.id.as_str())
            .cloned()
            .unwrap_or_else(|| step.id.clone());
        let depends_on = step
            .spec
            .dependencies
            .iter()
            .map(|dependency| {
                ids.get(dependency.as_str())
                    .cloned()
                    .unwrap_or_else(|| dependency.clone())
            })
            .collect::<Vec<_>>();
        let specialist = match &step.spec.origin {
            AgentOrigin::Specialist(specialist) => Some(specialist),
            AgentOrigin::LegacyTemplate | AgentOrigin::Temporary => None,
        };
        let output_schema = (step.spec.output_schema_source == AgentOutputSchemaSource::Task)
            .then(|| step.spec.output_contract.clone());
        let executor = step
            .spec
            .executor
            .as_ref()
            .map(AgentExecutorSelection::from_ref);
        let requested = step.spec.request_preferences.as_ref();
        proposal_tasks.push(DynamicAgentTaskProposal {
            id: id.clone(),
            instruction: step.spec.goal.clone(),
            depends_on: depends_on.clone(),
            capabilities: step.spec.capabilities.clone(),
            specialist_id: specialist.map(|value| value.id.clone()),
            output_schema: output_schema.clone(),
            isolated: requested.map_or(
                step.spec.workspace_policy == Some(AgentWorkspacePolicy::Isolated),
                |requested| requested.isolated,
            ),
            model_id: requested
                .map(|requested| requested.model_id.clone())
                .unwrap_or_else(|| step.spec.model.clone()),
            executor: requested
                .map(|requested| {
                    requested
                        .executor
                        .as_ref()
                        .map(AgentExecutorSelection::from_ref)
                })
                .unwrap_or_else(|| executor.clone()),
            budget: requested
                .map(|requested| requested.budget.as_ref().map(AgentBudgetProposal::from))
                .unwrap_or_else(|| Some(AgentBudgetProposal::from(&step.spec.budget))),
        });
        approval_reasons.extend(step.spec.approval_reasons.iter().map(|message| {
            AgentApprovalReasonSummary {
                task_id: id.clone(),
                message: message.clone(),
            }
        }));
        let latest_attempt = attempts
            .iter()
            .filter(|attempt| attempt.step_id == step.id)
            .max_by_key(|attempt| attempt.attempt);
        let executor_summary = match step.spec.executor.as_ref() {
            Some(executor) => {
                let selected = AgentExecutorSelection::from_ref(executor);
                AgentExecutorSummary {
                    kind: selected.kind,
                    profile_id: selected.profile_id,
                    model_id: step.spec.model.clone(),
                }
            }
            None => AgentExecutorSummary {
                kind: "unresolved".into(),
                profile_id: None,
                model_id: step.spec.model.clone(),
            },
        };
        tasks.push(ResolvedAgentTaskSummary {
            id,
            stored_step_id: step.id.clone(),
            instruction: step.spec.goal.clone(),
            depends_on,
            capabilities: step.spec.capabilities.clone(),
            specialist_id: specialist.map(|value| value.id.clone()),
            specialist_name: specialist.map(|value| value.name.clone()),
            executor: executor_summary,
            workspace_policy: workspace_policy_name(step.spec.workspace_policy),
            tools: step.spec.permissions.tools.clone(),
            can_write: step.spec.permissions.write,
            can_execute: step.spec.permissions.execute,
            can_access_network: step.spec.permissions.network,
            budget: AgentBudgetProposal::from(&step.spec.budget),
            timeout_secs: step.spec.timeout_secs,
            approval_reasons: step.spec.approval_reasons.clone(),
            output_schema,
            result: latest_attempt.map(result_summary),
        });
    }
    Ok(DynamicAgentWorkflowSummary {
        schema_version: plan.schema_version,
        approval_policy,
        editable_proposal: DynamicAgentWorkflowProposal {
            goal: plan.goal.clone(),
            context,
            approval_policy,
            tasks: proposal_tasks,
        },
        tasks,
        approval_reasons,
    })
}

fn result_summary(attempt: &AgentWorkflowAttempt) -> AgentResultSummary {
    let summary = serde_json::from_str::<Value>(&attempt.output_json)
        .ok()
        .and_then(|value| value.get("summary")?.as_str().map(str::to_string));
    AgentResultSummary {
        status: serde_json::to_value(attempt.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".into()),
        summary,
        error: attempt.error.clone(),
        child_frame_id: attempt.child_frame_id.clone(),
        input_tokens: attempt.input_tokens,
        output_tokens: attempt.output_tokens,
        tool_calls: attempt.tool_calls,
        cost_microunits: attempt.cost_microunits,
        duration_secs: attempt
            .started_at
            .zip(attempt.finished_at)
            .map(|(started, finished)| finished.saturating_sub(started)),
        full_result_available: attempt.response_json.is_some(),
    }
}

fn display_task_id(plan: &DelegationPlan, step: &wisp_core::DelegationPlanStep) -> String {
    step.input
        .get("task_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            step.id
                .strip_prefix(&format!("{}:", plan.id))
                .map(str::to_string)
        })
        .unwrap_or_else(|| step.id.clone())
}

fn workspace_policy_name(policy: Option<AgentWorkspacePolicy>) -> String {
    match policy {
        Some(AgentWorkspacePolicy::SharedReadOnly) => "shared_read_only",
        Some(AgentWorkspacePolicy::SerializedMutation) => "serialized_mutation",
        Some(AgentWorkspacePolicy::Isolated) => "isolated",
        None => "unresolved",
    }
    .into()
}

async fn specialist_snapshot(
    store: &Store,
    specialist_id: Option<&str>,
) -> Result<Option<SpecialistSnapshot>, String> {
    let Some(id) = specialist_id else {
        return Ok(None);
    };
    let specialist = specialists::get(store, id)
        .await
        .ok_or_else(|| format!("unknown Specialist: {id}"))?;
    Ok(Some(SpecialistSnapshot {
        id: specialist.id,
        name: specialist.name,
        instructions: specialist.instructions,
        model_id: (!specialist.model_id.trim().is_empty()).then_some(specialist.model_id),
        skills: specialist.skills,
        connectors: specialist.connectors,
    }))
}

fn validate_proposal(proposal: &DynamicAgentWorkflowProposal) -> Result<(), String> {
    let goal = proposal.goal.trim();
    if goal.is_empty() || goal.chars().count() > MAX_GOAL_CHARS {
        return Err(format!(
            "goal must contain 1 to {MAX_GOAL_CHARS} characters"
        ));
    }
    if proposal.context.chars().count() > MAX_CONTEXT_CHARS {
        return Err(format!(
            "shared context cannot exceed {MAX_CONTEXT_CHARS} characters"
        ));
    }
    if proposal.tasks.is_empty() || proposal.tasks.len() > MAX_DELEGATION_TASKS {
        return Err(format!(
            "a batch requires 1 to {MAX_DELEGATION_TASKS} tasks"
        ));
    }
    let mut ids = HashSet::new();
    for task in &proposal.tasks {
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
                    .is_ok_and(|bytes| bytes.len() > MAX_AGENT_OUTPUT_SCHEMA_BYTES)
        }) {
            return Err(format!(
                "task {} output_schema must be an object no larger than {MAX_AGENT_OUTPUT_SCHEMA_BYTES} bytes",
                task.id
            ));
        }
        if let Some(budget) = &task.budget {
            if budget.max_tokens == Some(0)
                || budget.max_tool_calls == Some(0)
                || budget.max_cost_microunits == Some(0)
            {
                return Err(format!("task {} budget limits must be positive", task.id));
            }
        }
    }
    for task in &proposal.tasks {
        let mut dependencies = HashSet::new();
        for dependency in &task.depends_on {
            if !dependencies.insert(dependency.as_str()) {
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
    let mut completed = HashSet::new();
    while completed.len() < proposal.tasks.len() {
        let before = completed.len();
        for task in &proposal.tasks {
            if !completed.contains(task.id.as_str())
                && task
                    .depends_on
                    .iter()
                    .all(|dependency| completed.contains(dependency.as_str()))
            {
                completed.insert(task.id.as_str());
            }
        }
        if completed.len() == before {
            return Err("task dependencies contain a cycle".into());
        }
    }
    Ok(())
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

fn stored_task_id(workflow_id: &str, task_id: &str) -> String {
    format!("{workflow_id}:{task_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, depends_on: &[&str]) -> DynamicAgentTaskProposal {
        DynamicAgentTaskProposal {
            id: id.into(),
            instruction: format!("Run {id}"),
            depends_on: depends_on.iter().map(|value| (*value).into()).collect(),
            capabilities: vec!["reasoning".into()],
            specialist_id: None,
            output_schema: None,
            isolated: false,
            model_id: None,
            executor: None,
            budget: None,
        }
    }

    #[test]
    fn proposal_validation_rejects_unknown_duplicate_and_cyclic_dependencies() {
        let proposal = |tasks| DynamicAgentWorkflowProposal {
            goal: "test".into(),
            context: String::new(),
            approval_policy: AgentApprovalPolicy::ReviewAll,
            tasks,
        };
        assert!(validate_proposal(&proposal(vec![task("a", &["missing"])]))
            .unwrap_err()
            .contains("unknown task"));
        assert!(
            validate_proposal(&proposal(vec![task("a", &[]), task("b", &["a", "a"])]))
                .unwrap_err()
                .contains("duplicate dependency")
        );
        assert!(
            validate_proposal(&proposal(vec![task("a", &["b"]), task("b", &["a"])]))
                .unwrap_err()
                .contains("cycle")
        );
    }

    #[test]
    fn executor_selection_is_strict_and_round_trips() {
        for executor in [
            AgentExecutorRef::Native,
            AgentExecutorRef::Acp {
                profile_id: "acp-1".into(),
            },
            AgentExecutorRef::External {
                profile_id: "external-1".into(),
            },
        ] {
            assert_eq!(
                AgentExecutorSelection::from_ref(&executor)
                    .into_ref()
                    .unwrap(),
                executor
            );
        }
        assert!(AgentExecutorSelection {
            kind: "native".into(),
            profile_id: Some("unexpected".into()),
        }
        .into_ref()
        .is_err());
    }

    #[test]
    fn legacy_modes_map_to_the_two_display_approval_policies() {
        assert_eq!(
            AgentApprovalPolicy::from_workflow_mode("manual"),
            AgentApprovalPolicy::ReviewAll
        );
        assert_eq!(
            AgentApprovalPolicy::from_workflow_mode("assisted"),
            AgentApprovalPolicy::ReviewAll
        );
        assert_eq!(
            AgentApprovalPolicy::from_workflow_mode("automatic"),
            AgentApprovalPolicy::AutoSafe
        );
    }

    #[test]
    fn editor_options_report_executor_availability_and_features() {
        let registry = CapabilityRegistry::builtins();
        let host = DelegationHostPolicy {
            revision: "editor-options-test".into(),
            enabled_capabilities: vec!["reasoning".into()],
            models: vec![
                wisp_core::ModelProfilePolicy {
                    id: "enabled".into(),
                    features: vec![],
                    external: false,
                    enabled: true,
                },
                wisp_core::ModelProfilePolicy {
                    id: "disabled".into(),
                    features: vec![],
                    external: true,
                    enabled: false,
                },
            ],
            executors: vec![
                wisp_core::ExecutorProfilePolicy {
                    executor: AgentExecutorRef::Native,
                    features: vec![],
                    model_ids: vec!["enabled".into()],
                    enabled: true,
                },
                wisp_core::ExecutorProfilePolicy {
                    executor: AgentExecutorRef::Acp {
                        profile_id: "disabled-acp".into(),
                    },
                    features: vec![],
                    model_ids: vec!["enabled".into()],
                    enabled: false,
                },
            ],
            default_model_id: Some("enabled".into()),
            permission_ceiling: wisp_core::PermissionSet::default(),
            context_ceiling: wisp_core::ContextPolicy::default(),
            budget_ceiling: AgentBudget::default(),
            auto_safe: true,
            ..DelegationHostPolicy::default()
        };

        let options = editor_options(&registry, &host);

        assert_eq!(options.capabilities.len(), 1);
        assert_eq!(options.capabilities[0].id, "reasoning");
        assert_eq!(options.models.len(), 1);
        assert_eq!(options.models[0].id, "enabled");
        assert_eq!(
            options.executors,
            [
                ExecutorProfileSummary {
                    id: "native".into(),
                    kind: "native".into(),
                    profile_id: None,
                    display_name: "Native".into(),
                    available: true,
                    supported_features: vec![],
                },
                ExecutorProfileSummary {
                    id: "acp:disabled-acp".into(),
                    kind: "acp".into(),
                    profile_id: Some("disabled-acp".into()),
                    display_name: "disabled-acp".into(),
                    available: false,
                    supported_features: vec![],
                },
            ]
        );
    }
}
