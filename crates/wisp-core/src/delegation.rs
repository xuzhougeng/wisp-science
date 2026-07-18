//! Stable runtime contracts for delegating work to another agent.
//!
//! This module intentionally contains only the protocol boundary. Backend
//! adapters and scheduling stay outside the core so a workflow can be stored,
//! inspected, and executed by different runtimes without changing its shape.

use async_trait::async_trait;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRole {
    Planner,
    Researcher,
    Coder,
    Reviewer,
    Analyst,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentBackend {
    Local,
    Acp,
    Http,
    Custom(String),
}

impl AgentRole {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Planner => "planner",
            Self::Researcher => "researcher",
            Self::Coder => "coder",
            Self::Reviewer => "reviewer",
            Self::Analyst => "analyst",
            Self::Custom(value) => value,
        }
    }
}

impl Serialize for AgentRole {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AgentRole {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "planner" => Self::Planner,
            "researcher" => Self::Researcher,
            "coder" => Self::Coder,
            "reviewer" => Self::Reviewer,
            "analyst" => Self::Analyst,
            _ if value.trim().is_empty() => return Err(D::Error::custom("role is required")),
            _ => Self::Custom(value),
        })
    }
}

impl AgentBackend {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Local => "local",
            Self::Acp => "acp",
            Self::Http => "http",
            Self::Custom(value) => value,
        }
    }
}

impl Serialize for AgentBackend {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AgentBackend {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "local" => Self::Local,
            "acp" => Self::Acp,
            "http" => Self::Http,
            _ if value.trim().is_empty() => return Err(D::Error::custom("backend is required")),
            _ => Self::Custom(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PermissionSet {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub network: bool,
}

impl PermissionSet {
    /// Return the permissions safe to grant when both the caller and backend
    /// impose a ceiling. Ordering follows the requested set for stable JSON.
    pub fn intersect(&self, granted: &Self) -> Self {
        Self {
            tools: self
                .tools
                .iter()
                .filter(|tool| granted.tools.iter().any(|allowed| allowed == *tool))
                .cloned()
                .collect(),
            paths: self
                .paths
                .iter()
                .filter(|path| granted.paths.iter().any(|allowed| allowed == *path))
                .cloned()
                .collect(),
            network: self.network && granted.network,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContextPolicy {
    #[serde(default)]
    pub include_history: bool,
    #[serde(default)]
    pub include_artifacts: bool,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

impl ContextPolicy {
    /// Restrict context to what both the workflow and backend allow.
    pub fn restrict(&self, allowed: &Self) -> Self {
        Self {
            include_history: self.include_history && allowed.include_history,
            include_artifacts: self.include_artifacts && allowed.include_artifacts,
            max_tokens: match (self.max_tokens, allowed.max_tokens) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (left, None) => left,
                (None, right) => right,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentBudget {
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub max_tool_calls: Option<u32>,
    #[serde(default)]
    pub max_cost_microunits: Option<u64>,
}

fn empty_json_object() -> Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub agent_id: String,
    pub role: AgentRole,
    pub backend: AgentBackend,
    #[serde(default)]
    pub model: Option<String>,
    pub prompt_template: String,
    #[serde(default = "empty_json_object")]
    pub input_contract: Value,
    #[serde(default = "empty_json_object")]
    pub output_contract: Value,
    #[serde(default)]
    pub permissions: PermissionSet,
    #[serde(default)]
    pub context_policy: ContextPolicy,
    #[serde(default)]
    pub budget: AgentBudget,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

impl AgentSpec {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.agent_id.trim().is_empty() {
            anyhow::bail!("agent_id is required");
        }
        if self.prompt_template.trim().is_empty() {
            anyhow::bail!("prompt_template is required");
        }
        if self.timeout_secs == Some(0) {
            anyhow::bail!("timeout_secs must be positive");
        }
        for (name, contract) in [
            ("input_contract", &self.input_contract),
            ("output_contract", &self.output_contract),
        ] {
            if !contract.is_object() {
                anyhow::bail!("{name} must be a JSON object");
            }
        }
        if self.context_policy.max_tokens == Some(0)
            || self.budget.max_tokens == Some(0)
            || self.budget.max_tool_calls == Some(0)
            || self.budget.max_cost_microunits == Some(0)
        {
            anyhow::bail!("agent budgets must be positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDelegationRequest {
    pub request_id: String,
    pub workflow_id: String,
    pub step_id: String,
    pub spec: AgentSpec,
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDelegationResponse {
    pub request_id: String,
    pub status: DelegationStatus,
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl AgentDelegationRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.request_id.trim().is_empty() {
            anyhow::bail!("request_id is required");
        }
        if self.workflow_id.trim().is_empty() {
            anyhow::bail!("workflow_id is required");
        }
        if self.step_id.trim().is_empty() {
            anyhow::bail!("step_id is required");
        }
        self.spec.validate()
    }
}

impl AgentDelegationResponse {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.request_id.trim().is_empty() {
            anyhow::bail!("request_id is required");
        }
        if self.status == DelegationStatus::Failed
            && self.error.as_deref().is_none_or(str::is_empty)
        {
            anyhow::bail!("failed delegation responses require an error");
        }
        if self.status == DelegationStatus::Succeeded && self.error.is_some() {
            anyhow::bail!("succeeded delegation responses cannot contain an error");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationStatus {
    Accepted,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[async_trait]
pub trait AgentDelegator: Send + Sync {
    async fn delegate(
        &self,
        request: AgentDelegationRequest,
    ) -> anyhow::Result<AgentDelegationResponse>;

    async fn status(&self, _request_id: &str) -> anyhow::Result<Option<AgentDelegationResponse>> {
        anyhow::bail!("agent delegation status is not supported by this backend")
    }

    async fn cancel(&self, _request_id: &str) -> anyhow::Result<bool> {
        anyhow::bail!("agent delegation cancellation is not supported by this backend")
    }
}

/// Explicit placeholder for runtimes that persist a workflow before a backend
/// adapter is configured. It fails loudly instead of pretending work ran.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredAgentDelegator;

#[async_trait]
impl AgentDelegator for UnconfiguredAgentDelegator {
    async fn delegate(
        &self,
        _request: AgentDelegationRequest,
    ) -> anyhow::Result<AgentDelegationResponse> {
        anyhow::bail!("agent delegation backend is not configured")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_spec_has_stable_json_shape() {
        let spec = AgentSpec {
            agent_id: "reviewer".into(),
            role: AgentRole::Reviewer,
            backend: AgentBackend::Acp,
            model: Some("reasoning-model".into()),
            prompt_template: "Review {{input}}".into(),
            input_contract: serde_json::json!({"type":"object"}),
            output_contract: serde_json::json!({"type":"object"}),
            permissions: PermissionSet {
                tools: vec!["read_file".into()],
                paths: vec!["src/**".into()],
                network: false,
            },
            context_policy: ContextPolicy {
                include_history: true,
                include_artifacts: true,
                max_tokens: Some(8_000),
            },
            budget: AgentBudget {
                max_tokens: Some(8_000),
                ..AgentBudget::default()
            },
            timeout_secs: Some(60),
        };
        spec.validate().unwrap();
        let value = serde_json::to_value(&spec).unwrap();
        assert_eq!(value["role"], "reviewer");
        assert_eq!(value["backend"], "acp");
        assert_eq!(value["budget"]["max_tokens"], 8_000);
    }

    #[test]
    fn zero_timeout_is_rejected() {
        let spec = AgentSpec {
            agent_id: "a".into(),
            role: AgentRole::Custom("specialist".into()),
            backend: AgentBackend::Local,
            model: None,
            prompt_template: "x".into(),
            input_contract: serde_json::json!({}),
            output_contract: serde_json::json!({}),
            permissions: PermissionSet::default(),
            context_policy: ContextPolicy::default(),
            budget: AgentBudget::default(),
            timeout_secs: Some(0),
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    fn permissions_and_context_are_intersected() {
        let requested = PermissionSet {
            tools: vec!["read".into(), "write".into()],
            paths: vec!["src/**".into(), "tmp/**".into()],
            network: true,
        };
        let granted = PermissionSet {
            tools: vec!["read".into()],
            paths: vec!["src/**".into()],
            network: false,
        };
        assert_eq!(requested.intersect(&granted).tools, vec!["read"]);
        assert!(!requested.intersect(&granted).network);

        let requested_context = ContextPolicy {
            include_history: true,
            include_artifacts: true,
            max_tokens: Some(4_000),
        };
        let granted_context = ContextPolicy {
            include_history: true,
            include_artifacts: false,
            max_tokens: Some(1_000),
        };
        assert_eq!(
            requested_context.restrict(&granted_context),
            ContextPolicy {
                include_history: true,
                include_artifacts: false,
                max_tokens: Some(1_000),
            }
        );
    }

    #[test]
    fn custom_values_use_scalar_wire_format_and_requests_validate() {
        let role = serde_json::to_string(&AgentRole::Custom("specialist".into())).unwrap();
        assert_eq!(role, "\"specialist\"");
        let backend: AgentBackend = serde_json::from_str("\"worker\"").unwrap();
        assert_eq!(backend, AgentBackend::Custom("worker".into()));

        let request = AgentDelegationRequest {
            request_id: "r1".into(),
            workflow_id: "wf".into(),
            step_id: "step".into(),
            spec: AgentSpec {
                agent_id: "specialist".into(),
                role: AgentRole::Reviewer,
                backend: AgentBackend::Local,
                model: None,
                prompt_template: "Review".into(),
                input_contract: serde_json::json!({}),
                output_contract: serde_json::json!({}),
                permissions: PermissionSet::default(),
                context_policy: ContextPolicy::default(),
                budget: AgentBudget::default(),
                timeout_secs: Some(1),
            },
            input: serde_json::json!({"diff":"..."}),
        };
        request.validate().unwrap();
        assert!(AgentDelegationResponse {
            request_id: "r1".into(),
            status: DelegationStatus::Failed,
            output: Value::Null,
            artifact_ids: vec![],
            error: None,
        }
        .validate()
        .is_err());
    }
}
