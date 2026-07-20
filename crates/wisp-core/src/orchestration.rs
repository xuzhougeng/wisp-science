//! Dynamic temporary-Agent delegation plans.

use crate::AgentSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const MAX_PARALLEL_AGENTS: usize = 2;
pub const MAX_DELEGATION_TASKS: usize = 8;
pub const DYNAMIC_DELEGATION_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationMode {
    Manual,
    Automatic,
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
    pub schema_version: u32,
    pub id: String,
    pub goal: String,
    pub mode: DelegationMode,
    pub requires_confirmation: bool,
    pub max_parallel: usize,
    pub steps: Vec<DelegationPlanStep>,
}

impl DelegationPlan {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.validate_structure()?;
        for step in &self.steps {
            step.spec.validate_dynamic_metadata()?;
        }
        Ok(())
    }

    pub fn validate_structure(&self) -> anyhow::Result<()> {
        if self.schema_version != DYNAMIC_DELEGATION_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported delegation plan schema version; only dynamic schema v{DYNAMIC_DELEGATION_SCHEMA_VERSION} is accepted"
            );
        }
        if self.id.trim().is_empty() || self.goal.trim().is_empty() {
            anyhow::bail!("delegation plan id and goal are required");
        }
        if self.max_parallel == 0 || self.max_parallel > MAX_PARALLEL_AGENTS {
            anyhow::bail!("max_parallel must be between 1 and {MAX_PARALLEL_AGENTS}");
        }
        if self.steps.is_empty() || self.steps.len() > MAX_DELEGATION_TASKS {
            anyhow::bail!(
                "delegation plan must contain between 1 and {MAX_DELEGATION_TASKS} tasks"
            );
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
        ensure_acyclic(&self.steps)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentBackend, AgentBudget, AgentExecutorRef, AgentOrigin, AgentOutputSchemaSource,
        AgentRole, AgentSessionPolicy, AgentWorkspacePolicy, ContextPolicy, PermissionSet,
    };
    use serde_json::json;

    fn dynamic_step(id: &str, dependencies: &[&str]) -> DelegationPlanStep {
        DelegationPlanStep {
            id: id.into(),
            spec: AgentSpec {
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
                executor: Some(AgentExecutorRef::Native),
                request_preferences: None,
                workspace_policy: Some(AgentWorkspacePolicy::SharedReadOnly),
                output_schema_source: AgentOutputSchemaSource::Standard,
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
        plan.validate().unwrap();
        let round_trip: DelegationPlan =
            serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(round_trip, plan);
    }

    #[test]
    fn fixed_schema_is_neither_defaulted_nor_accepted() {
        let plan = dynamic_plan(vec![dynamic_step("task", &[])]);
        let mut missing = serde_json::to_value(&plan).unwrap();
        missing.as_object_mut().unwrap().remove("schema_version");
        assert!(serde_json::from_value::<DelegationPlan>(missing).is_err());

        let mut fixed = plan;
        fixed.schema_version = 1;
        assert!(fixed.validate().is_err());
    }

    #[test]
    fn dynamic_plan_rejects_invalid_structure() {
        let overflow = dynamic_plan(
            (0..=MAX_DELEGATION_TASKS)
                .map(|index| dynamic_step(&format!("task-{index}"), &[]))
                .collect(),
        );
        assert!(overflow.validate().is_err());
        assert!(dynamic_plan(vec![]).validate().is_err());
        assert!(dynamic_plan(vec![dynamic_step("task", &["missing"])])
            .validate()
            .is_err());
        assert!(
            dynamic_plan(vec![dynamic_step("task", &[]), dynamic_step("task", &[])])
                .validate()
                .is_err()
        );
        assert!(dynamic_plan(vec![dynamic_step("task", &["task"])])
            .validate()
            .is_err());

        let mut invalid_concurrency = dynamic_plan(vec![dynamic_step("task", &[])]);
        invalid_concurrency.max_parallel = 0;
        assert!(invalid_concurrency.validate().is_err());
    }
}
