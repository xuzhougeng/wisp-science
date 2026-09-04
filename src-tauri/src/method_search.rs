//! Workflow-native computational method search.
//!
//! This module owns the host-side preparation boundary: exact project files
//! become immutable ArtifactVersions before an evaluator is allowed to run.
//! The candidate loop is implemented below this boundary as a structured Run;
//! it is not an adapter for an external optimization backend.

use crate::{snapshot_store, ActiveProject, AppState};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tauri::State;
use tokio::io::{AsyncRead, AsyncReadExt};
use wisp_core::method_search::{
    inject_python_reachability_sentinel, locate_python_symbol, parse_evaluator_output,
    summarize_baseline, BaselineAuditSummary, FinalVerificationSpec, MethodSearchBudget,
    MethodSearchEvaluatorSpec, MethodSearchGuardrail, MethodSearchInput, MethodSearchMetrics,
    MethodSearchSpec, MethodSearchTarget, MethodStrategySource, ScoreDirection,
    EVALUATOR_PROTOCOL_V1, MAX_EVALUATOR_OUTPUT_BYTES, METHOD_SEARCH_SCHEMA_V1,
};
use wisp_store::{
    logical_artifact_id, ArtifactCaptureTiming, ArtifactMaterialization, ArtifactVersionDraft,
    Store,
};
use wisp_tools::{Tool, ToolEnv, ToolResult};

const MAX_AUDIT_STDERR_BYTES: usize = 32 * 1024;
const MAX_PREPARATION_INPUTS: usize = 64;
const SENTINEL_MARKER: &str = "wisp_method_search_reachability_sentinel";

pub(crate) struct PrepareMethodSearchTool {
    store: Store,
    project_id: String,
    frame_id: String,
}

impl PrepareMethodSearchTool {
    pub(crate) fn new(
        store: Store,
        project_id: impl Into<String>,
        frame_id: impl Into<String>,
    ) -> Self {
        Self {
            store,
            project_id: project_id.into(),
            frame_id: frame_id.into(),
        }
    }
}

#[async_trait]
impl Tool for PrepareMethodSearchTool {
    fn name(&self) -> &str {
        "prepare_method_search"
    }

    fn schema(&self) -> wisp_llm::ToolSchema {
        wisp_llm::ToolSchema::new(
            self.name(),
            "Freeze and audit a local computational-method evaluation contract. Snapshots the exact Python target, evaluator, and local inputs; repeats the baseline; proves candidate reachability with a temporary failure sentinel; and returns exact spec/audit ArtifactVersion IDs. This does not start candidate search or modify the target source.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "objective": {"type":"string"},
                    "target_path": {"type":"string"},
                    "target_symbol": {"type":"string"},
                    "evaluator_path": {"type":"string"},
                    "primary_metric": {"type":"string"},
                    "direction": {"type":"string","enum":["maximize","minimize"]},
                    "guardrails": {"type":"array","items":{"type":"object","properties":{"metric":{"type":"string"},"op":{"type":"string","enum":["lte","lt","gte","gt","eq"]},"value":{"type":"number"}},"required":["metric","op","value"],"additionalProperties":false}},
                    "inputs": {"type":"array","items":{"type":"object","properties":{"role":{"type":"string"},"path":{"type":"string"}},"required":["role","path"],"additionalProperties":false}},
                    "constraints": {"type":"array","items":{"type":"string"}},
                    "strategy_sources": {"type":"array","maxItems":16,"items":{"type":"object","properties":{"source_ref":{"type":"string"},"title":{"type":"string"},"summary":{"type":"string"},"category":{"type":"string","enum":["literature_or_method","diagnostic","ablation_or_simplification","alternative_family"]}},"required":["source_ref","title","summary","category"],"additionalProperties":false}},
                    "repetitions": {"type":"integer","minimum":3,"maximum":10},
                    "evaluator_timeout_seconds": {"type":"integer","minimum":1,"maximum":300},
                    "max_candidates": {"type":"integer","minimum":1,"maximum":50},
                    "max_wall_seconds": {"type":"integer","minimum":1,"maximum":604800},
                    "max_cost_microunits": {"type":"integer","minimum":1},
                    "final_verification_path": {"type":["string","null"]},
                    "python_executable": {"type":["string","null"]}
                },
                "required":["objective","target_path","target_symbol","evaluator_path","primary_metric","direction","inputs","repetitions","evaluator_timeout_seconds","max_candidates","max_wall_seconds","max_cost_microunits"],
                "additionalProperties": false
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        format!(
            "[method-search audit] {}::{}",
            args.get("target_path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            args.get("target_symbol")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
        )
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let request = match serde_json::from_value::<PrepareMethodSearchRequest>(args.clone()) {
            Ok(request) => request,
            Err(error) => {
                return ToolResult::fail(format!("prepare_method_search args error: {error}"))
            }
        };
        let configured_root = match self.store.get_project(&self.project_id).await {
            Ok(Some((_, workspace))) => PathBuf::from(workspace),
            Ok(None) => return ToolResult::fail("prepare_method_search project no longer exists"),
            Err(error) => return ToolResult::fail(error.to_string()),
        };
        let configured_root = dunce::canonicalize(&configured_root);
        let tool_root = dunce::canonicalize(env.project_root());
        if configured_root.is_err() || configured_root.ok() != tool_root.ok() {
            return ToolResult::fail(
                "prepare_method_search project root does not match tool scope",
            );
        }
        match prepare_method_search_with_evaluator(
            &self.store,
            &self.project_id,
            env.project_root(),
            &self.frame_id,
            request,
            &LocalPythonMethodSearchEvaluator,
        )
        .await
        {
            Ok(prepared) => {
                ToolResult::ok(serde_json::to_string(&prepared).unwrap_or_else(|_| "{}".into()))
            }
            Err(error) => ToolResult::fail(format!("prepare_method_search error: {error}")),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MethodSearchPreparationInput {
    pub(crate) role: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareMethodSearchRequest {
    pub(crate) objective: String,
    #[serde(alias = "target_path")]
    pub(crate) target_path: String,
    #[serde(alias = "target_symbol")]
    pub(crate) target_symbol: String,
    #[serde(alias = "evaluator_path")]
    pub(crate) evaluator_path: String,
    #[serde(alias = "primary_metric")]
    pub(crate) primary_metric: String,
    pub(crate) direction: ScoreDirection,
    #[serde(default)]
    pub(crate) guardrails: Vec<MethodSearchGuardrail>,
    pub(crate) inputs: Vec<MethodSearchPreparationInput>,
    #[serde(default)]
    pub(crate) constraints: Vec<String>,
    #[serde(default, alias = "strategy_sources")]
    pub(crate) strategy_sources: Vec<MethodStrategySource>,
    pub(crate) repetitions: u32,
    #[serde(alias = "evaluator_timeout_seconds")]
    pub(crate) evaluator_timeout_seconds: u64,
    #[serde(alias = "max_candidates")]
    pub(crate) max_candidates: u32,
    #[serde(alias = "max_wall_seconds")]
    pub(crate) max_wall_seconds: u64,
    #[serde(alias = "max_cost_microunits")]
    pub(crate) max_cost_microunits: u64,
    #[serde(default, alias = "final_verification_path")]
    pub(crate) final_verification_path: Option<String>,
    /// Optional local interpreter selected through the approved execution
    /// context. It is executable identity, not a shell command.
    #[serde(default, alias = "python_executable")]
    pub(crate) python_executable: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProtectedFileAudit {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MethodSearchAuditReport {
    pub schema: String,
    pub preparation_id: String,
    pub baseline: BaselineAuditSummary,
    pub sentinel_reachable: bool,
    pub protected_files: Vec<ProtectedFileAudit>,
    pub target_source_sha256: String,
    pub evaluator_artifact_version_id: String,
    pub findings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedMethodSearch {
    pub method_search_spec_artifact_version_id: String,
    pub method_search_spec_sha256: String,
    pub audit_report_artifact_version_id: String,
    pub audit: MethodSearchAuditReport,
}

#[derive(Debug, Clone)]
pub(crate) struct EvaluationRequest {
    pub(crate) workspace: PathBuf,
    pub(crate) evaluator_path: String,
    pub(crate) target_path: String,
    pub(crate) timeout: Duration,
    pub(crate) python_executable: Option<String>,
    pub(crate) phase: String,
    pub(crate) final_verification_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct EvaluationExecution {
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) timed_out: bool,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
}

#[async_trait]
pub(crate) trait MethodSearchEvaluator: Send + Sync {
    async fn evaluate(&self, request: &EvaluationRequest) -> Result<EvaluationExecution, String>;
}

#[derive(Debug, Default)]
pub(crate) struct LocalPythonMethodSearchEvaluator;

async fn read_bounded<R>(mut reader: R, limit: usize) -> Result<(String, bool), String>
where
    R: AsyncRead + Unpin,
{
    let mut retained = Vec::with_capacity(limit.min(8 * 1024));
    let mut truncated = false;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..count.min(remaining)]);
        truncated |= count > remaining;
    }
    Ok((String::from_utf8_lossy(&retained).into_owned(), truncated))
}

#[async_trait]
impl MethodSearchEvaluator for LocalPythonMethodSearchEvaluator {
    async fn evaluate(&self, request: &EvaluationRequest) -> Result<EvaluationExecution, String> {
        let executable = request
            .python_executable
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(if cfg!(windows) { "python" } else { "python3" });
        if executable.contains('\0') || executable.contains('\n') || executable.contains('\r') {
            return Err("Python executable identity is invalid".into());
        }
        let mut command = tokio::process::Command::new(executable);
        command
            .arg(&request.evaluator_path)
            .current_dir(&request.workspace)
            .env("WISP_METHOD_SEARCH_TARGET", &request.target_path)
            .env("WISP_METHOD_SEARCH_PHASE", &request.phase)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(path) = &request.final_verification_path {
            command.env("WISP_METHOD_SEARCH_FINAL", path);
        }
        wisp_tools::process::hide_console_async(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Unable to start the configured Python evaluator: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Evaluator stdout was not captured".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Evaluator stderr was not captured".to_string())?;
        let stdout_task = tokio::spawn(read_bounded(stdout, MAX_EVALUATOR_OUTPUT_BYTES));
        let stderr_task = tokio::spawn(read_bounded(stderr, MAX_AUDIT_STDERR_BYTES));
        let (status, timed_out) = match tokio::time::timeout(request.timeout, child.wait()).await {
            Ok(status) => (Some(status.map_err(|error| error.to_string())?), false),
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                (None, true)
            }
        };
        let (stdout, stdout_truncated) = stdout_task.await.map_err(|error| error.to_string())??;
        let (stderr, stderr_truncated) = stderr_task.await.map_err(|error| error.to_string())??;
        Ok(EvaluationExecution {
            exit_code: status.and_then(|status| status.code()),
            stdout,
            stderr,
            timed_out,
            stdout_truncated,
            stderr_truncated,
        })
    }
}

fn portable_input_path(path: &str) -> Result<String, String> {
    let value = path.trim().replace('\\', "/");
    if value.is_empty() || value.contains('\0') {
        return Err("Method-search path is empty or invalid".into());
    }
    if value.len() >= 2
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value.as_bytes()[1] == b':'
        && !cfg!(windows)
    {
        return Err("Windows absolute paths are not valid on this host".into());
    }
    Ok(value)
}

fn project_file(root: &Path, value: &str) -> Result<(PathBuf, String), String> {
    let input = portable_input_path(value)?;
    let resolved = wisp_tools::safety::validate_file_path(root, &input)?;
    if !resolved.is_file() {
        return Err(format!(
            "Method-search path '{value}' is not an existing file"
        ));
    }
    let canonical_root = dunce::canonicalize(root).map_err(|error| error.to_string())?;
    let canonical = dunce::canonicalize(&resolved).map_err(|error| error.to_string())?;
    let relative = canonical
        .strip_prefix(&canonical_root)
        .map_err(|_| format!("Method-search path '{value}' is outside the project"))?;
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "Method-search path '{value}' is outside the project"
        ));
    }
    let relative = relative.to_string_lossy().replace('\\', "/");
    Ok((canonical, relative))
}

fn ensure_bounded_file(path: &Path, label: &str, max_bytes: u64) -> Result<(), String> {
    let size = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len();
    if size > max_bytes {
        return Err(format!(
            "Method-search {label} exceeds the {max_bytes}-byte preparation limit"
        ));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn verify_hashes(root: &Path, expected: &[ProtectedFileAudit]) -> Result<(), String> {
    for item in expected {
        let (path, relative) = project_file(root, &item.path)?;
        if relative != item.path || sha256_file(&path)? != item.sha256 {
            return Err(format!(
                "Protected method-search input '{}' changed",
                item.path
            ));
        }
    }
    Ok(())
}

fn copy_into_workspace(source: &Path, workspace: &Path, relative: &str) -> Result<(), String> {
    let destination = workspace.join(relative);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn remove_audit_workspace(path: &Path) {
    if path.exists() {
        let _ = std::fs::remove_dir_all(path);
    }
}

struct AuditWorkspace {
    path: PathBuf,
}

impl Drop for AuditWorkspace {
    fn drop(&mut self) {
        remove_audit_workspace(&self.path);
    }
}

async fn save_snapshot_artifact(
    store: &Store,
    root: &Path,
    project_id: &str,
    frame_id: &str,
    logical_key: &str,
    filename: &str,
    content_type: &str,
    source: &Path,
    max_bytes: u64,
) -> Result<(String, String), String> {
    let captured = snapshot_store::capture_file(
        root,
        source,
        snapshot_store::SnapshotPolicy::UpTo(max_bytes),
    )?;
    if captured.materialization != ArtifactMaterialization::Snapshot {
        return Err(format!(
            "Method-search snapshot '{}' exceeds the {max_bytes}-byte limit",
            source.display()
        ));
    }
    let artifact_id = logical_artifact_id(project_id, logical_key);
    let version_id = store
        .save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id,
            project_id: project_id.into(),
            root_frame_id: frame_id.into(),
            filename: filename.into(),
            content_type: content_type.into(),
            storage_path: captured.storage_path,
            logical_key: Some(logical_key.into()),
            size_bytes: Some(
                i64::try_from(captured.size_bytes)
                    .map_err(|_| "Method-search snapshot is too large".to_string())?,
            ),
            checksum: Some(captured.checksum.clone()),
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Snapshot,
            capture_timing: ArtifactCaptureTiming::AtCreation,
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok((version_id, captured.checksum))
}

async fn save_json_snapshot<T: Serialize>(
    store: &Store,
    root: &Path,
    project_id: &str,
    frame_id: &str,
    preparation_id: &str,
    role: &str,
    value: &T,
) -> Result<(String, String), String> {
    let json_value = serde_json::to_value(value).map_err(|error| error.to_string())?;
    let (canonical, sha256) = wisp_store::canonical_json_sha256(&json_value);
    let temp_dir = root.join(".wisp").join("method-search").join("preparation");
    std::fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
    let path = temp_dir.join(format!("{preparation_id}-{role}.json"));
    std::fs::write(&path, canonical.as_bytes()).map_err(|error| error.to_string())?;
    let result = save_snapshot_artifact(
        store,
        root,
        project_id,
        frame_id,
        &format!("method-search:{preparation_id}:{role}"),
        &format!("{role}.json"),
        "application/json",
        &path,
        512 * 1024,
    )
    .await;
    let _ = std::fs::remove_file(&path);
    result.map(|(version_id, stored_hash)| {
        debug_assert_eq!(sha256, stored_hash);
        (version_id, sha256)
    })
}

pub(crate) fn validate_execution(
    execution: &EvaluationExecution,
    spec: &MethodSearchSpec,
) -> Result<wisp_core::method_search::EvaluatorResult, String> {
    if execution.timed_out {
        return Err("Evaluator timed out".into());
    }
    if execution.stdout_truncated || execution.stderr_truncated {
        return Err("Evaluator output exceeded its bounded audit limit".into());
    }
    if execution.exit_code != Some(0) {
        return Err(format!(
            "Evaluator exited unsuccessfully: {}",
            execution.stderr.trim()
        ));
    }
    parse_evaluator_output(&execution.stdout, spec).map_err(|error| error.to_string())
}

pub(crate) async fn prepare_method_search_with_evaluator(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    frame_id: &str,
    request: PrepareMethodSearchRequest,
    evaluator: &dyn MethodSearchEvaluator,
) -> Result<PreparedMethodSearch, String> {
    if request.inputs.is_empty() || request.inputs.len() > MAX_PREPARATION_INPUTS {
        return Err("Method-search preparation requires 1-64 exact local inputs".into());
    }
    let preparation_id = uuid::Uuid::new_v4().to_string();
    let (target_path, target_relative) = project_file(project_root, &request.target_path)?;
    ensure_bounded_file(
        &target_path,
        "target source",
        wisp_core::method_search::MAX_CANDIDATE_SOURCE_BYTES as u64,
    )?;
    let target_source = std::fs::read_to_string(&target_path)
        .map_err(|error| format!("Target source must be UTF-8 Python: {error}"))?;
    locate_python_symbol(&target_source, &request.target_symbol)
        .map_err(|error| error.to_string())?;
    let (evaluator_path, evaluator_relative) = project_file(project_root, &request.evaluator_path)?;
    ensure_bounded_file(
        &evaluator_path,
        "evaluator",
        wisp_core::method_search::MAX_CANDIDATE_SOURCE_BYTES as u64,
    )?;

    let (source_version_id, source_hash) = save_snapshot_artifact(
        store,
        project_root,
        project_id,
        frame_id,
        &format!("method-search:{preparation_id}:target"),
        target_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("target.py"),
        "text/x-python",
        &target_path,
        wisp_core::method_search::MAX_CANDIDATE_SOURCE_BYTES as u64,
    )
    .await?;
    let (evaluator_version_id, evaluator_hash) = save_snapshot_artifact(
        store,
        project_root,
        project_id,
        frame_id,
        &format!("method-search:{preparation_id}:evaluator"),
        evaluator_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("evaluate.py"),
        "text/x-python",
        &evaluator_path,
        wisp_core::method_search::MAX_CANDIDATE_SOURCE_BYTES as u64,
    )
    .await?;

    let mut protected = vec![ProtectedFileAudit {
        path: evaluator_relative.clone(),
        sha256: evaluator_hash,
    }];
    let mut inputs = Vec::with_capacity(request.inputs.len());
    let mut resolved_inputs = Vec::with_capacity(request.inputs.len());
    let mut seen_roles = std::collections::HashSet::new();
    for input in &request.inputs {
        if !seen_roles.insert(input.role.as_str()) {
            return Err(format!(
                "Method-search input role '{}' is duplicated",
                input.role
            ));
        }
        let (path, relative) = project_file(project_root, &input.path)?;
        ensure_bounded_file(
            &path,
            &format!("input '{}'", input.role),
            snapshot_store::DEFAULT_SNAPSHOT_LIMIT,
        )?;
        let (version_id, checksum) = save_snapshot_artifact(
            store,
            project_root,
            project_id,
            frame_id,
            &format!("method-search:{preparation_id}:input:{}", input.role),
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("input.bin"),
            "application/octet-stream",
            &path,
            snapshot_store::DEFAULT_SNAPSHOT_LIMIT,
        )
        .await?;
        protected.push(ProtectedFileAudit {
            path: relative.clone(),
            sha256: checksum.clone(),
        });
        inputs.push(MethodSearchInput {
            role: input.role.clone(),
            path: relative.clone(),
            artifact_version_id: Some(version_id),
            external_resource_id: None,
            checksum,
        });
        resolved_inputs.push((path, relative));
    }

    let mut resolved_final = None;
    let final_verification = if let Some(value) = request.final_verification_path.as_deref() {
        let (path, relative) = project_file(project_root, value)?;
        ensure_bounded_file(
            &path,
            "final verification input",
            snapshot_store::DEFAULT_SNAPSHOT_LIMIT,
        )?;
        let (version_id, checksum) = save_snapshot_artifact(
            store,
            project_root,
            project_id,
            frame_id,
            &format!("method-search:{preparation_id}:final-verification"),
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("final-verification.bin"),
            "application/octet-stream",
            &path,
            snapshot_store::DEFAULT_SNAPSHOT_LIMIT,
        )
        .await?;
        protected.push(ProtectedFileAudit {
            path: relative.clone(),
            sha256: checksum,
        });
        resolved_final = Some((path, relative.clone()));
        Some(FinalVerificationSpec {
            artifact_version_id: version_id,
            path: relative,
            repetitions: request.repetitions.max(5).min(10),
        })
    } else {
        None
    };

    let spec = MethodSearchSpec {
        schema: METHOD_SEARCH_SCHEMA_V1.into(),
        objective: request.objective,
        target: MethodSearchTarget {
            language: "python".into(),
            source_artifact_version_id: source_version_id,
            source_path: target_relative.clone(),
            symbol: request.target_symbol,
        },
        evaluator: MethodSearchEvaluatorSpec {
            artifact_version_id: evaluator_version_id.clone(),
            entry_path: evaluator_relative.clone(),
            repetitions: request.repetitions,
            timeout_seconds: request.evaluator_timeout_seconds,
            protocol: EVALUATOR_PROTOCOL_V1.into(),
        },
        metrics: MethodSearchMetrics {
            primary: request.primary_metric,
            direction: request.direction,
            guardrails: request.guardrails,
        },
        inputs,
        protected_paths: protected.iter().map(|item| item.path.clone()).collect(),
        constraints: request.constraints,
        strategy_sources: request.strategy_sources,
        budget: MethodSearchBudget {
            max_candidates: request.max_candidates,
            max_wall_seconds: request.max_wall_seconds,
            max_evaluator_seconds: request.evaluator_timeout_seconds,
            max_cost_microunits: request.max_cost_microunits,
        },
        final_verification,
    };
    spec.validate().map_err(|error| error.to_string())?;
    verify_hashes(project_root, &protected)?;

    let workspace_path = project_root
        .join(".wisp")
        .join("method-search")
        .join("audit")
        .join(&preparation_id);
    std::fs::create_dir_all(&workspace_path).map_err(|error| error.to_string())?;
    let workspace = AuditWorkspace {
        path: workspace_path,
    };
    copy_into_workspace(&target_path, &workspace.path, &target_relative)?;
    copy_into_workspace(&evaluator_path, &workspace.path, &evaluator_relative)?;
    for (path, relative) in &resolved_inputs {
        copy_into_workspace(path, &workspace.path, relative)?;
    }
    if let Some((path, relative)) = &resolved_final {
        copy_into_workspace(path, &workspace.path, relative)?;
    }

    let evaluation_request = EvaluationRequest {
        workspace: workspace.path.clone(),
        evaluator_path: evaluator_relative,
        target_path: target_relative.clone(),
        timeout: Duration::from_secs(request.evaluator_timeout_seconds),
        python_executable: request.python_executable,
        phase: "audit".into(),
        final_verification_path: None,
    };
    let mut baseline_results = Vec::with_capacity(request.repetitions as usize);
    for _ in 0..request.repetitions {
        verify_hashes(project_root, &protected)?;
        let execution = evaluator.evaluate(&evaluation_request).await?;
        let result = validate_execution(&execution, &spec)?;
        if !result.passes_guardrails(&spec) {
            return Err("Baseline violates one or more hard guardrails".into());
        }
        baseline_results.push(Ok(result));
        verify_hashes(project_root, &protected)?;
    }
    let baseline = summarize_baseline(request.repetitions, &baseline_results)
        .map_err(|error| error.to_string())?;

    let sentinel_source = inject_python_reachability_sentinel(&target_source, &spec.target.symbol)
        .map_err(|error| error.to_string())?;
    std::fs::write(workspace.path.join(&target_relative), sentinel_source)
        .map_err(|error| error.to_string())?;
    verify_hashes(project_root, &protected)?;
    let sentinel_execution = evaluator.evaluate(&evaluation_request).await?;
    verify_hashes(project_root, &protected)?;
    let sentinel_reachable = !sentinel_execution.timed_out
        && sentinel_execution.exit_code != Some(0)
        && (sentinel_execution.stderr.contains(SENTINEL_MARKER)
            || sentinel_execution.stdout.contains(SENTINEL_MARKER));
    if !sentinel_reachable {
        return Err(
            "Evaluator did not observe the temporary target-symbol reachability sentinel".into(),
        );
    }

    let audit = MethodSearchAuditReport {
        schema: "wisp.method-search-audit.v1".into(),
        preparation_id: preparation_id.clone(),
        baseline,
        sentinel_reachable,
        protected_files: protected,
        target_source_sha256: source_hash,
        evaluator_artifact_version_id: evaluator_version_id,
        findings: Vec::new(),
    };
    let (audit_version_id, _) = save_json_snapshot(
        store,
        project_root,
        project_id,
        frame_id,
        &preparation_id,
        "audit",
        &audit,
    )
    .await?;
    let (spec_version_id, spec_sha256) = save_json_snapshot(
        store,
        project_root,
        project_id,
        frame_id,
        &preparation_id,
        "spec",
        &spec,
    )
    .await?;
    let mut dependencies = vec![
        ("method_search_audit".to_string(), audit_version_id.clone()),
        (
            "target_source".to_string(),
            spec.target.source_artifact_version_id.clone(),
        ),
        (
            "evaluator".to_string(),
            spec.evaluator.artifact_version_id.clone(),
        ),
    ];
    dependencies.extend(spec.inputs.iter().filter_map(|input| {
        input
            .artifact_version_id
            .clone()
            .map(|id| (format!("input:{}", input.role), id))
    }));
    if let Some(final_verification) = &spec.final_verification {
        dependencies.push((
            "final_verification".into(),
            final_verification.artifact_version_id.clone(),
        ));
    }
    for (reference_name, dependency_id) in dependencies {
        store
            .save_artifact_dependency(
                &uuid::Uuid::new_v4().to_string(),
                &spec_version_id,
                &dependency_id,
                Some(&reference_name),
                wisp_store::LineageBasis::Declared,
                wisp_store::LineageConfidence::Exact,
            )
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(PreparedMethodSearch {
        method_search_spec_artifact_version_id: spec_version_id,
        method_search_spec_sha256: spec_sha256,
        audit_report_artifact_version_id: audit_version_id,
        audit,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MethodSearchRunDetails {
    pub run: wisp_store::RunRecord,
    pub state: wisp_store::MethodSearchRunState,
    pub spec: MethodSearchSpec,
    pub audit: MethodSearchAuditReport,
    pub audit_artifact_version_id: String,
    pub candidates: Vec<wisp_store::MethodCandidate>,
    pub strategies: Vec<wisp_store::MethodStrategyStat>,
    pub outputs: Vec<wisp_store::RunOutput>,
    pub activity: Option<wisp_store::AgentWorkflowRunActivity>,
}

async fn method_search_details(
    store: &Store,
    project_root: &Path,
    project_id: &str,
    run_id: &str,
) -> Result<MethodSearchRunDetails, String> {
    let run = store
        .get_run(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search Run does not exist".to_string())?;
    if run.project_id != project_id || run.kind != "method_search" {
        return Err("Method-search Run does not belong to the active project".into());
    }
    let state = store
        .get_method_search_run_state(run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Method-search Run has no durable state".to_string())?;
    let (spec, audit, audit_artifact_version_id) =
        crate::method_search_coordinator::load_method_search_review_contract(
            store,
            project_root,
            project_id,
            &state,
        )
        .await?;
    Ok(MethodSearchRunDetails {
        run,
        state,
        spec,
        audit,
        audit_artifact_version_id,
        candidates: store
            .list_method_candidates(run_id)
            .await
            .map_err(|error| error.to_string())?,
        strategies: store
            .list_method_strategy_stats(run_id)
            .await
            .map_err(|error| error.to_string())?,
        outputs: store
            .list_run_outputs(run_id)
            .await
            .map_err(|error| error.to_string())?,
        activity: store
            .get_agent_workflow_run_activity_by_run(run_id)
            .await
            .map_err(|error| error.to_string())?,
    })
}

#[tauri::command]
pub(crate) async fn get_method_search_run(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<MethodSearchRunDetails, String> {
    let project = state.require_active(window.label())?;
    method_search_details(&state.store, &project.root, &project.id, &run_id).await
}

#[tauri::command]
pub(crate) async fn pause_method_search(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<MethodSearchRunDetails, String> {
    let project = state.require_active(window.label())?;
    let _activity = state.begin_project_activity(&project.id)?;
    method_search_details(&state.store, &project.root, &project.id, &run_id).await?;
    if !state
        .store
        .request_method_search_pause(&run_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Method-search Run is not running or already has a pause request".into());
    }
    method_search_details(&state.store, &project.root, &project.id, &run_id).await
}

async fn continue_recovered_workflow(
    store: Store,
    project: ActiveProject,
    run_manager: crate::run_context::RunManager,
    runtime_manager: wisp_runtime::RuntimeManager,
    app_data: PathBuf,
    run_id: String,
) {
    let workflow_id = match crate::method_search_coordinator::settle_linked_workflow_attempt(
        &store, &run_id,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(target: "wisp", %run_id, %error, "failed to settle recovered method-search Workflow attempt");
            return;
        }
    };
    if let Some(workflow_id) = workflow_id {
        if let Err(error) = crate::delegation_runtime::resume_inline_agent_workflow(
            &store,
            project,
            run_manager,
            runtime_manager,
            app_data,
            &workflow_id,
        )
        .await
        {
            tracing::error!(target: "wisp", %run_id, %workflow_id, %error, "failed to continue Workflow after recovered method search");
        }
    }
}

#[tauri::command]
pub(crate) async fn start_method_search(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<MethodSearchRunDetails, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    if matches!(&scope, wisp_store::StateScope::Exploration { .. }) {
        return Err(
            "exploration_scope_violation: Method Search is not scope-aware inside an exploration."
                .into(),
        );
    }
    let _activity = state.begin_project_activity(&project.id)?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let model_profile_id = crate::method_search_coordinator::validate_method_search_start(
        &state.store,
        &project.root,
        &project.id,
        &run_id,
    )
    .await?;
    // Resolve the approved provider profile and keyring-backed credential
    // before leaving Draft, so a failed start never consumes search state.
    let generator = crate::method_search_coordinator::ProviderCandidateGenerator::from_profile(
        &state.store,
        &model_profile_id,
    )
    .await?;
    if crate::method_search_coordinator::method_search_is_active(&run_id) {
        if !state
            .store
            .submit_method_search_run(&run_id)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Method-search Run could not start".into());
        }
        return method_search_details(&state.store, &project.root, &project.id, &run_id).await;
    }
    let guard = crate::method_search_coordinator::ActiveMethodSearchGuard::claim(&run_id)?;
    if !state
        .store
        .submit_method_search_run(&run_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Method-search Run could not start".into());
    }
    let store = state.store.clone();
    let run_manager = state.run_manager.clone();
    let runtime_manager = state.runtime_manager.clone();
    let app_data = state.app_data.clone();
    let run_id_for_task = run_id.clone();
    let project_for_task = project.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let evaluator = crate::method_search_coordinator::local_evaluator();
        match crate::method_search_coordinator::run_method_search_coordinator(
            &store,
            &project_for_task.root,
            &run_id_for_task,
            &generator,
            &evaluator,
        )
        .await
        {
            Ok(crate::method_search_coordinator::CoordinatorOutcome::Paused) => {}
            Ok(crate::method_search_coordinator::CoordinatorOutcome::Terminal) => {
                continue_recovered_workflow(
                    store,
                    project_for_task,
                    run_manager,
                    runtime_manager,
                    app_data,
                    run_id_for_task,
                )
                .await;
            }
            Err(error) => {
                let _ = store.fail_method_search_run(&run_id_for_task, &error).await;
                continue_recovered_workflow(
                    store,
                    project_for_task,
                    run_manager,
                    runtime_manager,
                    app_data,
                    run_id_for_task,
                )
                .await;
            }
        }
    });
    method_search_details(&state.store, &project.root, &project.id, &run_id).await
}

#[tauri::command]
pub(crate) async fn resume_method_search(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<MethodSearchRunDetails, String> {
    let (project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    if matches!(&scope, wisp_store::StateScope::Exploration { .. }) {
        return Err(
            "exploration_scope_violation: Method Search is not scope-aware inside an exploration."
                .into(),
        );
    }
    let _activity = state.begin_project_activity(&project.id)?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let model_profile_id = crate::method_search_coordinator::validate_method_search_resume(
        &state.store,
        &project.root,
        &project.id,
        &run_id,
    )
    .await?;
    if crate::method_search_coordinator::method_search_is_active(&run_id) {
        if !state
            .store
            .resume_method_search_run(&run_id)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("Method-search Run could not resume".into());
        }
        return method_search_details(&state.store, &project.root, &project.id, &run_id).await;
    }
    let generator = crate::method_search_coordinator::ProviderCandidateGenerator::from_profile(
        &state.store,
        &model_profile_id,
    )
    .await?;
    let guard = crate::method_search_coordinator::ActiveMethodSearchGuard::claim(&run_id)?;
    if !state
        .store
        .resume_method_search_run(&run_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Method-search Run could not resume".into());
    }
    let store = state.store.clone();
    let run_manager = state.run_manager.clone();
    let runtime_manager = state.runtime_manager.clone();
    let app_data = state.app_data.clone();
    let run_id_for_task = run_id.clone();
    let project_for_task = project.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let evaluator = crate::method_search_coordinator::local_evaluator();
        match crate::method_search_coordinator::run_method_search_coordinator(
            &store,
            &project_for_task.root,
            &run_id_for_task,
            &generator,
            &evaluator,
        )
        .await
        {
            Ok(crate::method_search_coordinator::CoordinatorOutcome::Paused) => {}
            Ok(crate::method_search_coordinator::CoordinatorOutcome::Terminal) => {
                continue_recovered_workflow(
                    store,
                    project_for_task,
                    run_manager,
                    runtime_manager,
                    app_data,
                    run_id_for_task,
                )
                .await;
            }
            Err(error) => {
                let _ = store.fail_method_search_run(&run_id_for_task, &error).await;
                continue_recovered_workflow(
                    store,
                    project_for_task,
                    run_manager,
                    runtime_manager,
                    app_data,
                    run_id_for_task,
                )
                .await;
            }
        }
    });
    method_search_details(&state.store, &project.root, &project.id, &run_id).await
}

#[tauri::command]
pub(crate) async fn cancel_method_search(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<MethodSearchRunDetails, String> {
    let project = state.require_active(window.label())?;
    let _activity = state.begin_project_activity(&project.id)?;
    let details = method_search_details(&state.store, &project.root, &project.id, &run_id).await?;
    if details.run.status.is_terminal() {
        return Ok(details);
    }
    if !state
        .store
        .request_run_cancellation(&run_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Method-search Run could not request cancellation".into());
    }
    if matches!(
        details.run.status,
        wisp_store::RunStatus::Draft | wisp_store::RunStatus::Paused
    ) && !crate::method_search_coordinator::method_search_is_active(&run_id)
    {
        let guard = crate::method_search_coordinator::ActiveMethodSearchGuard::claim(&run_id)?;
        let store = state.store.clone();
        let run_manager = state.run_manager.clone();
        let runtime_manager = state.runtime_manager.clone();
        let app_data = state.app_data.clone();
        let project_for_task = project.clone();
        let run_id_for_task = run_id.clone();
        tauri::async_runtime::spawn(async move {
            let _guard = guard;
            if let Err(error) = crate::method_search_coordinator::finish_cancelling_method_search(
                &store,
                &run_id_for_task,
            )
            .await
            {
                let _ = store.fail_method_search_run(&run_id_for_task, &error).await;
            }
            continue_recovered_workflow(
                store,
                project_for_task,
                run_manager,
                runtime_manager,
                app_data,
                run_id_for_task,
            )
            .await;
        });
    }
    method_search_details(&state.store, &project.root, &project.id, &run_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("wisp-method-search-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Clone)]
    struct FakeEvaluator {
        project_root: PathBuf,
        protected_to_mutate: Option<PathBuf>,
        ignores_candidate: bool,
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl MethodSearchEvaluator for FakeEvaluator {
        async fn evaluate(
            &self,
            request: &EvaluationRequest,
        ) -> Result<EvaluationExecution, String> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            if let Some(relative) = &self.protected_to_mutate {
                std::fs::write(self.project_root.join(relative), b"mutated")
                    .map_err(|error| error.to_string())?;
            }
            let target = if self.ignores_candidate {
                std::fs::read_to_string(self.project_root.join(&request.target_path))
            } else {
                std::fs::read_to_string(request.workspace.join(&request.target_path))
            }
            .map_err(|error| error.to_string())?;
            if target.contains(SENTINEL_MARKER) {
                return Ok(EvaluationExecution {
                    exit_code: Some(1),
                    stderr: format!("RuntimeError: {SENTINEL_MARKER}"),
                    ..Default::default()
                });
            }
            let primary = 0.5 + (*calls as f64 - 1.0) * 0.01;
            Ok(EvaluationExecution {
                exit_code: Some(0),
                stdout: format!(
                    "wisp_evaluate: {{\"primary\":{primary},\"metrics\":{{\"accuracy\":{primary},\"runtime_seconds\":1.0}}}}"
                ),
                ..Default::default()
            })
        }
    }

    fn request() -> PrepareMethodSearchRequest {
        PrepareMethodSearchRequest {
            objective: "Improve validation accuracy".into(),
            target_path: "analysis/model.py".into(),
            target_symbol: "fit_model".into(),
            evaluator_path: "analysis/evaluate.py".into(),
            primary_metric: "accuracy".into(),
            direction: ScoreDirection::Maximize,
            guardrails: vec![MethodSearchGuardrail {
                metric: "runtime_seconds".into(),
                op: wisp_core::method_search::GuardrailOperator::Lte,
                value: 10.0,
            }],
            inputs: vec![MethodSearchPreparationInput {
                role: "search_validation".into(),
                path: "data/validation.csv".into(),
            }],
            constraints: vec!["Keep the public signature unchanged".into()],
            strategy_sources: vec![],
            repetitions: 3,
            evaluator_timeout_seconds: 30,
            max_candidates: 20,
            max_wall_seconds: 3_600,
            max_cost_microunits: 1_000_000,
            final_verification_path: None,
            python_executable: None,
        }
    }

    async fn fixture() -> (TestDirectory, Store, String, String) {
        let temp = TestDirectory::new();
        std::fs::create_dir_all(temp.path().join("analysis")).unwrap();
        std::fs::create_dir_all(temp.path().join("data")).unwrap();
        std::fs::write(
            temp.path().join("analysis/model.py"),
            "def fit_model(rows):\n    return len(rows)\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("analysis/evaluate.py"),
            "# fake evaluator\n",
        )
        .unwrap();
        std::fs::write(temp.path().join("data/validation.csv"), "x,y\n1,1\n").unwrap();
        let store = Store::open(&temp.path().join("wisp.db")).await.unwrap();
        store
            .create_project("project", "Project", temp.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .create_frame("frame", "project", "Method search", "default")
            .await
            .unwrap();
        (temp, store, "project".into(), "frame".into())
    }

    #[tokio::test]
    async fn preparation_freezes_inputs_and_proves_candidate_reachability() {
        let (temp, store, project, frame) = fixture().await;
        let evaluator = FakeEvaluator {
            project_root: temp.path().into(),
            protected_to_mutate: None,
            ignores_candidate: false,
            calls: Arc::new(Mutex::new(0)),
        };
        let prepared = prepare_method_search_with_evaluator(
            &store,
            &project,
            temp.path(),
            &frame,
            request(),
            &evaluator,
        )
        .await
        .unwrap();
        assert!(prepared.audit.sentinel_reachable);
        assert_eq!(prepared.audit.baseline.repetitions, 3);
        assert!(store
            .get_artifact_version(&prepared.method_search_spec_artifact_version_id)
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("analysis/model.py")).unwrap(),
            "def fit_model(rows):\n    return len(rows)\n"
        );
    }

    #[tokio::test]
    async fn preparation_rejects_unreachable_candidate_and_protected_mutation() {
        let (temp, store, project, frame) = fixture().await;
        let unreachable = FakeEvaluator {
            project_root: temp.path().into(),
            protected_to_mutate: None,
            ignores_candidate: true,
            calls: Arc::new(Mutex::new(0)),
        };
        assert!(prepare_method_search_with_evaluator(
            &store,
            &project,
            temp.path(),
            &frame,
            request(),
            &unreachable,
        )
        .await
        .unwrap_err()
        .contains("reachability sentinel"));

        let mutating = FakeEvaluator {
            project_root: temp.path().into(),
            protected_to_mutate: Some(PathBuf::from("analysis/evaluate.py")),
            ignores_candidate: false,
            calls: Arc::new(Mutex::new(0)),
        };
        assert!(prepare_method_search_with_evaluator(
            &store,
            &project,
            temp.path(),
            &frame,
            request(),
            &mutating,
        )
        .await
        .unwrap_err()
        .contains("Protected method-search input"));
    }

    #[tokio::test]
    async fn preparation_rejects_oversized_target_before_snapshot_or_evaluation() {
        let (temp, store, project, frame) = fixture().await;
        let target = temp.path().join("analysis/model.py");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .unwrap()
            .set_len((wisp_core::method_search::MAX_CANDIDATE_SOURCE_BYTES + 1) as u64)
            .unwrap();
        let calls = Arc::new(Mutex::new(0));
        let evaluator = FakeEvaluator {
            project_root: temp.path().into(),
            protected_to_mutate: None,
            ignores_candidate: false,
            calls: Arc::clone(&calls),
        };
        let error = prepare_method_search_with_evaluator(
            &store,
            &project,
            temp.path(),
            &frame,
            request(),
            &evaluator,
        )
        .await
        .unwrap_err();
        assert!(error.contains("target source exceeds"));
        assert_eq!(*calls.lock().unwrap(), 0);
    }

    #[test]
    fn path_normalization_handles_posix_and_windows_separators_without_escape() {
        assert_eq!(
            portable_input_path("analysis/model.py").unwrap(),
            "analysis/model.py"
        );
        assert_eq!(
            portable_input_path("analysis\\model.py").unwrap(),
            "analysis/model.py"
        );
        if !cfg!(windows) {
            assert!(portable_input_path("C:\\outside\\model.py").is_err());
        }
    }

    #[test]
    fn execution_validation_fails_closed_for_process_and_protocol_errors() {
        let mut request = request();
        request.guardrails.clear();
        let spec = MethodSearchSpec {
            schema: METHOD_SEARCH_SCHEMA_V1.into(),
            objective: request.objective,
            target: MethodSearchTarget {
                language: "python".into(),
                source_artifact_version_id: "source".into(),
                source_path: request.target_path,
                symbol: request.target_symbol,
            },
            evaluator: MethodSearchEvaluatorSpec {
                artifact_version_id: "evaluator".into(),
                entry_path: request.evaluator_path.clone(),
                repetitions: 3,
                timeout_seconds: 30,
                protocol: EVALUATOR_PROTOCOL_V1.into(),
            },
            metrics: MethodSearchMetrics {
                primary: request.primary_metric,
                direction: request.direction,
                guardrails: vec![],
            },
            inputs: vec![MethodSearchInput {
                role: "validation".into(),
                path: "data/validation.csv".into(),
                artifact_version_id: Some("data".into()),
                external_resource_id: None,
                checksum: "a".repeat(64),
            }],
            protected_paths: vec![request.evaluator_path],
            constraints: vec![],
            strategy_sources: vec![],
            budget: MethodSearchBudget {
                max_candidates: 1,
                max_wall_seconds: 60,
                max_evaluator_seconds: 30,
                max_cost_microunits: 1,
            },
            final_verification: None,
        };
        assert!(validate_execution(
            &EvaluationExecution {
                timed_out: true,
                ..Default::default()
            },
            &spec
        )
        .is_err());
        assert!(validate_execution(
            &EvaluationExecution {
                exit_code: Some(1),
                ..Default::default()
            },
            &spec
        )
        .is_err());
        assert!(validate_execution(
            &EvaluationExecution {
                exit_code: Some(0),
                stdout: "no result".into(),
                ..Default::default()
            },
            &spec
        )
        .is_err());
    }
}
