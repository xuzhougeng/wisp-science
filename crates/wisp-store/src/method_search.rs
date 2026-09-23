use super::{RunStatus, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

const MAX_METHOD_CANDIDATES: i64 = 50;
const MAX_METHOD_BLOB_BYTES: i64 = 256 * 1024;
const MAX_METRICS_JSON_BYTES: usize = 32 * 1024;
const MAX_DIAGNOSTIC_CHARS: usize = 4_000;

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_json_shape(value: &str, array: bool, max_bytes: usize) -> bool {
    value.len() <= max_bytes
        && serde_json::from_str::<serde_json::Value>(value).is_ok_and(|value| {
            if array {
                value.is_array()
            } else {
                value.is_object()
            }
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodSearchRunState {
    pub run_id: String,
    pub spec_artifact_version_id: String,
    pub spec_sha256: String,
    pub activity_version: i64,
    pub checkpoint_json: String,
    pub control_state: String,
    pub result_status: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MethodSearchRunState {
    pub fn new(
        run_id: impl Into<String>,
        spec_artifact_version_id: impl Into<String>,
        spec_sha256: impl Into<String>,
    ) -> Result<Self> {
        let now = chrono::Utc::now().timestamp();
        let value = Self {
            run_id: run_id.into(),
            spec_artifact_version_id: spec_artifact_version_id.into(),
            spec_sha256: spec_sha256.into(),
            activity_version: 1,
            checkpoint_json: "{}".into(),
            control_state: "run".into(),
            result_status: None,
            created_at: now,
            updated_at: now,
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty()
            || self.spec_artifact_version_id.trim().is_empty()
            || !valid_sha256(&self.spec_sha256)
            || self.activity_version != 1
            || !valid_json_shape(&self.checkpoint_json, false, 64 * 1024)
            || !matches!(self.control_state.as_str(), "run" | "pause_requested")
        {
            anyhow::bail!("invalid method-search Run state");
        }
        if self
            .result_status
            .as_deref()
            .is_some_and(|status| !matches!(status, "verified" | "validation_only" | "incomplete"))
        {
            anyhow::bail!("invalid method-search result status");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodCandidateStatus {
    Proposed,
    Evaluating,
    Succeeded,
    Failed,
    Rejected,
}

impl MethodCandidateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Evaluating => "evaluating",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "proposed" => Ok(Self::Proposed),
            "evaluating" => Ok(Self::Evaluating),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "rejected" => Ok(Self::Rejected),
            _ => anyhow::bail!("unknown method candidate status: {value}"),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Rejected)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodCandidateBlob {
    pub id: String,
    pub run_id: String,
    pub kind: String,
    pub checksum: String,
    pub size_bytes: i64,
    pub storage_path: String,
    pub created_at: i64,
}

impl MethodCandidateBlob {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty()
            || self.run_id.trim().is_empty()
            || !matches!(self.kind.as_str(), "source" | "patch")
            || !valid_sha256(&self.checksum)
            || !(0..=MAX_METHOD_BLOB_BYTES).contains(&self.size_bytes)
            || self.storage_path.trim().is_empty()
            || self.storage_path.len() > 2_048
        {
            anyhow::bail!("invalid method candidate blob");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodCandidate {
    pub id: String,
    pub run_id: String,
    pub parent_candidate_id: Option<String>,
    pub sequence: i64,
    pub strategy_key: String,
    pub family: String,
    pub status: MethodCandidateStatus,
    pub primary_score: Option<f64>,
    pub utility: Option<f64>,
    pub metrics_json: String,
    pub runtime_ms: Option<i64>,
    pub source_sha256: String,
    pub patch_sha256: String,
    pub source_blob_id: Option<String>,
    pub patch_blob_id: Option<String>,
    pub changed_lines: Option<i64>,
    pub dependency_count: Option<i64>,
    pub rationale: Option<String>,
    pub diagnostic_summary: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

impl MethodCandidate {
    pub fn proposed(
        id: impl Into<String>,
        run_id: impl Into<String>,
        sequence: i64,
        strategy_key: impl Into<String>,
        family: impl Into<String>,
        source_sha256: impl Into<String>,
        patch_sha256: impl Into<String>,
    ) -> Result<Self> {
        let value = Self {
            id: id.into(),
            run_id: run_id.into(),
            parent_candidate_id: None,
            sequence,
            strategy_key: strategy_key.into(),
            family: family.into(),
            status: MethodCandidateStatus::Proposed,
            primary_score: None,
            utility: None,
            metrics_json: "{}".into(),
            runtime_ms: None,
            source_sha256: source_sha256.into(),
            patch_sha256: patch_sha256.into(),
            source_blob_id: None,
            patch_blob_id: None,
            changed_lines: None,
            dependency_count: None,
            rationale: None,
            diagnostic_summary: None,
            error: None,
            created_at: chrono::Utc::now().timestamp(),
            finished_at: None,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty()
            || self.run_id.trim().is_empty()
            || !(0..=MAX_METHOD_CANDIDATES).contains(&self.sequence)
            || self.strategy_key.trim().is_empty()
            || self.strategy_key.len() > 256
            || self.family.trim().is_empty()
            || self.family.len() > 128
            || !valid_sha256(&self.source_sha256)
            || !valid_sha256(&self.patch_sha256)
            || !valid_json_shape(&self.metrics_json, false, MAX_METRICS_JSON_BYTES)
            || self.primary_score.is_some_and(|value| !value.is_finite())
            || self.utility.is_some_and(|value| !value.is_finite())
            || self.runtime_ms.is_some_and(|value| value < 0)
            || self.changed_lines.is_some_and(|value| value < 0)
            || self.dependency_count.is_some_and(|value| value < 0)
            || self
                .diagnostic_summary
                .as_deref()
                .is_some_and(|value| value.chars().count() > MAX_DIAGNOSTIC_CHARS)
            || self
                .rationale
                .as_deref()
                .is_some_and(|value| value.chars().count() > MAX_DIAGNOSTIC_CHARS)
            || self
                .error
                .as_deref()
                .is_some_and(|value| value.chars().count() > MAX_DIAGNOSTIC_CHARS)
        {
            anyhow::bail!("invalid method candidate");
        }
        if self.status == MethodCandidateStatus::Succeeded
            && (self.primary_score.is_none()
                || self.utility.is_none()
                || self.source_blob_id.is_none())
        {
            anyhow::bail!("successful method candidate requires score, utility, and source blob");
        }
        if self.status.is_terminal() && self.finished_at.is_none() {
            anyhow::bail!("terminal method candidate requires finished_at");
        }
        if !self.status.is_terminal() && self.finished_at.is_some() {
            anyhow::bail!("unfinished method candidate cannot have finished_at");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodStrategyStat {
    pub run_id: String,
    pub strategy_key: String,
    pub category: String,
    pub weight: f64,
    pub attempts: i64,
    pub improvements: i64,
    pub cumulative_reward: f64,
    pub summary: String,
    pub source_refs_json: String,
    pub updated_at: i64,
}

impl MethodStrategyStat {
    fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty()
            || self.strategy_key.trim().is_empty()
            || !matches!(
                self.category.as_str(),
                "literature_or_method"
                    | "diagnostic"
                    | "ablation_or_simplification"
                    | "alternative_family"
            )
            || !self.weight.is_finite()
            || self.weight <= 0.0
            || self.attempts < 0
            || self.improvements < 0
            || self.improvements > self.attempts
            || !self.cumulative_reward.is_finite()
            || self.summary.chars().count() > 4_000
            || !valid_json_shape(&self.source_refs_json, true, 32 * 1024)
        {
            anyhow::bail!("invalid method strategy stat");
        }
        Ok(())
    }
}

fn run_state_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<MethodSearchRunState> {
    let value = MethodSearchRunState {
        run_id: row.try_get("run_id")?,
        spec_artifact_version_id: row.try_get("spec_artifact_version_id")?,
        spec_sha256: row.try_get("spec_sha256")?,
        activity_version: row.try_get("activity_version")?,
        checkpoint_json: row.try_get("checkpoint_json")?,
        control_state: row.try_get("control_state")?,
        result_status: row.try_get("result_status")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    };
    value.validate()?;
    Ok(value)
}

fn candidate_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<MethodCandidate> {
    let value = MethodCandidate {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        parent_candidate_id: row.try_get("parent_candidate_id")?,
        sequence: row.try_get("sequence")?,
        strategy_key: row.try_get("strategy_key")?,
        family: row.try_get("family")?,
        status: MethodCandidateStatus::from_storage(&row.try_get::<String, _>("status")?)?,
        primary_score: row.try_get("primary_score")?,
        utility: row.try_get("utility")?,
        metrics_json: row.try_get("metrics_json")?,
        runtime_ms: row.try_get("runtime_ms")?,
        source_sha256: row.try_get("source_sha256")?,
        patch_sha256: row.try_get("patch_sha256")?,
        source_blob_id: row.try_get("source_blob_id")?,
        patch_blob_id: row.try_get("patch_blob_id")?,
        changed_lines: row.try_get("changed_lines")?,
        dependency_count: row.try_get("dependency_count")?,
        rationale: row.try_get("rationale")?,
        diagnostic_summary: row.try_get("diagnostic_summary")?,
        error: row.try_get("error")?,
        created_at: row.try_get("created_at")?,
        finished_at: row.try_get("finished_at")?,
    };
    value.validate()?;
    Ok(value)
}

const SELECT_RUN_STATE: &str = "SELECT run_id,spec_artifact_version_id,spec_sha256,activity_version,checkpoint_json,control_state,result_status,created_at,updated_at FROM method_search_runs";
const SELECT_CANDIDATE: &str = "SELECT id,run_id,parent_candidate_id,sequence,strategy_key,family,status,primary_score,utility,metrics_json,runtime_ms,source_sha256,patch_sha256,source_blob_id,patch_blob_id,changed_lines,dependency_count,rationale,diagnostic_summary,error,created_at,finished_at FROM method_candidates";

impl Store {
    pub async fn create_method_search_run_state(&self, state: &MethodSearchRunState) -> Result<()> {
        if let Some(store) = self.route_entity("runs", "id", &state.run_id).await? {
            return Box::pin(store.create_method_search_run_state(state)).await;
        }
        state.validate()?;
        let valid_owner: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runs run \
             JOIN artifact_versions version ON version.id=? \
             JOIN artifacts artifact ON artifact.id=version.artifact_id \
             WHERE run.id=? AND run.kind='method_search' AND run.project_id=artifact.project_id",
        )
        .bind(&state.spec_artifact_version_id)
        .bind(&state.run_id)
        .fetch_one(&self.pool)
        .await?;
        if valid_owner != 1 {
            anyhow::bail!("method-search Run and spec ArtifactVersion must share a project");
        }
        sqlx::query(
            "INSERT INTO method_search_runs(run_id,spec_artifact_version_id,spec_sha256,activity_version,checkpoint_json,control_state,result_status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
        )
        .bind(&state.run_id)
        .bind(&state.spec_artifact_version_id)
        .bind(&state.spec_sha256)
        .bind(state.activity_version)
        .bind(&state.checkpoint_json)
        .bind(&state.control_state)
        .bind(state.result_status.as_deref())
        .bind(state.created_at)
        .bind(state.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_method_search_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<MethodSearchRunState>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.get_method_search_run_state(run_id)).await;
        }
        sqlx::query(&format!("{SELECT_RUN_STATE} WHERE run_id=?"))
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(run_state_from_row)
            .transpose()
    }

    pub async fn update_method_search_checkpoint(
        &self,
        run_id: &str,
        checkpoint_json: &str,
        result_status: Option<&str>,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.update_method_search_checkpoint(
                run_id,
                checkpoint_json,
                result_status,
            ))
            .await;
        }
        let mut state = self
            .get_method_search_run_state(run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("method-search Run state does not exist"))?;
        state.checkpoint_json = checkpoint_json.into();
        state.result_status = result_status.map(str::to_string);
        state.validate()?;
        let updated = sqlx::query(
            "UPDATE method_search_runs SET checkpoint_json=?,result_status=?,updated_at=? WHERE run_id=?",
        )
        .bind(checkpoint_json)
        .bind(result_status)
        .bind(chrono::Utc::now().timestamp())
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn request_method_search_pause(&self, run_id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.request_method_search_pause(run_id)).await;
        }
        let updated = sqlx::query(
            "UPDATE method_search_runs SET control_state='pause_requested',updated_at=? \
             WHERE run_id=? AND control_state='run' AND run_id IN (\
               SELECT id FROM runs WHERE kind='method_search' AND status IN ('submitted','running'))",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn submit_method_search_run(&self, run_id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.submit_method_search_run(run_id)).await;
        }
        let updated = sqlx::query(
            "UPDATE runs SET status='submitted' WHERE id=? AND kind='method_search' \
             AND status='draft' AND lifecycle_owner IS NULL \
             AND id IN (SELECT run_id FROM method_search_runs)",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn fail_method_search_run(&self, run_id: &str, error: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.fail_method_search_run(run_id, error)).await;
        }
        if error.trim().is_empty() {
            anyhow::bail!("failing method search requires an error");
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET status='failed',started_at=COALESCE(started_at,?),ended_at=?,\
             last_poll_error=?,stderr_tail=?,lifecycle_owner=NULL,lifecycle_lease_until=NULL \
             WHERE id=? AND kind='method_search' \
             AND status IN ('draft','submitted','running','cancelling')",
        )
        .bind(now)
        .bind(now)
        .bind(error)
        .bind(error.chars().take(4_000).collect::<String>())
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn method_search_pause_requested(&self, run_id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.method_search_pause_requested(run_id)).await;
        }
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT control_state FROM method_search_runs WHERE run_id=?",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .is_some_and(|value| value == "pause_requested"))
    }

    pub async fn update_method_search_progress_owned(
        &self,
        run_id: &str,
        owner: &str,
        progress_json: &str,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.update_method_search_progress_owned(
                run_id,
                owner,
                progress_json,
            ))
            .await;
        }
        if !valid_json_shape(progress_json, false, 64 * 1024) {
            anyhow::bail!("method-search progress must be a bounded JSON object");
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET progress_json=?,last_polled_at=? WHERE id=? AND kind='method_search' \
             AND lifecycle_owner=? AND lifecycle_lease_until>? \
             AND status IN ('submitted','running','cancelling')",
        )
        .bind(progress_json)
        .bind(now)
        .bind(run_id)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn save_method_candidate_blob(&self, blob: &MethodCandidateBlob) -> Result<()> {
        if let Some(store) = self.route_entity("runs", "id", &blob.run_id).await? {
            return Box::pin(store.save_method_candidate_blob(blob)).await;
        }
        blob.validate()?;
        sqlx::query(
            "INSERT INTO method_candidate_blobs(id,run_id,kind,checksum,size_bytes,storage_path,created_at) VALUES(?,?,?,?,?,?,?)",
        )
        .bind(&blob.id)
        .bind(&blob.run_id)
        .bind(&blob.kind)
        .bind(&blob.checksum)
        .bind(blob.size_bytes)
        .bind(&blob.storage_path)
        .bind(blob.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_method_candidate_blob(
        &self,
        run_id: &str,
        kind: &str,
        checksum: &str,
    ) -> Result<Option<MethodCandidateBlob>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.find_method_candidate_blob(run_id, kind, checksum)).await;
        }
        let row = sqlx::query(
            "SELECT id,run_id,kind,checksum,size_bytes,storage_path,created_at \
             FROM method_candidate_blobs WHERE run_id=? AND kind=? AND checksum=?",
        )
        .bind(run_id)
        .bind(kind)
        .bind(checksum)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let value = MethodCandidateBlob {
                id: row.try_get("id")?,
                run_id: row.try_get("run_id")?,
                kind: row.try_get("kind")?,
                checksum: row.try_get("checksum")?,
                size_bytes: row.try_get("size_bytes")?,
                storage_path: row.try_get("storage_path")?,
                created_at: row.try_get("created_at")?,
            };
            value.validate()?;
            Ok(value)
        })
        .transpose()
    }

    pub async fn insert_method_candidate(&self, candidate: &MethodCandidate) -> Result<()> {
        if let Some(store) = self.route_entity("runs", "id", &candidate.run_id).await? {
            return Box::pin(store.insert_method_candidate(candidate)).await;
        }
        candidate.validate()?;
        if candidate.status != MethodCandidateStatus::Proposed {
            anyhow::bail!("new method candidates must start proposed");
        }
        let mut tx = self.begin_write().await?;
        if let Some(parent) = candidate.parent_candidate_id.as_deref() {
            let same_run: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM method_candidates WHERE id=? AND run_id=?",
            )
            .bind(parent)
            .bind(&candidate.run_id)
            .fetch_one(&mut *tx)
            .await?;
            if same_run != 1 {
                anyhow::bail!("method candidate parent must belong to the same Run");
            }
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM method_candidates WHERE run_id=? AND sequence>0",
        )
        .bind(&candidate.run_id)
        .fetch_one(&mut *tx)
        .await?;
        if candidate.sequence > 0 && count >= MAX_METHOD_CANDIDATES {
            anyhow::bail!("method candidate limit is exhausted");
        }
        sqlx::query(
            "INSERT INTO method_candidates(id,run_id,parent_candidate_id,sequence,strategy_key,family,status,primary_score,utility,metrics_json,runtime_ms,source_sha256,patch_sha256,source_blob_id,patch_blob_id,changed_lines,dependency_count,rationale,diagnostic_summary,error,created_at,finished_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&candidate.id)
        .bind(&candidate.run_id)
        .bind(candidate.parent_candidate_id.as_deref())
        .bind(candidate.sequence)
        .bind(&candidate.strategy_key)
        .bind(&candidate.family)
        .bind(candidate.status.as_str())
        .bind(candidate.primary_score)
        .bind(candidate.utility)
        .bind(&candidate.metrics_json)
        .bind(candidate.runtime_ms)
        .bind(&candidate.source_sha256)
        .bind(&candidate.patch_sha256)
        .bind(candidate.source_blob_id.as_deref())
        .bind(candidate.patch_blob_id.as_deref())
        .bind(candidate.changed_lines)
        .bind(candidate.dependency_count)
        .bind(candidate.rationale.as_deref())
        .bind(candidate.diagnostic_summary.as_deref())
        .bind(candidate.error.as_deref())
        .bind(candidate.created_at)
        .bind(candidate.finished_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn transition_method_candidate_to_evaluating(&self, id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("method_candidates", "id", id).await? {
            return Box::pin(store.transition_method_candidate_to_evaluating(id)).await;
        }
        let updated = sqlx::query(
            "UPDATE method_candidates SET status='evaluating' WHERE id=? AND status='proposed'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn finish_method_candidate(
        &self,
        candidate: &MethodCandidate,
        expected: MethodCandidateStatus,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", &candidate.run_id).await? {
            return Box::pin(store.finish_method_candidate(candidate, expected)).await;
        }
        candidate.validate()?;
        if !candidate.status.is_terminal()
            || !matches!(
                expected,
                MethodCandidateStatus::Proposed | MethodCandidateStatus::Evaluating
            )
        {
            anyhow::bail!("invalid method candidate terminal transition");
        }
        let updated = sqlx::query(
            "UPDATE method_candidates SET status=?,primary_score=?,utility=?,metrics_json=?,runtime_ms=?,source_sha256=?,patch_sha256=?,source_blob_id=?,patch_blob_id=?,changed_lines=?,dependency_count=?,rationale=?,diagnostic_summary=?,error=?,finished_at=? WHERE id=? AND run_id=? AND status=?",
        )
        .bind(candidate.status.as_str())
        .bind(candidate.primary_score)
        .bind(candidate.utility)
        .bind(&candidate.metrics_json)
        .bind(candidate.runtime_ms)
        .bind(&candidate.source_sha256)
        .bind(&candidate.patch_sha256)
        .bind(candidate.source_blob_id.as_deref())
        .bind(candidate.patch_blob_id.as_deref())
        .bind(candidate.changed_lines)
        .bind(candidate.dependency_count)
        .bind(candidate.rationale.as_deref())
        .bind(candidate.diagnostic_summary.as_deref())
        .bind(candidate.error.as_deref())
        .bind(candidate.finished_at)
        .bind(&candidate.id)
        .bind(&candidate.run_id)
        .bind(expected.as_str())
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn get_method_candidate(&self, id: &str) -> Result<Option<MethodCandidate>> {
        if let Some(store) = self.route_entity("method_candidates", "id", id).await? {
            return Box::pin(store.get_method_candidate(id)).await;
        }
        sqlx::query(&format!("{SELECT_CANDIDATE} WHERE id=?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(candidate_from_row)
            .transpose()
    }

    pub async fn list_method_candidates(&self, run_id: &str) -> Result<Vec<MethodCandidate>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.list_method_candidates(run_id)).await;
        }
        let rows = sqlx::query(&format!(
            "{SELECT_CANDIDATE} WHERE run_id=? ORDER BY sequence,id"
        ))
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(candidate_from_row).collect()
    }

    pub async fn upsert_method_strategy_stat(&self, stat: &MethodStrategyStat) -> Result<()> {
        if let Some(store) = self.route_entity("runs", "id", &stat.run_id).await? {
            return Box::pin(store.upsert_method_strategy_stat(stat)).await;
        }
        stat.validate()?;
        sqlx::query(
            "INSERT INTO method_strategy_stats(run_id,strategy_key,category,weight,attempts,improvements,cumulative_reward,summary,source_refs_json,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(run_id,strategy_key) DO UPDATE SET category=excluded.category,weight=excluded.weight,attempts=excluded.attempts,improvements=excluded.improvements,cumulative_reward=excluded.cumulative_reward,summary=excluded.summary,source_refs_json=excluded.source_refs_json,updated_at=excluded.updated_at",
        )
        .bind(&stat.run_id)
        .bind(&stat.strategy_key)
        .bind(&stat.category)
        .bind(stat.weight)
        .bind(stat.attempts)
        .bind(stat.improvements)
        .bind(stat.cumulative_reward)
        .bind(&stat.summary)
        .bind(&stat.source_refs_json)
        .bind(stat.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_method_strategy_stats(
        &self,
        run_id: &str,
    ) -> Result<Vec<MethodStrategyStat>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.list_method_strategy_stats(run_id)).await;
        }
        let rows = sqlx::query(
            "SELECT run_id,strategy_key,category,weight,attempts,improvements,cumulative_reward,summary,source_refs_json,updated_at FROM method_strategy_stats WHERE run_id=? ORDER BY category,strategy_key",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let value = MethodStrategyStat {
                    run_id: row.try_get("run_id")?,
                    strategy_key: row.try_get("strategy_key")?,
                    category: row.try_get("category")?,
                    weight: row.try_get("weight")?,
                    attempts: row.try_get("attempts")?,
                    improvements: row.try_get("improvements")?,
                    cumulative_reward: row.try_get("cumulative_reward")?,
                    summary: row.try_get("summary")?,
                    source_refs_json: row.try_get("source_refs_json")?,
                    updated_at: row.try_get("updated_at")?,
                };
                value.validate()?;
                Ok(value)
            })
            .collect()
    }

    pub async fn pause_method_search_run_owned(
        &self,
        run_id: &str,
        owner: &str,
        reason: &str,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.pause_method_search_run_owned(run_id, owner, reason)).await;
        }
        if reason.trim().is_empty() {
            anyhow::bail!("pausing method search requires a reason");
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET status='paused',last_poll_error=?,lifecycle_owner=NULL,lifecycle_lease_until=NULL \
             WHERE id=? AND kind='method_search' AND lifecycle_owner=? AND lifecycle_lease_until>? \
             AND status IN ('submitted','running')",
        )
        .bind(reason)
        .bind(run_id)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 1 {
            sqlx::query(
                "UPDATE method_search_runs SET control_state='run',updated_at=? WHERE run_id=?",
            )
            .bind(now)
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(updated.rows_affected() == 1)
    }

    pub async fn resume_method_search_run(&self, run_id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.resume_method_search_run(run_id)).await;
        }
        let updated = sqlx::query(
            "UPDATE runs SET status='submitted',last_poll_error=NULL,ended_at=NULL WHERE id=? AND kind='method_search' AND status='paused' AND lifecycle_owner IS NULL",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 1 {
            sqlx::query(
                "UPDATE method_search_runs SET control_state='run',updated_at=? WHERE run_id=?",
            )
            .bind(chrono::Utc::now().timestamp())
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(updated.rows_affected() == 1)
    }

    pub async fn recover_interrupted_method_search_runs(&self) -> Result<u64> {
        if let Some(stores) = self.available_projects().await? {
            let mut result = 0;
            for store in stores {
                let value = Box::pin(store.recover_interrupted_method_search_runs()).await?;
                result += value;
            }
            return Ok(result);
        }
        let updated = sqlx::query(
            "UPDATE runs SET status='paused',last_poll_error='Method search was interrupted; review the checkpoint and resume explicitly.',lifecycle_owner=NULL,lifecycle_lease_until=NULL \
             WHERE kind='method_search' AND status IN ('submitted','running') \
             AND id IN (SELECT run_id FROM method_search_runs)",
        )
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected())
    }

    /// Persist a safe restart boundary before the desktop runtime exits.
    /// Candidate checkpoints are already written after each bounded evaluation;
    /// this transition prevents a local search from being mistaken for live work
    /// after its evaluator process is terminated with the application.
    pub async fn pause_method_searches_for_shutdown(&self) -> Result<u64> {
        if let Some(stores) = self.available_projects().await? {
            let mut result = 0;
            for store in stores {
                let value = Box::pin(store.pause_method_searches_for_shutdown()).await?;
                result += value;
            }
            return Ok(result);
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let updated = sqlx::query(
            "UPDATE runs SET status='paused',last_poll_error='Method search paused during graceful application shutdown; review the checkpoint and resume explicitly.',lifecycle_owner=NULL,lifecycle_lease_until=NULL \
             WHERE kind='method_search' AND status IN ('submitted','running') \
             AND id IN (SELECT run_id FROM method_search_runs)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE method_search_runs SET control_state='run',updated_at=? \
             WHERE run_id IN (SELECT id FROM runs WHERE kind='method_search' AND status='paused')",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(updated.rows_affected())
    }

    pub async fn method_search_run_status(&self, run_id: &str) -> Result<Option<RunStatus>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.method_search_run_status(run_id)).await;
        }
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM runs WHERE id=? AND kind='method_search'",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        status.as_deref().map(RunStatus::from_storage).transpose()
    }
}
