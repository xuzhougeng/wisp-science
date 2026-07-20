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
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub execute: bool,
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
            write: self.write && granted.write,
            execute: self.execute && granted.execute,
        }
    }
}

impl PermissionSet {
    pub fn is_subset_of(&self, ceiling: &Self) -> bool {
        self.intersect(ceiling) == *self
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

impl AgentBudget {
    /// Restrict a requested budget to the backend/workspace ceiling.
    pub fn restrict(&self, ceiling: &Self) -> Self {
        Self {
            max_tokens: restrict_limit(self.max_tokens, ceiling.max_tokens),
            max_tool_calls: restrict_limit(self.max_tool_calls, ceiling.max_tool_calls),
            max_cost_microunits: restrict_limit(
                self.max_cost_microunits,
                ceiling.max_cost_microunits,
            ),
        }
    }
}

pub(crate) fn restrict_limit<T: Ord + Copy>(requested: Option<T>, ceiling: Option<T>) -> Option<T> {
    match (requested, ceiling) {
        (Some(requested), Some(ceiling)) => Some(requested.min(ceiling)),
        (requested, None) => requested,
        (None, ceiling) => ceiling,
    }
}

fn empty_json_object() -> Value {
    serde_json::json!({})
}

pub const MAX_AGENT_OUTPUT_SCHEMA_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionPolicy {
    #[default]
    New,
    ReuseIfAvailable,
    RequireExisting,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SpecialistSnapshot {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub connectors: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRevision {
    pub id: String,
    pub revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAuthorizationSnapshot {
    pub registry_revision: String,
    pub policy_revision: String,
    pub capabilities: Vec<CapabilityRevision>,
    pub integrity_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "specialist", rename_all = "snake_case")]
pub enum AgentOrigin {
    #[default]
    Temporary,
    Specialist(SpecialistSnapshot),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentExecutorRef {
    Native,
    Acp { profile_id: String },
    External { profile_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkspacePolicy {
    SharedReadOnly,
    SerializedMutation,
    Isolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOutputSchemaSource {
    #[default]
    Standard,
    Task,
    Specialist,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentRequestPreferences {
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub executor: Option<AgentExecutorRef>,
    #[serde(default)]
    pub isolated: bool,
    #[serde(default)]
    pub budget: Option<AgentBudget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub agent_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub context_summary: String,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
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
    #[serde(default)]
    pub requires_review: bool,
    #[serde(default)]
    pub session_policy: AgentSessionPolicy,
    #[serde(default)]
    pub allow_delegation: bool,
    #[serde(default)]
    pub origin: AgentOrigin,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub executor: Option<AgentExecutorRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_preferences: Option<AgentRequestPreferences>,
    #[serde(default)]
    pub workspace_policy: Option<AgentWorkspacePolicy>,
    #[serde(default)]
    pub output_schema_source: AgentOutputSchemaSource,
    #[serde(default)]
    pub approval_reasons: Vec<String>,
    #[serde(default)]
    pub authorization: Option<AgentAuthorizationSnapshot>,
}

impl AgentSpec {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.agent_id.trim().is_empty() {
            anyhow::bail!("agent_id is required");
        }
        if self.name.trim().is_empty() {
            anyhow::bail!("agent name is required");
        }
        if self.goal.trim().is_empty() {
            anyhow::bail!("agent goal is required");
        }
        if self.role.as_str().trim().is_empty() {
            anyhow::bail!("role is required");
        }
        if self.backend.as_str().trim().is_empty() {
            anyhow::bail!("backend is required");
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
            if let Some(type_value) = contract.get("type") {
                let kind = type_value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("{name}.type must be a string"))?;
                if !matches!(
                    kind,
                    "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
                ) {
                    anyhow::bail!("{name} has unsupported JSON type: {kind}");
                }
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

    pub fn validate_dynamic_metadata(&self) -> anyhow::Result<()> {
        if serde_json::to_vec(&self.output_contract)?.len() > MAX_AGENT_OUTPUT_SCHEMA_BYTES {
            anyhow::bail!(
                "output_contract exceeds {MAX_AGENT_OUTPUT_SCHEMA_BYTES} serialized bytes"
            );
        }
        match &self.origin {
            AgentOrigin::Temporary => {}
            AgentOrigin::Specialist(snapshot) => {
                if snapshot.id.trim().is_empty() || snapshot.name.trim().is_empty() {
                    anyhow::bail!("specialist snapshot id and name are required");
                }
            }
        }
        if self.capabilities.is_empty() {
            anyhow::bail!("dynamic agent requires at least one capability");
        }
        let mut seen = std::collections::HashSet::new();
        for capability in &self.capabilities {
            if !valid_capability_id(capability) {
                anyhow::bail!("invalid capability id: {capability}");
            }
            if !seen.insert(capability) {
                anyhow::bail!("dynamic agent capabilities must be unique");
            }
        }
        let executor = self
            .executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("dynamic agent executor is required"))?;
        match executor {
            AgentExecutorRef::Native if self.model.is_none() => {
                anyhow::bail!("native executor requires a model profile")
            }
            AgentExecutorRef::Native => {}
            AgentExecutorRef::Acp { profile_id } | AgentExecutorRef::External { profile_id }
                if profile_id.trim().is_empty() =>
            {
                anyhow::bail!("external executor profile_id is required")
            }
            AgentExecutorRef::Acp { .. } | AgentExecutorRef::External { .. }
                if self.model.is_some() =>
            {
                anyhow::bail!("non-native executor cannot include a Native model profile")
            }
            AgentExecutorRef::Acp { .. } | AgentExecutorRef::External { .. } => {}
        }
        if self.workspace_policy.is_none() {
            anyhow::bail!("dynamic agent workspace_policy is required");
        }
        Ok(())
    }

    pub fn constrained_by(
        &self,
        permission_ceiling: &PermissionSet,
        context_ceiling: &ContextPolicy,
        budget_ceiling: &AgentBudget,
        timeout_ceiling: Option<u64>,
    ) -> Self {
        Self {
            permissions: self.permissions.intersect(permission_ceiling),
            context_policy: self.context_policy.restrict(context_ceiling),
            budget: self.budget.restrict(budget_ceiling),
            timeout_secs: restrict_limit(self.timeout_secs, timeout_ceiling),
            ..self.clone()
        }
    }
}

fn valid_capability_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDelegationRequest {
    pub request_id: String,
    pub workflow_id: String,
    pub step_id: String,
    pub spec: AgentSpec,
    #[serde(default)]
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<AgentDelegationLineage>,
}

pub const MAX_AGENT_DELEGATION_DEPTH: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDelegationLineage {
    pub root_workflow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_attempt_id: Option<String>,
    pub depth: u8,
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
    pub artifacts: Vec<AgentArtifact>,
    #[serde(default)]
    pub evidence: Vec<AgentEvidence>,
    #[serde(default)]
    pub usage: AgentUsage,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub child_frame_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub nested_results: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentArtifact {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentEvidence {
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default)]
    pub cost_microunits: u64,
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
        if let Some(lineage) = &self.lineage {
            if lineage.root_workflow_id.trim().is_empty() {
                anyhow::bail!("delegation root_workflow_id is required");
            }
            if lineage.depth == 0 || lineage.depth > MAX_AGENT_DELEGATION_DEPTH {
                anyhow::bail!(
                    "delegation depth must be between 1 and {MAX_AGENT_DELEGATION_DEPTH}"
                );
            }
            if lineage
                .parent_attempt_id
                .as_deref()
                .is_some_and(str::is_empty)
            {
                anyhow::bail!("delegation parent_attempt_id cannot be empty");
            }
        }
        self.spec.validate()?;
        if !matches_json_contract(&self.input, &self.spec.input_contract) {
            anyhow::bail!("delegation input does not satisfy input_contract");
        }
        Ok(())
    }
}

/// A request that has passed identity, contract, and resource validation.
/// The private field prevents adapters from manufacturing one by struct
/// literal; use `try_from` to obtain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAgentDelegationRequest {
    inner: AgentDelegationRequest,
}

impl ValidatedAgentDelegationRequest {
    pub fn as_request(&self) -> &AgentDelegationRequest {
        &self.inner
    }

    pub fn into_request(self) -> AgentDelegationRequest {
        self.inner
    }

    pub fn authorize(
        request: AgentDelegationRequest,
        registry: &crate::delegation_policy::CapabilityRegistry,
        host: &crate::delegation_policy::DelegationHostPolicy,
    ) -> anyhow::Result<Self> {
        request.validate()?;
        registry.validate_resolved_spec(&request.spec, host)?;
        Ok(Self { inner: request })
    }
}

impl AgentDelegationResponse {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.request_id.trim().is_empty() {
            anyhow::bail!("request_id is required");
        }
        if matches!(
            self.status,
            DelegationStatus::Failed | DelegationStatus::Blocked
        ) && self.error.as_deref().is_none_or(str::is_empty)
        {
            anyhow::bail!("failed or blocked delegation responses require an error");
        }
        if self.status == DelegationStatus::Succeeded && self.error.is_some() {
            anyhow::bail!("succeeded delegation responses cannot contain an error");
        }
        Ok(())
    }
}

fn matches_json_contract(value: &Value, contract: &Value) -> bool {
    matches_json_contract_at_depth(value, contract, 0)
}

fn matches_json_contract_at_depth(value: &Value, contract: &Value, depth: usize) -> bool {
    if depth > 32 || !contract.is_object() {
        return false;
    }
    if contract
        .get("const")
        .is_some_and(|expected| expected != value)
        || contract
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|values| !values.contains(value))
    {
        return false;
    }
    if contract
        .get("allOf")
        .and_then(Value::as_array)
        .is_some_and(|schemas| {
            !schemas
                .iter()
                .all(|schema| matches_json_contract_at_depth(value, schema, depth + 1))
        })
        || contract
            .get("anyOf")
            .and_then(Value::as_array)
            .is_some_and(|schemas| {
                !schemas
                    .iter()
                    .any(|schema| matches_json_contract_at_depth(value, schema, depth + 1))
            })
        || contract
            .get("oneOf")
            .and_then(Value::as_array)
            .is_some_and(|schemas| {
                schemas
                    .iter()
                    .filter(|schema| matches_json_contract_at_depth(value, schema, depth + 1))
                    .count()
                    != 1
            })
        || contract
            .get("not")
            .is_some_and(|schema| matches_json_contract_at_depth(value, schema, depth + 1))
    {
        return false;
    }

    let type_matches = match contract.get("type").and_then(Value::as_str) {
        None => true,
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("number") => value.is_number(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("boolean") => value.is_boolean(),
        Some("null") => value.is_null(),
        Some(_) => false,
    };
    if !type_matches {
        return false;
    }

    if let Some(object) = value.as_object() {
        if contract
            .get("minProperties")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| object.len() < minimum as usize)
            || contract
                .get("maxProperties")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| object.len() > maximum as usize)
        {
            return false;
        }
        if contract
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|key| !object.contains_key(key))
            })
        {
            return false;
        }
        let properties = contract.get("properties").and_then(Value::as_object);
        if properties.is_some_and(|properties| {
            properties.iter().any(|(key, schema)| {
                object
                    .get(key)
                    .is_some_and(|value| !matches_json_contract_at_depth(value, schema, depth + 1))
            })
        }) {
            return false;
        }
        if let Some(additional) = contract.get("additionalProperties") {
            for (key, value) in object {
                if properties.is_some_and(|properties| properties.contains_key(key)) {
                    continue;
                }
                match additional {
                    Value::Bool(true) => {}
                    Value::Bool(false) => return false,
                    Value::Object(_) => {
                        if !matches_json_contract_at_depth(value, additional, depth + 1) {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
        }
    }

    if let Some(array) = value.as_array() {
        if contract
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| array.len() < minimum as usize)
            || contract
                .get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| array.len() > maximum as usize)
            || contract.get("uniqueItems") == Some(&Value::Bool(true))
                && array
                    .iter()
                    .enumerate()
                    .any(|(index, item)| array[..index].contains(item))
        {
            return false;
        }
        if let Some(items) = contract.get("items") {
            if !array
                .iter()
                .all(|item| matches_json_contract_at_depth(item, items, depth + 1))
            {
                return false;
            }
        }
    }

    if let Some(text) = value.as_str() {
        let chars = text.chars().count();
        if contract
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|minimum| chars < minimum as usize)
            || contract
                .get("maxLength")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| chars > maximum as usize)
            || contract
                .get("pattern")
                .and_then(Value::as_str)
                .is_some_and(|pattern| {
                    regex::Regex::new(pattern).map_or(true, |regex| !regex.is_match(text))
                })
        {
            return false;
        }
    }

    if let Some(number) = value.as_f64() {
        if contract
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|minimum| number < minimum)
            || contract
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|maximum| number > maximum)
            || contract
                .get("exclusiveMinimum")
                .and_then(Value::as_f64)
                .is_some_and(|minimum| number <= minimum)
            || contract
                .get("exclusiveMaximum")
                .and_then(Value::as_f64)
                .is_some_and(|maximum| number >= maximum)
        {
            return false;
        }
    }
    true
}

fn task_output_envelope(data: Value, response: &AgentDelegationResponse) -> Value {
    let summary = data
        .get("summary")
        .and_then(Value::as_str)
        .filter(|summary| !summary.trim().is_empty())
        .unwrap_or("Structured task output is available in data.")
        .to_string();
    serde_json::json!({
        "summary": summary,
        "data": data,
        "artifacts": response.artifacts.clone(),
        "evidence": response.evidence.clone(),
        "tests": [],
        "risks": [],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationStatus {
    Accepted,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Blocked,
}

#[async_trait]
pub trait AgentDelegator: Send + Sync {
    /// Execute a request that the dynamic capability policy has authorized.
    async fn delegate_authorized(
        &self,
        request: ValidatedAgentDelegationRequest,
    ) -> anyhow::Result<AgentDelegationResponse> {
        let request_id = request.as_request().request_id.clone();
        let output_contract = request.as_request().spec.output_contract.clone();
        let task_output =
            request.as_request().spec.output_schema_source == AgentOutputSchemaSource::Task;
        let budget = request.as_request().spec.budget.clone();
        let mut response = self.delegate_validated(request).await?;
        response.validate()?;
        if response.request_id != request_id {
            anyhow::bail!("delegation response request_id does not match the request");
        }
        if let Some(reason) = budget_violation(&response.usage, &budget) {
            response.status = DelegationStatus::Failed;
            response.output = Value::Object(Default::default());
            response.error = Some(reason);
        }
        if response.status == DelegationStatus::Succeeded
            && !matches_json_contract(&response.output, &output_contract)
        {
            response.status = DelegationStatus::Failed;
            response.output = Value::Object(Default::default());
            response.error = Some("delegation output does not satisfy output_contract".into());
        } else if response.status == DelegationStatus::Succeeded && task_output {
            let data = std::mem::take(&mut response.output);
            response.output = task_output_envelope(data, &response);
        }
        response.validate()?;
        Ok(response)
    }

    /// Backend implementations receive requests only after the public method
    /// has validated their identity, contracts, and resource bounds.
    async fn delegate_validated(
        &self,
        request: ValidatedAgentDelegationRequest,
    ) -> anyhow::Result<AgentDelegationResponse>;

    async fn status(&self, _request_id: &str) -> anyhow::Result<Option<AgentDelegationResponse>> {
        anyhow::bail!("agent delegation status is not supported by this backend")
    }

    async fn cancel(&self, _request_id: &str) -> anyhow::Result<bool> {
        anyhow::bail!("agent delegation cancellation is not supported by this backend")
    }
}

fn budget_violation(usage: &AgentUsage, budget: &AgentBudget) -> Option<String> {
    let total_tokens = usage.input_tokens.saturating_add(usage.output_tokens);
    if budget
        .max_tokens
        .is_some_and(|limit| total_tokens > u64::from(limit))
    {
        return Some(format!(
            "Agent exceeded its token budget ({total_tokens} tokens)"
        ));
    }
    if budget
        .max_tool_calls
        .is_some_and(|limit| usage.tool_calls > u64::from(limit))
    {
        return Some(format!(
            "Agent exceeded its tool-call budget ({} calls)",
            usage.tool_calls
        ));
    }
    if budget
        .max_cost_microunits
        .is_some_and(|limit| usage.cost_microunits > limit)
    {
        return Some(format!(
            "Agent exceeded its cost budget ({} microunits)",
            usage.cost_microunits
        ));
    }
    None
}

/// Explicit placeholder for runtimes that persist a workflow before a backend
/// adapter is configured. It fails loudly instead of pretending work ran.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredAgentDelegator;

#[async_trait]
impl AgentDelegator for UnconfiguredAgentDelegator {
    async fn delegate_validated(
        &self,
        _request: ValidatedAgentDelegationRequest,
    ) -> anyhow::Result<AgentDelegationResponse> {
        anyhow::bail!("agent delegation backend is not configured")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_spec(agent_id: &str) -> AgentSpec {
        AgentSpec {
            agent_id: agent_id.into(),
            name: "Test Agent".into(),
            goal: "Complete the test task".into(),
            context_summary: String::new(),
            inputs: vec![],
            acceptance_criteria: vec![],
            dependencies: vec![],
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
            requires_review: false,
            session_policy: AgentSessionPolicy::New,
            allow_delegation: false,
            origin: AgentOrigin::Temporary,
            capabilities: vec![],
            executor: None,
            request_preferences: None,
            workspace_policy: None,
            output_schema_source: AgentOutputSchemaSource::Standard,
            approval_reasons: vec![],
            authorization: None,
        }
    }

    #[test]
    fn agent_spec_has_stable_json_shape() {
        let spec = AgentSpec {
            backend: AgentBackend::Acp,
            model: Some("reasoning-model".into()),
            prompt_template: "Review {{input}}".into(),
            input_contract: serde_json::json!({"type":"object"}),
            output_contract: serde_json::json!({"type":"object"}),
            permissions: PermissionSet {
                tools: vec!["read_file".into()],
                paths: vec!["src/**".into()],
                network: false,
                write: false,
                execute: false,
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
            ..test_spec("reviewer")
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
            role: AgentRole::Custom("specialist".into()),
            timeout_secs: Some(0),
            ..test_spec("a")
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    fn permissions_and_context_are_intersected() {
        let requested = PermissionSet {
            tools: vec!["read".into(), "write".into()],
            paths: vec!["src/**".into(), "tmp/**".into()],
            network: true,
            write: true,
            execute: true,
        };
        let granted = PermissionSet {
            tools: vec!["read".into()],
            paths: vec!["src/**".into()],
            network: false,
            write: false,
            execute: false,
        };
        assert_eq!(requested.intersect(&granted).tools, vec!["read"]);
        assert!(!requested.intersect(&granted).network);
        assert!(!requested.intersect(&granted).execute);
        assert_eq!(
            AgentBudget {
                max_tokens: Some(2_000),
                ..AgentBudget::default()
            }
            .restrict(&AgentBudget {
                max_tokens: Some(1_000),
                ..AgentBudget::default()
            })
            .max_tokens,
            Some(1_000)
        );

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
                ..test_spec("specialist")
            },
            input: serde_json::json!({"diff":"..."}),
            lineage: None,
        };
        request.validate().unwrap();
        assert!(AgentDelegationResponse {
            request_id: "r1".into(),
            status: DelegationStatus::Failed,
            output: Value::Null,
            artifact_ids: vec![],
            artifacts: vec![],
            evidence: vec![],
            usage: AgentUsage::default(),
            agent_session_id: None,
            child_frame_id: None,
            error: None,
            nested_results: vec![],
        }
        .validate()
        .is_err());
    }

    #[test]
    fn malformed_contract_type_is_rejected() {
        let mut spec = AgentSpec {
            input_contract: serde_json::json!({"type": ["object"]}),
            ..test_spec("a")
        };
        assert!(spec.validate().is_err());
        spec.input_contract = serde_json::json!({"type":"object"});
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn json_contract_checks_nested_required_properties_and_arrays() {
        let contract = serde_json::json!({
            "type": "object",
            "required": ["label", "scores"],
            "properties": {
                "label": {"type": "string", "minLength": 2},
                "scores": {
                    "type": "array",
                    "minItems": 1,
                    "items": {"type": "number", "minimum": 0.0, "maximum": 1.0}
                }
            },
            "additionalProperties": false
        });
        assert!(matches_json_contract(
            &serde_json::json!({"label": "ok", "scores": [0.2, 1.0]}),
            &contract
        ));
        assert!(!matches_json_contract(
            &serde_json::json!({"label": "x", "scores": [1.2]}),
            &contract
        ));
        assert!(!matches_json_contract(
            &serde_json::json!({"label": "ok", "scores": [], "extra": true}),
            &contract
        ));
    }

    #[test]
    fn task_data_is_wrapped_in_the_standard_parent_envelope() {
        let response = AgentDelegationResponse {
            request_id: "request".into(),
            status: DelegationStatus::Succeeded,
            output: serde_json::json!({}),
            artifact_ids: vec![],
            artifacts: vec![AgentArtifact {
                id: "artifact".into(),
                ..Default::default()
            }],
            evidence: vec![AgentEvidence {
                kind: "test".into(),
                summary: "verified".into(),
                reference: None,
            }],
            usage: AgentUsage::default(),
            agent_session_id: None,
            child_frame_id: None,
            error: None,
            nested_results: vec![],
        };

        let envelope = task_output_envelope(
            serde_json::json!({"summary": "measured", "value": 3}),
            &response,
        );

        assert_eq!(envelope["summary"], "measured");
        assert_eq!(envelope["data"]["value"], 3);
        assert_eq!(envelope["artifacts"][0]["id"], "artifact");
        assert_eq!(envelope["evidence"][0]["summary"], "verified");
    }

    #[test]
    fn dynamic_metadata_requires_bounded_capabilities_and_executor() {
        let mut spec = AgentSpec {
            origin: AgentOrigin::Temporary,
            capabilities: vec!["project_read".into()],
            executor: Some(AgentExecutorRef::Native),
            model: Some("test-model".into()),
            workspace_policy: Some(AgentWorkspacePolicy::SharedReadOnly),
            ..test_spec("temporary")
        };
        spec.validate_dynamic_metadata().unwrap();

        spec.capabilities.push("project_read".into());
        assert!(spec.validate_dynamic_metadata().is_err());
        spec.capabilities = vec!["Project Read".into()];
        assert!(spec.validate_dynamic_metadata().is_err());
        spec.capabilities = vec!["project_read".into()];
        spec.executor = Some(AgentExecutorRef::Acp {
            profile_id: String::new(),
        });
        assert!(spec.validate_dynamic_metadata().is_err());
    }

    #[test]
    fn oversized_output_contract_is_rejected() {
        let spec = AgentSpec {
            output_contract: serde_json::json!({
                "type": "object",
                "description": "x".repeat(MAX_AGENT_OUTPUT_SCHEMA_BYTES),
            }),
            origin: AgentOrigin::Temporary,
            capabilities: vec!["project_read".into()],
            executor: Some(AgentExecutorRef::Native),
            model: Some("test-model".into()),
            workspace_policy: Some(AgentWorkspacePolicy::SharedReadOnly),
            ..test_spec("temporary")
        };
        assert!(spec.validate().is_ok());
        assert!(spec.validate_dynamic_metadata().is_err());
    }
}
