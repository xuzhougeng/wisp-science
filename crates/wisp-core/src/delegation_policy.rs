//! Pure policy resolution for dynamic delegated Agents.
//!
//! Model-authored proposals contain capability IDs, never raw permissions.
//! This module resolves those IDs against host-owned definitions and produces
//! a versioned, revalidatable execution snapshot.

use crate::orchestration::MAX_PARALLEL_AGENTS;
use crate::{
    AgentAuthorizationSnapshot, AgentBackend, AgentBudget, AgentExecutorRef, AgentOrigin,
    AgentOutputSchemaSource, AgentRole, AgentSessionPolicy, AgentSpec, AgentWorkspacePolicy,
    CapabilityRevision, ContextPolicy, DelegationMode, DelegationPlan, DelegationPlanStep,
    PermissionSet, SpecialistSnapshot, DYNAMIC_DELEGATION_SCHEMA_VERSION, MAX_DELEGATION_TASKS,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

const ELEVATED_TOKEN_THRESHOLD: u32 = 20_000;
const ELEVATED_COST_THRESHOLD_MICROUNITS: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRisk {
    ReadOnly,
    Write,
    Execute,
    Network,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorFeature {
    ProjectRead,
    ProjectWrite,
    CodeExecution,
    NetworkAccess,
    LiteratureAccess,
    Vision,
    Isolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFeature {
    Vision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDefinition {
    pub id: String,
    pub revision: u32,
    pub display_name: String,
    pub description: String,
    pub risk: CapabilityRisk,
    pub permissions: PermissionSet,
    #[serde(default)]
    pub required_executor_features: Vec<ExecutorFeature>,
    #[serde(default)]
    pub required_model_features: Vec<ModelFeature>,
    #[serde(default)]
    pub required_skills: Vec<String>,
    #[serde(default)]
    pub required_connectors: Vec<String>,
    pub context_ceiling: ContextPolicy,
    pub default_budget: AgentBudget,
    pub budget_ceiling: AgentBudget,
    pub workspace_policy: AgentWorkspacePolicy,
    #[serde(default)]
    pub approval_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CapabilityRegistry {
    revision: String,
    definitions: HashMap<String, CapabilityDefinition>,
    order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProfilePolicy {
    pub id: String,
    #[serde(default)]
    pub features: Vec<ModelFeature>,
    #[serde(default)]
    pub external: bool,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutorProfilePolicy {
    pub executor: AgentExecutorRef,
    #[serde(default)]
    pub features: Vec<ExecutorFeature>,
    #[serde(default)]
    pub model_ids: Vec<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

const fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DelegationHostPolicy {
    pub revision: String,
    #[serde(default)]
    pub enabled_capabilities: Vec<String>,
    #[serde(default)]
    pub available_skills: Vec<String>,
    #[serde(default)]
    pub available_connectors: Vec<String>,
    #[serde(default)]
    pub models: Vec<ModelProfilePolicy>,
    #[serde(default)]
    pub executors: Vec<ExecutorProfilePolicy>,
    #[serde(default)]
    pub default_model_id: Option<String>,
    #[serde(default)]
    pub permission_ceiling: PermissionSet,
    #[serde(default)]
    pub context_ceiling: ContextPolicy,
    #[serde(default)]
    pub budget_ceiling: AgentBudget,
    #[serde(default)]
    pub default_timeout_secs: Option<u64>,
    #[serde(default)]
    pub timeout_ceiling_secs: Option<u64>,
    #[serde(default)]
    pub auto_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedTaskProposal {
    pub id: String,
    pub instruction: String,
    #[serde(default)]
    pub context_summary: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub specialist: Option<SpecialistSnapshot>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub isolated: bool,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub executor: Option<AgentExecutorRef>,
    #[serde(default)]
    pub budget: Option<AgentBudget>,
    #[serde(default = "empty_object")]
    pub input: Value,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentTask {
    spec: AgentSpec,
    input: Value,
    risk: CapabilityRisk,
    budget_ceiling: AgentBudget,
    required_executor_features: Vec<ExecutorFeature>,
}

impl ResolvedAgentTask {
    pub fn spec(&self) -> &AgentSpec {
        &self.spec
    }

    pub fn input(&self) -> &Value {
        &self.input
    }

    pub fn risk(&self) -> CapabilityRisk {
        self.risk
    }

    pub fn budget_ceiling(&self) -> &AgentBudget {
        &self.budget_ceiling
    }

    pub fn required_executor_features(&self) -> &[ExecutorFeature] {
        &self.required_executor_features
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDelegationPlan {
    plan: DelegationPlan,
}

impl ResolvedDelegationPlan {
    pub fn as_plan(&self) -> &DelegationPlan {
        &self.plan
    }

    pub fn into_plan(self) -> DelegationPlan {
        self.plan
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResolutionError {
    #[error("invalid delegation proposal: {0}")]
    InvalidProposal(String),
    #[error("unknown capability: {0}")]
    UnknownCapability(String),
    #[error("capability is disabled or unavailable: {0}")]
    UnavailableCapability(String),
    #[error("Specialist {specialist_id} does not allow {kind}: {value}")]
    SpecialistRestriction {
        specialist_id: String,
        kind: &'static str,
        value: String,
    },
    #[error("unknown or unavailable model profile: {0}")]
    UnknownModel(String),
    #[error("unknown or unavailable executor profile: {0}")]
    UnknownExecutor(String),
    #[error("no eligible model is configured")]
    NoEligibleModel,
    #[error("no eligible executor is configured")]
    NoEligibleExecutor,
    #[error("requested task budget exceeds the capability or host ceiling")]
    BudgetExceeded,
    #[error("dynamic delegation snapshot is stale: {0}")]
    StaleSnapshot(String),
    #[error("dynamic delegation snapshot integrity check failed")]
    IntegrityMismatch,
    #[error("dynamic delegation snapshot does not match current policy")]
    SnapshotMismatch,
}

impl CapabilityRegistry {
    pub fn new(
        revision: impl Into<String>,
        definitions: Vec<CapabilityDefinition>,
    ) -> Result<Self, ResolutionError> {
        let revision = revision.into();
        if revision.trim().is_empty() {
            return Err(ResolutionError::InvalidProposal(
                "capability registry revision is required".into(),
            ));
        }
        let mut by_id = HashMap::new();
        let mut order = Vec::with_capacity(definitions.len());
        for definition in definitions {
            if !valid_id(&definition.id) || definition.revision == 0 {
                return Err(ResolutionError::InvalidProposal(format!(
                    "invalid capability definition: {}",
                    definition.id
                )));
            }
            let id = definition.id.clone();
            if by_id.insert(id.clone(), definition).is_some() {
                return Err(ResolutionError::InvalidProposal(format!(
                    "duplicate capability definition: {id}"
                )));
            }
            order.push(id);
        }
        Ok(Self {
            revision,
            definitions: by_id,
            order,
        })
    }

    pub fn builtins() -> Self {
        let mut definitions = builtin_capabilities();
        definitions
            .iter_mut()
            .find(|definition| definition.id == "code_run")
            .expect("code_run capability")
            .revision = 2;
        Self::new("wisp-capabilities-v3", definitions)
            .expect("built-in capability definitions must be valid")
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn get(&self, id: &str) -> Option<&CapabilityDefinition> {
        self.definitions.get(id)
    }

    /// Return capability definitions in their stable display/resolution order.
    /// Hosts use this to adapt resource-backed tools (for example, whichever
    /// scientific runtimes are currently configured) without duplicating the
    /// built-in capability catalog.
    pub fn definitions(&self) -> Vec<CapabilityDefinition> {
        self.order
            .iter()
            .filter_map(|id| self.definitions.get(id).cloned())
            .collect()
    }

    pub fn available_ids(&self, host: &DelegationHostPolicy) -> Vec<String> {
        self.order
            .iter()
            .filter(|id| {
                self.definitions
                    .get(*id)
                    .is_some_and(|definition| definition_available(definition, host))
            })
            .cloned()
            .collect()
    }

    pub fn resolve_task(
        &self,
        proposal: DelegatedTaskProposal,
        host: &DelegationHostPolicy,
    ) -> Result<ResolvedAgentTask, ResolutionError> {
        validate_host(host)?;
        validate_proposal(&proposal)?;

        let mut definitions = Vec::with_capacity(proposal.capabilities.len());
        for capability in &proposal.capabilities {
            let definition = self
                .get(capability)
                .ok_or_else(|| ResolutionError::UnknownCapability(capability.clone()))?;
            if !definition_available(definition, host) {
                return Err(ResolutionError::UnavailableCapability(capability.clone()));
            }
            validate_specialist_restrictions(proposal.specialist.as_ref(), definition)?;
            definitions.push(definition);
        }

        let grant = compose_grant(&definitions, host)?;
        let workspace_policy = if proposal.isolated {
            AgentWorkspacePolicy::Isolated
        } else {
            grant.workspace_policy
        };
        let mut executor_features = grant.required_executor_features.clone();
        if workspace_policy == AgentWorkspacePolicy::Isolated {
            insert_sorted(&mut executor_features, ExecutorFeature::Isolation);
        }
        let (executor, model) = select_executor_and_model(
            &proposal,
            &executor_features,
            &grant.required_model_features,
            host,
        )?;
        let budget = select_budget(
            &grant.default_budget,
            proposal.budget.as_ref(),
            &grant.budget_ceiling,
        )?;
        let timeout_secs = select_timeout(host)?;
        let mut approval_reasons =
            approval_reasons(&grant, model, &executor, workspace_policy, &budget);
        approval_reasons.sort();
        approval_reasons.dedup();

        let origin = proposal
            .specialist
            .clone()
            .map(AgentOrigin::Specialist)
            .unwrap_or(AgentOrigin::Temporary);
        let name = proposal
            .specialist
            .as_ref()
            .map(|specialist| specialist.name.clone())
            .unwrap_or_else(|| proposal.id.clone());
        let role = if proposal.specialist.is_some() {
            AgentRole::Custom("specialist".into())
        } else {
            AgentRole::Custom("temporary".into())
        };
        let output_schema_source = if proposal.output_schema.is_some() {
            AgentOutputSchemaSource::Task
        } else {
            AgentOutputSchemaSource::Standard
        };
        let output_contract = proposal.output_schema.clone().unwrap_or_else(|| {
            json!({
                "type": "object"
            })
        });
        let mut spec = AgentSpec {
            template_id: String::new(),
            agent_id: proposal.id.clone(),
            name,
            goal: proposal.instruction.clone(),
            context_summary: proposal.context_summary.clone(),
            inputs: vec![],
            acceptance_criteria: vec![],
            dependencies: proposal.depends_on.clone(),
            role,
            backend: backend_for(&executor),
            model: model.map(|model| model.id.clone()),
            prompt_template: resolved_prompt(&proposal),
            input_contract: json!({"type": "object"}),
            output_contract,
            permissions: grant.permissions.clone(),
            context_policy: grant.context_policy.clone(),
            budget,
            timeout_secs,
            requires_review: false,
            session_policy: AgentSessionPolicy::New,
            allow_delegation: false,
            origin,
            capabilities: proposal.capabilities.clone(),
            executor: Some(executor),
            request_preferences: Some(crate::AgentRequestPreferences {
                model_id: proposal.model_id.clone(),
                executor: proposal.executor.clone(),
                isolated: proposal.isolated,
                budget: proposal.budget.clone(),
            }),
            workspace_policy: Some(workspace_policy),
            output_schema_source,
            approval_reasons,
            authorization: Some(AgentAuthorizationSnapshot {
                registry_revision: self.revision.clone(),
                policy_revision: host.revision.clone(),
                capabilities: definitions
                    .iter()
                    .map(|definition| CapabilityRevision {
                        id: definition.id.clone(),
                        revision: definition.revision,
                    })
                    .collect(),
                integrity_hash: String::new(),
            }),
        };
        spec.validate()
            .and_then(|_| spec.validate_dynamic_metadata())
            .map_err(|error| ResolutionError::InvalidProposal(error.to_string()))?;
        set_integrity_hash(&mut spec)?;

        Ok(ResolvedAgentTask {
            spec,
            input: proposal.input,
            risk: grant.risk,
            budget_ceiling: grant.budget_ceiling,
            required_executor_features: executor_features,
        })
    }

    pub fn resolve_plan(
        &self,
        goal: impl Into<String>,
        mode: DelegationMode,
        max_parallel: usize,
        tasks: Vec<DelegatedTaskProposal>,
        host: &DelegationHostPolicy,
    ) -> Result<ResolvedDelegationPlan, ResolutionError> {
        self.resolve_plan_with_id(
            uuid::Uuid::new_v4().to_string(),
            goal.into(),
            mode,
            max_parallel,
            tasks,
            host,
        )
    }

    pub fn resolve_plan_with_id(
        &self,
        id: String,
        goal: String,
        mode: DelegationMode,
        max_parallel: usize,
        tasks: Vec<DelegatedTaskProposal>,
        host: &DelegationHostPolicy,
    ) -> Result<ResolvedDelegationPlan, ResolutionError> {
        if tasks.is_empty() || tasks.len() > MAX_DELEGATION_TASKS {
            return Err(ResolutionError::InvalidProposal(format!(
                "a delegation plan requires 1 to {MAX_DELEGATION_TASKS} tasks"
            )));
        }
        if !(1..=MAX_PARALLEL_AGENTS).contains(&max_parallel) {
            return Err(ResolutionError::InvalidProposal(format!(
                "max_parallel must be between 1 and {MAX_PARALLEL_AGENTS}"
            )));
        }
        let resolved = tasks
            .into_iter()
            .map(|task| self.resolve_task(task, host))
            .collect::<Result<Vec<_>, _>>()?;
        let has_approval_reason = resolved
            .iter()
            .any(|task| !task.spec.approval_reasons.is_empty());
        let requires_confirmation = match mode {
            DelegationMode::Manual | DelegationMode::Assisted => true,
            DelegationMode::Automatic => !host.auto_safe || has_approval_reason,
        };
        let plan = DelegationPlan {
            schema_version: DYNAMIC_DELEGATION_SCHEMA_VERSION,
            id,
            goal,
            mode,
            requires_confirmation,
            max_parallel,
            steps: resolved
                .into_iter()
                .map(|task| DelegationPlanStep {
                    id: task.spec.agent_id.clone(),
                    spec: task.spec,
                    input: task.input,
                })
                .collect(),
        };
        plan.validate_structure()
            .map_err(|error| ResolutionError::InvalidProposal(error.to_string()))?;
        Ok(ResolvedDelegationPlan { plan })
    }

    pub fn validate_resolved_spec(
        &self,
        spec: &AgentSpec,
        host: &DelegationHostPolicy,
    ) -> Result<(), ResolutionError> {
        spec.validate()
            .and_then(|_| spec.validate_dynamic_metadata())
            .map_err(|error| ResolutionError::InvalidProposal(error.to_string()))?;
        let authorization = spec
            .authorization
            .as_ref()
            .ok_or_else(|| ResolutionError::StaleSnapshot("authorization is missing".into()))?;
        if authorization.registry_revision != self.revision {
            return Err(ResolutionError::StaleSnapshot(
                "capability registry revision changed".into(),
            ));
        }
        if authorization.policy_revision != host.revision {
            return Err(ResolutionError::StaleSnapshot(
                "host policy revision changed".into(),
            ));
        }
        let expected_revisions = spec
            .capabilities
            .iter()
            .map(|id| {
                self.get(id)
                    .map(|definition| CapabilityRevision {
                        id: id.clone(),
                        revision: definition.revision,
                    })
                    .ok_or_else(|| ResolutionError::UnknownCapability(id.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if authorization.capabilities != expected_revisions {
            return Err(ResolutionError::StaleSnapshot(
                "capability definition revision changed".into(),
            ));
        }
        if integrity_hash(spec)? != authorization.integrity_hash {
            return Err(ResolutionError::IntegrityMismatch);
        }
        let expected = self.resolve_task(proposal_from_spec(spec)?, host)?;
        if expected.spec != *spec {
            return Err(ResolutionError::SnapshotMismatch);
        }
        Ok(())
    }

    pub fn validate_resolved_plan(
        &self,
        plan: &DelegationPlan,
        host: &DelegationHostPolicy,
    ) -> Result<(), ResolutionError> {
        if plan.schema_version != DYNAMIC_DELEGATION_SCHEMA_VERSION {
            return Err(ResolutionError::InvalidProposal(
                "resolved policy validation requires a v2 plan".into(),
            ));
        }
        plan.validate_structure()
            .map_err(|error| ResolutionError::InvalidProposal(error.to_string()))?;
        for step in &plan.steps {
            self.validate_resolved_spec(&step.spec, host)?;
        }
        let proposals = plan
            .steps
            .iter()
            .map(|step| {
                let mut proposal = proposal_from_spec(&step.spec)?;
                proposal.input = step.input.clone();
                Ok(proposal)
            })
            .collect::<Result<Vec<_>, ResolutionError>>()?;
        let expected = self.resolve_plan_with_id(
            plan.id.clone(),
            plan.goal.clone(),
            plan.mode,
            plan.max_parallel,
            proposals,
            host,
        )?;
        if expected.plan != *plan {
            return Err(ResolutionError::SnapshotMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct ComposedGrant {
    risk: CapabilityRisk,
    permissions: PermissionSet,
    required_executor_features: Vec<ExecutorFeature>,
    required_model_features: Vec<ModelFeature>,
    context_policy: ContextPolicy,
    default_budget: AgentBudget,
    budget_ceiling: AgentBudget,
    workspace_policy: AgentWorkspacePolicy,
    approval_reasons: Vec<String>,
}

fn compose_grant(
    definitions: &[&CapabilityDefinition],
    host: &DelegationHostPolicy,
) -> Result<ComposedGrant, ResolutionError> {
    let mut risk = CapabilityRisk::ReadOnly;
    let mut tools = Vec::new();
    let mut required_paths = Vec::new();
    let mut network = false;
    let mut write = false;
    let mut execute = false;
    let mut executor_features = Vec::new();
    let mut model_features = Vec::new();
    let mut context = None;
    let mut default_budget = None;
    let mut budget_ceiling = None;
    let mut workspace = AgentWorkspacePolicy::SharedReadOnly;
    let mut approval_reasons = Vec::new();

    for definition in definitions {
        risk = risk.max(definition.risk);
        extend_unique(&mut tools, &definition.permissions.tools);
        extend_unique(&mut required_paths, &definition.permissions.paths);
        network |= definition.permissions.network;
        write |= definition.permissions.write;
        execute |= definition.permissions.execute;
        for feature in &definition.required_executor_features {
            insert_sorted(&mut executor_features, *feature);
        }
        for feature in &definition.required_model_features {
            insert_sorted(&mut model_features, *feature);
        }
        context = Some(match context {
            Some(current) => restrict_context(current, &definition.context_ceiling),
            None => definition.context_ceiling.clone(),
        });
        default_budget = Some(match default_budget {
            Some(current) => restrict_budget(current, &definition.default_budget),
            None => definition.default_budget.clone(),
        });
        budget_ceiling = Some(match budget_ceiling {
            Some(current) => restrict_budget(current, &definition.budget_ceiling),
            None => definition.budget_ceiling.clone(),
        });
        workspace = stricter_workspace(workspace, definition.workspace_policy);
        if let Some(reason) = &definition.approval_reason {
            approval_reasons.push(reason.clone());
        }
    }
    if write {
        risk = risk.max(CapabilityRisk::Write);
    }
    if execute {
        risk = risk.max(CapabilityRisk::Execute);
    }
    if network {
        risk = risk.max(CapabilityRisk::Network);
    }

    if tools
        .iter()
        .any(|tool| !host.permission_ceiling.tools.contains(tool))
        || (network && !host.permission_ceiling.network)
        || (write && !host.permission_ceiling.write)
        || (execute && !host.permission_ceiling.execute)
    {
        return Err(ResolutionError::UnavailableCapability(
            "host permission ceiling".into(),
        ));
    }
    let paths = resolve_paths(&required_paths, &host.permission_ceiling.paths);
    if !required_paths.is_empty() && paths.is_empty() {
        return Err(ResolutionError::UnavailableCapability(
            "project path ceiling".into(),
        ));
    }
    let context_policy = restrict_context(context.unwrap_or_default(), &host.context_ceiling);
    let budget_ceiling = restrict_budget(budget_ceiling.unwrap_or_default(), &host.budget_ceiling);
    let default_budget = restrict_budget(default_budget.unwrap_or_default(), &budget_ceiling);
    Ok(ComposedGrant {
        risk,
        permissions: PermissionSet {
            tools,
            paths,
            network,
            write,
            execute,
        },
        required_executor_features: executor_features,
        required_model_features: model_features,
        context_policy,
        default_budget,
        budget_ceiling,
        workspace_policy: workspace,
        approval_reasons,
    })
}

fn validate_host(host: &DelegationHostPolicy) -> Result<(), ResolutionError> {
    if host.revision.trim().is_empty() {
        return Err(ResolutionError::InvalidProposal(
            "host policy revision is required".into(),
        ));
    }
    if host.default_timeout_secs == Some(0) || host.timeout_ceiling_secs == Some(0) {
        return Err(ResolutionError::InvalidProposal(
            "host timeouts must be positive".into(),
        ));
    }
    Ok(())
}

fn validate_proposal(proposal: &DelegatedTaskProposal) -> Result<(), ResolutionError> {
    if !valid_task_id(&proposal.id) || proposal.instruction.trim().is_empty() {
        return Err(ResolutionError::InvalidProposal(
            "task id and instruction are required".into(),
        ));
    }
    if proposal.capabilities.is_empty() {
        return Err(ResolutionError::InvalidProposal(
            "at least one capability is required".into(),
        ));
    }
    let mut seen = HashSet::new();
    if proposal
        .capabilities
        .iter()
        .any(|id| !valid_id(id) || !seen.insert(id))
    {
        return Err(ResolutionError::InvalidProposal(
            "capability IDs must be valid and unique".into(),
        ));
    }
    if !proposal.input.is_object() {
        return Err(ResolutionError::InvalidProposal(
            "task input must be an object".into(),
        ));
    }
    Ok(())
}

fn valid_task_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b':')
        })
}

fn definition_available(definition: &CapabilityDefinition, host: &DelegationHostPolicy) -> bool {
    let permissions_available = host.enabled_capabilities.contains(&definition.id)
        && definition
            .required_skills
            .iter()
            .all(|id| host.available_skills.contains(id))
        && definition
            .required_connectors
            .iter()
            .all(|id| host.available_connectors.contains(id))
        && definition
            .permissions
            .tools
            .iter()
            .all(|id| host.permission_ceiling.tools.contains(id))
        && (!definition.permissions.network || host.permission_ceiling.network)
        && (!definition.permissions.write || host.permission_ceiling.write)
        && (!definition.permissions.execute || host.permission_ceiling.execute)
        && (definition.permissions.paths.is_empty()
            || !resolve_paths(
                &definition.permissions.paths,
                &host.permission_ceiling.paths,
            )
            .is_empty());
    if !permissions_available {
        return false;
    }
    let mut executor_features = definition.required_executor_features.clone();
    if definition.workspace_policy == AgentWorkspacePolicy::Isolated {
        insert_sorted(&mut executor_features, ExecutorFeature::Isolation);
    }
    host.executors.iter().any(|executor| {
        executor_eligible(executor, &executor_features)
            && match &executor.executor {
                AgentExecutorRef::Native => host.models.iter().any(|model| {
                    model_eligible(model, &definition.required_model_features)
                        && executor_accepts_model(executor, &model.id)
                }),
                AgentExecutorRef::Acp { .. } | AgentExecutorRef::External { .. } => true,
            }
    })
}

fn validate_specialist_restrictions(
    specialist: Option<&SpecialistSnapshot>,
    definition: &CapabilityDefinition,
) -> Result<(), ResolutionError> {
    let Some(specialist) = specialist else {
        return Ok(());
    };
    if let Some(allowed) = &specialist.skills {
        if let Some(value) = definition
            .required_skills
            .iter()
            .find(|value| !allowed.contains(value))
        {
            return Err(ResolutionError::SpecialistRestriction {
                specialist_id: specialist.id.clone(),
                kind: "skill",
                value: value.clone(),
            });
        }
    }
    if let Some(allowed) = &specialist.connectors {
        if let Some(value) = definition
            .required_connectors
            .iter()
            .find(|value| !allowed.contains(value))
        {
            return Err(ResolutionError::SpecialistRestriction {
                specialist_id: specialist.id.clone(),
                kind: "connector",
                value: value.clone(),
            });
        }
    }
    Ok(())
}

fn select_native_model<'a>(
    proposal: &DelegatedTaskProposal,
    required: &[ModelFeature],
    executor: &ExecutorProfilePolicy,
    host: &'a DelegationHostPolicy,
) -> Result<&'a ModelProfilePolicy, ResolutionError> {
    if let Some(id) = &proposal.model_id {
        return host
            .models
            .iter()
            .find(|model| {
                model.id == *id
                    && model_eligible(model, required)
                    && executor_accepts_model(executor, &model.id)
            })
            .ok_or_else(|| ResolutionError::UnknownModel(id.clone()));
    }
    if let Some(id) = proposal
        .specialist
        .as_ref()
        .and_then(|specialist| specialist.model_id.as_ref())
    {
        if let Some(model) = host.models.iter().find(|model| {
            model.id == *id
                && model_eligible(model, required)
                && executor_accepts_model(executor, &model.id)
        }) {
            return Ok(model);
        }
    }
    if let Some(id) = &host.default_model_id {
        if let Some(model) = host.models.iter().find(|model| {
            model.id == *id
                && model_eligible(model, required)
                && executor_accepts_model(executor, &model.id)
        }) {
            return Ok(model);
        }
    }
    host.models
        .iter()
        .find(|model| {
            model_eligible(model, required) && executor_accepts_model(executor, &model.id)
        })
        .ok_or(ResolutionError::NoEligibleModel)
}

fn model_eligible(model: &ModelProfilePolicy, required: &[ModelFeature]) -> bool {
    model.enabled
        && required
            .iter()
            .all(|feature| model.features.contains(feature))
}

fn select_executor_and_model<'a>(
    proposal: &DelegatedTaskProposal,
    required_executor: &[ExecutorFeature],
    required_model: &[ModelFeature],
    host: &'a DelegationHostPolicy,
) -> Result<(AgentExecutorRef, Option<&'a ModelProfilePolicy>), ResolutionError> {
    if let Some(requested) = proposal.executor.as_ref() {
        let profile = host
            .executors
            .iter()
            .find(|profile| {
                profile.executor == *requested && executor_eligible(profile, required_executor)
            })
            .ok_or_else(|| ResolutionError::UnknownExecutor(executor_name(requested)))?;
        return match requested {
            AgentExecutorRef::Native => Ok((
                AgentExecutorRef::Native,
                Some(select_native_model(
                    proposal,
                    required_model,
                    profile,
                    host,
                )?),
            )),
            AgentExecutorRef::Acp { .. } | AgentExecutorRef::External { .. } => {
                if let Some(model_id) = &proposal.model_id {
                    return Err(ResolutionError::InvalidProposal(format!(
                        "model profile {model_id} can only be used with the Native executor"
                    )));
                }
                Ok((requested.clone(), None))
            }
        };
    }

    if let Some(native) = host.executors.iter().find(|profile| {
        profile.executor == AgentExecutorRef::Native
            && executor_eligible(profile, required_executor)
    }) {
        match select_native_model(proposal, required_model, native, host) {
            Ok(model) => return Ok((AgentExecutorRef::Native, Some(model))),
            Err(error) if proposal.model_id.is_some() => return Err(error),
            Err(_) => {}
        }
    }
    if let Some(model_id) = &proposal.model_id {
        return Err(ResolutionError::UnknownModel(model_id.clone()));
    }
    host.executors
        .iter()
        .find(|profile| {
            !matches!(&profile.executor, AgentExecutorRef::Native)
                && executor_eligible(profile, required_executor)
        })
        .map(|profile| (profile.executor.clone(), None))
        .ok_or(ResolutionError::NoEligibleExecutor)
}

fn executor_eligible(profile: &ExecutorProfilePolicy, required: &[ExecutorFeature]) -> bool {
    profile.enabled
        && required
            .iter()
            .all(|feature| profile.features.contains(feature))
}

fn executor_accepts_model(profile: &ExecutorProfilePolicy, model_id: &str) -> bool {
    profile.model_ids.is_empty() || profile.model_ids.iter().any(|id| id == model_id)
}

fn select_budget(
    defaults: &AgentBudget,
    requested: Option<&AgentBudget>,
    ceiling: &AgentBudget,
) -> Result<AgentBudget, ResolutionError> {
    let selected = AgentBudget {
        max_tokens: requested
            .and_then(|value| value.max_tokens)
            .or(defaults.max_tokens),
        max_tool_calls: requested
            .and_then(|value| value.max_tool_calls)
            .or(defaults.max_tool_calls),
        max_cost_microunits: requested
            .and_then(|value| value.max_cost_microunits)
            .or(defaults.max_cost_microunits),
    };
    if !limit_within(selected.max_tokens, ceiling.max_tokens)
        || !limit_within(selected.max_tool_calls, ceiling.max_tool_calls)
        || !limit_within(selected.max_cost_microunits, ceiling.max_cost_microunits)
    {
        return Err(ResolutionError::BudgetExceeded);
    }
    Ok(selected)
}

fn select_timeout(host: &DelegationHostPolicy) -> Result<Option<u64>, ResolutionError> {
    match (host.default_timeout_secs, host.timeout_ceiling_secs) {
        (Some(default), Some(ceiling)) if default > ceiling => Err(
            ResolutionError::InvalidProposal("default timeout exceeds host ceiling".into()),
        ),
        (default, _) => Ok(default),
    }
}

fn approval_reasons(
    grant: &ComposedGrant,
    model: Option<&ModelProfilePolicy>,
    executor: &AgentExecutorRef,
    workspace: AgentWorkspacePolicy,
    budget: &AgentBudget,
) -> Vec<String> {
    let mut reasons = grant.approval_reasons.clone();
    if grant.permissions.write {
        reasons.push("Task can modify project files".into());
    }
    if grant.permissions.execute {
        reasons.push("Task can execute code".into());
    }
    if grant.permissions.network {
        reasons.push("Task can access network services".into());
    }
    if grant.risk == CapabilityRisk::External || model.is_some_and(|model| model.external) {
        reasons.push("Task uses an external service or model".into());
    }
    match executor {
        AgentExecutorRef::Native => {}
        AgentExecutorRef::Acp { profile_id } => {
            reasons.push(format!("Task uses ACP executor profile {profile_id}"));
        }
        AgentExecutorRef::External { profile_id } => {
            reasons.push(format!("Task uses external executor profile {profile_id}"));
        }
    }
    if workspace == AgentWorkspacePolicy::Isolated {
        reasons.push(
            "Task uses a temporary Git worktree and an automatic conflict-checked cherry-pick"
                .into(),
        );
    }
    if budget_is_elevated(budget, &grant.default_budget) {
        reasons.push("Task requests an elevated resource budget".into());
    }
    reasons
}

fn budget_is_elevated(selected: &AgentBudget, defaults: &AgentBudget) -> bool {
    limit_greater(selected.max_tokens, defaults.max_tokens)
        || limit_greater(selected.max_tool_calls, defaults.max_tool_calls)
        || limit_greater(selected.max_cost_microunits, defaults.max_cost_microunits)
        || selected
            .max_tokens
            .is_some_and(|value| value > ELEVATED_TOKEN_THRESHOLD)
        || selected
            .max_cost_microunits
            .is_some_and(|value| value > ELEVATED_COST_THRESHOLD_MICROUNITS)
}

fn proposal_from_spec(spec: &AgentSpec) -> Result<DelegatedTaskProposal, ResolutionError> {
    let specialist = match &spec.origin {
        AgentOrigin::Temporary => None,
        AgentOrigin::Specialist(snapshot) => Some(snapshot.clone()),
        AgentOrigin::LegacyTemplate => {
            return Err(ResolutionError::InvalidProposal(
                "legacy templates cannot be revalidated as dynamic tasks".into(),
            ))
        }
    };
    let output_schema = match spec.output_schema_source {
        AgentOutputSchemaSource::Standard => None,
        AgentOutputSchemaSource::Task => Some(spec.output_contract.clone()),
        AgentOutputSchemaSource::Specialist => {
            return Err(ResolutionError::SnapshotMismatch);
        }
    };
    let preferences = spec.request_preferences.as_ref();
    Ok(DelegatedTaskProposal {
        id: spec.agent_id.clone(),
        instruction: spec.goal.clone(),
        context_summary: spec.context_summary.clone(),
        depends_on: spec.dependencies.clone(),
        capabilities: spec.capabilities.clone(),
        specialist,
        output_schema,
        isolated: preferences.map_or(
            spec.workspace_policy == Some(AgentWorkspacePolicy::Isolated),
            |preferences| preferences.isolated,
        ),
        model_id: preferences
            .map(|preferences| preferences.model_id.clone())
            .unwrap_or_else(|| spec.model.clone()),
        executor: preferences
            .map(|preferences| preferences.executor.clone())
            .unwrap_or_else(|| spec.executor.clone()),
        budget: preferences
            .map(|preferences| preferences.budget.clone())
            .unwrap_or_else(|| Some(spec.budget.clone())),
        input: json!({}),
    })
}

fn resolved_prompt(proposal: &DelegatedTaskProposal) -> String {
    let base = "You are a bounded Wisp sub-Agent. Complete only the assigned task, return evidence to the parent Agent, and do not delegate further.";
    match &proposal.specialist {
        Some(specialist) => format!(
            "{base}\n\nSpecialist identity: {} ({})\nSpecialist instructions:\n{}",
            specialist.name,
            specialist.id,
            specialist.instructions.trim()
        ),
        None => format!("{base}\n\nRole: temporary generic Agent."),
    }
}

fn backend_for(executor: &AgentExecutorRef) -> AgentBackend {
    match executor {
        AgentExecutorRef::Native => AgentBackend::Local,
        AgentExecutorRef::Acp { .. } => AgentBackend::Acp,
        AgentExecutorRef::External { .. } => AgentBackend::Custom("external".into()),
    }
}

fn executor_name(executor: &AgentExecutorRef) -> String {
    match executor {
        AgentExecutorRef::Native => "native".into(),
        AgentExecutorRef::Acp { profile_id } => format!("acp:{profile_id}"),
        AgentExecutorRef::External { profile_id } => format!("external:{profile_id}"),
    }
}

fn set_integrity_hash(spec: &mut AgentSpec) -> Result<(), ResolutionError> {
    let hash = integrity_hash(spec)?;
    spec.authorization
        .as_mut()
        .expect("resolver always attaches authorization")
        .integrity_hash = hash;
    Ok(())
}

fn integrity_hash(spec: &AgentSpec) -> Result<String, ResolutionError> {
    let mut canonical = spec.clone();
    if let Some(authorization) = canonical.authorization.as_mut() {
        authorization.integrity_hash.clear();
    }
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| ResolutionError::InvalidProposal(error.to_string()))?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn resolve_paths(required: &[String], host: &[String]) -> Vec<String> {
    if required.is_empty() {
        return vec![];
    }
    host.iter()
        .filter(|candidate| {
            required.iter().any(|ceiling| {
                ceiling == *candidate
                    || (ceiling == "project://**" && candidate.starts_with("project://"))
            })
        })
        .cloned()
        .collect()
}

fn restrict_context(left: ContextPolicy, right: &ContextPolicy) -> ContextPolicy {
    left.restrict(right)
}

fn restrict_budget(left: AgentBudget, right: &AgentBudget) -> AgentBudget {
    left.restrict(right)
}

fn stricter_workspace(
    left: AgentWorkspacePolicy,
    right: AgentWorkspacePolicy,
) -> AgentWorkspacePolicy {
    if workspace_rank(right) > workspace_rank(left) {
        right
    } else {
        left
    }
}

const fn workspace_rank(policy: AgentWorkspacePolicy) -> u8 {
    match policy {
        AgentWorkspacePolicy::SharedReadOnly => 0,
        AgentWorkspacePolicy::SerializedMutation => 1,
        AgentWorkspacePolicy::Isolated => 2,
    }
}

fn extend_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
}

fn insert_sorted<T: Ord + Copy>(target: &mut Vec<T>, value: T) {
    if !target.contains(&value) {
        target.push(value);
        target.sort();
    }
}

fn limit_within<T: Ord>(selected: Option<T>, ceiling: Option<T>) -> bool {
    match (selected, ceiling) {
        (_, None) => true,
        (Some(selected), Some(ceiling)) => selected <= ceiling,
        (None, Some(_)) => false,
    }
}

fn limit_greater<T: Ord>(selected: Option<T>, defaults: Option<T>) -> bool {
    match (selected, defaults) {
        (Some(selected), Some(default)) => selected > default,
        (None, Some(_)) => true,
        _ => false,
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn builtin_capabilities() -> Vec<CapabilityDefinition> {
    vec![
        capability(
            "reasoning",
            "Reasoning",
            "Reason without project tools.",
            CapabilityRisk::ReadOnly,
            &[],
            false,
            false,
            false,
            &[],
            &[],
            &[],
            AgentWorkspacePolicy::SharedReadOnly,
            8_000,
            16_000,
        ),
        capability(
            "project_read",
            "Project read",
            "Read and search project files.",
            CapabilityRisk::ReadOnly,
            &["read", "search", "grep"],
            false,
            false,
            false,
            &[ExecutorFeature::ProjectRead],
            &[],
            &[],
            AgentWorkspacePolicy::SharedReadOnly,
            8_000,
            20_000,
        ),
        capability(
            "project_write",
            "Project write",
            "Read and modify project files.",
            CapabilityRisk::Write,
            &["read", "search", "grep", "write", "edit"],
            true,
            false,
            false,
            &[ExecutorFeature::ProjectRead, ExecutorFeature::ProjectWrite],
            &[],
            &[],
            AgentWorkspacePolicy::SerializedMutation,
            10_000,
            24_000,
        ),
        capability(
            "code_run",
            "Code execution",
            "Run bounded project code.",
            CapabilityRisk::Execute,
            &[
                "read",
                "search",
                "grep",
                "run_in_context",
                "get_run",
                "cancel_run",
            ],
            false,
            false,
            true,
            &[ExecutorFeature::ProjectRead, ExecutorFeature::CodeExecution],
            &[],
            &[],
            AgentWorkspacePolicy::SerializedMutation,
            12_000,
            32_000,
        ),
        capability(
            "literature_search",
            "Literature search",
            "Search configured scholarly sources.",
            CapabilityRisk::Network,
            &["literature_search"],
            false,
            true,
            false,
            &[
                ExecutorFeature::NetworkAccess,
                ExecutorFeature::LiteratureAccess,
            ],
            &[],
            &["literature"],
            AgentWorkspacePolicy::SharedReadOnly,
            10_000,
            20_000,
        ),
        capability(
            "external_research",
            "External research",
            "Use configured external research sources.",
            CapabilityRisk::External,
            &["web_search"],
            false,
            true,
            false,
            &[ExecutorFeature::NetworkAccess],
            &[],
            &["web"],
            AgentWorkspacePolicy::SharedReadOnly,
            10_000,
            20_000,
        ),
        capability(
            "visualization",
            "Visualization",
            "Create figures with bounded runtime tools.",
            CapabilityRisk::Execute,
            &["read", "search", "grep", "write", "edit", "python", "r"],
            true,
            false,
            true,
            &[
                ExecutorFeature::ProjectRead,
                ExecutorFeature::ProjectWrite,
                ExecutorFeature::CodeExecution,
            ],
            &[],
            &[],
            AgentWorkspacePolicy::SerializedMutation,
            12_000,
            32_000,
        ),
        capability(
            "review",
            "Review",
            "Inspect project evidence without modifying it.",
            CapabilityRisk::ReadOnly,
            &["read", "search", "grep"],
            false,
            false,
            false,
            &[ExecutorFeature::ProjectRead],
            &[],
            &[],
            AgentWorkspacePolicy::SharedReadOnly,
            8_000,
            16_000,
        ),
        capability_with_model(
            "image_inspection",
            "Image inspection",
            "Inspect local images with a configured vision model.",
            CapabilityRisk::External,
            &["read", "view_image"],
            &[ExecutorFeature::ProjectRead, ExecutorFeature::Vision],
            &[ModelFeature::Vision],
            8_000,
            16_000,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn capability(
    id: &str,
    display_name: &str,
    description: &str,
    risk: CapabilityRisk,
    tools: &[&str],
    write: bool,
    network: bool,
    execute: bool,
    executor_features: &[ExecutorFeature],
    skills: &[&str],
    connectors: &[&str],
    workspace_policy: AgentWorkspacePolicy,
    default_tokens: u32,
    max_tokens: u32,
) -> CapabilityDefinition {
    CapabilityDefinition {
        id: id.into(),
        revision: 1,
        display_name: display_name.into(),
        description: description.into(),
        risk,
        permissions: PermissionSet {
            tools: tools.iter().map(|value| (*value).into()).collect(),
            paths: if tools.is_empty() {
                vec![]
            } else {
                vec!["project://**".into()]
            },
            network,
            write,
            execute,
        },
        required_executor_features: executor_features.to_vec(),
        required_model_features: vec![],
        required_skills: skills.iter().map(|value| (*value).into()).collect(),
        required_connectors: connectors.iter().map(|value| (*value).into()).collect(),
        context_ceiling: ContextPolicy {
            include_history: false,
            include_artifacts: true,
            max_tokens: Some(max_tokens),
        },
        default_budget: AgentBudget {
            max_tokens: Some(default_tokens),
            max_tool_calls: Some(32),
            max_cost_microunits: Some(500_000),
        },
        budget_ceiling: AgentBudget {
            max_tokens: Some(max_tokens),
            max_tool_calls: Some(64),
            max_cost_microunits: Some(1_000_000),
        },
        workspace_policy,
        approval_reason: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn capability_with_model(
    id: &str,
    display_name: &str,
    description: &str,
    risk: CapabilityRisk,
    tools: &[&str],
    executor_features: &[ExecutorFeature],
    model_features: &[ModelFeature],
    default_tokens: u32,
    max_tokens: u32,
) -> CapabilityDefinition {
    let mut definition = capability(
        id,
        display_name,
        description,
        risk,
        tools,
        false,
        false,
        false,
        executor_features,
        &[],
        &[],
        AgentWorkspacePolicy::SharedReadOnly,
        default_tokens,
        max_tokens,
    );
    definition.required_model_features = model_features.to_vec();
    definition
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn host_policy() -> DelegationHostPolicy {
        let registry = CapabilityRegistry::builtins();
        let tools = registry
            .order
            .iter()
            .flat_map(|id| registry.get(id).unwrap().permissions.tools.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        DelegationHostPolicy {
            revision: "test-policy-v1".into(),
            enabled_capabilities: registry.order.clone(),
            available_skills: vec![],
            available_connectors: vec!["literature".into(), "web".into()],
            models: vec![
                ModelProfilePolicy {
                    id: "local".into(),
                    features: vec![],
                    external: false,
                    enabled: true,
                },
                ModelProfilePolicy {
                    id: "vision".into(),
                    features: vec![ModelFeature::Vision],
                    external: true,
                    enabled: true,
                },
            ],
            executors: vec![
                ExecutorProfilePolicy {
                    executor: AgentExecutorRef::Native,
                    features: vec![
                        ExecutorFeature::ProjectRead,
                        ExecutorFeature::ProjectWrite,
                        ExecutorFeature::CodeExecution,
                        ExecutorFeature::NetworkAccess,
                        ExecutorFeature::LiteratureAccess,
                        ExecutorFeature::Vision,
                        ExecutorFeature::Isolation,
                    ],
                    model_ids: vec![],
                    enabled: true,
                },
                ExecutorProfilePolicy {
                    executor: AgentExecutorRef::Acp {
                        profile_id: "acp-general".into(),
                    },
                    features: vec![
                        ExecutorFeature::ProjectRead,
                        ExecutorFeature::ProjectWrite,
                        ExecutorFeature::CodeExecution,
                        ExecutorFeature::NetworkAccess,
                        ExecutorFeature::LiteratureAccess,
                        ExecutorFeature::Vision,
                        ExecutorFeature::Isolation,
                    ],
                    model_ids: vec!["local".into(), "vision".into()],
                    enabled: true,
                },
            ],
            default_model_id: Some("local".into()),
            permission_ceiling: PermissionSet {
                tools,
                paths: vec!["project://**".into()],
                network: true,
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
            default_timeout_secs: Some(600),
            timeout_ceiling_secs: Some(1_800),
            auto_safe: true,
        }
    }

    fn proposal(id: &str, capabilities: &[&str]) -> DelegatedTaskProposal {
        DelegatedTaskProposal {
            id: id.into(),
            instruction: format!("Complete {id}"),
            context_summary: String::new(),
            depends_on: vec![],
            capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
            specialist: None,
            output_schema: None,
            isolated: false,
            model_id: None,
            executor: None,
            budget: None,
            input: json!({}),
        }
    }

    #[test]
    fn builtins_resolve_to_exact_tool_and_permission_grants() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy();
        let cases = [
            ("reasoning", vec![], false, false, false),
            (
                "project_read",
                vec!["read", "search", "grep"],
                false,
                false,
                false,
            ),
            (
                "project_write",
                vec!["read", "search", "grep", "write", "edit"],
                true,
                false,
                false,
            ),
            (
                "code_run",
                vec![
                    "read",
                    "search",
                    "grep",
                    "run_in_context",
                    "get_run",
                    "cancel_run",
                ],
                false,
                false,
                true,
            ),
            (
                "literature_search",
                vec!["literature_search"],
                false,
                true,
                false,
            ),
            ("external_research", vec!["web_search"], false, true, false),
            (
                "visualization",
                vec!["read", "search", "grep", "write", "edit", "python", "r"],
                true,
                false,
                true,
            ),
            (
                "review",
                vec!["read", "search", "grep"],
                false,
                false,
                false,
            ),
            (
                "image_inspection",
                vec!["read", "view_image"],
                false,
                false,
                false,
            ),
        ];
        for (id, tools, write, network, execute) in cases {
            let resolved = registry.resolve_task(proposal(id, &[id]), &host).unwrap();
            assert_eq!(resolved.spec().permissions.tools, tools, "{id}");
            assert_eq!(resolved.spec().permissions.write, write, "{id}");
            assert_eq!(resolved.spec().permissions.network, network, "{id}");
            assert_eq!(resolved.spec().permissions.execute, execute, "{id}");
            assert_eq!(
                resolved.spec().permissions.paths.is_empty(),
                id == "reasoning",
                "{id}"
            );
        }
    }

    #[test]
    fn composed_capabilities_use_highest_risk_and_narrowest_budget() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy();
        let resolved = registry
            .resolve_task(proposal("work", &["project_read", "code_run"]), &host)
            .unwrap();
        assert_eq!(resolved.risk(), CapabilityRisk::Execute);
        assert_eq!(resolved.budget_ceiling().max_tokens, Some(20_000));
        assert!(resolved
            .spec()
            .permissions
            .tools
            .contains(&"run_in_context".into()));
        assert!(!resolved.spec().permissions.tools.contains(&"shell".into()));

        let mut too_large = proposal("work", &["project_read", "code_run"]);
        too_large.budget = Some(AgentBudget {
            max_tokens: Some(20_001),
            ..Default::default()
        });
        assert_eq!(
            registry.resolve_task(too_large, &host).unwrap_err(),
            ResolutionError::BudgetExceeded
        );
    }

    #[test]
    fn unknown_disabled_and_unavailable_capabilities_fail_closed() {
        let registry = CapabilityRegistry::builtins();
        let mut host = host_policy();
        assert!(matches!(
            registry.resolve_task(proposal("x", &["missing"]), &host),
            Err(ResolutionError::UnknownCapability(_))
        ));
        host.enabled_capabilities.retain(|id| id != "project_read");
        assert!(matches!(
            registry.resolve_task(proposal("x", &["project_read"]), &host),
            Err(ResolutionError::UnavailableCapability(_))
        ));
        host.enabled_capabilities.push("project_read".into());
        host.permission_ceiling.tools.retain(|tool| tool != "read");
        assert!(matches!(
            registry.resolve_task(proposal("x", &["project_read"]), &host),
            Err(ResolutionError::UnavailableCapability(_))
        ));

        let mut host = host_policy();
        host.permission_ceiling.execute = false;
        assert!(matches!(
            registry.resolve_task(proposal("x", &["code_run"]), &host),
            Err(ResolutionError::UnavailableCapability(_))
        ));
    }

    #[test]
    fn advertised_capabilities_follow_runtime_resource_availability() {
        let registry = CapabilityRegistry::builtins();
        let mut host = host_policy();
        assert!(registry
            .available_ids(&host)
            .contains(&"literature_search".into()));
        host.available_connectors
            .retain(|connector| connector != "literature");
        assert!(!registry
            .available_ids(&host)
            .contains(&"literature_search".into()));

        assert!(registry
            .available_ids(&host)
            .contains(&"image_inspection".into()));
        host.models
            .iter_mut()
            .find(|model| model.id == "vision")
            .unwrap()
            .enabled = false;
        assert!(registry
            .available_ids(&host)
            .contains(&"image_inspection".into()));
        host.executors
            .iter_mut()
            .find(|profile| matches!(&profile.executor, AgentExecutorRef::Acp { .. }))
            .unwrap()
            .features
            .retain(|feature| *feature != ExecutorFeature::Vision);
        assert!(!registry
            .available_ids(&host)
            .contains(&"image_inspection".into()));
    }

    #[test]
    fn specialist_allowlists_only_narrow_capabilities() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy();
        let mut restricted = proposal("papers", &["literature_search"]);
        restricted.specialist = Some(SpecialistSnapshot {
            id: "bio".into(),
            name: "Biology".into(),
            instructions: "Use primary sources".into(),
            model_id: None,
            skills: None,
            connectors: Some(vec![]),
        });
        assert!(matches!(
            registry.resolve_task(restricted.clone(), &host),
            Err(ResolutionError::SpecialistRestriction { .. })
        ));

        restricted.specialist.as_mut().unwrap().connectors =
            Some(vec!["literature".into(), "untrusted-extra".into()]);
        let resolved = registry.resolve_task(restricted, &host).unwrap();
        assert_eq!(resolved.spec().permissions.tools, vec!["literature_search"]);
        assert!(!resolved
            .spec()
            .permissions
            .tools
            .contains(&"untrusted-extra".into()));
    }

    #[test]
    fn specialist_identity_model_and_prompt_are_snapshotted_without_a_fixed_team() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy();
        let mut task = proposal("expert", &["project_read"]);
        task.specialist = Some(SpecialistSnapshot {
            id: "domain-expert".into(),
            name: "Domain expert".into(),
            instructions: "Use the saved domain rubric.".into(),
            model_id: Some("vision".into()),
            skills: None,
            connectors: None,
        });

        let resolved = registry.resolve_task(task, &host).unwrap();
        assert_eq!(resolved.spec().model.as_deref(), Some("vision"));
        assert!(resolved
            .spec()
            .prompt_template
            .contains("Specialist identity: Domain expert (domain-expert)"));
        assert!(resolved
            .spec()
            .prompt_template
            .contains("Use the saved domain rubric."));
        assert!(!resolved.spec().prompt_template.contains("Complete expert"));

        let AgentOrigin::Specialist(snapshot) = &resolved.spec().origin else {
            panic!("expected Specialist snapshot");
        };
        let mut dangling = proposal("fallback", &["reasoning"]);
        dangling.specialist = Some(SpecialistSnapshot {
            model_id: Some("deleted-model".into()),
            ..snapshot.clone()
        });
        let fallback = registry.resolve_task(dangling, &host).unwrap();
        assert_eq!(fallback.spec().model.as_deref(), Some("local"));

        let plan = registry
            .resolve_plan(
                "one temporary task",
                DelegationMode::Manual,
                2,
                vec![proposal("generic", &["reasoning"])],
                &host,
            )
            .unwrap();
        assert_eq!(plan.as_plan().steps.len(), 1);
        assert_eq!(plan.as_plan().steps[0].spec.origin, AgentOrigin::Temporary);
    }

    #[test]
    fn model_and_executor_overrides_must_select_configured_eligible_profiles() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy();
        let mut task = proposal("inspect", &["image_inspection"]);
        task.model_id = Some("vision".into());
        task.executor = Some(AgentExecutorRef::Native);
        let resolved = registry.resolve_task(task.clone(), &host).unwrap();
        assert_eq!(resolved.spec().model.as_deref(), Some("vision"));
        assert_eq!(resolved.spec().executor, task.executor);

        task.model_id = Some("local".into());
        assert!(matches!(
            registry.resolve_task(task.clone(), &host),
            Err(ResolutionError::UnknownModel(_))
        ));
        task.model_id = None;
        task.executor = Some(AgentExecutorRef::Acp {
            profile_id: "acp-general".into(),
        });
        let resolved = registry.resolve_task(task.clone(), &host).unwrap();
        assert_eq!(resolved.spec().model, None);
        assert_eq!(resolved.spec().executor, task.executor);

        task.executor = Some(AgentExecutorRef::Acp {
            profile_id: "missing".into(),
        });
        assert!(matches!(
            registry.resolve_task(task, &host),
            Err(ResolutionError::UnknownExecutor(_))
        ));
    }

    #[test]
    fn automatic_executor_selection_prefers_native_over_configured_acp() {
        let resolved = CapabilityRegistry::builtins()
            .resolve_task(proposal("inspect", &["project_read"]), &host_policy())
            .unwrap();

        assert_eq!(resolved.spec().executor, Some(AgentExecutorRef::Native));
        assert_eq!(resolved.spec().backend, AgentBackend::Local);
        let requested = resolved.spec().request_preferences.as_ref().unwrap();
        assert_eq!(requested.executor, None);
        assert_eq!(requested.model_id, None);
    }

    #[test]
    fn acp_executor_does_not_require_or_record_a_native_model() {
        let registry = CapabilityRegistry::builtins();
        let mut host = host_policy();
        for model in &mut host.models {
            model.enabled = false;
        }
        host.executors
            .iter_mut()
            .find(|profile| profile.executor == AgentExecutorRef::Native)
            .unwrap()
            .enabled = false;

        assert!(registry
            .available_ids(&host)
            .contains(&"project_read".into()));
        let resolved = registry
            .resolve_task(proposal("inspect", &["project_read"]), &host)
            .unwrap();
        assert_eq!(resolved.spec().model, None);
        assert_eq!(
            resolved.spec().executor,
            Some(AgentExecutorRef::Acp {
                profile_id: "acp-general".into(),
            })
        );
        assert_eq!(resolved.spec().backend, AgentBackend::Acp);
        assert_eq!(
            resolved
                .spec()
                .request_preferences
                .as_ref()
                .unwrap()
                .executor,
            None
        );

        let mut with_native_model = proposal("model", &["project_read"]);
        with_native_model.model_id = Some("local".into());
        assert!(matches!(
            registry.resolve_task(with_native_model, &host),
            Err(ResolutionError::UnknownModel(_))
        ));
    }

    #[test]
    fn native_read_only_auto_safe_batch_skips_confirmation() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy();
        let mut review = proposal("review", &["review"]);
        review.depends_on = vec!["inspect".into()];
        let plan = registry
            .resolve_plan(
                "inspect and review",
                DelegationMode::Automatic,
                2,
                vec![proposal("inspect", &["project_read"]), review],
                &host,
            )
            .unwrap();
        assert!(!plan.as_plan().requires_confirmation);
        registry
            .validate_resolved_plan(plan.as_plan(), &host)
            .unwrap();
    }

    #[test]
    fn risky_choices_each_produce_an_explicit_confirmation_reason() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy();
        let cases = [
            ("project_write", "modify project files"),
            ("code_run", "execute code"),
            ("literature_search", "network services"),
            ("image_inspection", "external service or model"),
        ];
        for (capability, fragment) in cases {
            let resolved = registry
                .resolve_task(proposal(capability, &[capability]), &host)
                .unwrap();
            assert!(resolved
                .spec()
                .approval_reasons
                .iter()
                .any(|reason| reason.contains(fragment)));
        }

        let mut acp = proposal("acp", &["reasoning"]);
        acp.executor = Some(AgentExecutorRef::Acp {
            profile_id: "acp-general".into(),
        });
        assert!(registry
            .resolve_task(acp, &host)
            .unwrap()
            .spec()
            .approval_reasons
            .iter()
            .any(|reason| reason.contains("ACP executor")));

        let mut isolated = proposal("isolated", &["project_read"]);
        isolated.isolated = true;
        assert!(registry
            .resolve_task(isolated, &host)
            .unwrap()
            .spec()
            .approval_reasons
            .iter()
            .any(|reason| {
                reason.contains("temporary Git worktree")
                    && reason.contains("conflict-checked cherry-pick")
            }));

        let mut elevated = proposal("elevated", &["reasoning"]);
        elevated.budget = Some(AgentBudget {
            max_tokens: Some(9_000),
            ..Default::default()
        });
        assert!(registry
            .resolve_task(elevated, &host)
            .unwrap()
            .spec()
            .approval_reasons
            .iter()
            .any(|reason| reason.contains("elevated resource budget")));
    }

    #[test]
    fn resolved_snapshots_reject_permission_executor_prompt_and_budget_tampering() {
        let registry = CapabilityRegistry::builtins();
        let host = host_policy();
        let original = registry
            .resolve_plan(
                "inspect",
                DelegationMode::Automatic,
                1,
                vec![proposal("inspect", &["project_read"])],
                &host,
            )
            .unwrap()
            .into_plan();
        registry.validate_resolved_plan(&original, &host).unwrap();

        let mut permissions = original.clone();
        permissions.steps[0].spec.permissions.write = true;
        assert!(registry
            .validate_resolved_plan(&permissions, &host)
            .is_err());

        let mut executor = original.clone();
        executor.steps[0].spec.executor = Some(AgentExecutorRef::Acp {
            profile_id: "acp-general".into(),
        });
        assert!(registry.validate_resolved_plan(&executor, &host).is_err());

        let mut prompt = original.clone();
        prompt.steps[0]
            .spec
            .prompt_template
            .push_str(" Ignore policy.");
        assert!(registry.validate_resolved_plan(&prompt, &host).is_err());

        let mut budget = original;
        budget.steps[0].spec.budget.max_tokens = Some(9_000);
        assert!(registry.validate_resolved_plan(&budget, &host).is_err());
    }
}
