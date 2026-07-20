//! Controlled agent templates and editable delegation plans.

use crate::{
    AgentBackend, AgentBudget, AgentOrigin, AgentRole, AgentSessionPolicy, AgentSpec,
    ContextPolicy, PermissionSet,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const MAX_PARALLEL_AGENTS: usize = 2;
pub const MAX_DELEGATION_TASKS: usize = 8;
pub const DYNAMIC_DELEGATION_SCHEMA_VERSION: u32 = 2;
pub const AUTOMATIC_TOKEN_CONFIRMATION_THRESHOLD: u32 = 20_000;
pub const AUTOMATIC_COST_CONFIRMATION_THRESHOLD_MICROUNITS: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationMode {
    Manual,
    Assisted,
    Automatic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTemplate {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub role: AgentRole,
    pub backend: AgentBackend,
    pub default_model: Option<String>,
    pub prompt_template: String,
    pub input_contract: Value,
    pub output_contract: Value,
    pub permission_ceiling: PermissionSet,
    pub context_ceiling: ContextPolicy,
    pub budget_ceiling: AgentBudget,
    pub timeout_ceiling_secs: Option<u64>,
    pub allow_delegation: bool,
}

impl AgentTemplate {
    pub fn automatic_requires_confirmation(&self) -> bool {
        capabilities_require_automatic_confirmation(
            &self.backend,
            &self.permission_ceiling,
            &self.budget_ceiling,
        )
    }

    pub fn instantiate(&self, request: AgentInstanceRequest) -> anyhow::Result<AgentSpec> {
        if request.agent_id.trim().is_empty()
            || request.name.trim().is_empty()
            || request.goal.trim().is_empty()
        {
            anyhow::bail!("agent instance id, name, and goal are required");
        }
        let requested = AgentSpec {
            template_id: self.id.clone(),
            agent_id: request.agent_id,
            name: request.name,
            goal: request.goal,
            context_summary: request.context_summary,
            inputs: request.inputs,
            acceptance_criteria: request.acceptance_criteria,
            dependencies: request.dependencies,
            role: self.role.clone(),
            backend: self.backend.clone(),
            model: request.model.or_else(|| self.default_model.clone()),
            prompt_template: self.prompt_template.clone(),
            input_contract: self.input_contract.clone(),
            output_contract: self.output_contract.clone(),
            permissions: request.permissions,
            context_policy: request.context_policy,
            budget: request.budget,
            timeout_secs: request.timeout_secs,
            requires_review: request.requires_review,
            session_policy: request.session_policy,
            allow_delegation: request.allow_delegation && self.allow_delegation,
            origin: AgentOrigin::LegacyTemplate,
            capabilities: vec![],
            executor: None,
            request_preferences: None,
            workspace_policy: None,
            output_schema_source: Default::default(),
            approval_reasons: vec![],
            authorization: None,
        }
        .constrained_by(
            &self.permission_ceiling,
            &self.context_ceiling,
            &self.budget_ceiling,
            self.timeout_ceiling_secs,
        );
        requested.validate()?;
        Ok(requested)
    }

    pub fn validate_spec(&self, spec: &AgentSpec) -> anyhow::Result<()> {
        spec.validate()?;
        if spec.template_id != self.id
            || spec.role != self.role
            || spec.backend != self.backend
            || spec.prompt_template != self.prompt_template
            || spec.input_contract != self.input_contract
            || spec.output_contract != self.output_contract
            || spec.origin != AgentOrigin::LegacyTemplate
            || !spec.capabilities.is_empty()
            || spec.executor.is_some()
            || spec.workspace_policy.is_some()
            || spec.output_schema_source != Default::default()
            || !spec.approval_reasons.is_empty()
            || spec.authorization.is_some()
        {
            anyhow::bail!("agent spec changes fixed template fields");
        }
        if !spec.permissions.is_subset_of(&self.permission_ceiling)
            || spec.context_policy.restrict(&self.context_ceiling) != spec.context_policy
            || spec.budget.restrict(&self.budget_ceiling) != spec.budget
            || super::delegation::restrict_limit(spec.timeout_secs, self.timeout_ceiling_secs)
                != spec.timeout_secs
            || (spec.allow_delegation && !self.allow_delegation)
        {
            anyhow::bail!("agent spec exceeds template capability ceiling");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstanceRequest {
    pub agent_id: String,
    pub name: String,
    pub goal: String,
    #[serde(default)]
    pub context_summary: String,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permissions: PermissionSet,
    #[serde(default)]
    pub context_policy: ContextPolicy,
    #[serde(default)]
    pub budget: AgentBudget,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub requires_review: bool,
    #[serde(default)]
    pub session_policy: AgentSessionPolicy,
    #[serde(default)]
    pub allow_delegation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationPlanStep {
    pub id: String,
    pub spec: AgentSpec,
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationPlan {
    #[serde(default = "legacy_delegation_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub goal: String,
    pub mode: DelegationMode,
    pub requires_confirmation: bool,
    pub max_parallel: usize,
    pub steps: Vec<DelegationPlanStep>,
}

impl DelegationPlan {
    pub fn validate(&self, templates: &AgentTemplateRegistry) -> anyhow::Result<()> {
        self.validate_structure()?;
        for step in &self.steps {
            if self.schema_version == DYNAMIC_DELEGATION_SCHEMA_VERSION {
                step.spec.validate_dynamic_metadata()?;
            } else {
                templates
                    .get(&step.spec.template_id)
                    .ok_or_else(|| {
                        anyhow::anyhow!("unknown agent template: {}", step.spec.template_id)
                    })?
                    .validate_spec(&step.spec)?;
            }
        }
        Ok(())
    }

    pub fn validate_structure(&self) -> anyhow::Result<()> {
        if !matches!(self.schema_version, 1 | DYNAMIC_DELEGATION_SCHEMA_VERSION) {
            anyhow::bail!("unsupported delegation plan schema version");
        }
        if self.id.trim().is_empty() || self.goal.trim().is_empty() {
            anyhow::bail!("delegation plan id and goal are required");
        }
        if self.max_parallel == 0 || self.max_parallel > MAX_PARALLEL_AGENTS {
            anyhow::bail!("max_parallel must be between 1 and {MAX_PARALLEL_AGENTS}");
        }
        if self.steps.len() > MAX_DELEGATION_TASKS {
            anyhow::bail!("delegation plan cannot contain more than {MAX_DELEGATION_TASKS} tasks");
        }
        let ids = self
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<HashSet<_>>();
        if ids.len() != self.steps.len() {
            anyhow::bail!("delegation plan step ids must be unique");
        }
        for step in &self.steps {
            if step.id != step.spec.agent_id {
                anyhow::bail!("plan step id must match spec agent_id");
            }
            step.spec.validate()?;
            for dependency in &step.spec.dependencies {
                if dependency == &step.id || !ids.contains(dependency.as_str()) {
                    anyhow::bail!("invalid dependency {dependency} for step {}", step.id);
                }
            }
        }
        ensure_acyclic(&self.steps)?;
        Ok(())
    }
}

const fn legacy_delegation_schema_version() -> u32 {
    1
}

fn ensure_acyclic(steps: &[DelegationPlanStep]) -> anyhow::Result<()> {
    let dependencies = steps
        .iter()
        .map(|step| (step.id.as_str(), step.spec.dependencies.as_slice()))
        .collect::<HashMap<_, _>>();
    fn visit<'a>(
        id: &'a str,
        dependencies: &HashMap<&'a str, &'a [String]>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> anyhow::Result<()> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            anyhow::bail!("delegation plan contains a dependency cycle");
        }
        if let Some(items) = dependencies.get(id) {
            for dependency in *items {
                visit(dependency, dependencies, visiting, visited)?;
            }
        }
        visiting.remove(id);
        visited.insert(id);
        Ok(())
    }
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for step in steps {
        visit(&step.id, &dependencies, &mut visiting, &mut visited)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AgentTemplateRegistry {
    templates: HashMap<String, AgentTemplate>,
}

impl AgentTemplateRegistry {
    pub fn builtins() -> Self {
        let templates = builtin_templates()
            .into_iter()
            .map(|template| (template.id.clone(), template))
            .collect();
        Self { templates }
    }

    pub fn get(&self, id: &str) -> Option<&AgentTemplate> {
        self.templates.get(id)
    }

    pub fn list(&self) -> Vec<&AgentTemplate> {
        let mut templates = self.templates.values().collect::<Vec<_>>();
        templates.sort_by(|left, right| left.id.cmp(&right.id));
        templates
    }
}

impl Default for AgentTemplateRegistry {
    fn default() -> Self {
        Self::builtins()
    }
}

#[derive(Debug, Clone, Default)]
pub struct DelegationPlanner;

impl DelegationPlanner {
    pub fn suggest(
        &self,
        goal: &str,
        mode: DelegationMode,
        context_summary: &str,
        inputs: &[String],
        acceptance_criteria: &[String],
        templates: &AgentTemplateRegistry,
    ) -> anyhow::Result<DelegationPlan> {
        if goal.trim().is_empty() {
            anyhow::bail!("delegation goal is required");
        }
        if mode == DelegationMode::Manual {
            anyhow::bail!("manual delegation requires an explicit ordered Agent selection");
        }
        let lower = goal.to_lowercase();
        let mut selected = Vec::new();
        if contains_any(
            &lower,
            &[
                "code", "python", " r ", "analysis", "统计", "分析", "代码", "运行",
            ],
        ) {
            selected.push("code_execution");
        }
        if contains_any(
            &lower,
            &[
                "biology", "gene", "pathway", "cell", "生物", "基因", "通路", "细胞",
            ],
        ) {
            selected.push("biology_interpreter");
        }
        if contains_any(&lower, &["figure", "plot", "visual", "图", "可视化"]) {
            selected.push("visualization");
        }
        if selected.is_empty() {
            return Ok(DelegationPlan {
                schema_version: 1,
                id: uuid::Uuid::new_v4().to_string(),
                goal: goal.into(),
                mode,
                requires_confirmation: false,
                max_parallel: MAX_PARALLEL_AGENTS,
                steps: vec![],
            });
        }

        self.from_template_ids(
            goal,
            mode,
            context_summary,
            inputs,
            acceptance_criteria,
            &selected.into_iter().map(str::to_string).collect::<Vec<_>>(),
            templates,
        )
    }

    pub fn from_template_ids(
        &self,
        goal: &str,
        mode: DelegationMode,
        context_summary: &str,
        inputs: &[String],
        acceptance_criteria: &[String],
        template_ids: &[String],
        templates: &AgentTemplateRegistry,
    ) -> anyhow::Result<DelegationPlan> {
        if goal.trim().is_empty() {
            anyhow::bail!("delegation goal is required");
        }
        if template_ids.is_empty() {
            anyhow::bail!("select at least one Agent template");
        }
        let mut seen = HashSet::new();
        for template_id in template_ids {
            if !seen.insert(template_id.as_str()) {
                anyhow::bail!("Agent template selections must be unique");
            }
            if templates.get(template_id).is_none() {
                anyhow::bail!("unknown agent template: {template_id}");
            }
        }

        let selected = if mode == DelegationMode::Manual {
            if template_ids
                .iter()
                .position(|template_id| template_id == "reviewer")
                .is_some_and(|position| position + 1 != template_ids.len())
            {
                anyhow::bail!("the Reviewer Agent must be last in a manual workflow");
            }
            template_ids.to_vec()
        } else {
            let mut selected = template_ids
                .iter()
                .filter(|template_id| template_id.as_str() != "reviewer")
                .cloned()
                .collect::<Vec<_>>();
            if let Some(position) = selected
                .iter()
                .position(|template_id| template_id == "code_execution")
            {
                let code = selected.remove(position);
                selected.insert(0, code);
            }
            selected.push("reviewer".into());
            selected
        };

        let mut steps = Vec::new();
        for template_id in selected {
            let dependencies = if mode == DelegationMode::Manual {
                steps
                    .last()
                    .map(|step: &DelegationPlanStep| vec![step.id.clone()])
                    .unwrap_or_default()
            } else if template_id == "reviewer" {
                steps
                    .iter()
                    .map(|step: &DelegationPlanStep| step.id.clone())
                    .collect()
            } else if template_id != "code_execution"
                && steps
                    .iter()
                    .any(|step: &DelegationPlanStep| step.id == "code_execution")
            {
                vec!["code_execution".into()]
            } else {
                vec![]
            };
            let is_reviewer = template_id == "reviewer";
            steps.push(build_step(
                &template_id,
                if is_reviewer {
                    "Independently review the delegated results against the original goal and acceptance criteria."
                } else {
                    goal
                },
                context_summary,
                inputs,
                acceptance_criteria,
                dependencies,
                !is_reviewer,
                templates,
            )?);
        }
        let requires_confirmation = match mode {
            DelegationMode::Manual | DelegationMode::Assisted => true,
            DelegationMode::Automatic => steps.iter().any(|step| {
                capabilities_require_automatic_confirmation(
                    &step.spec.backend,
                    &step.spec.permissions,
                    &step.spec.budget,
                )
            }),
        };
        let plan = DelegationPlan {
            schema_version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            goal: goal.into(),
            mode,
            requires_confirmation,
            max_parallel: if mode == DelegationMode::Manual {
                1
            } else {
                MAX_PARALLEL_AGENTS
            },
            steps,
        };
        plan.validate(templates)?;
        Ok(plan)
    }
}

fn capabilities_require_automatic_confirmation(
    backend: &AgentBackend,
    permissions: &PermissionSet,
    budget: &AgentBudget,
) -> bool {
    matches!(
        backend,
        AgentBackend::Acp | AgentBackend::Http | AgentBackend::Custom(_)
    ) || permissions.write
        || permissions.execute
        || permissions.network
        || budget
            .max_tokens
            .is_some_and(|tokens| tokens > AUTOMATIC_TOKEN_CONFIRMATION_THRESHOLD)
        || budget
            .max_cost_microunits
            .is_some_and(|cost| cost > AUTOMATIC_COST_CONFIRMATION_THRESHOLD_MICROUNITS)
}

fn build_step(
    template_id: &str,
    goal: &str,
    context_summary: &str,
    inputs: &[String],
    acceptance_criteria: &[String],
    dependencies: Vec<String>,
    requires_review: bool,
    templates: &AgentTemplateRegistry,
) -> anyhow::Result<DelegationPlanStep> {
    let template = templates
        .get(template_id)
        .ok_or_else(|| anyhow::anyhow!("missing built-in template: {template_id}"))?;
    let spec = template.instantiate(AgentInstanceRequest {
        agent_id: template_id.into(),
        name: template.display_name.clone(),
        goal: goal.into(),
        context_summary: context_summary.into(),
        inputs: inputs.to_vec(),
        acceptance_criteria: acceptance_criteria.to_vec(),
        dependencies,
        model: None,
        permissions: template.permission_ceiling.clone(),
        context_policy: template.context_ceiling.clone(),
        budget: template.budget_ceiling.clone(),
        timeout_secs: template.timeout_ceiling_secs,
        requires_review,
        session_policy: AgentSessionPolicy::New,
        allow_delegation: false,
    })?;
    Ok(DelegationPlanStep {
        id: template_id.into(),
        input: json!({
            "goal": spec.goal,
            "context_summary": spec.context_summary,
            "inputs": spec.inputs,
            "acceptance_criteria": spec.acceptance_criteria,
        }),
        spec,
    })
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn builtin_templates() -> Vec<AgentTemplate> {
    vec![
        template(
            "code_execution",
            "Code execution Agent",
            "Execute and verify project-scoped code through Codex ACP.",
            AgentRole::Coder,
            AgentBackend::Acp,
            &["read_file", "write_file", "codex_project_exec"],
            true,
            false,
            32_000,
            1_800,
        ),
        template(
            "biology_interpreter",
            "Biology interpretation Agent",
            "Interpret biological meaning with evidence and uncertainty labels.",
            AgentRole::Analyst,
            AgentBackend::Local,
            &["read_file"],
            false,
            false,
            20_000,
            900,
        ),
        template(
            "visualization",
            "Visualization Agent",
            "Design scientific figures and produce validated plotting artifacts.",
            AgentRole::Coder,
            AgentBackend::Acp,
            &["read_file", "write_file", "codex_project_exec"],
            true,
            false,
            24_000,
            1_200,
        ),
        template(
            "reviewer",
            "Reviewer Agent",
            "Independently inspect outputs and return a structured issue list without modifying artifacts.",
            AgentRole::Reviewer,
            AgentBackend::Local,
            &["read_file"],
            false,
            false,
            16_000,
            600,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn template(
    id: &str,
    display_name: &str,
    description: &str,
    role: AgentRole,
    backend: AgentBackend,
    tools: &[&str],
    write: bool,
    network: bool,
    max_tokens: u32,
    timeout_secs: u64,
) -> AgentTemplate {
    AgentTemplate {
        id: id.into(),
        display_name: display_name.into(),
        description: description.into(),
        role,
        backend,
        default_model: None,
        prompt_template: format!("You are Wisp's controlled {display_name}. {description}"),
        input_contract: json!({"type":"object"}),
        output_contract: json!({"type":"object"}),
        permission_ceiling: PermissionSet {
            tools: tools.iter().map(|value| (*value).into()).collect(),
            paths: vec!["project://**".into()],
            network,
            write,
            execute: tools
                .iter()
                .any(|tool| matches!(*tool, "codex_project_exec" | "shell")),
        },
        context_ceiling: ContextPolicy {
            include_history: true,
            include_artifacts: true,
            max_tokens: Some(max_tokens),
        },
        budget_ceiling: AgentBudget {
            max_tokens: Some(max_tokens),
            max_tool_calls: Some(64),
            // The built-in providers do not all report a comparable monetary
            // cost. Generated plans therefore leave cost uncapped instead of
            // presenting an unenforceable default; an explicit cap is handled
            // fail-closed by backends that receive one.
            max_cost_microunits: None,
        },
        timeout_ceiling_secs: Some(timeout_secs),
        allow_delegation: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentDelegationRequest, AgentDelegationResponse, AgentDelegator,
        DelegationRequestValidator, DelegationStatus, ValidatedAgentDelegationRequest,
    };

    struct EchoDelegator;

    struct OverBudgetDelegator;

    #[async_trait::async_trait]
    impl AgentDelegator for EchoDelegator {
        async fn delegate_validated(
            &self,
            request: ValidatedAgentDelegationRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            Ok(AgentDelegationResponse {
                request_id: request.as_request().request_id.clone(),
                status: DelegationStatus::Succeeded,
                output: json!({}),
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

    #[async_trait::async_trait]
    impl AgentDelegator for OverBudgetDelegator {
        async fn delegate_validated(
            &self,
            request: ValidatedAgentDelegationRequest,
        ) -> anyhow::Result<AgentDelegationResponse> {
            Ok(AgentDelegationResponse {
                request_id: request.as_request().request_id.clone(),
                status: DelegationStatus::Succeeded,
                output: json!({}),
                artifact_ids: vec![],
                artifacts: vec![],
                evidence: vec![],
                usage: crate::AgentUsage {
                    input_tokens: 2,
                    ..Default::default()
                },
                agent_session_id: Some("session".into()),
                child_frame_id: Some("frame".into()),
                error: None,
                nested_results: vec![],
            })
        }
    }

    #[test]
    fn template_caps_permissions_and_disables_nested_delegation() {
        let templates = AgentTemplateRegistry::builtins();
        let reviewer = templates.get("reviewer").unwrap();
        let spec = reviewer
            .instantiate(AgentInstanceRequest {
                agent_id: "review".into(),
                name: "Review".into(),
                goal: "Review results".into(),
                context_summary: String::new(),
                inputs: vec![],
                acceptance_criteria: vec![],
                dependencies: vec![],
                model: None,
                permissions: PermissionSet {
                    tools: vec!["read_file".into(), "shell".into()],
                    paths: vec!["project://**".into(), "secret://**".into()],
                    network: true,
                    write: true,
                    execute: true,
                },
                context_policy: reviewer.context_ceiling.clone(),
                budget: AgentBudget {
                    max_tokens: Some(99_999),
                    max_tool_calls: Some(100),
                    max_cost_microunits: None,
                },
                timeout_secs: Some(99_999),
                requires_review: false,
                session_policy: AgentSessionPolicy::New,
                allow_delegation: true,
            })
            .unwrap();
        assert_eq!(spec.permissions.tools, vec!["read_file"]);
        assert!(!spec.permissions.network);
        assert!(!spec.permissions.write);
        assert!(!spec.allow_delegation);
        assert_eq!(spec.timeout_secs, reviewer.timeout_ceiling_secs);
        assert_eq!(spec.budget.max_cost_microunits, None);
        reviewer.validate_spec(&spec).unwrap();
    }

    #[test]
    fn planner_selects_parallel_specialists_and_final_reviewer() {
        let templates = AgentTemplateRegistry::builtins();
        let plan = DelegationPlanner
            .suggest(
                "分析基因通路并绘制可视化图",
                DelegationMode::Assisted,
                "confirmed context",
                &["data.tsv".into()],
                &["tests pass".into()],
                &templates,
            )
            .unwrap();
        assert!(plan.requires_confirmation);
        assert_eq!(plan.max_parallel, 2);
        let code = plan
            .steps
            .iter()
            .find(|step| step.id == "code_execution")
            .unwrap();
        assert!(code
            .spec
            .permissions
            .tools
            .iter()
            .any(|tool| tool == "codex_project_exec"));
        assert!(!code.spec.permissions.network);
        let reviewer = plan.steps.last().unwrap();
        assert_eq!(reviewer.id, "reviewer");
        assert_eq!(reviewer.spec.dependencies.len(), plan.steps.len() - 1);
        plan.validate(&templates).unwrap();
    }

    #[test]
    fn manual_plans_require_and_preserve_an_explicit_sequential_team() {
        let templates = AgentTemplateRegistry::builtins();
        assert!(DelegationPlanner
            .suggest(
                "analyze data",
                DelegationMode::Manual,
                "",
                &[],
                &[],
                &templates,
            )
            .is_err());
        let plan = DelegationPlanner
            .from_template_ids(
                "interpret results and review them",
                DelegationMode::Manual,
                "",
                &[],
                &[],
                &["biology_interpreter".into(), "reviewer".into()],
                &templates,
            )
            .unwrap();
        assert_eq!(plan.max_parallel, 1);
        assert!(plan.requires_confirmation);
        assert_eq!(plan.steps[0].id, "biology_interpreter");
        assert_eq!(plan.steps[1].id, "reviewer");
        assert_eq!(plan.steps[1].spec.dependencies, vec!["biology_interpreter"]);
    }

    #[test]
    fn automatic_plans_only_skip_confirmation_for_low_risk_local_steps() {
        let templates = AgentTemplateRegistry::builtins();
        let biology = DelegationPlanner
            .from_template_ids(
                "interpret the biological result",
                DelegationMode::Automatic,
                "",
                &[],
                &[],
                &["biology_interpreter".into()],
                &templates,
            )
            .unwrap();
        assert!(!biology.requires_confirmation);
        assert_eq!(biology.steps.last().unwrap().id, "reviewer");

        let code = DelegationPlanner
            .from_template_ids(
                "write and execute a workflow",
                DelegationMode::Automatic,
                "",
                &[],
                &[],
                &["code_execution".into()],
                &templates,
            )
            .unwrap();
        assert!(code.requires_confirmation);
        assert!(templates
            .get("code_execution")
            .unwrap()
            .automatic_requires_confirmation());
        assert!(!templates
            .get("reviewer")
            .unwrap()
            .automatic_requires_confirmation());
    }

    #[test]
    fn simple_goal_selects_no_subagents() {
        let templates = AgentTemplateRegistry::builtins();
        let plan = DelegationPlanner
            .suggest(
                "say hello",
                DelegationMode::Automatic,
                "",
                &[],
                &[],
                &templates,
            )
            .unwrap();
        assert!(plan.steps.is_empty());
        plan.validate(&templates).unwrap();
    }

    #[test]
    fn dependency_cycles_are_rejected() {
        let templates = AgentTemplateRegistry::builtins();
        let mut plan = DelegationPlanner
            .suggest(
                "分析代码",
                DelegationMode::Assisted,
                "",
                &[],
                &[],
                &templates,
            )
            .unwrap();
        plan.steps[0].spec.dependencies = vec!["reviewer".into()];
        assert!(plan.validate(&templates).is_err());
    }

    #[tokio::test]
    async fn delegation_boundary_rejects_tampered_template_fields() {
        let templates = AgentTemplateRegistry::builtins();
        let mut spec = templates
            .get("reviewer")
            .unwrap()
            .instantiate(AgentInstanceRequest {
                agent_id: "review".into(),
                name: "Reviewer".into(),
                goal: "Review".into(),
                context_summary: String::new(),
                inputs: vec![],
                acceptance_criteria: vec![],
                dependencies: vec![],
                model: None,
                permissions: PermissionSet::default(),
                context_policy: ContextPolicy::default(),
                budget: AgentBudget::default(),
                timeout_secs: None,
                requires_review: false,
                session_policy: AgentSessionPolicy::New,
                allow_delegation: false,
            })
            .unwrap();
        spec.permissions.write = true;
        let request = AgentDelegationRequest {
            request_id: "request".into(),
            workflow_id: "workflow".into(),
            step_id: "review".into(),
            spec,
            input: json!({}),
            lineage: None,
        };
        assert!(EchoDelegator
            .delegate(request, DelegationRequestValidator::Legacy(&templates))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn delegation_boundary_fails_over_budget_results_without_losing_provenance() {
        let templates = AgentTemplateRegistry::builtins();
        let template = templates.get("reviewer").unwrap().clone();
        let spec = template
            .instantiate(AgentInstanceRequest {
                agent_id: "review".into(),
                name: "Reviewer".into(),
                goal: "Review".into(),
                context_summary: String::new(),
                inputs: vec![],
                acceptance_criteria: vec![],
                dependencies: vec![],
                model: None,
                permissions: PermissionSet::default(),
                context_policy: ContextPolicy::default(),
                budget: AgentBudget {
                    max_tokens: Some(1),
                    max_tool_calls: Some(1),
                    max_cost_microunits: Some(1),
                },
                timeout_secs: None,
                requires_review: false,
                session_policy: AgentSessionPolicy::New,
                allow_delegation: false,
            })
            .unwrap();
        let response = OverBudgetDelegator
            .delegate(
                AgentDelegationRequest {
                    request_id: "request".into(),
                    workflow_id: "workflow".into(),
                    step_id: "review".into(),
                    spec,
                    input: json!({}),
                    lineage: None,
                },
                DelegationRequestValidator::Legacy(&templates),
            )
            .await
            .unwrap();
        assert_eq!(response.status, DelegationStatus::Failed);
        assert!(response.error.unwrap().contains("token budget"));
        assert_eq!(response.agent_session_id.as_deref(), Some("session"));
        assert_eq!(response.child_frame_id.as_deref(), Some("frame"));
    }

    fn dynamic_step(id: &str, dependencies: &[&str]) -> DelegationPlanStep {
        DelegationPlanStep {
            id: id.into(),
            spec: AgentSpec {
                template_id: "dynamic".into(),
                agent_id: id.into(),
                name: id.into(),
                goal: "Complete the assigned task".into(),
                context_summary: String::new(),
                inputs: vec![],
                acceptance_criteria: vec![],
                dependencies: dependencies.iter().map(|value| (*value).into()).collect(),
                role: AgentRole::Custom("worker".into()),
                backend: AgentBackend::Local,
                model: Some("test-model".into()),
                prompt_template: "Complete only the assigned task.".into(),
                input_contract: json!({"type":"object"}),
                output_contract: json!({"type":"object"}),
                permissions: PermissionSet::default(),
                context_policy: ContextPolicy::default(),
                budget: AgentBudget::default(),
                timeout_secs: Some(60),
                requires_review: false,
                session_policy: AgentSessionPolicy::New,
                allow_delegation: false,
                origin: AgentOrigin::Temporary,
                capabilities: vec!["project_read".into()],
                executor: Some(crate::AgentExecutorRef::Native),
                request_preferences: None,
                workspace_policy: Some(crate::AgentWorkspacePolicy::SharedReadOnly),
                output_schema_source: Default::default(),
                approval_reasons: vec![],
                authorization: None,
            },
            input: json!({}),
        }
    }

    fn dynamic_plan(steps: Vec<DelegationPlanStep>) -> DelegationPlan {
        DelegationPlan {
            schema_version: DYNAMIC_DELEGATION_SCHEMA_VERSION,
            id: "dynamic-plan".into(),
            goal: "Complete a dynamic batch".into(),
            mode: DelegationMode::Automatic,
            requires_confirmation: false,
            max_parallel: 2,
            steps,
        }
    }

    #[test]
    fn dynamic_plan_accepts_parallel_fan_in_and_round_trips() {
        let plan = dynamic_plan(vec![
            dynamic_step("inspect", &[]),
            dynamic_step("research", &[]),
            dynamic_step("synthesize", &["inspect", "research"]),
        ]);
        plan.validate(&AgentTemplateRegistry::builtins()).unwrap();
        assert_eq!(
            plan.steps
                .iter()
                .map(|step| step.id.as_str())
                .collect::<Vec<_>>(),
            ["inspect", "research", "synthesize"]
        );
        let round_trip: DelegationPlan =
            serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(round_trip, plan);
    }

    #[test]
    fn legacy_plan_without_v2_fields_keeps_v1_meaning() {
        let plan = DelegationPlanner
            .from_template_ids(
                "interpret results",
                DelegationMode::Manual,
                "",
                &[],
                &[],
                &["biology_interpreter".into()],
                &AgentTemplateRegistry::builtins(),
            )
            .unwrap();
        let mut value = serde_json::to_value(&plan).unwrap();
        value.as_object_mut().unwrap().remove("schema_version");
        for step in value["steps"].as_array_mut().unwrap() {
            let spec = step["spec"].as_object_mut().unwrap();
            spec["permissions"]
                .as_object_mut()
                .unwrap()
                .remove("execute");
            for key in [
                "origin",
                "capabilities",
                "executor",
                "workspace_policy",
                "output_schema_source",
                "approval_reasons",
            ] {
                spec.remove(key);
            }
        }
        let restored: DelegationPlan = serde_json::from_value(value).unwrap();
        assert_eq!(restored.schema_version, 1);
        assert_eq!(restored.steps[0].spec.origin, AgentOrigin::LegacyTemplate);
        assert!(!restored.steps[0].spec.permissions.execute);
        restored
            .validate(&AgentTemplateRegistry::builtins())
            .unwrap();
    }

    #[test]
    fn dynamic_plan_rejects_task_overflow_and_bad_dependencies() {
        let overflow = dynamic_plan(
            (0..=MAX_DELEGATION_TASKS)
                .map(|index| dynamic_step(&format!("task-{index}"), &[]))
                .collect(),
        );
        assert!(overflow
            .validate(&AgentTemplateRegistry::builtins())
            .is_err());

        let missing = dynamic_plan(vec![dynamic_step("task", &["missing"])]);
        assert!(missing
            .validate(&AgentTemplateRegistry::builtins())
            .is_err());

        let duplicate = dynamic_plan(vec![dynamic_step("task", &[]), dynamic_step("task", &[])]);
        assert!(duplicate
            .validate(&AgentTemplateRegistry::builtins())
            .is_err());

        let self_dependency = dynamic_plan(vec![dynamic_step("task", &["task"])]);
        assert!(self_dependency
            .validate(&AgentTemplateRegistry::builtins())
            .is_err());

        let mut invalid_concurrency = dynamic_plan(vec![dynamic_step("task", &[])]);
        invalid_concurrency.max_parallel = 0;
        assert!(invalid_concurrency
            .validate(&AgentTemplateRegistry::builtins())
            .is_err());

        let mut unknown_schema = dynamic_plan(vec![dynamic_step("task", &[])]);
        unknown_schema.schema_version = 99;
        assert!(unknown_schema
            .validate(&AgentTemplateRegistry::builtins())
            .is_err());
    }
}
