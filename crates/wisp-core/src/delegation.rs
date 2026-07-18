//! Stable runtime contracts for delegating work to another agent.
//!
//! This module intentionally contains only the protocol boundary. Backend
//! adapters and scheduling stay outside the core so a workflow can be stored,
//! inspected, and executed by different runtimes without changing its shape.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Planner,
    Researcher,
    Coder,
    Reviewer,
    Analyst,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentBackend {
    Local,
    Acp,
    Http,
    Custom(String),
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContextPolicy {
    #[serde(default)]
    pub include_history: bool,
    #[serde(default)]
    pub include_artifacts: bool,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub agent_id: String,
    pub role: AgentRole,
    pub backend: AgentBackend,
    #[serde(default)]
    pub model: Option<String>,
    pub prompt_template: String,
    #[serde(default)]
    pub permissions: PermissionSet,
    #[serde(default)]
    pub context_policy: ContextPolicy,
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
            timeout_secs: Some(60),
        };
        spec.validate().unwrap();
        let value = serde_json::to_value(&spec).unwrap();
        assert_eq!(value["role"], "reviewer");
        assert_eq!(value["backend"], "acp");
        assert_eq!(value["context_policy"]["max_tokens"], 8_000);
    }

    #[test]
    fn zero_timeout_is_rejected() {
        let spec = AgentSpec {
            agent_id: "a".into(),
            role: AgentRole::Custom("specialist".into()),
            backend: AgentBackend::Local,
            model: None,
            prompt_template: "x".into(),
            permissions: PermissionSet::default(),
            context_policy: ContextPolicy::default(),
            timeout_secs: Some(0),
        };
        assert!(spec.validate().is_err());
    }
}
