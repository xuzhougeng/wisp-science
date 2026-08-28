//! Durable coordinator for one `Run(kind="method_search")`.

use crate::method_search::{
    validate_execution, EvaluationRequest, LocalPythonMethodSearchEvaluator,
    MethodSearchAuditReport, MethodSearchEvaluator,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::Duration;
use wisp_core::method_search::{
    default_strategy_cards, normalized_strategy_reward, replace_python_symbol,
    select_diverse_top_k, select_strategy_index, update_strategy_weight, EvaluatorResult,
    MethodCandidateRank, MethodSearchSpec, MethodStrategyCard, MAX_CANDIDATE_SOURCE_BYTES,
};
use wisp_store::{
    logical_artifact_id, AgentWorkflowAttemptStatus, AgentWorkflowRunActivity,
    ArtifactCaptureTiming, ArtifactMaterialization, ArtifactVersionContext, ArtifactVersionDraft,
    ExecutionContext, ExecutionContextKind, LineageBasis, LineageConfidence, MethodCandidate,
    MethodCandidateBlob, MethodCandidateStatus, MethodSearchRunState, MethodStrategyStat,
    ResearchEdge, ResearchNode, ResearchNodeKind, RunCodeSnapshot, RunInput, RunOutput, RunRecord,
    RunStatus, Store,
};

const LEASE_SECONDS: i64 = 420;
const GENERATOR_TIMEOUT_SECONDS: u64 = 180;
const MAX_GENERATOR_RESPONSE_BYTES: usize = MAX_CANDIDATE_SOURCE_BYTES + 16 * 1024;
const DIVERSITY_FLOOR: f64 = 0.25;
const TOP_K: usize = 3;

fn active_method_searches() -> &'static StdMutex<std::collections::HashSet<String>> {
    static ACTIVE: OnceLock<StdMutex<std::collections::HashSet<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| StdMutex::new(std::collections::HashSet::new()))
}

pub(crate) fn method_search_is_active(run_id: &str) -> bool {
    active_method_searches().lock().unwrap().contains(run_id)
}

pub(crate) struct ActiveMethodSearchGuard {
    run_id: String,
}

impl ActiveMethodSearchGuard {
    pub(crate) fn claim(run_id: &str) -> Result<Self, String> {
        if !active_method_searches()
            .lock()
            .unwrap()
            .insert(run_id.to_string())
        {
            return Err("Method-search Run already has an in-process coordinator".into());
        }
        Ok(Self {
            run_id: run_id.into(),
        })
    }
}

impl Drop for ActiveMethodSearchGuard {
    fn drop(&mut self) {
        active_method_searches()
            .lock()
            .unwrap()
            .remove(&self.run_id);
    }
}

pub(crate) struct StoreWorkflowRunActivityDriver {
    store: Store,
    project_id: String,
    project_root: PathBuf,
}

impl StoreWorkflowRunActivityDriver {
    pub(crate) fn new(store: Store, project_id: String, project_root: PathBuf) -> Self {
        Self {
            store,
            project_id,
            project_root,
        }
    }

    fn spec_version_id(request: &wisp_core::WorkflowRunActivityRequest) -> Result<String, String> {
        request
            .input
            .get("dependency_results")
            .and_then(|value| value.get(&request.activity.input_task_id))
            .and_then(|value| value.get(&request.activity.spec_output_pointer))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                "Run activity dependency omitted method_search_spec_artifact_version_id".into()
            })
    }

    async fn execute_inner(
        &self,
        request: &wisp_core::WorkflowRunActivityRequest,
    ) -> Result<wisp_core::AgentDelegationResponse, String> {
        request
            .activity
            .validate()
            .map_err(|error| error.to_string())?;
        if request.activity.activity != "method_search" {
            return Err("Unsupported Workflow Run activity".into());
        }
        let attempt = self
            .store
            .get_agent_workflow_attempt_by_request_id(&request.request_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Workflow Run activity attempt is not persisted".to_string())?;
        if attempt.workflow_id != request.workflow_id
            || attempt.step_id != request.step_id
            || attempt.status != AgentWorkflowAttemptStatus::Running
        {
            return Err("Workflow Run activity attempt is not the active approved attempt".into());
        }
        let workflow = self
            .store
            .get_agent_workflow(&request.workflow_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Workflow Run activity owner disappeared".to_string())?;
        if workflow.project_id != self.project_id {
            return Err("Workflow Run activity crossed project ownership".into());
        }
        let frame_id = workflow
            .frame_id
            .clone()
            .ok_or_else(|| "Workflow Run activity requires an evidence frame".to_string())?;
        let context = self
            .store
            .get_execution_context(&request.activity.context_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Approved method-search ExecutionContext disappeared".to_string())?;
        if context.kind != ExecutionContextKind::Local {
            return Err("Method-search v0 supports only the local ExecutionContext".into());
        }
        let context_value = serde_json::to_value(&context).map_err(|error| error.to_string())?;
        let (_, context_revision) = wisp_store::canonical_json_sha256(&context_value);
        if context_revision != request.activity.context_revision {
            return Err("Approved method-search ExecutionContext revision changed".into());
        }
        let spec_version_id = Self::spec_version_id(request)?;
        let (spec_context, spec_bytes) = load_artifact_bytes(
            &self.store,
            &self.project_root,
            &self.project_id,
            &spec_version_id,
            512 * 1024,
        )
        .await?;
        let spec: MethodSearchSpec =
            serde_json::from_slice(&spec_bytes).map_err(|error| error.to_string())?;
        spec.validate().map_err(|error| error.to_string())?;
        if spec.budget.max_candidates != request.activity.max_candidates
            || spec.budget.max_wall_seconds != request.activity.max_wall_seconds
            || spec.budget.max_evaluator_seconds != request.activity.max_evaluator_seconds
            || spec.budget.max_cost_microunits != request.activity.max_cost_microunits
        {
            return Err(
                "Frozen method-search budget does not match approved activity authority".into(),
            );
        }
        let spec_sha256 = spec_context
            .version
            .checksum
            .clone()
            .ok_or_else(|| "Method-search spec ArtifactVersion has no checksum".to_string())?;
        if sha256(&spec_bytes) != spec_sha256 {
            return Err("Method-search spec ArtifactVersion checksum mismatch".into());
        }
        let model_profile_id = request
            .activity
            .model_profile_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Method-search activity has no approved model profile".to_string())?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let state = method_state(&run_id, &spec_version_id, &spec_sha256)?;
        let (_, audit, audit_version_id) = load_method_search_review_contract(
            &self.store,
            &self.project_root,
            &self.project_id,
            &state,
        )
        .await?;
        let mut run = wisp_store::RunRecord::new(
            &run_id,
            &self.project_id,
            &request.activity.context_id,
            "Develop computational method",
            "method_search",
        );
        run.frame_id = Some(frame_id);
        run.timeout_secs = i64::try_from(request.activity.max_wall_seconds).ok();
        run.input_refs_json = serde_json::json!([
            {
                "role": "spec",
                "artifact_version_id": spec_version_id,
                "sha256": spec_sha256
            },
            {
                "role": "audit",
                "artifact_version_id": audit_version_id
            }
        ])
        .to_string();
        run.output_specs_json = serde_json::json!([
            "selected_method",
            "candidate_history",
            "verification_report"
        ])
        .to_string();
        run.progress_json = serde_json::json!({
            "schema": "wisp.method-search-progress.v1",
            "phase": "awaiting_approval",
            "baseline_primary": audit.baseline.median_primary,
            "best_primary": audit.baseline.median_primary,
            "candidate_count": 0,
            "successful_count": 0,
            "failed_count": 0,
            "cost_microunits": 0,
            "current_strategy": null,
            "last_checkpoint_at": chrono::Utc::now().timestamp(),
            "best_candidate_id": null
        })
        .to_string();
        run.env_snapshot_json = serde_json::to_string(&context_value).map_err(|e| e.to_string())?;
        let link = activity_link(
            &attempt.id,
            &run_id,
            linked_activity_state(&spec_version_id, &spec_sha256, model_profile_id),
        )?;
        self.store
            .create_method_search_workflow_run_activity(&run, &link, &state)
            .await
            .map_err(|error| error.to_string())?;
        if let Err(error) =
            bind_method_search_run_inputs(&self.store, &self.project_root, &run_id).await
        {
            let _ = self.store.fail_method_search_run(&run_id, &error).await;
            return terminal_activity_response(&self.store, &request.request_id, &run_id).await;
        }
        let _active_guard = ActiveMethodSearchGuard::claim(&run_id)?;
        loop {
            let status = self
                .store
                .method_search_run_status(&run_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Linked method-search Run disappeared".to_string())?;
            match status {
                RunStatus::Draft => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                RunStatus::Submitted | RunStatus::Running => break,
                RunStatus::Cancelling => {
                    finish_cancelling_method_search(&self.store, &run_id).await?;
                    return terminal_activity_response(&self.store, &request.request_id, &run_id)
                        .await;
                }
                status if status.is_terminal() => {
                    return terminal_activity_response(&self.store, &request.request_id, &run_id)
                        .await;
                }
                _ => return Err("Prepared method-search Run entered an invalid state".into()),
            }
        }
        let generator =
            match ProviderCandidateGenerator::from_profile(&self.store, model_profile_id).await {
                Ok(generator) => generator,
                Err(error) => {
                    let _ = self.store.fail_method_search_run(&run_id, &error).await;
                    return terminal_activity_response(&self.store, &request.request_id, &run_id)
                        .await;
                }
            };
        let evaluator = local_evaluator();
        loop {
            let status = self
                .store
                .method_search_run_status(&run_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Linked method-search Run disappeared".to_string())?;
            if status.is_terminal() {
                return terminal_activity_response(&self.store, &request.request_id, &run_id).await;
            }
            if status == RunStatus::Paused {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            match run_method_search_coordinator(
                &self.store,
                &self.project_root,
                &run_id,
                &generator,
                &evaluator,
            )
            .await
            {
                Ok(CoordinatorOutcome::Paused) => continue,
                Ok(CoordinatorOutcome::Terminal) => {
                    return terminal_activity_response(&self.store, &request.request_id, &run_id)
                        .await
                }
                Err(error) => {
                    let _ = self.store.fail_method_search_run(&run_id, &error).await;
                    return terminal_activity_response(&self.store, &request.request_id, &run_id)
                        .await;
                }
            }
        }
    }
}

#[async_trait]
impl wisp_core::WorkflowRunActivityDriver for StoreWorkflowRunActivityDriver {
    async fn execute(
        &self,
        request: wisp_core::WorkflowRunActivityRequest,
    ) -> anyhow::Result<wisp_core::AgentDelegationResponse> {
        self.execute_inner(&request)
            .await
            .map_err(anyhow::Error::msg)
    }

    async fn cancel(&self, request_id: &str) -> anyhow::Result<bool> {
        let Some(attempt) = self
            .store
            .get_agent_workflow_attempt_by_request_id(request_id)
            .await?
        else {
            return Ok(false);
        };
        let Some(link) = self
            .store
            .get_agent_workflow_run_activity(&attempt.id)
            .await?
        else {
            return Ok(false);
        };
        self.store.request_run_cancellation(&link.run_id).await
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CandidateGenerationRequest {
    objective: String,
    constraints: Vec<String>,
    target_symbol: String,
    target_source: String,
    strategy: MethodStrategyCard,
    parent_metrics: serde_json::Value,
    recent_feedback: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateWireResponse {
    replacement: String,
    rationale: String,
    family: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateGenerationResponse {
    pub replacement: String,
    pub rationale: String,
    pub family: String,
    pub cost_microunits: u64,
}

#[async_trait]
pub(crate) trait CandidateGenerator: Send + Sync {
    async fn propose(
        &self,
        request: CandidateGenerationRequest,
    ) -> Result<CandidateGenerationResponse, String>;
}

pub(crate) struct ProviderCandidateGenerator {
    provider: Arc<dyn wisp_llm::Provider>,
}

impl ProviderCandidateGenerator {
    pub(crate) async fn from_profile(store: &Store, profile_id: &str) -> Result<Self, String> {
        let (provider, api_url, model, api_key, max_tokens, reasoning_effort, service_tier) =
            crate::models::profile_llm(store, profile_id)
                .await
                .ok_or_else(|| {
                    format!("Method-search model profile '{profile_id}' is unavailable")
                })?;
        let config = crate::build_provider_config(
            &provider,
            &api_url,
            &api_key,
            &model,
            max_tokens.min(8_192),
            &reasoning_effort,
            &service_tier,
        )?;
        Ok(Self {
            provider: Arc::from(wisp_llm::build(config)),
        })
    }
}

#[async_trait]
impl CandidateGenerator for ProviderCandidateGenerator {
    async fn propose(
        &self,
        request: CandidateGenerationRequest,
    ) -> Result<CandidateGenerationResponse, String> {
        if request.target_source.len() > MAX_CANDIDATE_SOURCE_BYTES {
            return Err("Candidate parent source exceeds the bounded prompt limit".into());
        }
        let payload = serde_json::to_string(&request).map_err(|error| error.to_string())?;
        let system = "You improve one declared Python symbol for a frozen scientific evaluator. Return only one JSON object with exactly replacement, rationale, and family. replacement must contain only the complete target function/class with the identical header. Do not return markdown, commands, imports outside the symbol, paths, evaluator changes, data changes, or explanations outside JSON.";
        let completion = tokio::time::timeout(
            Duration::from_secs(GENERATOR_TIMEOUT_SECONDS),
            self.provider.complete(
                &[
                    wisp_llm::Message::system(system),
                    wisp_llm::Message::user(payload),
                ],
                &[],
            ),
        )
        .await
        .map_err(|_| "Candidate generation timed out".to_string())?
        .map_err(|error| error.to_string())?;
        if completion.content.len() > MAX_GENERATOR_RESPONSE_BYTES {
            return Err("Candidate generator response exceeds the bounded limit".into());
        }
        let wire: CandidateWireResponse = serde_json::from_str(completion.content.trim())
            .map_err(|error| format!("Candidate generator returned invalid exact JSON: {error}"))?;
        if wire.replacement.is_empty()
            || wire.replacement.len() > MAX_CANDIDATE_SOURCE_BYTES
            || wire.rationale.trim().is_empty()
            || wire.rationale.chars().count() > 4_000
            || wire.family.trim().is_empty()
            || wire.family.chars().count() > 128
        {
            return Err("Candidate generator response is outside bounded fields".into());
        }
        Ok(CandidateGenerationResponse {
            replacement: wire.replacement,
            rationale: wire.rationale.trim().into(),
            family: wire.family.trim().into(),
            // Provider pricing is profile-specific. V0 uses a deterministic
            // token-denominated microunit reservation rather than inventing a
            // monetary price; the exact usage remains visible in history.
            cost_microunits: completion
                .usage
                .input_tokens
                .saturating_add(completion.usage.output_tokens)
                .max(1),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MethodSearchCheckpoint {
    schema: String,
    next_sequence: u32,
    cost_microunits: u64,
    best_candidate_id: Option<String>,
    terminal_reason: Option<String>,
    updated_at: i64,
}

impl Default for MethodSearchCheckpoint {
    fn default() -> Self {
        Self {
            schema: "wisp.method-search-checkpoint.v1".into(),
            next_sequence: 1,
            cost_microunits: 0,
            best_candidate_id: None,
            terminal_reason: None,
            updated_at: chrono::Utc::now().timestamp(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MethodSearchProgress {
    schema: String,
    phase: String,
    baseline_primary: f64,
    best_primary: f64,
    candidate_count: usize,
    successful_count: usize,
    failed_count: usize,
    cost_microunits: u64,
    current_strategy: Option<String>,
    last_checkpoint_at: i64,
    best_candidate_id: Option<String>,
}

#[derive(Debug, Clone)]
struct FrozenFile {
    path: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct FrozenContract {
    spec: MethodSearchSpec,
    audit: MethodSearchAuditReport,
    target_source: String,
    evaluator: FrozenFile,
    inputs: Vec<FrozenFile>,
    final_verification: Option<FrozenFile>,
    python_executable: Option<String>,
}

fn configured_python(context: &ExecutionContext) -> Result<Option<String>, String> {
    for (label, json) in [
        ("ExecutionContext config", &context.config_json),
        ("ExecutionContext capabilities", &context.capabilities_json),
    ] {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|error| format!("Invalid {label}: {error}"))?;
        let object = value
            .as_object()
            .ok_or_else(|| format!("{label} must be a JSON object"))?;
        for key in ["python_executable", "python_path"] {
            if let Some(value) = object.get(key) {
                let value = value
                    .as_str()
                    .ok_or_else(|| format!("{label}.{key} must be a string"))?
                    .trim();
                if !value.is_empty() {
                    return Ok(Some(value.to_string()));
                }
            }
        }
    }
    Ok(None)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn method_search_research_id(kind: &str, run_id: &str, value: &str) -> String {
    let digest = sha256(value.as_bytes());
    format!("{kind}:method-search:{run_id}:{}", &digest[..16])
}

fn exact_artifact_path(root: &Path, context: &ArtifactVersionContext) -> Result<PathBuf, String> {
    if context.version.materialization != ArtifactMaterialization::Snapshot {
        return Err(format!(
            "ArtifactVersion '{}' is not an immutable snapshot",
            context.version.id
        ));
    }
    let path = wisp_tools::safety::validate_file_path(root, &context.version.storage_path)?;
    if !path.is_file() {
        return Err(format!(
            "ArtifactVersion '{}' snapshot bytes are missing",
            context.version.id
        ));
    }
    Ok(path)
}

async fn load_artifact_bytes(
    store: &Store,
    root: &Path,
    project_id: &str,
    version_id: &str,
    max_bytes: usize,
) -> Result<(ArtifactVersionContext, Vec<u8>), String> {
    let context = store
        .get_artifact_version_context(version_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("ArtifactVersion '{version_id}' does not exist"))?;
    if context.project_id != project_id {
        return Err(format!(
            "ArtifactVersion '{version_id}' belongs to another project"
        ));
    }
    let path = exact_artifact_path(root, &context)?;
    let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    if metadata.len() > max_bytes as u64 {
        return Err(format!(
            "ArtifactVersion '{version_id}' exceeds the bounded input size"
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let checksum = sha256(&bytes);
    if context.version.checksum.as_deref() != Some(checksum.as_str())
        || context.version.size_bytes != i64::try_from(bytes.len()).ok()
    {
        return Err(format!(
            "ArtifactVersion '{version_id}' no longer matches its checksum"
        ));
    }
    Ok((context, bytes))
}

async fn load_frozen_contract(
    store: &Store,
    root: &Path,
    run: &RunRecord,
    state: &MethodSearchRunState,
) -> Result<FrozenContract, String> {
    let project_id = &run.project_id;
    let frozen_context: ExecutionContext = serde_json::from_str(&run.env_snapshot_json)
        .map_err(|error| format!("Invalid frozen method-search ExecutionContext: {error}"))?;
    if frozen_context.id != run.context_id || frozen_context.kind != ExecutionContextKind::Local {
        return Err("Frozen method-search ExecutionContext does not match the local Run".into());
    }
    let python_executable = configured_python(&frozen_context)?;
    let (_, spec_bytes) = load_artifact_bytes(
        store,
        root,
        project_id,
        &state.spec_artifact_version_id,
        512 * 1024,
    )
    .await?;
    if sha256(&spec_bytes) != state.spec_sha256 {
        return Err("Method-search specification hash changed".into());
    }
    let spec: MethodSearchSpec =
        serde_json::from_slice(&spec_bytes).map_err(|error| error.to_string())?;
    spec.validate().map_err(|error| error.to_string())?;
    let audit_id = store
        .list_artifact_dependencies(&state.spec_artifact_version_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|dependency| dependency.reference_name.as_deref() == Some("method_search_audit"))
        .map(|dependency| dependency.depends_on_version_id)
        .ok_or_else(|| "Method-search spec has no exact audit dependency".to_string())?;
    let (_, audit_bytes) =
        load_artifact_bytes(store, root, project_id, &audit_id, 512 * 1024).await?;
    let audit: MethodSearchAuditReport =
        serde_json::from_slice(&audit_bytes).map_err(|error| error.to_string())?;
    if !audit.sentinel_reachable
        || audit.baseline.successful_repetitions != spec.evaluator.repetitions
    {
        return Err("Method-search audit is incomplete or incompatible with the spec".into());
    }
    let (_, target_bytes) = load_artifact_bytes(
        store,
        root,
        project_id,
        &spec.target.source_artifact_version_id,
        MAX_CANDIDATE_SOURCE_BYTES,
    )
    .await?;
    let target_source =
        String::from_utf8(target_bytes).map_err(|_| "Frozen target is not UTF-8 Python")?;
    if sha256(target_source.as_bytes()) != audit.target_source_sha256 {
        return Err("Frozen target does not match its evaluator audit".into());
    }
    let (_, evaluator_bytes) = load_artifact_bytes(
        store,
        root,
        project_id,
        &spec.evaluator.artifact_version_id,
        MAX_CANDIDATE_SOURCE_BYTES,
    )
    .await?;
    let evaluator = FrozenFile {
        path: spec.evaluator.entry_path.clone(),
        bytes: evaluator_bytes,
    };
    let mut inputs = Vec::with_capacity(spec.inputs.len());
    for input in &spec.inputs {
        let version_id = input.artifact_version_id.as_deref().ok_or_else(|| {
            "Local v0 coordinator cannot materialize an external-only input".to_string()
        })?;
        let (_, bytes) = load_artifact_bytes(
            store,
            root,
            project_id,
            version_id,
            crate::snapshot_store::DEFAULT_SNAPSHOT_LIMIT as usize,
        )
        .await?;
        if sha256(&bytes) != input.checksum {
            return Err(format!("Frozen input '{}' checksum mismatch", input.role));
        }
        inputs.push(FrozenFile {
            path: input.path.clone(),
            bytes,
        });
    }
    let final_verification = if let Some(final_spec) = &spec.final_verification {
        let (_, bytes) = load_artifact_bytes(
            store,
            root,
            project_id,
            &final_spec.artifact_version_id,
            crate::snapshot_store::DEFAULT_SNAPSHOT_LIMIT as usize,
        )
        .await?;
        Some(FrozenFile {
            path: final_spec.path.clone(),
            bytes,
        })
    } else {
        None
    };
    let mut required_protected = vec![evaluator.path.clone()];
    required_protected.extend(inputs.iter().map(|input| input.path.clone()));
    required_protected.extend(final_verification.iter().map(|input| input.path.clone()));
    required_protected.sort();
    let mut declared_protected = spec.protected_paths.clone();
    declared_protected.sort();
    if declared_protected != required_protected {
        return Err("Method-search protected path set is not exact".into());
    }
    Ok(FrozenContract {
        spec,
        audit,
        target_source,
        evaluator,
        inputs,
        final_verification,
        python_executable,
    })
}

pub(crate) async fn load_method_search_review_contract(
    store: &Store,
    root: &Path,
    project_id: &str,
    state: &MethodSearchRunState,
) -> Result<(MethodSearchSpec, MethodSearchAuditReport, String), String> {
    let (_, spec_bytes) = load_artifact_bytes(
        store,
        root,
        project_id,
        &state.spec_artifact_version_id,
        512 * 1024,
    )
    .await?;
    if sha256(&spec_bytes) != state.spec_sha256 {
        return Err("Method-search specification hash changed".into());
    }
    let spec: MethodSearchSpec =
        serde_json::from_slice(&spec_bytes).map_err(|error| error.to_string())?;
    spec.validate().map_err(|error| error.to_string())?;
    let audit_id = store
        .list_artifact_dependencies(&state.spec_artifact_version_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|dependency| dependency.reference_name.as_deref() == Some("method_search_audit"))
        .map(|dependency| dependency.depends_on_version_id)
        .ok_or_else(|| "Method-search spec has no exact audit dependency".to_string())?;
    let (_, audit_bytes) =
        load_artifact_bytes(store, root, project_id, &audit_id, 512 * 1024).await?;
    let audit: MethodSearchAuditReport =
        serde_json::from_slice(&audit_bytes).map_err(|error| error.to_string())?;
    if !audit.sentinel_reachable
        || audit.baseline.successful_repetitions != spec.evaluator.repetitions
    {
        return Err("Method-search audit is incomplete or incompatible with the spec".into());
    }
    Ok((spec, audit, audit_id))
}

struct CandidateWorkspace(PathBuf);

impl Drop for CandidateWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_workspace_file(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), String> {
    let destination = root.join(relative);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(destination, bytes).map_err(|error| error.to_string())
}

fn workspace_manifest(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut manifest = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.file_type().is_symlink() {
            return Err("Candidate workspace contains a symlink".into());
        }
        if entry.file_type().is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            manifest.insert(
                relative,
                sha256(&std::fs::read(entry.path()).map_err(|e| e.to_string())?),
            );
        }
    }
    Ok(manifest)
}

fn build_candidate_workspace(
    project_root: &Path,
    run_id: &str,
    contract: &FrozenContract,
    source: &str,
) -> Result<CandidateWorkspace, String> {
    let root = project_root
        .join(".wisp")
        .join("method-search")
        .join("work")
        .join(run_id)
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let workspace = CandidateWorkspace(root);
    write_workspace_file(
        &workspace.0,
        &contract.spec.target.source_path,
        source.as_bytes(),
    )?;
    write_workspace_file(
        &workspace.0,
        &contract.evaluator.path,
        &contract.evaluator.bytes,
    )?;
    for input in &contract.inputs {
        write_workspace_file(&workspace.0, &input.path, &input.bytes)?;
    }
    if let Some(input) = &contract.final_verification {
        write_workspace_file(&workspace.0, &input.path, &input.bytes)?;
    }
    Ok(workspace)
}

async fn evaluate_source(
    project_root: &Path,
    run_id: &str,
    contract: &FrozenContract,
    source: &str,
    phase: &str,
    evaluator: &dyn MethodSearchEvaluator,
) -> Result<EvaluatorResult, String> {
    let workspace = build_candidate_workspace(project_root, run_id, contract, source)?;
    let before = workspace_manifest(&workspace.0)?;
    let execution = evaluator
        .evaluate(&EvaluationRequest {
            workspace: workspace.0.clone(),
            evaluator_path: contract.spec.evaluator.entry_path.clone(),
            target_path: contract.spec.target.source_path.clone(),
            timeout: Duration::from_secs(contract.spec.budget.max_evaluator_seconds),
            python_executable: contract.python_executable.clone(),
            phase: phase.into(),
            final_verification_path: contract
                .final_verification
                .as_ref()
                .map(|input| input.path.clone()),
        })
        .await?;
    let after = workspace_manifest(&workspace.0)?;
    if after != before {
        return Err(
            "Evaluator changed or created files in the isolated candidate workspace".into(),
        );
    }
    validate_execution(&execution, &contract.spec)
}

fn secure_blob_directory(root: &Path, components: &[&str]) -> Result<PathBuf, String> {
    let mut current = root.to_path_buf();
    for component in components {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("Method-search blob storage is not a safe directory".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(current)
}

async fn save_candidate_blob(
    store: &Store,
    project_root: &Path,
    run_id: &str,
    kind: &str,
    bytes: &[u8],
) -> Result<MethodCandidateBlob, String> {
    if bytes.len() > MAX_CANDIDATE_SOURCE_BYTES {
        return Err("Method candidate blob exceeds the v0 size limit".into());
    }
    let checksum = sha256(bytes);
    if let Some(blob) = store
        .find_method_candidate_blob(run_id, kind, &checksum)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(blob);
    }
    let directory = secure_blob_directory(
        project_root,
        &[".wisp", "method-search", "blobs", "sha256", &checksum[..2]],
    )?;
    let path = directory.join(format!("{checksum}.{kind}"));
    if path.exists() {
        if sha256(&std::fs::read(&path).map_err(|error| error.to_string())?) != checksum {
            return Err("Method candidate blob checksum collision".into());
        }
    } else {
        let temp = directory.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        std::fs::rename(&temp, &path).map_err(|error| error.to_string())?;
    }
    let root = dunce::canonicalize(project_root).map_err(|error| error.to_string())?;
    let canonical_path = dunce::canonicalize(&path).map_err(|error| error.to_string())?;
    let storage_path = canonical_path
        .strip_prefix(&root)
        .map_err(|_| "Candidate blob escaped the project".to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let blob = MethodCandidateBlob {
        id: uuid::Uuid::new_v4().to_string(),
        run_id: run_id.into(),
        kind: kind.into(),
        checksum,
        size_bytes: i64::try_from(bytes.len()).map_err(|_| "Candidate blob is too large")?,
        storage_path,
        created_at: chrono::Utc::now().timestamp(),
    };
    store
        .save_method_candidate_blob(&blob)
        .await
        .map_err(|error| error.to_string())?;
    Ok(blob)
}

fn read_candidate_blob(project_root: &Path, blob: &MethodCandidateBlob) -> Result<Vec<u8>, String> {
    let path = wisp_tools::safety::validate_file_path(project_root, &blob.storage_path)?;
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() != blob.size_bytes as usize || sha256(&bytes) != blob.checksum {
        return Err(format!("Candidate blob '{}' is corrupt", blob.id));
    }
    Ok(bytes)
}

fn changed_lines(baseline: &str, candidate: &str) -> i64 {
    let left = baseline.lines().collect::<Vec<_>>();
    let right = candidate.lines().collect::<Vec<_>>();
    let changed = left
        .iter()
        .zip(&right)
        .filter(|(left, right)| left != right)
        .count()
        + left.len().abs_diff(right.len());
    i64::try_from(changed).unwrap_or(i64::MAX)
}

fn dependency_count(source: &str) -> i64 {
    i64::try_from(
        source
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with("import ") || line.starts_with("from ")
            })
            .count(),
    )
    .unwrap_or(i64::MAX)
}

fn checkpoint_from_state(state: &MethodSearchRunState) -> Result<MethodSearchCheckpoint, String> {
    if state.checkpoint_json.trim() == "{}" {
        return Ok(MethodSearchCheckpoint::default());
    }
    let checkpoint: MethodSearchCheckpoint =
        serde_json::from_str(&state.checkpoint_json).map_err(|error| error.to_string())?;
    if checkpoint.schema != "wisp.method-search-checkpoint.v1"
        || checkpoint.next_sequence == 0
        || checkpoint.next_sequence > 51
    {
        return Err("Method-search checkpoint is incompatible".into());
    }
    Ok(checkpoint)
}

async fn persist_checkpoint(
    store: &Store,
    run_id: &str,
    checkpoint: &mut MethodSearchCheckpoint,
    result_status: Option<&str>,
) -> Result<(), String> {
    checkpoint.updated_at = chrono::Utc::now().timestamp();
    store
        .update_method_search_checkpoint(
            run_id,
            &serde_json::to_string(checkpoint).map_err(|error| error.to_string())?,
            result_status,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn persist_progress(
    store: &Store,
    run_id: &str,
    owner: &str,
    contract: &FrozenContract,
    checkpoint: &MethodSearchCheckpoint,
    current_strategy: Option<String>,
) -> Result<(), String> {
    let candidates = store
        .list_method_candidates(run_id)
        .await
        .map_err(|error| error.to_string())?;
    let successful = candidates
        .iter()
        .filter(|candidate| candidate.status == MethodCandidateStatus::Succeeded)
        .collect::<Vec<_>>();
    let best = successful.iter().copied().max_by(|left, right| {
        left.utility
            .unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&right.utility.unwrap_or(f64::NEG_INFINITY))
    });
    let progress = MethodSearchProgress {
        schema: "wisp.method-search-progress.v1".into(),
        phase: "search".into(),
        baseline_primary: contract.audit.baseline.median_primary,
        best_primary: best
            .and_then(|candidate| candidate.primary_score)
            .unwrap_or(contract.audit.baseline.median_primary),
        candidate_count: candidates
            .iter()
            .filter(|candidate| candidate.sequence > 0)
            .count(),
        successful_count: candidates
            .iter()
            .filter(|candidate| {
                candidate.sequence > 0 && candidate.status == MethodCandidateStatus::Succeeded
            })
            .count(),
        failed_count: candidates
            .iter()
            .filter(|candidate| {
                candidate.sequence > 0 && candidate.status != MethodCandidateStatus::Succeeded
            })
            .count(),
        cost_microunits: checkpoint.cost_microunits,
        current_strategy,
        last_checkpoint_at: checkpoint.updated_at,
        best_candidate_id: best.map(|candidate| candidate.id.clone()),
    };
    let progress_json = serde_json::to_string(&progress).map_err(|error| error.to_string())?;
    if !store
        .update_method_search_progress_owned(run_id, owner, &progress_json)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Method-search coordinator lost its lifecycle lease".into());
    }
    Ok(())
}

async fn initialize_baseline(
    store: &Store,
    project_root: &Path,
    run_id: &str,
    contract: &FrozenContract,
) -> Result<(), String> {
    if store
        .list_method_candidates(run_id)
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .any(|candidate| candidate.sequence == 0)
    {
        return Ok(());
    }
    let source_blob = save_candidate_blob(
        store,
        project_root,
        run_id,
        "source",
        contract.target_source.as_bytes(),
    )
    .await?;
    let patch_blob = save_candidate_blob(store, project_root, run_id, "patch", b"").await?;
    let source_hash = sha256(contract.target_source.as_bytes());
    let mut candidate = MethodCandidate::proposed(
        uuid::Uuid::new_v4().to_string(),
        run_id,
        0,
        "baseline",
        "baseline",
        source_hash,
        patch_blob.checksum.clone(),
    )
    .map_err(|error| error.to_string())?;
    candidate.source_blob_id = Some(source_blob.id);
    candidate.patch_blob_id = Some(patch_blob.id);
    store
        .insert_method_candidate(&candidate)
        .await
        .map_err(|error| error.to_string())?;
    let mut finished = candidate;
    finished.status = MethodCandidateStatus::Succeeded;
    finished.primary_score = Some(contract.audit.baseline.median_primary);
    finished.utility = Some(
        contract
            .spec
            .normalized_utility(contract.audit.baseline.median_primary)
            .map_err(|error| error.to_string())?,
    );
    finished.metrics_json = serde_json::json!({
        contract.spec.metrics.primary.clone(): contract.audit.baseline.median_primary
    })
    .to_string();
    finished.runtime_ms = Some(0);
    finished.changed_lines = Some(0);
    finished.dependency_count = Some(dependency_count(&contract.target_source));
    finished.finished_at = Some(chrono::Utc::now().timestamp());
    if !store
        .finish_method_candidate(&finished, MethodCandidateStatus::Proposed)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Baseline candidate changed concurrently".into());
    }
    Ok(())
}

async fn initialize_strategies(
    store: &Store,
    run_id: &str,
    spec: &MethodSearchSpec,
) -> Result<(), String> {
    if !store
        .list_method_strategy_stats(run_id)
        .await
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Ok(());
    }
    for mut card in default_strategy_cards() {
        let sources = spec
            .strategy_sources
            .iter()
            .filter(|source| source.category == card.category)
            .collect::<Vec<_>>();
        card.source_refs = sources
            .iter()
            .map(|source| source.source_ref.clone())
            .collect();
        if !sources.is_empty() {
            let evidence = sources
                .iter()
                .map(|source| format!("{}: {}", source.title, source.summary))
                .collect::<Vec<_>>()
                .join(" | ");
            card.summary = format!("{} Evidence: {evidence}", card.summary);
        }
        store
            .upsert_method_strategy_stat(&MethodStrategyStat {
                run_id: run_id.into(),
                strategy_key: card.key,
                category: card.category,
                weight: card.weight,
                attempts: 0,
                improvements: 0,
                cumulative_reward: 0.0,
                summary: card.summary,
                source_refs_json: serde_json::to_string(&card.source_refs)
                    .map_err(|error| error.to_string())?,
                updated_at: chrono::Utc::now().timestamp(),
            })
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn cards_from_stats(stats: &[MethodStrategyStat]) -> Vec<MethodStrategyCard> {
    stats
        .iter()
        .map(|stat| MethodStrategyCard {
            key: stat.strategy_key.clone(),
            category: stat.category.clone(),
            family: stat.category.clone(),
            weight: stat.weight,
            summary: stat.summary.clone(),
            source_refs: serde_json::from_str(&stat.source_refs_json).unwrap_or_default(),
        })
        .collect()
}

async fn candidate_source(
    store: &Store,
    project_root: &Path,
    candidate: &MethodCandidate,
) -> Result<String, String> {
    let blob_id = candidate
        .source_blob_id
        .as_deref()
        .ok_or_else(|| format!("Candidate '{}' has no source blob", candidate.id))?;
    let blob = store
        .find_method_candidate_blob(&candidate.run_id, "source", &candidate.source_sha256)
        .await
        .map_err(|error| error.to_string())?
        .filter(|blob| blob.id == blob_id)
        .ok_or_else(|| format!("Candidate '{}' source blob is missing", candidate.id))?;
    String::from_utf8(read_candidate_blob(project_root, &blob)?)
        .map_err(|_| "Candidate source blob is not UTF-8".into())
}

async fn update_strategy(
    store: &Store,
    run_id: &str,
    strategy_key: &str,
    reward: f64,
) -> Result<(), String> {
    let mut stat = store
        .list_method_strategy_stats(run_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|stat| stat.strategy_key == strategy_key)
        .ok_or_else(|| "Selected strategy state disappeared".to_string())?;
    stat.attempts += 1;
    if reward > 0.0 {
        stat.improvements += 1;
    }
    stat.cumulative_reward += reward;
    stat.weight = update_strategy_weight(stat.weight, reward).map_err(|error| error.to_string())?;
    stat.updated_at = chrono::Utc::now().timestamp();
    store
        .upsert_method_strategy_stat(&stat)
        .await
        .map_err(|error| error.to_string())
}

async fn record_failed_candidate(
    store: &Store,
    run_id: &str,
    sequence: u32,
    parent: &MethodCandidate,
    strategy: &MethodStrategyCard,
    error: String,
) -> Result<(), String> {
    let mut candidate = MethodCandidate::proposed(
        uuid::Uuid::new_v4().to_string(),
        run_id,
        i64::from(sequence),
        &strategy.key,
        &strategy.family,
        &parent.source_sha256,
        sha256(b""),
    )
    .map_err(|error| error.to_string())?;
    candidate.parent_candidate_id = Some(parent.id.clone());
    store
        .insert_method_candidate(&candidate)
        .await
        .map_err(|error| error.to_string())?;
    candidate.status = MethodCandidateStatus::Failed;
    candidate.error = Some(error.chars().take(4_000).collect());
    candidate.finished_at = Some(chrono::Utc::now().timestamp());
    if !store
        .finish_method_candidate(&candidate, MethodCandidateStatus::Proposed)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Failed candidate changed concurrently".into());
    }
    Ok(())
}

async fn promote_bytes(
    store: &Store,
    project_root: &Path,
    project_id: &str,
    frame_id: &str,
    run_id: &str,
    role: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<(String, String), String> {
    let temp_dir = project_root
        .join(".wisp")
        .join("method-search")
        .join("outputs")
        .join(run_id);
    std::fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
    let temp = temp_dir.join(format!("{}-{filename}", uuid::Uuid::new_v4()));
    std::fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    let captured = crate::snapshot_store::capture_file(
        project_root,
        &temp,
        crate::snapshot_store::SnapshotPolicy::Always,
    )?;
    let _ = std::fs::remove_file(&temp);
    let logical_key = format!("method-search-run:{run_id}:{role}");
    let artifact_id = logical_artifact_id(project_id, &logical_key);
    let version_id = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id: artifact_id.clone(),
            project_id: project_id.into(),
            root_frame_id: frame_id.into(),
            filename: filename.into(),
            content_type: content_type.into(),
            storage_path: captured.storage_path,
            logical_key: Some(logical_key),
            size_bytes: Some(
                i64::try_from(captured.size_bytes).map_err(|_| "Output is too large")?,
            ),
            checksum: Some(captured.checksum),
            producing_run_id: Some(run_id.into()),
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .map_err(|error| error.to_string())?;
    store
        .save_run_output(&RunOutput {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: run_id.into(),
            artifact_version_id: version_id.clone(),
            role: role.into(),
            logical_output_key: role.into(),
            source_path: filename.into(),
            created_at: chrono::Utc::now().timestamp(),
        })
        .await
        .map_err(|error| error.to_string())?;
    store
        .save_run_artifact_link(
            &format!("run-artifact:{run_id}:{artifact_id}"),
            run_id,
            &artifact_id,
            role,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok((artifact_id, version_id))
}

async fn finalize_search(
    store: &Store,
    project_root: &Path,
    run_id: &str,
    owner: &str,
    contract: &FrozenContract,
    checkpoint: &mut MethodSearchCheckpoint,
    evaluator: &dyn MethodSearchEvaluator,
) -> Result<(), String> {
    let run = store
        .get_run(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search Run disappeared".to_string())?;
    let frame_id = run
        .frame_id
        .as_deref()
        .ok_or_else(|| "Method-search Run has no evidence frame".to_string())?;
    let candidates = store
        .list_method_candidates(run_id)
        .await
        .map_err(|error| error.to_string())?;
    let succeeded = candidates
        .iter()
        .filter(|candidate| candidate.status == MethodCandidateStatus::Succeeded)
        .cloned()
        .collect::<Vec<_>>();
    if succeeded.is_empty() {
        return Err("Method search has no successful baseline or candidate".into());
    }
    let mut sources = HashMap::new();
    let mut ranks = Vec::new();
    for candidate in &succeeded {
        let source = candidate_source(store, project_root, candidate).await?;
        sources.insert(candidate.id.clone(), source.clone());
        ranks.push(MethodCandidateRank {
            id: candidate.id.clone(),
            family: candidate.family.clone(),
            source,
            utility: candidate.utility.unwrap_or(f64::NEG_INFINITY),
            runtime_ms: candidate.runtime_ms.unwrap_or(i64::MAX),
            changed_lines: candidate.changed_lines.unwrap_or(i64::MAX),
            dependency_count: candidate.dependency_count.unwrap_or(i64::MAX),
        });
    }
    let top_ids = select_diverse_top_k(
        &ranks,
        TOP_K.min(ranks.len()),
        contract.audit.baseline.noise_floor,
        DIVERSITY_FLOOR,
    )
    .map_err(|error| error.to_string())?;
    let mut verification = Vec::<serde_json::Value>::new();
    let mut selected_id = top_ids[0].clone();
    if let Some(final_spec) = &contract.spec.final_verification {
        let mut verified_ranks = Vec::new();
        for id in &top_ids {
            let source = &sources[id];
            let mut results = Vec::with_capacity(final_spec.repetitions as usize);
            for _ in 0..final_spec.repetitions {
                results.push(
                    evaluate_source(
                        project_root,
                        run_id,
                        contract,
                        source,
                        "final_verification",
                        evaluator,
                    )
                    .await?,
                );
            }
            if results
                .iter()
                .any(|result| !result.passes_guardrails(&contract.spec))
            {
                continue;
            }
            let mut primary = results
                .iter()
                .map(|result| result.primary)
                .collect::<Vec<_>>();
            primary.sort_by(f64::total_cmp);
            let median = primary[primary.len() / 2];
            verification.push(serde_json::json!({
                "candidate_id": id,
                "repetitions": final_spec.repetitions,
                "primary_values": primary,
                "median_primary": median
            }));
            let original = ranks.iter().find(|rank| rank.id == *id).unwrap();
            let mut rank = original.clone();
            rank.utility = contract
                .spec
                .normalized_utility(median)
                .map_err(|error| error.to_string())?;
            verified_ranks.push(rank);
        }
        if verified_ranks.is_empty() {
            return Err("No finalist passed independent final verification".into());
        }
        selected_id = select_diverse_top_k(
            &verified_ranks,
            1,
            contract.audit.baseline.noise_floor,
            DIVERSITY_FLOOR,
        )
        .map_err(|error| error.to_string())?
        .remove(0);
    }

    let mut top_artifacts = Vec::new();
    for (index, id) in top_ids.iter().enumerate() {
        let (_, version_id) = promote_bytes(
            store,
            project_root,
            &run.project_id,
            frame_id,
            run_id,
            &format!("top_k_{}", index + 1),
            &format!("top-k-{}.py", index + 1),
            "text/x-python",
            sources[id].as_bytes(),
        )
        .await?;
        top_artifacts
            .push(serde_json::json!({"candidate_id": id, "artifact_version_id": version_id}));
    }
    let (selected_artifact_id, selected_version_id) = promote_bytes(
        store,
        project_root,
        &run.project_id,
        frame_id,
        run_id,
        "selected_method",
        "selected-method.py",
        "text/x-python",
        sources[&selected_id].as_bytes(),
    )
    .await?;
    let history = serde_json::to_vec_pretty(&serde_json::json!({
        "schema": "wisp.method-search-history.v1",
        "run_id": run_id,
        "candidates": candidates,
        "strategies": store.list_method_strategy_stats(run_id).await.map_err(|e| e.to_string())?
    }))
    .map_err(|error| error.to_string())?;
    let (_, history_version_id) = promote_bytes(
        store,
        project_root,
        &run.project_id,
        frame_id,
        run_id,
        "candidate_history",
        "candidate-history.json",
        "application/json",
        &history,
    )
    .await?;
    let result_status = if contract.spec.final_verification.is_some() {
        "verified"
    } else {
        "validation_only"
    };
    let report_value = serde_json::json!({
        "schema": "wisp.method-search-verification.v1",
        "run_id": run_id,
        "result_status": result_status,
        "selected_candidate_id": selected_id,
        "selected_artifact_version_id": selected_version_id,
        "top_k": top_artifacts,
        "verification": verification,
        "search_metrics_did_not_observe_final_verification": true
    });
    let (_, report_version_id) = promote_bytes(
        store,
        project_root,
        &run.project_id,
        frame_id,
        run_id,
        "verification_report",
        "verification-report.json",
        "application/json",
        &serde_json::to_vec_pretty(&report_value).map_err(|error| error.to_string())?,
    )
    .await?;
    checkpoint.best_candidate_id = Some(selected_id.clone());
    if checkpoint.terminal_reason.is_none() {
        checkpoint.terminal_reason = Some("candidate_budget_completed".into());
    }
    persist_checkpoint(store, run_id, checkpoint, Some(result_status)).await?;

    let mut decision = ResearchNode::new(
        format!("decision:method-search:{run_id}"),
        &run.project_id,
        ResearchNodeKind::Decision,
        "Select method-search finalist",
    )
    .map_err(|error| error.to_string())?;
    decision.ref_id = Some(selected_id);
    decision.metadata_json = serde_json::json!({
        "selected_artifact_version_id": selected_version_id,
        "verification_report_artifact_version_id": report_version_id,
        "history_artifact_version_id": history_version_id,
        "result_status": result_status
    })
    .to_string();
    store
        .save_research_node(&decision)
        .await
        .map_err(|error| error.to_string())?;
    store
        .save_research_edge(
            &ResearchEdge::new(
                format!("run-decision:{run_id}"),
                &run.project_id,
                format!("run:{run_id}"),
                decision.id.clone(),
                "informed",
            )
            .map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    for source in &contract.spec.strategy_sources {
        let paper_id = method_search_research_id("paper", run_id, &source.source_ref);
        store
            .save_research_edge(
                &ResearchEdge::new(
                    method_search_research_id("paper-decision", run_id, &source.source_ref),
                    &run.project_id,
                    paper_id,
                    decision.id.clone(),
                    "informed",
                )
                .map_err(|error| error.to_string())?,
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    store
        .save_research_edge(
            &ResearchEdge::new(
                format!("decision-artifact:{run_id}"),
                &run.project_id,
                decision.id,
                format!("artifact:{selected_artifact_id}"),
                "selected",
            )
            .map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;

    if !store
        .finish_active_run_owned(run_id, owner, RunStatus::Succeeded, Some(0))
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Method-search Run lost its lease before completion".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoordinatorOutcome {
    Terminal,
    Paused,
}

pub(crate) async fn run_method_search_coordinator(
    store: &Store,
    project_root: &Path,
    run_id: &str,
    generator: &dyn CandidateGenerator,
    evaluator: &dyn MethodSearchEvaluator,
) -> Result<CoordinatorOutcome, String> {
    let owner = format!("method-search:{}", uuid::Uuid::new_v4());
    if !store
        .claim_run_lifecycle(run_id, &owner, LEASE_SECONDS)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Method-search Run already has an active coordinator".into());
    }
    let status = store
        .method_search_run_status(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search Run does not exist".to_string())?;
    if status == RunStatus::Submitted
        && !store
            .transition_run_to_running_owned(run_id, &owner)
            .await
            .map_err(|error| error.to_string())?
    {
        return Err("Method-search Run could not enter running state".into());
    }
    if status == RunStatus::Cancelling {
        store
            .finish_active_run_owned(run_id, &owner, RunStatus::Cancelled, None)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(CoordinatorOutcome::Terminal);
    }
    let state = store
        .get_method_search_run_state(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search checkpoint state is missing".to_string())?;
    let run = store
        .get_run(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search Run disappeared".to_string())?;
    let contract = load_frozen_contract(store, project_root, &run, &state).await?;
    let mut checkpoint = checkpoint_from_state(&state)?;
    initialize_baseline(store, project_root, run_id, &contract).await?;
    initialize_strategies(store, run_id, &contract.spec).await?;

    loop {
        if !store
            .renew_run_lifecycle(run_id, &owner, LEASE_SECONDS)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Method-search coordinator lost its lifecycle lease".into());
        }
        match store
            .method_search_run_status(run_id)
            .await
            .map_err(|error| error.to_string())?
        {
            Some(RunStatus::Cancelling) => {
                persist_checkpoint(store, run_id, &mut checkpoint, Some("incomplete")).await?;
                store
                    .finish_active_run_owned(run_id, &owner, RunStatus::Cancelled, None)
                    .await
                    .map_err(|error| error.to_string())?;
                return Ok(CoordinatorOutcome::Terminal);
            }
            Some(RunStatus::Running) => {}
            Some(status) if status.is_terminal() => return Ok(CoordinatorOutcome::Terminal),
            Some(other) => {
                return Err(format!(
                    "Method-search Run entered unexpected state {}",
                    other.as_str()
                ))
            }
            None => return Err("Method-search Run disappeared".into()),
        }
        if store
            .method_search_pause_requested(run_id)
            .await
            .map_err(|error| error.to_string())?
        {
            persist_checkpoint(store, run_id, &mut checkpoint, None).await?;
            if !store
                .pause_method_search_run_owned(
                    run_id,
                    &owner,
                    "Paused at a durable candidate boundary.",
                )
                .await
                .map_err(|error| error.to_string())?
            {
                return Err("Method-search Run could not pause at its checkpoint".into());
            }
            return Ok(CoordinatorOutcome::Paused);
        }
        let now = chrono::Utc::now().timestamp();
        let elapsed = now.saturating_sub(run.started_at.unwrap_or(run.created_at));
        if elapsed >= i64::try_from(contract.spec.budget.max_wall_seconds).unwrap_or(i64::MAX) {
            checkpoint.terminal_reason = Some("wall_time_budget_exhausted".into());
            persist_checkpoint(store, run_id, &mut checkpoint, Some("incomplete")).await?;
            store
                .finish_active_run_owned(run_id, &owner, RunStatus::TimedOut, None)
                .await
                .map_err(|error| error.to_string())?;
            return Ok(CoordinatorOutcome::Terminal);
        }
        if checkpoint.cost_microunits >= contract.spec.budget.max_cost_microunits {
            checkpoint.terminal_reason = Some("provider_cost_budget_exhausted".into());
            finalize_search(
                store,
                project_root,
                run_id,
                &owner,
                &contract,
                &mut checkpoint,
                evaluator,
            )
            .await?;
            return Ok(CoordinatorOutcome::Terminal);
        }
        if checkpoint.next_sequence > contract.spec.budget.max_candidates {
            finalize_search(
                store,
                project_root,
                run_id,
                &owner,
                &contract,
                &mut checkpoint,
                evaluator,
            )
            .await?;
            return Ok(CoordinatorOutcome::Terminal);
        }

        let candidates = store
            .list_method_candidates(run_id)
            .await
            .map_err(|error| error.to_string())?;
        if candidates
            .iter()
            .any(|candidate| candidate.sequence == i64::from(checkpoint.next_sequence))
        {
            checkpoint.next_sequence += 1;
            persist_checkpoint(store, run_id, &mut checkpoint, None).await?;
            continue;
        }
        let baseline = candidates
            .iter()
            .find(|candidate| candidate.sequence == 0)
            .ok_or_else(|| "Method-search baseline candidate disappeared".to_string())?;
        let best = candidates
            .iter()
            .filter(|candidate| candidate.status == MethodCandidateStatus::Succeeded)
            .max_by(|left, right| {
                left.utility
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&right.utility.unwrap_or(f64::NEG_INFINITY))
            })
            .unwrap_or(baseline);
        let parent = if checkpoint.next_sequence % 5 == 0 {
            baseline
        } else {
            best
        };
        let parent_source = candidate_source(store, project_root, parent).await?;
        let stats = store
            .list_method_strategy_stats(run_id)
            .await
            .map_err(|error| error.to_string())?;
        let cards = cards_from_stats(&stats);
        let strategy = cards[select_strategy_index(run_id, checkpoint.next_sequence, &cards)
            .map_err(|error| error.to_string())?]
        .clone();
        persist_progress(
            store,
            run_id,
            &owner,
            &contract,
            &checkpoint,
            Some(strategy.key.clone()),
        )
        .await?;
        let feedback = candidates
            .iter()
            .rev()
            .filter_map(|candidate| {
                candidate
                    .error
                    .clone()
                    .or(candidate.diagnostic_summary.clone())
            })
            .take(3)
            .collect::<Vec<_>>();
        let generated = generator
            .propose(CandidateGenerationRequest {
                objective: contract.spec.objective.clone(),
                constraints: contract.spec.constraints.clone(),
                target_symbol: contract.spec.target.symbol.clone(),
                target_source: parent_source.clone(),
                strategy: strategy.clone(),
                parent_metrics: serde_json::from_str(&parent.metrics_json)
                    .unwrap_or_else(|_| serde_json::json!({})),
                recent_feedback: feedback,
            })
            .await;
        let generated = match generated {
            Ok(response) => response,
            Err(error) => {
                record_failed_candidate(
                    store,
                    run_id,
                    checkpoint.next_sequence,
                    parent,
                    &strategy,
                    error,
                )
                .await?;
                update_strategy(store, run_id, &strategy.key, -1.0).await?;
                checkpoint.next_sequence += 1;
                persist_checkpoint(store, run_id, &mut checkpoint, None).await?;
                continue;
            }
        };
        checkpoint.cost_microunits = checkpoint
            .cost_microunits
            .saturating_add(generated.cost_microunits);
        if checkpoint.cost_microunits > contract.spec.budget.max_cost_microunits {
            record_failed_candidate(
                store,
                run_id,
                checkpoint.next_sequence,
                parent,
                &strategy,
                "Candidate generation exceeded the approved provider budget".into(),
            )
            .await?;
            checkpoint.next_sequence += 1;
            persist_checkpoint(store, run_id, &mut checkpoint, None).await?;
            continue;
        }
        let candidate_source = match replace_python_symbol(
            &contract.target_source,
            &contract.spec.target.symbol,
            &generated.replacement,
        ) {
            Ok(source) => source,
            Err(error) => {
                record_failed_candidate(
                    store,
                    run_id,
                    checkpoint.next_sequence,
                    parent,
                    &strategy,
                    format!("Candidate replacement was rejected: {error}"),
                )
                .await?;
                update_strategy(store, run_id, &strategy.key, -2.0).await?;
                checkpoint.next_sequence += 1;
                persist_checkpoint(store, run_id, &mut checkpoint, None).await?;
                continue;
            }
        };
        let source_blob = save_candidate_blob(
            store,
            project_root,
            run_id,
            "source",
            candidate_source.as_bytes(),
        )
        .await?;
        let patch_blob = save_candidate_blob(
            store,
            project_root,
            run_id,
            "patch",
            generated.replacement.as_bytes(),
        )
        .await?;
        let mut candidate = MethodCandidate::proposed(
            uuid::Uuid::new_v4().to_string(),
            run_id,
            i64::from(checkpoint.next_sequence),
            &strategy.key,
            &generated.family,
            source_blob.checksum.clone(),
            patch_blob.checksum.clone(),
        )
        .map_err(|error| error.to_string())?;
        candidate.parent_candidate_id = Some(parent.id.clone());
        candidate.source_blob_id = Some(source_blob.id);
        candidate.patch_blob_id = Some(patch_blob.id);
        candidate.changed_lines = Some(changed_lines(&contract.target_source, &candidate_source));
        candidate.dependency_count = Some(dependency_count(&generated.replacement));
        candidate.rationale = Some(generated.rationale);
        store
            .insert_method_candidate(&candidate)
            .await
            .map_err(|error| error.to_string())?;
        if !store
            .transition_method_candidate_to_evaluating(&candidate.id)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Candidate could not enter evaluating state".into());
        }
        let started = std::time::Instant::now();
        let evaluated = evaluate_source(
            project_root,
            run_id,
            &contract,
            &candidate_source,
            "search",
            evaluator,
        )
        .await;
        let runtime_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let parent_utility = parent.utility.unwrap_or(f64::NEG_INFINITY);
        match evaluated {
            Ok(result) => {
                candidate.primary_score = Some(result.primary);
                candidate.utility = Some(
                    contract
                        .spec
                        .normalized_utility(result.primary)
                        .map_err(|error| error.to_string())?,
                );
                candidate.metrics_json =
                    serde_json::to_string(&result.metrics).map_err(|error| error.to_string())?;
                candidate.runtime_ms = Some(runtime_ms);
                if result.passes_guardrails(&contract.spec) {
                    candidate.status = MethodCandidateStatus::Succeeded;
                } else {
                    candidate.status = MethodCandidateStatus::Rejected;
                    candidate.error = Some("Candidate violated one or more hard guardrails".into());
                }
                candidate.finished_at = Some(chrono::Utc::now().timestamp());
                if !store
                    .finish_method_candidate(&candidate, MethodCandidateStatus::Evaluating)
                    .await
                    .map_err(|error| error.to_string())?
                {
                    return Err("Candidate terminal state changed concurrently".into());
                }
                let reward = if candidate.status == MethodCandidateStatus::Succeeded {
                    normalized_strategy_reward(
                        candidate.utility.unwrap(),
                        parent_utility,
                        contract.audit.baseline.noise_floor,
                    )
                    .map_err(|error| error.to_string())?
                } else {
                    -5.0
                };
                update_strategy(store, run_id, &strategy.key, reward).await?;
            }
            Err(error) => {
                candidate.status = MethodCandidateStatus::Failed;
                candidate.runtime_ms = Some(runtime_ms);
                candidate.error = Some(error.chars().take(4_000).collect());
                candidate.finished_at = Some(chrono::Utc::now().timestamp());
                if !store
                    .finish_method_candidate(&candidate, MethodCandidateStatus::Evaluating)
                    .await
                    .map_err(|error| error.to_string())?
                {
                    return Err("Candidate failure state changed concurrently".into());
                }
                update_strategy(store, run_id, &strategy.key, -2.0).await?;
            }
        }
        checkpoint.next_sequence += 1;
        let current_best = store
            .list_method_candidates(run_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|candidate| candidate.status == MethodCandidateStatus::Succeeded)
            .max_by(|left, right| {
                left.utility
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&right.utility.unwrap_or(f64::NEG_INFINITY))
            });
        checkpoint.best_candidate_id = current_best.map(|candidate| candidate.id);
        persist_checkpoint(store, run_id, &mut checkpoint, None).await?;
        persist_progress(store, run_id, &owner, &contract, &checkpoint, None).await?;
    }
}

pub(crate) async fn bind_method_search_run_inputs(
    store: &Store,
    project_root: &Path,
    run_id: &str,
) -> Result<(), String> {
    let state = store
        .get_method_search_run_state(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search state is missing".to_string())?;
    let run = store
        .get_run(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search Run is missing".to_string())?;
    let contract = load_frozen_contract(store, project_root, &run, &state).await?;
    let audit_version_id = store
        .list_artifact_dependencies(&state.spec_artifact_version_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|dependency| dependency.reference_name.as_deref() == Some("method_search_audit"))
        .map(|dependency| dependency.depends_on_version_id)
        .ok_or_else(|| "Method-search spec has no exact audit dependency".to_string())?;
    let mut exact_inputs = vec![
        ("spec".to_string(), state.spec_artifact_version_id.clone()),
        ("audit".to_string(), audit_version_id),
        (
            "target_source".to_string(),
            contract.spec.target.source_artifact_version_id.clone(),
        ),
        (
            "evaluator".to_string(),
            contract.spec.evaluator.artifact_version_id.clone(),
        ),
    ];
    exact_inputs.extend(contract.spec.inputs.iter().filter_map(|input| {
        input
            .artifact_version_id
            .clone()
            .map(|id| (format!("input:{}", input.role), id))
    }));
    if let Some(final_verification) = &contract.spec.final_verification {
        exact_inputs.push((
            "final_verification".into(),
            final_verification.artifact_version_id.clone(),
        ));
    }
    for (role, version_id) in exact_inputs {
        store
            .save_run_input(&RunInput {
                id: format!("run-input:{run_id}:{role}"),
                run_id: run_id.into(),
                artifact_version_id: Some(version_id.clone()),
                external_resource_id: None,
                source_ref: version_id,
                role,
                required: true,
                basis: LineageBasis::Declared,
                confidence: LineageConfidence::Exact,
                created_at: chrono::Utc::now().timestamp(),
            })
            .await
            .map_err(|error| error.to_string())?;
    }
    for input in &contract.spec.inputs {
        let source_ref = input
            .artifact_version_id
            .as_deref()
            .or(input.external_resource_id.as_deref())
            .ok_or_else(|| "Method-search input lost its exact source".to_string())?;
        let node_id = method_search_research_id("data", run_id, &input.role);
        let mut node = ResearchNode::new(
            &node_id,
            &run.project_id,
            ResearchNodeKind::DataAsset,
            format!("Method-search input: {}", input.role),
        )
        .map_err(|error| error.to_string())?;
        node.ref_id = Some(source_ref.to_string());
        node.metadata_json = serde_json::json!({
            "role": input.role,
            "path": input.path,
            "checksum": input.checksum,
            "artifact_version_id": input.artifact_version_id,
            "external_resource_id": input.external_resource_id
        })
        .to_string();
        store
            .save_research_node(&node)
            .await
            .map_err(|error| error.to_string())?;
        store
            .save_research_edge(
                &ResearchEdge::new(
                    method_search_research_id("data-run", run_id, &input.role),
                    &run.project_id,
                    node_id,
                    format!("run:{run_id}"),
                    "input_to",
                )
                .map_err(|error| error.to_string())?,
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    for source in &contract.spec.strategy_sources {
        let node_id = method_search_research_id("paper", run_id, &source.source_ref);
        let mut node = ResearchNode::new(
            &node_id,
            &run.project_id,
            ResearchNodeKind::Paper,
            &source.title,
        )
        .map_err(|error| error.to_string())?;
        node.ref_id = Some(source.source_ref.clone());
        node.metadata_json = serde_json::json!({
            "category": source.category,
            "summary": source.summary
        })
        .to_string();
        store
            .save_research_node(&node)
            .await
            .map_err(|error| error.to_string())?;
        store
            .save_research_edge(
                &ResearchEdge::new(
                    method_search_research_id("paper-run", run_id, &source.source_ref),
                    &run.project_id,
                    node_id,
                    format!("run:{run_id}"),
                    "informed",
                )
                .map_err(|error| error.to_string())?,
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    store
        .save_run_code_snapshot(&RunCodeSnapshot {
            id: format!("run-code:{run_id}:baseline"),
            run_id: run_id.into(),
            source_kind: "method_search_baseline".into(),
            source_path: Some(contract.spec.target.source_path),
            source_text: contract.target_source.clone(),
            checksum: sha256(contract.target_source.as_bytes()),
            storage_path: None,
            git_commit: None,
            dirty_patch: None,
            created_at: chrono::Utc::now().timestamp(),
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn validate_method_search_activation(
    store: &Store,
    project_root: &Path,
    project_id: &str,
    run_id: &str,
    expected_status: RunStatus,
) -> Result<String, String> {
    let run = store
        .get_run(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search Run does not exist".to_string())?;
    if run.project_id != project_id || run.kind != "method_search" || run.status != expected_status
    {
        return Err(format!(
            "Only a {} method-search Run in the active project can be activated",
            expected_status.as_str()
        ));
    }
    let context = store
        .get_execution_context(&run.context_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search ExecutionContext disappeared".to_string())?;
    let (current_context, _) = wisp_store::canonical_json_sha256(
        &serde_json::to_value(&context).map_err(|error| error.to_string())?,
    );
    let stored_context: serde_json::Value =
        serde_json::from_str(&run.env_snapshot_json).map_err(|error| error.to_string())?;
    let (stored_context, _) = wisp_store::canonical_json_sha256(&stored_context);
    if current_context != stored_context {
        return Err("Method-search ExecutionContext changed since approval".into());
    }
    let state = store
        .get_method_search_run_state(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search checkpoint state is missing".to_string())?;
    let _ = checkpoint_from_state(&state)?;
    let _ = load_frozen_contract(store, project_root, &run, &state).await?;
    for candidate in store
        .list_method_candidates(run_id)
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .filter(|candidate| candidate.source_blob_id.is_some())
    {
        let _ = candidate_source(store, project_root, candidate).await?;
    }
    let link = store
        .get_agent_workflow_run_activity_by_run(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search Run has no owning Workflow activity".to_string())?;
    let state: serde_json::Value =
        serde_json::from_str(&link.state_json).map_err(|error| error.to_string())?;
    state
        .get("model_profile_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Method-search activity lost its approved model profile".into())
}

pub(crate) async fn validate_method_search_start(
    store: &Store,
    project_root: &Path,
    project_id: &str,
    run_id: &str,
) -> Result<String, String> {
    validate_method_search_activation(store, project_root, project_id, run_id, RunStatus::Draft)
        .await
}

pub(crate) async fn validate_method_search_resume(
    store: &Store,
    project_root: &Path,
    project_id: &str,
    run_id: &str,
) -> Result<String, String> {
    validate_method_search_activation(store, project_root, project_id, run_id, RunStatus::Paused)
        .await
}

pub(crate) async fn finish_cancelling_method_search(
    store: &Store,
    run_id: &str,
) -> Result<(), String> {
    let owner = format!("method-search-cancel:{}", uuid::Uuid::new_v4());
    if !store
        .claim_run_lifecycle(run_id, &owner, LEASE_SECONDS)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Cancelling method-search Run already has an active coordinator".into());
    }
    if !store
        .finish_active_run_owned(run_id, &owner, RunStatus::Cancelled, None)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Method-search Run could not finish cancellation".into());
    }
    Ok(())
}

pub(crate) async fn terminal_activity_response(
    store: &Store,
    request_id: &str,
    run_id: &str,
) -> Result<wisp_core::AgentDelegationResponse, String> {
    let run = store
        .get_run(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Linked method-search Run disappeared".to_string())?;
    if !run.status.is_terminal() {
        return Err("Linked method-search Run is not terminal".into());
    }
    let outputs = store
        .list_run_outputs(run_id)
        .await
        .map_err(|error| error.to_string())?;
    let state = store
        .get_method_search_run_state(run_id)
        .await
        .map_err(|error| error.to_string())?;
    let status = match run.status {
        RunStatus::Succeeded => wisp_core::DelegationStatus::Succeeded,
        RunStatus::Cancelled => wisp_core::DelegationStatus::Cancelled,
        _ => wisp_core::DelegationStatus::Failed,
    };
    Ok(wisp_core::AgentDelegationResponse {
        request_id: request_id.into(),
        status,
        output: serde_json::json!({
            "run_id": run_id,
            "run_status": run.status.as_str(),
            "result_status": state.and_then(|state| state.result_status),
            "artifact_version_ids": outputs.iter().map(|output| output.artifact_version_id.clone()).collect::<Vec<_>>()
        }),
        artifact_ids: outputs
            .iter()
            .map(|output| output.artifact_version_id.clone())
            .collect(),
        artifacts: vec![],
        evidence: vec![],
        usage: Default::default(),
        agent_session_id: None,
        child_frame_id: None,
        error: (status == wisp_core::DelegationStatus::Failed).then(|| {
            run.last_poll_error
                .unwrap_or_else(|| "Method-search Run failed".into())
        }),
        nested_results: vec![],
    })
}

pub(crate) async fn settle_linked_workflow_attempt(
    store: &Store,
    run_id: &str,
) -> Result<Option<String>, String> {
    let Some(link) = store
        .get_agent_workflow_run_activity_by_run(run_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let mut attempt = store
        .get_agent_workflow_attempt(&link.attempt_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Linked Workflow attempt disappeared".to_string())?;
    if attempt.status.is_terminal() {
        return Ok(Some(attempt.workflow_id));
    }
    if attempt.status != AgentWorkflowAttemptStatus::WaitingRun {
        return Err("Linked Workflow attempt is not waiting on its Run".into());
    }
    let response = terminal_activity_response(store, &attempt.request_id, run_id).await?;
    attempt.status = match response.status {
        wisp_core::DelegationStatus::Succeeded => AgentWorkflowAttemptStatus::Succeeded,
        wisp_core::DelegationStatus::Cancelled => AgentWorkflowAttemptStatus::Cancelled,
        _ => AgentWorkflowAttemptStatus::Failed,
    };
    attempt.response_json = Some(serde_json::to_string(&response).map_err(|e| e.to_string())?);
    attempt.output_json = serde_json::to_string(&response.output).map_err(|e| e.to_string())?;
    attempt.artifact_ids_json =
        serde_json::to_string(&response.artifact_ids).map_err(|e| e.to_string())?;
    attempt.evidence_json = serde_json::to_string(&response.evidence).map_err(|e| e.to_string())?;
    attempt.error = response.error;
    attempt.finished_at = Some(chrono::Utc::now().timestamp());
    if !store
        .update_agent_workflow_attempt(&attempt, AgentWorkflowAttemptStatus::WaitingRun)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Linked Workflow attempt changed while its Run was settling".into());
    }
    Ok(Some(attempt.workflow_id))
}

pub(crate) fn linked_activity_state(
    spec_artifact_version_id: &str,
    spec_sha256: &str,
    model_profile_id: &str,
) -> String {
    serde_json::json!({
        "schema": "wisp.workflow-run-activity.v1",
        "spec_artifact_version_id": spec_artifact_version_id,
        "spec_sha256": spec_sha256,
        "model_profile_id": model_profile_id
    })
    .to_string()
}

pub(crate) fn method_state(
    run_id: &str,
    spec_artifact_version_id: &str,
    spec_sha256: &str,
) -> Result<MethodSearchRunState, String> {
    MethodSearchRunState::new(run_id, spec_artifact_version_id, spec_sha256)
        .map_err(|error| error.to_string())
}

pub(crate) fn activity_link(
    attempt_id: &str,
    run_id: &str,
    state_json: String,
) -> Result<AgentWorkflowRunActivity, String> {
    let mut link = AgentWorkflowRunActivity::new(attempt_id, run_id, "method_search")
        .map_err(|error| error.to_string())?;
    link.state_json = state_json;
    Ok(link)
}

pub(crate) fn local_evaluator() -> LocalPythonMethodSearchEvaluator {
    LocalPythonMethodSearchEvaluator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::method_search::{
        prepare_method_search_with_evaluator, MethodSearchPreparationInput,
        PrepareMethodSearchRequest,
    };
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wisp-method-coordinator-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct ScriptedEvaluator;

    #[async_trait]
    impl MethodSearchEvaluator for ScriptedEvaluator {
        async fn evaluate(
            &self,
            request: &EvaluationRequest,
        ) -> Result<crate::method_search::EvaluationExecution, String> {
            let source = std::fs::read_to_string(request.workspace.join(&request.target_path))
                .map_err(|error| error.to_string())?;
            if source.contains("wisp_method_search_reachability_sentinel") {
                return Ok(crate::method_search::EvaluationExecution {
                    exit_code: Some(1),
                    stderr: "RuntimeError: wisp_method_search_reachability_sentinel".into(),
                    ..Default::default()
                });
            }
            let sequence = source
                .lines()
                .find_map(|line| line.trim().strip_prefix("return len(rows) + "))
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0);
            if sequence == 5 {
                return Ok(crate::method_search::EvaluationExecution {
                    exit_code: Some(1),
                    stderr: "scripted evaluator failure".into(),
                    ..Default::default()
                });
            }
            let primary = 0.5 + f64::from(sequence) / 100.0;
            let runtime = if sequence == 19 { 20.0 } else { 1.0 };
            Ok(crate::method_search::EvaluationExecution {
                exit_code: Some(0),
                stdout: format!(
                    "wisp_evaluate: {{\"primary\":{primary},\"metrics\":{{\"accuracy\":{primary},\"runtime_seconds\":{runtime}}}}}"
                ),
                ..Default::default()
            })
        }
    }

    struct SearchTimeoutEvaluator;

    #[async_trait]
    impl MethodSearchEvaluator for SearchTimeoutEvaluator {
        async fn evaluate(
            &self,
            request: &EvaluationRequest,
        ) -> Result<crate::method_search::EvaluationExecution, String> {
            if request.phase == "search" {
                Ok(crate::method_search::EvaluationExecution {
                    timed_out: true,
                    ..Default::default()
                })
            } else {
                ScriptedEvaluator.evaluate(request).await
            }
        }
    }

    struct ScriptedGenerator {
        calls: AtomicU32,
        pause_enabled: AtomicBool,
        cancel_at: AtomicU32,
        delay_ms: AtomicU32,
        store: Store,
        run_id: String,
    }

    #[async_trait]
    impl CandidateGenerator for ScriptedGenerator {
        async fn propose(
            &self,
            _request: CandidateGenerationRequest,
        ) -> Result<CandidateGenerationResponse, String> {
            let sequence = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if sequence == 3 && self.pause_enabled.load(Ordering::SeqCst) {
                self.store
                    .request_method_search_pause(&self.run_id)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            if sequence == self.cancel_at.load(Ordering::SeqCst) {
                self.store
                    .request_run_cancellation(&self.run_id)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            let delay_ms = self.delay_ms.load(Ordering::SeqCst);
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
            }
            let replacement = if sequence == 4 {
                "def fit_model(changed):\n    return 0\n".into()
            } else {
                format!("def fit_model(rows):\n    return len(rows) + {sequence}\n")
            };
            Ok(CandidateGenerationResponse {
                replacement,
                rationale: format!("scripted candidate {sequence}"),
                family: if sequence % 2 == 0 { "linear" } else { "tree" }.into(),
                cost_microunits: 1,
            })
        }
    }

    async fn fixture_with_budget(
        max_candidates: u32,
        max_wall_seconds: u64,
        max_cost_microunits: u64,
    ) -> (
        TestDirectory,
        Store,
        String,
        Arc<ScriptedGenerator>,
        ScriptedEvaluator,
    ) {
        let root = TestDirectory::new();
        std::fs::create_dir_all(root.0.join("analysis")).unwrap();
        std::fs::create_dir_all(root.0.join("data")).unwrap();
        std::fs::write(
            root.0.join("analysis/model.py"),
            "def fit_model(rows):\n    return len(rows)\n",
        )
        .unwrap();
        std::fs::write(root.0.join("analysis/evaluate.py"), "# scripted\n").unwrap();
        std::fs::write(root.0.join("data/validation.csv"), "x,y\n1,1\n").unwrap();
        std::fs::write(root.0.join("data/final.csv"), "x,y\n2,2\n").unwrap();
        let store = Store::open(&root.0.join("wisp.db")).await.unwrap();
        store
            .create_project("project", "Project", root.0.to_str().unwrap())
            .await
            .unwrap();
        store
            .create_frame("frame", "project", "Method search", "default")
            .await
            .unwrap();
        let evaluator = ScriptedEvaluator;
        let prepared = prepare_method_search_with_evaluator(
            &store,
            "project",
            &root.0,
            "frame",
            PrepareMethodSearchRequest {
                objective: "Improve validation accuracy".into(),
                target_path: "analysis/model.py".into(),
                target_symbol: "fit_model".into(),
                evaluator_path: "analysis/evaluate.py".into(),
                primary_metric: "accuracy".into(),
                direction: wisp_core::method_search::ScoreDirection::Maximize,
                guardrails: vec![wisp_core::method_search::MethodSearchGuardrail {
                    metric: "runtime_seconds".into(),
                    op: wisp_core::method_search::GuardrailOperator::Lte,
                    value: 10.0,
                }],
                inputs: vec![MethodSearchPreparationInput {
                    role: "search_validation".into(),
                    path: "data/validation.csv".into(),
                }],
                constraints: vec!["Keep the public signature unchanged".into()],
                strategy_sources: vec![wisp_core::method_search::MethodStrategySource {
                    source_ref: "doi:10.0000/wisp.fixture".into(),
                    title: "Fixture method".into(),
                    summary: "Try a bounded representation improvement.".into(),
                    category: "literature_or_method".into(),
                }],
                repetitions: 3,
                evaluator_timeout_seconds: 30,
                max_candidates,
                max_wall_seconds,
                max_cost_microunits,
                final_verification_path: Some("data/final.csv".into()),
                python_executable: None,
            },
            &evaluator,
        )
        .await
        .unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut run = wisp_store::RunRecord::new(
            &run_id,
            "project",
            "local",
            "Method search",
            "method_search",
        );
        run.frame_id = Some("frame".into());
        let context = ExecutionContext::new("local", "Local").unwrap();
        run.env_snapshot_json = serde_json::to_string(&context).unwrap();
        store.create_run(&run).await.unwrap();
        store
            .create_method_search_run_state(
                &MethodSearchRunState::new(
                    &run_id,
                    &prepared.method_search_spec_artifact_version_id,
                    &prepared.method_search_spec_sha256,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        bind_method_search_run_inputs(&store, &root.0, &run_id)
            .await
            .unwrap();
        let generator = Arc::new(ScriptedGenerator {
            calls: AtomicU32::new(0),
            pause_enabled: AtomicBool::new(true),
            cancel_at: AtomicU32::new(0),
            delay_ms: AtomicU32::new(0),
            store: store.clone(),
            run_id: run_id.clone(),
        });
        (root, store, run_id, generator, evaluator)
    }

    async fn fixture() -> (
        TestDirectory,
        Store,
        String,
        Arc<ScriptedGenerator>,
        ScriptedEvaluator,
    ) {
        fixture_with_budget(20, 3_600, 1_000).await
    }

    #[tokio::test]
    async fn twenty_candidate_search_pauses_resumes_and_promotes_verified_outputs() {
        let (root, store, run_id, generator, evaluator) = fixture().await;
        assert_eq!(
            store.method_search_run_status(&run_id).await.unwrap(),
            Some(RunStatus::Draft)
        );
        assert!(store
            .list_method_candidates(&run_id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(generator.calls.load(Ordering::SeqCst), 0);
        assert!(store.submit_method_search_run(&run_id).await.unwrap());
        assert_eq!(
            run_method_search_coordinator(
                &store,
                &root.0,
                &run_id,
                generator.as_ref(),
                &evaluator,
            )
            .await
            .unwrap(),
            CoordinatorOutcome::Paused
        );
        assert_eq!(
            store.method_search_run_status(&run_id).await.unwrap(),
            Some(RunStatus::Paused)
        );
        let before = store.list_method_candidates(&run_id).await.unwrap();
        assert_eq!(
            before
                .iter()
                .filter(|candidate| candidate.sequence > 0)
                .count(),
            3
        );
        generator.pause_enabled.store(false, Ordering::SeqCst);
        assert!(store.resume_method_search_run(&run_id).await.unwrap());
        assert_eq!(
            run_method_search_coordinator(
                &store,
                &root.0,
                &run_id,
                generator.as_ref(),
                &evaluator,
            )
            .await
            .unwrap(),
            CoordinatorOutcome::Terminal
        );
        let candidates = store.list_method_candidates(&run_id).await.unwrap();
        assert_eq!(candidates.len(), 21);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.sequence)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            21
        );
        assert!(candidates.iter().any(|candidate| {
            candidate.sequence == 19 && candidate.status == MethodCandidateStatus::Rejected
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate.sequence == 5 && candidate.status == MethodCandidateStatus::Failed
        }));
        assert_eq!(
            store.method_search_run_status(&run_id).await.unwrap(),
            Some(RunStatus::Succeeded)
        );
        assert!(store.list_run_outputs(&run_id).await.unwrap().len() >= 4);
        let strategy_sources = store
            .list_method_strategy_stats(&run_id)
            .await
            .unwrap()
            .into_iter()
            .flat_map(|strategy| {
                serde_json::from_str::<Vec<String>>(&strategy.source_refs_json).unwrap_or_default()
            })
            .collect::<Vec<_>>();
        assert_eq!(strategy_sources, ["doi:10.0000/wisp.fixture"]);
        let graph = store.research_graph("project").await.unwrap();
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == ResearchNodeKind::DataAsset));
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == ResearchNodeKind::Paper));
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == ResearchNodeKind::Decision));
        assert!(graph.edges.iter().any(|edge| edge.relation == "input_to"));
        assert!(graph.edges.iter().any(|edge| {
            edge.source_id == format!("run:{run_id}") && edge.relation == "produced"
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.target_id.starts_with("decision:method-search:") && edge.relation == "informed"
        }));
        assert_eq!(
            store
                .get_method_search_run_state(&run_id)
                .await
                .unwrap()
                .unwrap()
                .result_status
                .as_deref(),
            Some("verified")
        );
        assert_eq!(
            std::fs::read_to_string(root.0.join("analysis/model.py")).unwrap(),
            "def fit_model(rows):\n    return len(rows)\n"
        );
    }

    #[tokio::test]
    async fn cancellation_at_candidate_boundary_creates_no_later_candidate() {
        let (root, store, run_id, generator, evaluator) = fixture().await;
        generator.pause_enabled.store(false, Ordering::SeqCst);
        generator.cancel_at.store(2, Ordering::SeqCst);
        assert!(store.submit_method_search_run(&run_id).await.unwrap());
        assert_eq!(
            run_method_search_coordinator(
                &store,
                &root.0,
                &run_id,
                generator.as_ref(),
                &evaluator,
            )
            .await
            .unwrap(),
            CoordinatorOutcome::Terminal
        );
        assert_eq!(
            store.method_search_run_status(&run_id).await.unwrap(),
            Some(RunStatus::Cancelled)
        );
        let sequences = store
            .list_method_candidates(&run_id)
            .await
            .unwrap()
            .into_iter()
            .filter(|candidate| candidate.sequence > 0)
            .map(|candidate| candidate.sequence)
            .collect::<Vec<_>>();
        assert_eq!(sequences, [1, 2]);
        assert!(store.list_run_outputs(&run_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn provider_cost_budget_stops_before_a_second_candidate() {
        let (root, store, run_id, generator, evaluator) = fixture_with_budget(20, 3_600, 1).await;
        generator.pause_enabled.store(false, Ordering::SeqCst);
        assert!(store.submit_method_search_run(&run_id).await.unwrap());
        assert_eq!(
            run_method_search_coordinator(
                &store,
                &root.0,
                &run_id,
                generator.as_ref(),
                &evaluator,
            )
            .await
            .unwrap(),
            CoordinatorOutcome::Terminal
        );
        assert_eq!(
            store
                .list_method_candidates(&run_id)
                .await
                .unwrap()
                .iter()
                .filter(|candidate| candidate.sequence > 0)
                .count(),
            1
        );
        let state = store
            .get_method_search_run_state(&run_id)
            .await
            .unwrap()
            .unwrap();
        let checkpoint: MethodSearchCheckpoint =
            serde_json::from_str(&state.checkpoint_json).unwrap();
        assert_eq!(
            checkpoint.terminal_reason.as_deref(),
            Some("provider_cost_budget_exhausted")
        );
    }

    #[tokio::test]
    async fn wall_budget_times_out_at_the_next_durable_boundary() {
        let (root, store, run_id, generator, evaluator) = fixture_with_budget(20, 1, 1_000).await;
        generator.pause_enabled.store(false, Ordering::SeqCst);
        generator.delay_ms.store(1_100, Ordering::SeqCst);
        assert!(store.submit_method_search_run(&run_id).await.unwrap());
        assert_eq!(
            run_method_search_coordinator(
                &store,
                &root.0,
                &run_id,
                generator.as_ref(),
                &evaluator,
            )
            .await
            .unwrap(),
            CoordinatorOutcome::Terminal
        );
        assert_eq!(
            store.method_search_run_status(&run_id).await.unwrap(),
            Some(RunStatus::TimedOut)
        );
        assert!(generator.calls.load(Ordering::SeqCst) <= 1);
    }

    #[tokio::test]
    async fn evaluator_timeout_is_a_failed_candidate_and_not_a_score() {
        let (root, store, run_id, generator, _) = fixture_with_budget(1, 3_600, 100).await;
        generator.pause_enabled.store(false, Ordering::SeqCst);
        assert!(store.submit_method_search_run(&run_id).await.unwrap());
        assert_eq!(
            run_method_search_coordinator(
                &store,
                &root.0,
                &run_id,
                generator.as_ref(),
                &SearchTimeoutEvaluator,
            )
            .await
            .unwrap(),
            CoordinatorOutcome::Terminal
        );
        let candidate = store
            .list_method_candidates(&run_id)
            .await
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.sequence == 1)
            .unwrap();
        assert_eq!(candidate.status, MethodCandidateStatus::Failed);
        assert!(candidate.primary_score.is_none());
        assert!(candidate.error.as_deref().unwrap().contains("timed out"));
    }

    #[test]
    fn frozen_context_python_configuration_is_used_without_shell_parsing() {
        let mut context = ExecutionContext::new("local", "Local").unwrap();
        context.config_json = serde_json::json!({
            "python_executable": "/opt/wisp/python"
        })
        .to_string();
        context.capabilities_json = serde_json::json!({
            "python_executable": "/ignored/python"
        })
        .to_string();
        assert_eq!(
            configured_python(&context).unwrap().as_deref(),
            Some("/opt/wisp/python")
        );
    }
}
