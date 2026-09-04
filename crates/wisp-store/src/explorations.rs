use super::{Store, FRAME_DEFAULT_EXECUTION_CONTEXT_PREFIX};

const FRAME_SETTING_PREFIXES: &[&str] = &[
    "frame_specialist:",
    "frame_delegation_enabled:",
    "frame_plan_mode:",
    "frame_agent_completion:",
    FRAME_DEFAULT_EXECUTION_CONTEXT_PREFIX,
];
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};

pub const MAINLINE_SCOPE_KEY: &str = "mainline";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateScope {
    Mainline {
        project_id: String,
    },
    Exploration {
        project_id: String,
        exploration_id: String,
    },
}

impl StateScope {
    pub fn mainline(project_id: impl Into<String>) -> Self {
        Self::Mainline {
            project_id: project_id.into(),
        }
    }

    pub fn exploration(project_id: impl Into<String>, exploration_id: impl Into<String>) -> Self {
        Self::Exploration {
            project_id: project_id.into(),
            exploration_id: exploration_id.into(),
        }
    }

    pub fn project_id(&self) -> &str {
        match self {
            Self::Mainline { project_id } | Self::Exploration { project_id, .. } => project_id,
        }
    }

    pub fn scope_key(&self) -> &str {
        match self {
            Self::Mainline { .. } => MAINLINE_SCOPE_KEY,
            Self::Exploration { exploration_id, .. } => exploration_id,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.project_id().trim().is_empty() {
            anyhow::bail!("State scope project id is required");
        }
        if matches!(self, Self::Exploration { exploration_id, .. } if exploration_id.trim().is_empty())
        {
            anyhow::bail!("Exploration state scope id is required");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorationStatus {
    Creating,
    Active,
    Promoting,
    Failed,
}

impl ExplorationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Active => "active",
            Self::Promoting => "promoting",
            Self::Failed => "failed",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "creating" => Ok(Self::Creating),
            "active" => Ok(Self::Active),
            "promoting" => Ok(Self::Promoting),
            "failed" => Ok(Self::Failed),
            _ => anyhow::bail!("Unknown exploration status '{value}'"),
        }
    }

    fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Creating, Self::Active | Self::Failed)
                | (Self::Active, Self::Promoting)
                | (Self::Promoting, Self::Active | Self::Failed)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorationFamily {
    pub id: String,
    pub project_id: String,
    pub root_frame_id: String,
    pub mainline_frame_id: String,
    pub generation: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshotRecord {
    pub id: String,
    pub project_id: String,
    pub manifest_json: String,
    pub manifest_sha256: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextArchiveRecord {
    pub id: String,
    pub project_id: String,
    pub frame_id: String,
    pub storage_path: String,
    pub checksum: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorationCheckpoint {
    pub id: String,
    pub family_id: String,
    pub project_id: String,
    pub source_frame_id: String,
    pub source_message_seq: i64,
    pub source_frame_head_seq: i64,
    pub source_ui_event_seq: i64,
    pub source_family_generation: i64,
    pub source_state_generation: i64,
    pub workspace_snapshot_id: String,
    pub context_archive_id: String,
    pub guard_hash: String,
    pub entity_hash: String,
    pub isolation_summary_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exploration {
    pub id: String,
    pub checkpoint_id: String,
    pub frame_id: String,
    pub name: String,
    pub status: ExplorationStatus,
    pub workspace_dir: String,
    pub workspace_backend: String,
    pub scope_generation: i64,
    pub warnings_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Sidebar-oriented exploration metadata. Candidates stay grouped under the
/// source mainline while their round remains unresolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorationSummary {
    pub exploration: Exploration,
    pub source_frame_id: String,
    pub checkpoint_user_index: usize,
    pub isolation_summary_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorationBaselineEntity {
    pub checkpoint_id: String,
    pub entity_kind: String,
    pub entity_id: String,
    pub version_id: Option<String>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorationBaselineArtifactHead {
    pub checkpoint_id: String,
    pub logical_key: String,
    pub artifact_id: String,
    pub artifact_version_id: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactHead {
    pub project_id: String,
    pub scope_key: String,
    pub logical_key: String,
    pub artifact_id: String,
    pub artifact_version_id: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorationEffect {
    pub id: String,
    pub exploration_id: String,
    pub effect_kind: String,
    pub recoverability: String,
    pub target_summary: String,
    pub metadata_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorationPromotionStatus {
    Prepared,
    FilesApplied,
    MetadataCommitted,
    Committed,
    RolledBack,
    Failed,
}

impl ExplorationPromotionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::FilesApplied => "files_applied",
            Self::MetadataCommitted => "metadata_committed",
            Self::Committed => "committed",
            Self::RolledBack => "rolled_back",
            Self::Failed => "failed",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "files_applied" => Ok(Self::FilesApplied),
            "metadata_committed" => Ok(Self::MetadataCommitted),
            "committed" => Ok(Self::Committed),
            "rolled_back" => Ok(Self::RolledBack),
            "failed" => Ok(Self::Failed),
            _ => anyhow::bail!("Unknown exploration promotion status '{value}'"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorationPromotion {
    pub id: String,
    pub exploration_id: String,
    pub expected_guard_hash: String,
    pub status: ExplorationPromotionStatus,
    pub diff_json: String,
    pub journal_path: Option<String>,
    pub error: Option<String>,
    pub started_at: i64,
    pub committed_at: Option<i64>,
}

impl Store {
    pub async fn create_workspace_snapshot(
        &self,
        snapshot: &WorkspaceSnapshotRecord,
    ) -> Result<()> {
        validate_id("Workspace snapshot", &snapshot.id)?;
        validate_id("Workspace snapshot project", &snapshot.project_id)?;
        validate_sha256("Workspace snapshot manifest", &snapshot.manifest_sha256)?;
        validate_json("Workspace snapshot manifest", &snapshot.manifest_json)?;
        ensure_project_exists(&self.pool, &snapshot.project_id).await?;
        sqlx::query(
            "INSERT INTO workspace_snapshots(id,project_id,manifest_json,manifest_sha256,created_at) \
             VALUES(?,?,?,?,?)",
        )
        .bind(&snapshot.id)
        .bind(&snapshot.project_id)
        .bind(&snapshot.manifest_json)
        .bind(&snapshot.manifest_sha256)
        .bind(snapshot.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_context_archive(&self, archive: &ContextArchiveRecord) -> Result<()> {
        validate_id("Context archive", &archive.id)?;
        validate_id("Context archive project", &archive.project_id)?;
        validate_id("Context archive frame", &archive.frame_id)?;
        validate_nonempty("Context archive storage path", &archive.storage_path)?;
        validate_sha256("Context archive", &archive.checksum)?;
        ensure_frame_project(&self.pool, &archive.frame_id, &archive.project_id).await?;
        sqlx::query(
            "INSERT INTO context_archives(id,project_id,frame_id,storage_path,checksum,created_at) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&archive.id)
        .bind(&archive.project_id)
        .bind(&archive.frame_id)
        .bind(&archive.storage_path)
        .bind(&archive.checksum)
        .bind(archive.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_context_archive(
        &self,
        archive_id: &str,
    ) -> Result<Option<ContextArchiveRecord>> {
        let row = sqlx::query(
            "SELECT id,project_id,frame_id,storage_path,checksum,created_at \
             FROM context_archives WHERE id=?",
        )
        .bind(archive_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ContextArchiveRecord {
                id: row.try_get("id")?,
                project_id: row.try_get("project_id")?,
                frame_id: row.try_get("frame_id")?,
                storage_path: row.try_get("storage_path")?,
                checksum: row.try_get("checksum")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .transpose()
    }

    pub async fn create_exploration_family(&self, family: &ExplorationFamily) -> Result<()> {
        validate_id("Exploration family", &family.id)?;
        validate_id("Exploration family project", &family.project_id)?;
        if family.generation != 0 {
            anyhow::bail!("A new exploration family must start at generation zero");
        }
        ensure_frame_project(&self.pool, &family.root_frame_id, &family.project_id).await?;
        ensure_frame_project(&self.pool, &family.mainline_frame_id, &family.project_id).await?;
        sqlx::query(
            "INSERT INTO exploration_families(\
               id,project_id,root_frame_id,mainline_frame_id,generation,created_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?)",
        )
        .bind(&family.id)
        .bind(&family.project_id)
        .bind(&family.root_frame_id)
        .bind(&family.mainline_frame_id)
        .bind(family.generation)
        .bind(family.created_at)
        .bind(family.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_exploration_family(
        &self,
        family_id: &str,
    ) -> Result<Option<ExplorationFamily>> {
        let row = sqlx::query(
            "SELECT id,project_id,root_frame_id,mainline_frame_id,generation,created_at,updated_at \
             FROM exploration_families WHERE id=?",
        )
        .bind(family_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(exploration_family_from_row).transpose()
    }

    pub async fn exploration_family_for_mainline(
        &self,
        project_id: &str,
        frame_id: &str,
    ) -> Result<Option<ExplorationFamily>> {
        let row = sqlx::query(
            "SELECT id,project_id,root_frame_id,mainline_frame_id,generation,created_at,updated_at \
             FROM exploration_families WHERE project_id=? AND mainline_frame_id=?",
        )
        .bind(project_id)
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(exploration_family_from_row).transpose()
    }

    pub async fn get_workspace_snapshot_record(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<WorkspaceSnapshotRecord>> {
        let row = sqlx::query(
            "SELECT id,project_id,manifest_json,manifest_sha256,created_at \
             FROM workspace_snapshots WHERE id=?",
        )
        .bind(snapshot_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(WorkspaceSnapshotRecord {
                id: row.try_get("id")?,
                project_id: row.try_get("project_id")?,
                manifest_json: row.try_get("manifest_json")?,
                manifest_sha256: row.try_get("manifest_sha256")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .transpose()
    }

    /// Persistent snapshot manifests that still have a database owner. The
    /// desktop layer uses these to conservatively retain content-addressed
    /// blobs while removing storage for a resolved exploration round.
    pub async fn list_workspace_snapshot_manifests(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT manifest_json FROM workspace_snapshots ORDER BY id")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn frame_state_scope(&self, frame_id: &str) -> Result<Option<StateScope>> {
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT project_id,exploration_id FROM frames WHERE id=?")
                .bind(frame_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(
            row.map(|(project_id, exploration_id)| match exploration_id {
                Some(exploration_id) => StateScope::exploration(project_id, exploration_id),
                None => StateScope::mainline(project_id),
            }),
        )
    }

    pub async fn frame_message_head(&self, frame_id: &str) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM messages WHERE frame_id=?")
                .bind(frame_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn frame_ui_event_head(&self, frame_id: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq),0) FROM session_ui_events WHERE frame_id=?",
        )
        .bind(frame_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn clone_exploration_frame(
        &self,
        source_frame_id: &str,
        target_frame_id: &str,
        message_head_seq: i64,
        ui_event_head_seq: i64,
    ) -> Result<()> {
        validate_id("Exploration source frame", source_frame_id)?;
        validate_id("Exploration target frame", target_frame_id)?;
        if message_head_seq <= 0 || ui_event_head_seq < 0 {
            anyhow::bail!("Exploration clone boundary is invalid");
        }
        let mut tx = self.begin_write().await?;
        let source = sqlx::query(
            "SELECT project_id,agent_name,status,model,reasoning_effort,service_tier,input_tokens,output_tokens,completed_at,title \
             FROM frames WHERE id=? AND parent_frame_id=id",
        )
        .bind(source_frame_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Exploration source conversation was not found"))?;
        let actual_head: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM messages WHERE frame_id=?")
                .bind(source_frame_id)
                .fetch_one(&mut *tx)
                .await?;
        let actual_ui_head: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq),0) FROM session_ui_events WHERE frame_id=?",
        )
        .bind(source_frame_id)
        .fetch_one(&mut *tx)
        .await?;
        if actual_head < message_head_seq || actual_ui_head < ui_event_head_seq {
            anyhow::bail!("Exploration checkpoint history is no longer available");
        }

        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO frames(\
               id,parent_frame_id,root_frame_id,agent_name,status,project_id,branched_from,model,reasoning_effort,service_tier,\
               input_tokens,output_tokens,created_at,updated_at,completed_at,title\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(target_frame_id)
        .bind(target_frame_id)
        .bind(target_frame_id)
        .bind(source.try_get::<String, _>("agent_name")?)
        .bind(source.try_get::<String, _>("status")?)
        .bind(source.try_get::<String, _>("project_id")?)
        .bind(source_frame_id)
        .bind(source.try_get::<Option<String>, _>("model")?)
        .bind(source.try_get::<Option<String>, _>("reasoning_effort")?)
        .bind(source.try_get::<Option<String>, _>("service_tier")?)
        .bind(source.try_get::<Option<i64>, _>("input_tokens")?)
        .bind(source.try_get::<Option<i64>, _>("output_tokens")?)
        .bind(now)
        .bind(now)
        .bind(source.try_get::<Option<i64>, _>("completed_at")?)
        .bind(source.try_get::<Option<String>, _>("title")?)
        .execute(&mut *tx)
        .await?;

        let messages = sqlx::query(
            "SELECT seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name \
             FROM messages WHERE frame_id=? AND seq<=? ORDER BY seq",
        )
        .bind(source_frame_id)
        .bind(message_head_seq)
        .fetch_all(&mut *tx)
        .await?;
        for message in messages {
            sqlx::query(
                "INSERT INTO messages(\
                   id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name\
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(target_frame_id)
            .bind(message.try_get::<i64, _>("seq")?)
            .bind(message.try_get::<String, _>("role")?)
            .bind(message.try_get::<Option<String>, _>("content")?)
            .bind(message.try_get::<Option<String>, _>("tool_calls")?)
            .bind(message.try_get::<Option<String>, _>("tool_call_id")?)
            .bind(message.try_get::<Option<String>, _>("tool_name")?)
            .bind(message.try_get::<Option<String>, _>("reasoning")?)
            .bind(message.try_get::<i64, _>("ts")?)
            .bind(message.try_get::<Option<String>, _>("model_name")?)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO session_reviews(id,frame_id,message_seq,report_json,created_at,updated_at) \
             SELECT lower(hex(randomblob(16))),?,message_seq,report_json,created_at,updated_at \
             FROM session_reviews WHERE frame_id=? AND message_seq<=?",
        )
        .bind(target_frame_id)
        .bind(source_frame_id)
        .bind(message_head_seq)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO session_ui_events(frame_id,seq,event_json) \
             SELECT ?,seq,json_set(event_json,'$.frame_id',?) FROM session_ui_events \
             WHERE frame_id=? AND seq<=? ORDER BY seq",
        )
        .bind(target_frame_id)
        .bind(target_frame_id)
        .bind(source_frame_id)
        .bind(ui_event_head_seq)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO message_resource_links(\
               id,frame_id,message_seq,ordinal,original_reference,artifact_id,artifact_version_id,\
               display_name,resource_kind,mime_type,status,error,created_artifact,created_version,created_at\
             ) SELECT lower(hex(randomblob(16))),?,message_seq,ordinal,original_reference,\
                      artifact_id,artifact_version_id,display_name,resource_kind,mime_type,status,error,0,0,created_at \
               FROM message_resource_links WHERE frame_id=? AND message_seq<=?",
        )
        .bind(target_frame_id)
        .bind(source_frame_id)
        .bind(message_head_seq)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO session_execution_contexts(frame_id,context_id,created_at) \
             SELECT ?,context_id,created_at FROM session_execution_contexts WHERE frame_id=?",
        )
        .bind(target_frame_id)
        .bind(source_frame_id)
        .execute(&mut *tx)
        .await?;
        for prefix in FRAME_SETTING_PREFIXES {
            let source_key = format!("{prefix}{source_frame_id}");
            let target_key = format!("{prefix}{target_frame_id}");
            sqlx::query(
                "INSERT INTO settings(key,value) SELECT ?,value FROM settings WHERE key=? \
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            )
            .bind(target_key)
            .bind(source_key)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Convert legacy absolute compaction paths only inside known compaction
    /// markers on a freshly cloned frame. The corresponding `.wisp/history`
    /// files came from the checkpoint workspace snapshot, so the logical URI
    /// resolves against whichever WorkingProject later opens the clone.
    pub async fn rewrite_cloned_context_archive_references(
        &self,
        frame_id: &str,
        source_root: &std::path::Path,
    ) -> Result<u64> {
        validate_id("Exploration frame", frame_id)?;
        let native_prefix = source_root
            .join(".wisp")
            .join("history")
            .to_string_lossy()
            .trim_end_matches(['/', '\\'])
            .to_string()
            + std::path::MAIN_SEPARATOR_STR;
        let slash_prefix = format!(
            "{}/",
            source_root
                .to_string_lossy()
                .trim_end_matches(['/', '\\'])
                .replace('\\', "/")
        ) + ".wisp/history/";
        let rows = sqlx::query("SELECT id,content FROM messages WHERE frame_id=?")
            .bind(frame_id)
            .fetch_all(&self.pool)
            .await?;
        let mut rewritten = 0;
        let mut tx = self.begin_write().await?;
        for row in rows {
            let id: String = row.try_get("id")?;
            let encoded: String = row.try_get("content")?;
            let Ok(mut content) = serde_json::from_str::<wisp_llm::Content>(&encoded) else {
                continue;
            };
            let wisp_llm::Content::Text(text) = &mut content else {
                continue;
            };
            if !(text.starts_with("[compacted;")
                || text.starts_with("[context summary checkpoint]"))
            {
                continue;
            }
            let updated = text
                .replace(&native_prefix, "wisp-history:")
                .replace(&slash_prefix, "wisp-history:");
            if updated == *text {
                continue;
            }
            *text = updated;
            sqlx::query("UPDATE messages SET content=? WHERE id=? AND frame_id=?")
                .bind(serde_json::to_string(&content)?)
                .bind(id)
                .bind(frame_id)
                .execute(&mut *tx)
                .await?;
            rewritten += 1;
        }
        tx.commit().await?;
        Ok(rewritten)
    }

    /// Replace the model transcript copied into a not-yet-registered
    /// exploration frame with the immutable checkpoint archive. This is used
    /// when compaction has rewritten the source frame's `messages` rows after
    /// the selected turn, while its visual UI events remain available.
    pub async fn replace_exploration_clone_history(
        &self,
        frame_id: &str,
        messages: &[wisp_llm::Message],
    ) -> Result<()> {
        validate_id("Exploration frame", frame_id)?;
        if messages.is_empty() {
            anyhow::bail!("Exploration checkpoint transcript is empty");
        }
        let clone_ready: Option<i64> = sqlx::query_scalar(
            "SELECT COUNT(*) FROM frames WHERE id=? AND exploration_id IS NULL \
             AND branched_from IS NOT NULL",
        )
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?;
        if clone_ready != Some(1) {
            anyhow::bail!("Exploration frame is not a fresh clone");
        }
        sqlx::query("DELETE FROM session_reviews WHERE frame_id=?")
            .bind(frame_id)
            .execute(&self.pool)
            .await?;
        self.replace_messages(frame_id, messages).await
    }

    pub async fn create_exploration_checkpoint(
        &self,
        checkpoint: &ExplorationCheckpoint,
    ) -> Result<()> {
        validate_checkpoint(checkpoint)?;
        let family = self
            .get_exploration_family(&checkpoint.family_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Exploration family not found"))?;
        if family.project_id != checkpoint.project_id
            || family.mainline_frame_id != checkpoint.source_frame_id
            || family.generation != checkpoint.source_family_generation
        {
            anyhow::bail!("Exploration checkpoint source is not the current family mainline");
        }
        ensure_frame_project(
            &self.pool,
            &checkpoint.source_frame_id,
            &checkpoint.project_id,
        )
        .await?;
        let actual_head: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM messages WHERE frame_id=?")
                .bind(&checkpoint.source_frame_id)
                .fetch_one(&self.pool)
                .await?;
        if checkpoint.source_message_seq != checkpoint.source_frame_head_seq {
            anyhow::bail!("Exploration checkpoint message boundaries disagree");
        }
        let historical_generation: Option<i64> = sqlx::query_scalar(
            "SELECT state_generation FROM project_state_revisions \
             WHERE project_id=? AND frame_id=? AND message_seq=? AND workspace_snapshot_id=?",
        )
        .bind(&checkpoint.project_id)
        .bind(&checkpoint.source_frame_id)
        .bind(checkpoint.source_message_seq)
        .bind(&checkpoint.workspace_snapshot_id)
        .fetch_optional(&self.pool)
        .await?;
        if checkpoint.source_frame_head_seq != actual_head && historical_generation.is_none() {
            anyhow::bail!("Exploration checkpoint history has no matching project state revision");
        }
        let generation = match historical_generation {
            Some(generation) => generation,
            None => {
                self.project_state_generation(&checkpoint.project_id)
                    .await?
            }
        };
        if generation != checkpoint.source_state_generation {
            anyhow::bail!("Project mainline state changed before checkpoint creation");
        }
        let snapshot_project: Option<String> =
            sqlx::query_scalar("SELECT project_id FROM workspace_snapshots WHERE id=?")
                .bind(&checkpoint.workspace_snapshot_id)
                .fetch_optional(&self.pool)
                .await?;
        if snapshot_project.as_deref() != Some(checkpoint.project_id.as_str()) {
            anyhow::bail!("Workspace snapshot does not belong to the checkpoint project");
        }
        let archive_owner: Option<(String, String)> =
            sqlx::query_as("SELECT project_id,frame_id FROM context_archives WHERE id=?")
                .bind(&checkpoint.context_archive_id)
                .fetch_optional(&self.pool)
                .await?;
        if archive_owner.as_ref().map(|owner| (&owner.0, &owner.1))
            != Some((&checkpoint.project_id, &checkpoint.source_frame_id))
        {
            anyhow::bail!("Context archive does not belong to the checkpoint source");
        }
        sqlx::query(
            "INSERT INTO exploration_checkpoints(\
               id,family_id,project_id,source_frame_id,source_message_seq,source_frame_head_seq,\
               source_ui_event_seq,source_family_generation,source_state_generation,\
               workspace_snapshot_id,context_archive_id,guard_hash,entity_hash,\
               isolation_summary_json,created_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&checkpoint.id)
        .bind(&checkpoint.family_id)
        .bind(&checkpoint.project_id)
        .bind(&checkpoint.source_frame_id)
        .bind(checkpoint.source_message_seq)
        .bind(checkpoint.source_frame_head_seq)
        .bind(checkpoint.source_ui_event_seq)
        .bind(checkpoint.source_family_generation)
        .bind(checkpoint.source_state_generation)
        .bind(&checkpoint.workspace_snapshot_id)
        .bind(&checkpoint.context_archive_id)
        .bind(&checkpoint.guard_hash)
        .bind(&checkpoint.entity_hash)
        .bind(&checkpoint.isolation_summary_json)
        .bind(checkpoint.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_exploration_checkpoint(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<ExplorationCheckpoint>> {
        let row = sqlx::query(
            "SELECT id,family_id,project_id,source_frame_id,source_message_seq,\
                    source_frame_head_seq,source_ui_event_seq,source_family_generation,\
                    source_state_generation,workspace_snapshot_id,context_archive_id,guard_hash,\
                    entity_hash,isolation_summary_json,created_at \
             FROM exploration_checkpoints WHERE id=?",
        )
        .bind(checkpoint_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(exploration_checkpoint_from_row).transpose()
    }

    pub async fn get_exploration_checkpoint_by_guard(
        &self,
        family_id: &str,
        source_frame_id: &str,
        source_message_seq: i64,
        guard_hash: &str,
    ) -> Result<Option<ExplorationCheckpoint>> {
        let row = sqlx::query(
            "SELECT id,family_id,project_id,source_frame_id,source_message_seq,\
                    source_frame_head_seq,source_ui_event_seq,source_family_generation,\
                    source_state_generation,workspace_snapshot_id,context_archive_id,guard_hash,\
                    entity_hash,isolation_summary_json,created_at \
             FROM exploration_checkpoints \
             WHERE family_id=? AND source_frame_id=? AND source_message_seq=? AND guard_hash=?",
        )
        .bind(family_id)
        .bind(source_frame_id)
        .bind(source_message_seq)
        .bind(guard_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(exploration_checkpoint_from_row).transpose()
    }

    pub async fn current_exploration_checkpoint_for_source(
        &self,
        project_id: &str,
        source_frame_id: &str,
        family_id: &str,
        family_generation: i64,
    ) -> Result<Option<ExplorationCheckpoint>> {
        let row = sqlx::query(
            "SELECT checkpoint.id,checkpoint.family_id,checkpoint.project_id,\
                    checkpoint.source_frame_id,checkpoint.source_message_seq,\
                    checkpoint.source_frame_head_seq,checkpoint.source_ui_event_seq,\
                    checkpoint.source_family_generation,checkpoint.source_state_generation,\
                    checkpoint.workspace_snapshot_id,checkpoint.context_archive_id,\
                    checkpoint.guard_hash,checkpoint.entity_hash,\
                    checkpoint.isolation_summary_json,checkpoint.created_at \
             FROM exploration_checkpoints checkpoint \
             WHERE checkpoint.project_id=? AND checkpoint.source_frame_id=? \
               AND checkpoint.family_id=? AND checkpoint.source_family_generation=? \
               AND EXISTS(SELECT 1 FROM explorations exploration \
                          WHERE exploration.checkpoint_id=checkpoint.id \
                            AND exploration.status IN ('creating','active','promoting','failed')) \
             ORDER BY checkpoint.created_at DESC,checkpoint.id DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(source_frame_id)
        .bind(family_id)
        .bind(family_generation)
        .fetch_optional(&self.pool)
        .await?;
        row.map(exploration_checkpoint_from_row).transpose()
    }

    pub async fn create_exploration(&self, exploration: &Exploration) -> Result<()> {
        validate_exploration(exploration)?;
        if exploration.status != ExplorationStatus::Creating || exploration.scope_generation != 0 {
            anyhow::bail!("A new exploration must start in creating at generation zero");
        }
        let checkpoint = self
            .get_exploration_checkpoint(&exploration.checkpoint_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Exploration checkpoint not found"))?;
        ensure_frame_project(&self.pool, &exploration.frame_id, &checkpoint.project_id).await?;
        let mut tx = self.begin_write().await?;
        let frame_scope: Option<String> =
            sqlx::query_scalar("SELECT exploration_id FROM frames WHERE id=?")
                .bind(&exploration.frame_id)
                .fetch_one(&mut *tx)
                .await?;
        if frame_scope.is_some() {
            anyhow::bail!("Conversation frame already belongs to an exploration");
        }
        sqlx::query(
            "INSERT INTO explorations(\
               id,checkpoint_id,frame_id,name,status,workspace_dir,workspace_backend,\
               scope_generation,warnings_json,created_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&exploration.id)
        .bind(&exploration.checkpoint_id)
        .bind(&exploration.frame_id)
        .bind(exploration.name.trim())
        .bind(exploration.status.as_str())
        .bind(&exploration.workspace_dir)
        .bind(&exploration.workspace_backend)
        .bind(exploration.scope_generation)
        .bind(&exploration.warnings_json)
        .bind(exploration.created_at)
        .bind(exploration.updated_at)
        .execute(&mut *tx)
        .await?;
        let updated = sqlx::query("UPDATE frames SET exploration_id=? WHERE id=?")
            .bind(&exploration.id)
            .bind(&exploration.frame_id)
            .execute(&mut *tx)
            .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Exploration conversation frame not found");
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_exploration(&self, exploration_id: &str) -> Result<Option<Exploration>> {
        let row = sqlx::query(
            "SELECT id,checkpoint_id,frame_id,name,status,workspace_dir,workspace_backend,\
                    scope_generation,warnings_json,created_at,updated_at \
             FROM explorations WHERE id=?",
        )
        .bind(exploration_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(exploration_from_row).transpose()
    }

    pub async fn exploration_for_frame(&self, frame_id: &str) -> Result<Option<Exploration>> {
        let row = sqlx::query(
            "SELECT e.id,e.checkpoint_id,e.frame_id,e.name,e.status,e.workspace_dir,\
                    e.workspace_backend,e.scope_generation,e.warnings_json,e.created_at,e.updated_at \
             FROM explorations e JOIN frames f ON f.exploration_id=e.id \
             WHERE f.id=?",
        )
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(exploration_from_row).transpose()
    }

    pub async fn list_explorations(&self, source_frame_id: &str) -> Result<Vec<Exploration>> {
        let rows = sqlx::query(
            "SELECT e.id,e.checkpoint_id,e.frame_id,e.name,e.status,e.workspace_dir,\
                    e.workspace_backend,e.scope_generation,e.warnings_json,e.created_at,e.updated_at \
             FROM explorations e \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=e.checkpoint_id \
             WHERE checkpoint.source_frame_id=? ORDER BY e.created_at,e.id",
        )
        .bind(source_frame_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(exploration_from_row).collect()
    }

    pub async fn list_project_explorations(
        &self,
        project_id: &str,
    ) -> Result<Vec<ExplorationSummary>> {
        let rows = sqlx::query(
            "SELECT e.id,e.checkpoint_id,e.frame_id,e.name,e.status,e.workspace_dir,\
                    e.workspace_backend,e.scope_generation,e.warnings_json,e.created_at,e.updated_at,\
                    family.mainline_frame_id AS source_frame_id,\
                    MAX((SELECT COUNT(*) FROM messages source_message \
                         WHERE source_message.frame_id=checkpoint.source_frame_id \
                           AND source_message.seq<=checkpoint.source_message_seq \
                           AND source_message.role='user') - 1, 0) AS checkpoint_user_index,\
                    checkpoint.isolation_summary_json AS isolation_summary_json \
             FROM explorations e \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=e.checkpoint_id \
             JOIN exploration_families family ON family.id=checkpoint.family_id \
             WHERE checkpoint.project_id=? \
               AND e.status IN ('creating','active','promoting','failed') \
               AND checkpoint.source_frame_id=family.mainline_frame_id \
               AND checkpoint.source_family_generation=family.generation \
             ORDER BY e.created_at,e.id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let source_frame_id = row.try_get("source_frame_id")?;
                let checkpoint_user_index =
                    usize::try_from(row.try_get::<i64, _>("checkpoint_user_index")?)?;
                let isolation_summary_json = row.try_get("isolation_summary_json")?;
                Ok(ExplorationSummary {
                    exploration: exploration_from_row(row)?,
                    source_frame_id,
                    checkpoint_user_index,
                    isolation_summary_json,
                })
            })
            .collect()
    }

    /// Candidates created from the same immutable family generation. This is
    /// captured before promotion so callers can stop their in-memory workers
    /// and dispose their app-data workspaces after the metadata transaction.
    pub async fn list_exploration_round_candidates(
        &self,
        exploration_id: &str,
    ) -> Result<Vec<Exploration>> {
        let rows = sqlx::query(
            "SELECT candidate.id,candidate.checkpoint_id,candidate.frame_id,candidate.name,\
                    candidate.status,candidate.workspace_dir,candidate.workspace_backend,\
                    candidate.scope_generation,candidate.warnings_json,candidate.created_at,\
                    candidate.updated_at \
             FROM explorations selected \
             JOIN exploration_checkpoints selected_checkpoint \
               ON selected_checkpoint.id=selected.checkpoint_id \
             JOIN exploration_checkpoints candidate_checkpoint \
               ON candidate_checkpoint.family_id=selected_checkpoint.family_id \
              AND candidate_checkpoint.source_family_generation=selected_checkpoint.source_family_generation \
             JOIN explorations candidate ON candidate.checkpoint_id=candidate_checkpoint.id \
             WHERE selected.id=? \
               AND candidate.status IN ('creating','active','promoting','failed') \
             ORDER BY candidate.created_at,candidate.id",
        )
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(exploration_from_row).collect()
    }

    pub async fn discard_exploration_scope(&self, exploration_id: &str) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let row = sqlx::query(
            "SELECT checkpoint.project_id,checkpoint.family_id,\
                    checkpoint.source_family_generation,exploration.status \
             FROM explorations exploration \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
             WHERE exploration.id=?",
        )
        .bind(exploration_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Exploration not found"))?;
        let project_id: String = row.try_get("project_id")?;
        let family_id: String = row.try_get("family_id")?;
        let family_generation: i64 = row.try_get("source_family_generation")?;
        let status: String = row.try_get("status")?;
        if status != "active" {
            anyhow::bail!("Only an active exploration can be discarded");
        }
        purge_exploration_scope_in_tx(&mut tx, &project_id, exploration_id, None).await?;
        cleanup_resolved_round_metadata_in_tx(&mut tx, &family_id, family_generation).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Abandon the complete round that contains `exploration_id`. Candidate
    /// scopes are purged atomically, then the family generation advances while
    /// retaining the original mainline frame. Advancing the generation is the
    /// durable marker that releases the mainline; individually discarded or
    /// failed candidates never release it by themselves.
    pub async fn abandon_exploration_round(
        &self,
        exploration_id: &str,
    ) -> Result<Vec<Exploration>> {
        let candidates = self
            .list_exploration_round_candidates(exploration_id)
            .await?;
        if candidates.is_empty() {
            anyhow::bail!("Exploration round not found");
        }
        let mut tx = self.begin_write().await?;
        let round = sqlx::query(
            "SELECT checkpoint.project_id,checkpoint.family_id,checkpoint.source_frame_id,\
                    checkpoint.source_family_generation \
             FROM explorations exploration \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
             JOIN exploration_families family ON family.id=checkpoint.family_id \
             WHERE exploration.id=? \
               AND checkpoint.source_frame_id=family.mainline_frame_id \
               AND checkpoint.source_family_generation=family.generation",
        )
        .bind(exploration_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Exploration round is no longer current"))?;
        let project_id: String = round.try_get("project_id")?;
        let family_id: String = round.try_get("family_id")?;
        let source_frame_id: String = round.try_get("source_frame_id")?;
        let generation: i64 = round.try_get("source_family_generation")?;
        for candidate in &candidates {
            purge_exploration_scope_in_tx(&mut tx, &project_id, &candidate.id, None).await?;
        }
        let now = chrono::Utc::now().timestamp();
        let advanced = sqlx::query(
            "UPDATE exploration_families SET generation=generation+1,updated_at=? \
             WHERE id=? AND project_id=? AND mainline_frame_id=? AND generation=?",
        )
        .bind(now)
        .bind(&family_id)
        .bind(&project_id)
        .bind(&source_frame_id)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
        if advanced.rows_affected() != 1 {
            anyhow::bail!("Exploration mainline changed before the round was abandoned");
        }
        cleanup_resolved_round_metadata_in_tx(&mut tx, &family_id, generation).await?;
        tx.commit().await?;
        Ok(candidates)
    }

    pub async fn project_has_private_explorations(&self, project_id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM explorations exploration \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
             JOIN exploration_families family ON family.id=checkpoint.family_id \
             WHERE checkpoint.project_id=? \
               AND checkpoint.source_frame_id=family.mainline_frame_id \
               AND checkpoint.source_family_generation=family.generation \
               AND exploration.status IN ('creating','active','promoting','failed'))",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Whether an unresolved exploration round currently owns this project's
    /// mainline. Candidate status is deliberately irrelevant: failure or an
    /// individual discard cannot release the mainline. Only
    /// promotion or explicit round abandonment advances the family generation.
    pub async fn project_mainline_is_frozen(&self, project_id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM explorations exploration \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
             JOIN exploration_families family ON family.id=checkpoint.family_id \
             WHERE checkpoint.project_id=? \
               AND checkpoint.source_frame_id=family.mainline_frame_id \
               AND checkpoint.source_family_generation=family.generation \
               AND exploration.status IN ('creating','active','promoting','failed'))",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Whether this exact conversation is the immutable source mainline for
    /// the unresolved exploration round. Other conversations may continue to
    /// chat, but their project tools remain read-only through the project-wide
    /// freeze above.
    pub async fn mainline_frame_is_frozen(&self, frame_id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM explorations exploration \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
             JOIN exploration_families family ON family.id=checkpoint.family_id \
             WHERE checkpoint.source_frame_id=? \
               AND checkpoint.source_frame_id=family.mainline_frame_id \
               AND checkpoint.source_family_generation=family.generation \
               AND exploration.status IN ('creating','active','promoting','failed'))",
        )
        .bind(frame_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn project_has_current_exploration_for_other_source(
        &self,
        project_id: &str,
        source_frame_id: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM explorations exploration \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
             JOIN exploration_families family ON family.id=checkpoint.family_id \
             WHERE checkpoint.project_id=? \
               AND checkpoint.source_frame_id<>? \
               AND checkpoint.source_frame_id=family.mainline_frame_id \
               AND checkpoint.source_family_generation=family.generation \
               AND exploration.status IN ('creating','active','promoting','failed'))",
        )
        .bind(project_id)
        .bind(source_frame_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn transition_exploration(
        &self,
        exploration_id: &str,
        expected: ExplorationStatus,
        next: ExplorationStatus,
    ) -> Result<bool> {
        if !expected.can_transition_to(next) {
            anyhow::bail!(
                "Invalid exploration status transition: {} -> {}",
                expected.as_str(),
                next.as_str()
            );
        }
        let now = chrono::Utc::now().timestamp();
        let updated =
            sqlx::query("UPDATE explorations SET status=?,updated_at=? WHERE id=? AND status=?")
                .bind(next.as_str())
                .bind(now)
                .bind(exploration_id)
                .bind(expected.as_str())
                .execute(&self.pool)
                .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn project_state_generation(&self, project_id: &str) -> Result<i64> {
        ensure_project_exists(&self.pool, project_id).await?;
        Ok(sqlx::query_scalar(
            "SELECT mainline_generation FROM project_state_counters WHERE project_id=?",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0))
    }

    pub async fn state_generation(&self, scope: &StateScope) -> Result<i64> {
        scope.validate()?;
        match scope {
            StateScope::Mainline { project_id } => self.project_state_generation(project_id).await,
            StateScope::Exploration {
                project_id,
                exploration_id,
            } => {
                let generation: Option<i64> = sqlx::query_scalar(
                    "SELECT exploration.scope_generation FROM explorations exploration \
                     JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
                     WHERE exploration.id=? AND checkpoint.project_id=?",
                )
                .bind(exploration_id)
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?;
                generation.ok_or_else(|| anyhow::anyhow!("Exploration state scope not found"))
            }
        }
    }

    pub async fn bump_state_generation(&self, scope: &StateScope) -> Result<i64> {
        scope.validate()?;
        let mut tx = self.begin_write().await?;
        let generation = self.bump_state_generation_in_tx(&mut tx, scope).await?;
        tx.commit().await?;
        Ok(generation)
    }

    pub(crate) async fn bump_state_generation_in_tx(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        scope: &StateScope,
    ) -> Result<i64> {
        match scope {
            StateScope::Mainline { project_id } => {
                let now = chrono::Utc::now().timestamp();
                sqlx::query(
                    "INSERT INTO project_state_counters(project_id,mainline_generation,updated_at) \
                     VALUES(?,1,?) ON CONFLICT(project_id) DO UPDATE SET \
                     mainline_generation=project_state_counters.mainline_generation+1,\
                     updated_at=excluded.updated_at",
                )
                .bind(project_id)
                .bind(now)
                .execute(&mut **tx)
                .await?;
                Ok(sqlx::query_scalar(
                    "SELECT mainline_generation FROM project_state_counters WHERE project_id=?",
                )
                .bind(project_id)
                .fetch_one(&mut **tx)
                .await?)
            }
            StateScope::Exploration {
                project_id,
                exploration_id,
            } => {
                let now = chrono::Utc::now().timestamp();
                let updated = sqlx::query(
                    "UPDATE explorations SET scope_generation=scope_generation+1,updated_at=? \
                     WHERE id=? AND checkpoint_id IN (\
                       SELECT id FROM exploration_checkpoints WHERE project_id=?\
                    ) AND status IN ('creating','active','promoting')",
                )
                .bind(now)
                .bind(exploration_id)
                .bind(project_id)
                .execute(&mut **tx)
                .await?;
                if updated.rows_affected() != 1 {
                    anyhow::bail!("Exploration state scope is not writable");
                }
                Ok(
                    sqlx::query_scalar("SELECT scope_generation FROM explorations WHERE id=?")
                        .bind(exploration_id)
                        .fetch_one(&mut **tx)
                        .await?,
                )
            }
        }
    }

    pub async fn record_exploration_baseline_entity(
        &self,
        entity: &ExplorationBaselineEntity,
    ) -> Result<()> {
        validate_id("Baseline checkpoint", &entity.checkpoint_id)?;
        validate_nonempty("Baseline entity kind", &entity.entity_kind)?;
        validate_id("Baseline entity", &entity.entity_id)?;
        validate_sha256("Baseline entity fingerprint", &entity.fingerprint)?;
        ensure_checkpoint_exists(&self.pool, &entity.checkpoint_id).await?;
        sqlx::query(
            "INSERT INTO exploration_baseline_entities(\
               checkpoint_id,entity_kind,entity_id,version_id,fingerprint\
             ) VALUES(?,?,?,?,?)",
        )
        .bind(&entity.checkpoint_id)
        .bind(&entity.entity_kind)
        .bind(&entity.entity_id)
        .bind(entity.version_id.as_deref())
        .bind(&entity.fingerprint)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn capture_exploration_baseline_entities(
        &self,
        checkpoint_id: &str,
    ) -> Result<Vec<ExplorationBaselineEntity>> {
        let project_id: String =
            sqlx::query_scalar("SELECT project_id FROM exploration_checkpoints WHERE id=?")
                .bind(checkpoint_id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Exploration checkpoint not found"))?;
        let mut entities = Vec::new();
        for (kind, query) in [
            (
                "run",
                "SELECT id,status,title,COALESCE(ended_at,created_at) AS version \
                 FROM runs WHERE project_id=? AND exploration_id IS NULL \
                   AND status NOT IN ('submitted','running','cancelling') ORDER BY id",
            ),
            (
                "research_node",
                "SELECT id,kind || ':' || title AS status,metadata_json AS title,updated_at AS version \
                 FROM research_nodes WHERE project_id=? AND exploration_id IS NULL ORDER BY id",
            ),
            (
                "research_edge",
                "SELECT id,relation AS status,source_id || ':' || target_id || ':' || metadata_json AS title,created_at AS version \
                 FROM research_edges WHERE project_id=? AND exploration_id IS NULL ORDER BY id",
            ),
            (
                "external_resource",
                "SELECT id,visibility AS status,kind || ':' || uri AS title,updated_at AS version \
                 FROM external_resources WHERE project_id=? AND exploration_id IS NULL ORDER BY id",
            ),
        ] {
            for row in sqlx::query(query)
                .bind(&project_id)
                .fetch_all(&self.pool)
                .await?
            {
                let entity_id: String = row.try_get("id")?;
                let status: String = row.try_get("status")?;
                let title: String = row.try_get("title")?;
                let version: i64 = row.try_get("version")?;
                let fingerprint = hex::encode(Sha256::digest(
                    serde_json::to_vec(&serde_json::json!({
                        "kind": kind,
                        "id": &entity_id,
                        "status": &status,
                        "title": &title,
                        "version": version,
                    }))?,
                ));
                entities.push(ExplorationBaselineEntity {
                    checkpoint_id: checkpoint_id.to_string(),
                    entity_kind: kind.to_string(),
                    entity_id,
                    version_id: Some(version.to_string()),
                    fingerprint,
                });
            }
        }
        for entity in &entities {
            self.record_exploration_baseline_entity(entity).await?;
        }
        Ok(entities)
    }

    pub async fn list_exploration_baseline_entities(
        &self,
        checkpoint_id: &str,
    ) -> Result<Vec<ExplorationBaselineEntity>> {
        let rows = sqlx::query(
            "SELECT checkpoint_id,entity_kind,entity_id,version_id,fingerprint \
             FROM exploration_baseline_entities WHERE checkpoint_id=? \
             ORDER BY entity_kind,entity_id",
        )
        .bind(checkpoint_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ExplorationBaselineEntity {
                    checkpoint_id: row.try_get("checkpoint_id")?,
                    entity_kind: row.try_get("entity_kind")?,
                    entity_id: row.try_get("entity_id")?,
                    version_id: row.try_get("version_id")?,
                    fingerprint: row.try_get("fingerprint")?,
                })
            })
            .collect()
    }

    pub async fn snapshot_mainline_entities(
        &self,
        project_id: &str,
    ) -> Result<Vec<ExplorationBaselineEntity>> {
        snapshot_mainline_entities_from(&self.pool, project_id, "").await
    }

    pub async fn record_exploration_baseline_artifact_head(
        &self,
        head: &ExplorationBaselineArtifactHead,
    ) -> Result<()> {
        validate_id("Baseline checkpoint", &head.checkpoint_id)?;
        validate_nonempty("Baseline Artifact logical key", &head.logical_key)?;
        validate_sha256("Baseline Artifact fingerprint", &head.fingerprint)?;
        ensure_checkpoint_exists(&self.pool, &head.checkpoint_id).await?;
        ensure_artifact_version(&self.pool, &head.artifact_id, &head.artifact_version_id).await?;
        sqlx::query(
            "INSERT INTO exploration_baseline_artifact_heads(\
               checkpoint_id,logical_key,artifact_id,artifact_version_id,fingerprint\
             ) VALUES(?,?,?,?,?)",
        )
        .bind(&head.checkpoint_id)
        .bind(&head.logical_key)
        .bind(&head.artifact_id)
        .bind(&head.artifact_version_id)
        .bind(&head.fingerprint)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_exploration_baseline_artifact_heads(
        &self,
        checkpoint_id: &str,
    ) -> Result<Vec<ExplorationBaselineArtifactHead>> {
        let rows = sqlx::query(
            "SELECT checkpoint_id,logical_key,artifact_id,artifact_version_id,fingerprint \
             FROM exploration_baseline_artifact_heads WHERE checkpoint_id=? ORDER BY logical_key",
        )
        .bind(checkpoint_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ExplorationBaselineArtifactHead {
                    checkpoint_id: row.try_get("checkpoint_id")?,
                    logical_key: row.try_get("logical_key")?,
                    artifact_id: row.try_get("artifact_id")?,
                    artifact_version_id: row.try_get("artifact_version_id")?,
                    fingerprint: row.try_get("fingerprint")?,
                })
            })
            .collect()
    }

    pub async fn upsert_artifact_head(&self, head: &ArtifactHead) -> Result<()> {
        validate_id("Artifact head project", &head.project_id)?;
        validate_nonempty("Artifact head scope", &head.scope_key)?;
        validate_nonempty("Artifact head logical key", &head.logical_key)?;
        ensure_artifact_version(&self.pool, &head.artifact_id, &head.artifact_version_id).await?;
        let artifact_project: Option<String> =
            sqlx::query_scalar("SELECT project_id FROM artifacts WHERE id=?")
                .bind(&head.artifact_id)
                .fetch_optional(&self.pool)
                .await?;
        if artifact_project.as_deref() != Some(head.project_id.as_str()) {
            anyhow::bail!("Artifact head does not belong to its project");
        }
        if head.scope_key != MAINLINE_SCOPE_KEY {
            let exploration_project: Option<String> = sqlx::query_scalar(
                "SELECT checkpoint.project_id FROM explorations exploration \
                 JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
                 WHERE exploration.id=?",
            )
            .bind(&head.scope_key)
            .fetch_optional(&self.pool)
            .await?;
            if exploration_project.as_deref() != Some(head.project_id.as_str()) {
                anyhow::bail!("Artifact head exploration scope does not belong to its project");
            }
        }
        sqlx::query(
            "INSERT INTO artifact_heads(\
               project_id,scope_key,logical_key,artifact_id,artifact_version_id,updated_at\
             ) VALUES(?,?,?,?,?,?) ON CONFLICT(project_id,scope_key,logical_key) DO UPDATE SET \
               artifact_id=excluded.artifact_id,artifact_version_id=excluded.artifact_version_id,\
               updated_at=excluded.updated_at",
        )
        .bind(&head.project_id)
        .bind(&head.scope_key)
        .bind(&head.logical_key)
        .bind(&head.artifact_id)
        .bind(&head.artifact_version_id)
        .bind(head.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_artifact_head(
        &self,
        project_id: &str,
        scope_key: &str,
        logical_key: &str,
    ) -> Result<Option<ArtifactHead>> {
        let row = sqlx::query(
            "SELECT project_id,scope_key,logical_key,artifact_id,artifact_version_id,updated_at \
             FROM artifact_heads WHERE project_id=? AND scope_key=? AND logical_key=?",
        )
        .bind(project_id)
        .bind(scope_key)
        .bind(logical_key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(artifact_head_from_row).transpose()
    }

    pub async fn list_artifact_heads(
        &self,
        project_id: &str,
        scope_key: &str,
    ) -> Result<Vec<ArtifactHead>> {
        let rows = sqlx::query(
            "SELECT project_id,scope_key,logical_key,artifact_id,artifact_version_id,updated_at \
             FROM artifact_heads WHERE project_id=? AND scope_key=? ORDER BY logical_key",
        )
        .bind(project_id)
        .bind(scope_key)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(artifact_head_from_row).collect()
    }

    pub async fn list_exploration_effects(
        &self,
        exploration_id: &str,
    ) -> Result<Vec<ExplorationEffect>> {
        let rows = sqlx::query(
            "SELECT id,exploration_id,effect_kind,recoverability,target_summary,metadata_json,created_at \
             FROM exploration_effects WHERE exploration_id=? ORDER BY created_at,id",
        )
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ExplorationEffect {
                    id: row.try_get("id")?,
                    exploration_id: row.try_get("exploration_id")?,
                    effect_kind: row.try_get("effect_kind")?,
                    recoverability: row.try_get("recoverability")?,
                    target_summary: row.try_get("target_summary")?,
                    metadata_json: row.try_get("metadata_json")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn create_exploration_promotion(
        &self,
        promotion: &ExplorationPromotion,
    ) -> Result<()> {
        validate_id("Exploration promotion", &promotion.id)?;
        validate_id("Exploration promotion scope", &promotion.exploration_id)?;
        validate_sha256(
            "Exploration promotion guard",
            &promotion.expected_guard_hash,
        )?;
        validate_json("Exploration promotion diff", &promotion.diff_json)?;
        if promotion.status != ExplorationPromotionStatus::Prepared
            || promotion.committed_at.is_some()
        {
            anyhow::bail!("A new exploration promotion must start prepared");
        }
        if let Some(path) = promotion.journal_path.as_deref() {
            validate_nonempty("Exploration promotion journal", path)?;
        }
        sqlx::query(
            "INSERT INTO exploration_promotions(\
               id,exploration_id,expected_guard_hash,status,diff_json,journal_path,error,started_at,committed_at\
             ) VALUES(?,?,?,?,?,?,?,?,?)",
        )
        .bind(&promotion.id)
        .bind(&promotion.exploration_id)
        .bind(&promotion.expected_guard_hash)
        .bind(promotion.status.as_str())
        .bind(&promotion.diff_json)
        .bind(promotion.journal_path.as_deref())
        .bind(promotion.error.as_deref())
        .bind(promotion.started_at)
        .bind(promotion.committed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_exploration_promotion(
        &self,
        promotion_id: &str,
    ) -> Result<Option<ExplorationPromotion>> {
        let row = sqlx::query(
            "SELECT id,exploration_id,expected_guard_hash,status,diff_json,journal_path,error,started_at,committed_at \
             FROM exploration_promotions WHERE id=?",
        )
        .bind(promotion_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(exploration_promotion_from_row).transpose()
    }

    pub async fn list_incomplete_exploration_promotions(
        &self,
    ) -> Result<Vec<ExplorationPromotion>> {
        let rows = sqlx::query(
            "SELECT id,exploration_id,expected_guard_hash,status,diff_json,journal_path,error,started_at,committed_at \
             FROM exploration_promotions \
             WHERE status IN ('prepared','files_applied','metadata_committed','committed') \
             ORDER BY started_at,id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(exploration_promotion_from_row)
            .collect()
    }

    pub async fn delete_exploration_promotion(&self, promotion_id: &str) -> Result<bool> {
        Ok(sqlx::query("DELETE FROM exploration_promotions WHERE id=?")
            .bind(promotion_id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    pub async fn transition_exploration_promotion(
        &self,
        promotion_id: &str,
        expected: ExplorationPromotionStatus,
        next: ExplorationPromotionStatus,
        error: Option<&str>,
    ) -> Result<bool> {
        let committed_at = matches!(next, ExplorationPromotionStatus::Committed)
            .then(|| chrono::Utc::now().timestamp());
        let updated = sqlx::query(
            "UPDATE exploration_promotions SET status=?,error=?,\
               committed_at=COALESCE(?,committed_at) WHERE id=? AND status=?",
        )
        .bind(next.as_str())
        .bind(error)
        .bind(committed_at)
        .bind(promotion_id)
        .bind(expected.as_str())
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Atomically merges the selected exploration back into its original
    /// mainline conversation. File changes must already be durably journaled
    /// and applied. The source frame keeps its identity and ordinary branch
    /// children; only conversation rows created after the checkpoint move back
    /// from the selected clone. Every exploration in the round is then
    /// discarded, while the selected promotion row remains as the audit record.
    pub async fn commit_exploration_promotion_metadata(&self, promotion_id: &str) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let row = sqlx::query(
            "SELECT promotion.exploration_id,promotion.status,exploration.frame_id,\
                    exploration.status AS exploration_status,checkpoint.project_id,\
                    checkpoint.family_id,checkpoint.source_frame_id,\
                    checkpoint.source_frame_head_seq,checkpoint.source_ui_event_seq,\
                    checkpoint.source_family_generation \
             FROM exploration_promotions promotion \
             JOIN explorations exploration ON exploration.id=promotion.exploration_id \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id \
             WHERE promotion.id=?",
        )
        .bind(promotion_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Exploration promotion not found"))?;
        let exploration_id: String = row.try_get("exploration_id")?;
        let promotion_status: String = row.try_get("status")?;
        let exploration_status: String = row.try_get("exploration_status")?;
        let frame_id: String = row.try_get("frame_id")?;
        let project_id: String = row.try_get("project_id")?;
        let family_id: String = row.try_get("family_id")?;
        let source_frame_id: String = row.try_get("source_frame_id")?;
        let source_frame_head_seq: i64 = row.try_get("source_frame_head_seq")?;
        let source_ui_event_seq: i64 = row.try_get("source_ui_event_seq")?;
        let source_family_generation: i64 = row.try_get("source_family_generation")?;
        if promotion_status != ExplorationPromotionStatus::FilesApplied.as_str()
            || exploration_status != ExplorationStatus::Promoting.as_str()
        {
            anyhow::bail!("Exploration promotion is not ready for metadata commit");
        }

        let now = chrono::Utc::now().timestamp();
        let family = sqlx::query(
            "UPDATE exploration_families SET generation=generation+1,updated_at=? \
             WHERE id=? AND project_id=? AND mainline_frame_id=? AND generation=?",
        )
        .bind(now)
        .bind(&family_id)
        .bind(&project_id)
        .bind(&source_frame_id)
        .bind(source_family_generation)
        .execute(&mut *tx)
        .await?;
        if family.rows_affected() != 1 {
            anyhow::bail!("Exploration family mainline advanced before metadata commit");
        }

        merge_selected_exploration_into_mainline_in_tx(
            &mut tx,
            &project_id,
            &exploration_id,
            &frame_id,
            &source_frame_id,
            source_frame_head_seq,
            source_ui_event_seq,
            now,
        )
        .await?;

        sqlx::query(
            "INSERT INTO artifact_heads(project_id,scope_key,logical_key,artifact_id,artifact_version_id,updated_at) \
             SELECT project_id,?,logical_key,artifact_id,artifact_version_id,? FROM artifact_heads \
             WHERE project_id=? AND scope_key=? \
             ON CONFLICT(project_id,scope_key,logical_key) DO UPDATE SET \
               artifact_id=excluded.artifact_id,artifact_version_id=excluded.artifact_version_id,\
               updated_at=excluded.updated_at",
        )
        .bind(MAINLINE_SCOPE_KEY)
        .bind(now)
        .bind(&project_id)
        .bind(&exploration_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE artifacts SET exploration_id=NULL,root_frame_id=?,latest_version_id=COALESCE((\
               SELECT head.artifact_version_id FROM artifact_heads head \
               WHERE head.project_id=artifacts.project_id AND head.scope_key=? \
                  AND head.artifact_id=artifacts.id LIMIT 1),latest_version_id) \
             WHERE project_id=? AND exploration_id=?",
        )
        .bind(&source_frame_id)
        .bind(&exploration_id)
        .bind(&project_id)
        .bind(&exploration_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE runs SET exploration_id=NULL,frame_id=? \
             WHERE project_id=? AND exploration_id=?",
        )
        .bind(&source_frame_id)
        .bind(&project_id)
        .bind(&exploration_id)
        .execute(&mut *tx)
        .await?;
        for table in ["research_nodes", "research_edges", "external_resources"] {
            let statement = format!(
                "UPDATE {table} SET exploration_id=NULL WHERE project_id=? AND exploration_id=?"
            );
            sqlx::query(&statement)
                .bind(&project_id)
                .bind(&exploration_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM artifact_heads WHERE project_id=? AND scope_key=?")
            .bind(&project_id)
            .bind(&exploration_id)
            .execute(&mut *tx)
            .await?;

        let sibling_ids: Vec<String> = sqlx::query_scalar(
            "SELECT sibling.id FROM explorations sibling \
             JOIN exploration_checkpoints checkpoint ON checkpoint.id=sibling.checkpoint_id \
             WHERE sibling.id<>? AND checkpoint.family_id=? \
               AND checkpoint.source_family_generation=?",
        )
        .bind(&exploration_id)
        .bind(&family_id)
        .bind(source_family_generation)
        .fetch_all(&mut *tx)
        .await?;
        for sibling_id in sibling_ids {
            purge_exploration_scope_in_tx(&mut tx, &project_id, &sibling_id, None).await?;
        }
        purge_exploration_scope_in_tx(&mut tx, &project_id, &exploration_id, Some(promotion_id))
            .await?;
        cleanup_resolved_round_metadata_in_tx(&mut tx, &family_id, source_family_generation)
            .await?;
        self.bump_state_generation_in_tx(&mut tx, &StateScope::mainline(&project_id))
            .await?;
        let promotion = sqlx::query(
            "UPDATE exploration_promotions SET status='metadata_committed' \
             WHERE id=? AND status='files_applied'",
        )
        .bind(promotion_id)
        .execute(&mut *tx)
        .await?;
        if promotion.rows_affected() != 1 {
            anyhow::bail!("Exploration promotion metadata status changed concurrently");
        }
        tx.commit().await?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn merge_selected_exploration_into_mainline_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    exploration_id: &str,
    exploration_frame_id: &str,
    source_frame_id: &str,
    source_message_head: i64,
    source_ui_event_head: i64,
    now: i64,
) -> Result<()> {
    let current_message_head: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM messages WHERE frame_id=?")
            .bind(source_frame_id)
            .fetch_one(&mut **tx)
            .await?;
    let current_ui_event_head: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM session_ui_events WHERE frame_id=?")
            .bind(source_frame_id)
            .fetch_one(&mut **tx)
            .await?;
    if current_message_head != source_message_head || current_ui_event_head != source_ui_event_head
    {
        anyhow::bail!("Exploration source conversation advanced before metadata commit");
    }

    // These tables are keyed to message sequence numbers. The exploration
    // clone owns an independent copy of the checkpoint prefix, so moving the
    // whole frame would duplicate history. Only rows beyond the immutable
    // checkpoint belong to the selected exploration's result.
    for (statement, boundary) in [
        (
            "UPDATE session_reviews SET frame_id=? WHERE frame_id=? AND message_seq>?",
            source_message_head,
        ),
        (
            "UPDATE message_resource_links SET frame_id=? WHERE frame_id=? AND message_seq>?",
            source_message_head,
        ),
        (
            "UPDATE turn_file_undo SET frame_id=?,\
                 reversible=CASE WHEN before_snapshot_path IS NULL THEN reversible ELSE 0 END,\
                 reason=CASE WHEN before_snapshot_path IS NULL THEN reason \
                    ELSE 'Exploration was merged; its isolated undo snapshot was discarded' END \
             WHERE frame_id=? AND user_message_seq>?",
            source_message_head,
        ),
        (
            "UPDATE messages SET frame_id=? WHERE frame_id=? AND seq>?",
            source_message_head,
        ),
    ] {
        sqlx::query(statement)
            .bind(source_frame_id)
            .bind(exploration_frame_id)
            .bind(boundary)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query(
        "UPDATE session_ui_events SET frame_id=?,event_json=json_set(event_json,'$.frame_id',?) \
         WHERE frame_id=? AND seq>?",
    )
    .bind(source_frame_id)
    .bind(source_frame_id)
    .bind(exploration_frame_id)
    .bind(source_ui_event_head)
    .execute(&mut **tx)
    .await?;

    // Exploration-only runtime records were not cloned from the mainline and
    // can retain their stable ids while changing conversation ownership.
    let execution_offset: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(cell_index),-1)+1 FROM execution_log WHERE frame_id=?",
    )
    .bind(source_frame_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("UPDATE execution_log SET cell_index=cell_index+?,frame_id=? WHERE frame_id=?")
        .bind(execution_offset)
        .bind(source_frame_id)
        .bind(exploration_frame_id)
        .execute(&mut **tx)
        .await?;
    for table in [
        "codex_turn_configs",
        "ask_user_requests",
        "context_archives",
    ] {
        let statement = format!("UPDATE {table} SET frame_id=? WHERE frame_id=?");
        sqlx::query(&statement)
            .bind(source_frame_id)
            .bind(exploration_frame_id)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query("UPDATE global_memories SET source_frame_id=? WHERE source_frame_id=?")
        .bind(source_frame_id)
        .bind(exploration_frame_id)
        .execute(&mut **tx)
        .await?;

    // Plan revisions are frame-local counters. Offset exploration revisions so
    // historic mainline plans keep their identity and the selected plans remain
    // addressable after the merge.
    let plan_offset: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(revision),0) FROM proposed_plans WHERE frame_id=?")
            .bind(source_frame_id)
            .fetch_one(&mut **tx)
            .await?;
    sqlx::query("UPDATE proposed_plans SET revision=revision+?,frame_id=? WHERE frame_id=?")
        .bind(plan_offset)
        .bind(source_frame_id)
        .bind(exploration_frame_id)
        .execute(&mut **tx)
        .await?;

    // Execution-context selection and frame-local preferences are mutable
    // clone state: accepting an exploration accepts their final values too.
    sqlx::query("DELETE FROM session_execution_contexts WHERE frame_id=?")
        .bind(source_frame_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE session_execution_contexts SET frame_id=? WHERE frame_id=?")
        .bind(source_frame_id)
        .bind(exploration_frame_id)
        .execute(&mut **tx)
        .await?;
    for prefix in FRAME_SETTING_PREFIXES {
        let source_key = format!("{prefix}{source_frame_id}");
        let exploration_key = format!("{prefix}{exploration_frame_id}");
        sqlx::query(
            "INSERT INTO settings(key,value) SELECT ?,value FROM settings WHERE key=? \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        )
        .bind(&source_key)
        .bind(&exploration_key)
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM settings WHERE key=?")
            .bind(exploration_key)
            .execute(&mut **tx)
            .await?;
    }

    sqlx::query(
        "UPDATE frames SET model=(SELECT model FROM frames WHERE id=?),\
             reasoning_effort=(SELECT reasoning_effort FROM frames WHERE id=?),\
             service_tier=(SELECT service_tier FROM frames WHERE id=?),\
             input_tokens=(SELECT input_tokens FROM frames WHERE id=?),\
             output_tokens=(SELECT output_tokens FROM frames WHERE id=?),\
             completed_at=(SELECT completed_at FROM frames WHERE id=?),\
             seen_at=MAX(COALESCE(seen_at,0),COALESCE((SELECT seen_at FROM frames WHERE id=?),0)),\
             updated_at=? WHERE id=? AND project_id=? AND exploration_id IS NULL",
    )
    .bind(exploration_frame_id)
    .bind(exploration_frame_id)
    .bind(exploration_frame_id)
    .bind(exploration_frame_id)
    .bind(exploration_frame_id)
    .bind(exploration_frame_id)
    .bind(exploration_frame_id)
    .bind(now)
    .bind(source_frame_id)
    .bind(project_id)
    .execute(&mut **tx)
    .await?;

    // The selected scope is merged into the original mainline above, while its
    // frame remains an exploration-owned tombstone so it cannot surface as a
    // second mainline.
    let owner: Option<String> =
        sqlx::query_scalar("SELECT exploration_id FROM frames WHERE id=? AND project_id=?")
            .bind(exploration_frame_id)
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();
    if owner.as_deref() != Some(exploration_id) {
        anyhow::bail!("Selected exploration frame ownership changed before metadata commit");
    }
    Ok(())
}

/// Permanently removes an exploration, its frame, and all private records.
/// Dependencies are deleted explicitly so the whole cleanup participates in
/// the promotion transaction.
pub(crate) async fn purge_exploration_scope_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    exploration_id: &str,
    preserve_promotion_id: Option<&str>,
) -> Result<()> {
    let frame_id: String = sqlx::query_scalar(
        "SELECT frame_id FROM explorations WHERE id=? AND checkpoint_id IN (\
           SELECT id FROM exploration_checkpoints WHERE project_id=?\
         )",
    )
    .bind(exploration_id)
    .bind(project_id)
    .fetch_one(&mut **tx)
    .await?;

    let active_runs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runs WHERE exploration_id=? \
         AND status IN ('submitted','running','cancelling')",
    )
    .bind(exploration_id)
    .fetch_one(&mut **tx)
    .await?;
    if active_runs != 0 {
        anyhow::bail!("Losing exploration still has active Runs");
    }

    // Scoped graph rows can point at scoped artifacts and Runs.
    sqlx::query("DELETE FROM research_edges WHERE exploration_id=?")
        .bind(exploration_id)
        .execute(&mut **tx)
        .await?;

    // Publication evidence must never retain links to data that is about to
    // cease existing. These rows should be unreachable through normal scoped
    // UI, but explicit cleanup keeps the transaction self-contained.
    let private_binding_predicate = "binding.run_id IN (SELECT id FROM runs WHERE exploration_id=?) \
        OR binding.external_resource_id IN (SELECT id FROM external_resources WHERE exploration_id=?) \
        OR binding.artifact_version_id IN (SELECT version.id FROM artifact_versions version \
             JOIN artifacts artifact ON artifact.id=version.artifact_id WHERE artifact.exploration_id=?)";
    let binding_ids = format!(
        "SELECT binding.id FROM evidence_bindings binding WHERE {private_binding_predicate}"
    );
    sqlx::query(&format!(
        "DELETE FROM evidence_reviews WHERE binding_id IN ({binding_ids})"
    ))
    .bind(exploration_id)
    .bind(exploration_id)
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(&format!(
        "DELETE FROM evidence_supersessions WHERE old_binding_id IN ({binding_ids}) \
         OR new_binding_id IN ({binding_ids})"
    ))
    .bind(exploration_id)
    .bind(exploration_id)
    .bind(exploration_id)
    .bind(exploration_id)
    .bind(exploration_id)
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(&format!(
        "DELETE FROM evidence_bindings AS binding WHERE {private_binding_predicate}"
    ))
    .bind(exploration_id)
    .bind(exploration_id)
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM reproduction_results WHERE reproduction_run_id IN (\
           SELECT reproduction.id FROM reproduction_runs reproduction \
           JOIN runs run ON run.id=reproduction.source_run_id WHERE run.exploration_id=?\
         )",
    )
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM reproduction_runs WHERE source_run_id IN (\
           SELECT id FROM runs WHERE exploration_id=?\
         )",
    )
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM research_nodes WHERE exploration_id=?")
        .bind(exploration_id)
        .execute(&mut **tx)
        .await?;

    // Run-owned detail tables, including optional method-search records.
    for statement in [
        "DELETE FROM method_strategy_stats WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM method_candidates WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM method_candidate_blobs WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM method_search_runs WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM agent_workflow_run_activities WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM run_environment_snapshots WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM run_code_snapshots WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM run_outputs WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM run_inputs WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
        "DELETE FROM run_artifacts WHERE run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
    ] {
        sqlx::query(statement)
            .bind(exploration_id)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query(
        "UPDATE artifact_versions SET producing_run_id=NULL \
         WHERE producing_run_id IN (SELECT id FROM runs WHERE exploration_id=?)",
    )
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM runs WHERE exploration_id=?")
        .bind(exploration_id)
        .execute(&mut **tx)
        .await?;

    // Conversation links must be removed before their private Artifacts.
    for statement in [
        "DELETE FROM message_resource_links WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM artifact_heads WHERE project_id=? AND scope_key=?",
    ] {
        let mut query = sqlx::query(statement).bind(if statement.contains("project_id") {
            project_id
        } else {
            exploration_id
        });
        if statement.contains("project_id") {
            query = query.bind(exploration_id);
        }
        query.execute(&mut **tx).await?;
    }
    sqlx::query(
        "DELETE FROM artifact_dependencies WHERE artifact_version_id IN (\
           SELECT version.id FROM artifact_versions version JOIN artifacts artifact \
             ON artifact.id=version.artifact_id WHERE artifact.exploration_id=?\
         ) OR depends_on_version_id IN (\
           SELECT version.id FROM artifact_versions version JOIN artifacts artifact \
             ON artifact.id=version.artifact_id WHERE artifact.exploration_id=?\
         )",
    )
    .bind(exploration_id)
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM artifact_versions WHERE artifact_id IN (\
           SELECT id FROM artifacts WHERE exploration_id=?\
         )",
    )
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM artifacts WHERE exploration_id=?")
        .bind(exploration_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM external_resources WHERE exploration_id=?")
        .bind(exploration_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM exploration_effects WHERE exploration_id=?")
        .bind(exploration_id)
        .execute(&mut **tx)
        .await?;
    match preserve_promotion_id {
        Some(promotion_id) => {
            sqlx::query("DELETE FROM exploration_promotions WHERE exploration_id=? AND id<>?")
                .bind(exploration_id)
                .bind(promotion_id)
                .execute(&mut **tx)
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM exploration_promotions WHERE exploration_id=?")
                .bind(exploration_id)
                .execute(&mut **tx)
                .await?;
        }
    }

    // Remove frame-owned transcript and agent state before deleting the frame.
    sqlx::query(
        "UPDATE agent_workflows SET status='draft' \
         WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
    )
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE global_memories SET source_frame_id=NULL \
         WHERE source_frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
    )
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE agent_workflow_attempts SET child_frame_id=NULL \
         WHERE child_frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
    )
    .bind(exploration_id)
    .execute(&mut **tx)
    .await?;
    for statement in [
        "DELETE FROM agent_workflow_run_activities WHERE attempt_id IN (SELECT attempt.id FROM agent_workflow_attempts attempt JOIN agent_workflows workflow ON workflow.id=attempt.workflow_id WHERE workflow.frame_id IN (SELECT id FROM frames WHERE exploration_id=?))",
        "DELETE FROM agent_workflow_attempts WHERE workflow_id IN (SELECT id FROM agent_workflows WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?))",
        "DELETE FROM agent_workflow_steps WHERE workflow_id IN (SELECT id FROM agent_workflows WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?))",
    ] {
        sqlx::query(statement)
            .bind(exploration_id)
            .execute(&mut **tx)
            .await?;
    }
    for statement in [
        "DELETE FROM agent_workflow_deliveries WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM agent_workflows WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM session_branch_merges WHERE source_frame_id IN (SELECT id FROM frames WHERE exploration_id=?) OR branch_frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM message_resource_links WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM turn_file_undo WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM session_execution_contexts WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM session_reviews WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM session_ui_events WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM project_state_revisions WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM context_archives WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM proposed_plans WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM codex_turn_configs WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM acp_sessions WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM codex_imports WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM session_imports WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM ask_user_requests WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM execution_log WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
        "DELETE FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE exploration_id=?)",
    ] {
        let mut query = sqlx::query(statement).bind(exploration_id);
        if statement.contains(" OR branch_frame_id") {
            query = query.bind(exploration_id);
        }
        query.execute(&mut **tx).await?;
    }
    for prefix in FRAME_SETTING_PREFIXES {
        sqlx::query("DELETE FROM settings WHERE key=?")
            .bind(format!("{prefix}{frame_id}"))
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query("DELETE FROM explorations WHERE id=?")
        .bind(exploration_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM frames WHERE exploration_id=? OR id=?")
        .bind(exploration_id)
        .bind(&frame_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn cleanup_resolved_round_metadata_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    family_id: &str,
    generation: i64,
) -> Result<()> {
    let checkpoints = sqlx::query(
        "SELECT id,workspace_snapshot_id,context_archive_id \
         FROM exploration_checkpoints checkpoint \
         WHERE checkpoint.family_id=? AND checkpoint.source_family_generation=? \
           AND NOT EXISTS(SELECT 1 FROM explorations exploration \
                          WHERE exploration.checkpoint_id=checkpoint.id)",
    )
    .bind(family_id)
    .bind(generation)
    .fetch_all(&mut **tx)
    .await?;
    for checkpoint in checkpoints {
        let checkpoint_id: String = checkpoint.try_get("id")?;
        let snapshot_id: String = checkpoint.try_get("workspace_snapshot_id")?;
        let archive_id: String = checkpoint.try_get("context_archive_id")?;
        sqlx::query("DELETE FROM exploration_checkpoints WHERE id=?")
            .bind(&checkpoint_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "DELETE FROM workspace_snapshots WHERE id=? \
             AND NOT EXISTS(SELECT 1 FROM exploration_checkpoints WHERE workspace_snapshot_id=?) \
             AND NOT EXISTS(SELECT 1 FROM project_state_revisions WHERE workspace_snapshot_id=?)",
        )
        .bind(&snapshot_id)
        .bind(&snapshot_id)
        .bind(&snapshot_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "DELETE FROM context_archives WHERE id=? \
             AND NOT EXISTS(SELECT 1 FROM exploration_checkpoints WHERE context_archive_id=?) \
             AND NOT EXISTS(SELECT 1 FROM project_state_revisions WHERE context_archive_id=?)",
        )
        .bind(&archive_id)
        .bind(&archive_id)
        .bind(&archive_id)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query(
        "DELETE FROM exploration_families WHERE id=? \
         AND NOT EXISTS(SELECT 1 FROM exploration_checkpoints WHERE family_id=?)",
    )
    .bind(family_id)
    .bind(family_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn snapshot_mainline_entities_from<'e, E>(
    executor: E,
    project_id: &str,
    checkpoint_id: &str,
) -> Result<Vec<ExplorationBaselineEntity>>
where
    E: sqlx::Executor<'e, Database = Sqlite> + Copy,
{
    let mut entities = Vec::new();
    for (kind, query) in [
        (
            "run",
            "SELECT id,status,title,COALESCE(ended_at,created_at) AS version \
             FROM runs WHERE project_id=? AND exploration_id IS NULL \
               AND status NOT IN ('submitted','running','cancelling') ORDER BY id",
        ),
        (
            "research_node",
            "SELECT id,kind || ':' || title AS status,metadata_json AS title,updated_at AS version \
             FROM research_nodes WHERE project_id=? AND exploration_id IS NULL ORDER BY id",
        ),
        (
            "research_edge",
            "SELECT id,relation AS status,source_id || ':' || target_id || ':' || metadata_json AS title,created_at AS version \
             FROM research_edges WHERE project_id=? AND exploration_id IS NULL ORDER BY id",
        ),
        (
            "external_resource",
            "SELECT id,visibility AS status,kind || ':' || uri AS title,updated_at AS version \
             FROM external_resources WHERE project_id=? AND exploration_id IS NULL ORDER BY id",
        ),
    ] {
        for row in sqlx::query(query).bind(project_id).fetch_all(executor).await? {
            let entity_id: String = row.try_get("id")?;
            let status: String = row.try_get("status")?;
            let title: String = row.try_get("title")?;
            let version: i64 = row.try_get("version")?;
            let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(
                &serde_json::json!({
                    "kind": kind,
                    "id": &entity_id,
                    "status": &status,
                    "title": &title,
                    "version": version,
                }),
            )?));
            entities.push(ExplorationBaselineEntity {
                checkpoint_id: checkpoint_id.to_string(),
                entity_kind: kind.to_string(),
                entity_id,
                version_id: Some(version.to_string()),
                fingerprint,
            });
        }
    }
    Ok(entities)
}

fn validate_checkpoint(checkpoint: &ExplorationCheckpoint) -> Result<()> {
    for (label, value) in [
        ("Exploration checkpoint", checkpoint.id.as_str()),
        (
            "Exploration checkpoint family",
            checkpoint.family_id.as_str(),
        ),
        (
            "Exploration checkpoint project",
            checkpoint.project_id.as_str(),
        ),
        (
            "Exploration checkpoint source",
            checkpoint.source_frame_id.as_str(),
        ),
        (
            "Exploration checkpoint workspace snapshot",
            checkpoint.workspace_snapshot_id.as_str(),
        ),
        (
            "Exploration checkpoint context archive",
            checkpoint.context_archive_id.as_str(),
        ),
    ] {
        validate_id(label, value)?;
    }
    if checkpoint.source_message_seq <= 0
        || checkpoint.source_frame_head_seq <= 0
        || checkpoint.source_ui_event_seq < 0
        || checkpoint.source_family_generation < 0
        || checkpoint.source_state_generation < 0
    {
        anyhow::bail!("Exploration checkpoint sequence and generation values are invalid");
    }
    validate_sha256("Exploration checkpoint guard", &checkpoint.guard_hash)?;
    validate_sha256("Exploration checkpoint entities", &checkpoint.entity_hash)?;
    validate_json(
        "Exploration checkpoint isolation summary",
        &checkpoint.isolation_summary_json,
    )
}

fn validate_exploration(exploration: &Exploration) -> Result<()> {
    validate_id("Exploration", &exploration.id)?;
    validate_id("Exploration checkpoint", &exploration.checkpoint_id)?;
    validate_id("Exploration frame", &exploration.frame_id)?;
    validate_nonempty("Exploration name", &exploration.name)?;
    validate_nonempty("Exploration workspace", &exploration.workspace_dir)?;
    validate_nonempty(
        "Exploration workspace backend",
        &exploration.workspace_backend,
    )?;
    if exploration.scope_generation < 0 {
        anyhow::bail!("Exploration generation cannot be negative");
    }
    let warnings = serde_json::from_str::<serde_json::Value>(&exploration.warnings_json)
        .map_err(|_| anyhow::anyhow!("Exploration warnings must be valid JSON"))?;
    if !warnings.is_array() {
        anyhow::bail!("Exploration warnings must be a JSON array");
    }
    Ok(())
}

fn validate_id(label: &str, value: &str) -> Result<()> {
    validate_nonempty(label, value)?;
    if value.len() > 256 || value.chars().any(char::is_control) {
        anyhow::bail!("{label} is invalid");
    }
    Ok(())
}

fn validate_nonempty(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{label} is required");
    }
    Ok(())
}

fn validate_sha256(label: &str, value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        anyhow::bail!("{label} must be a lowercase SHA-256 digest");
    }
    Ok(())
}

fn validate_json(label: &str, value: &str) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(value)
        .map(|_| ())
        .map_err(|_| anyhow::anyhow!("{label} must be valid JSON"))
}

async fn ensure_project_exists(pool: &sqlx::SqlitePool, project_id: &str) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?)")
        .bind(project_id)
        .fetch_one(pool)
        .await?;
    if !exists {
        anyhow::bail!("Project not found");
    }
    Ok(())
}

async fn ensure_frame_project(
    pool: &sqlx::SqlitePool,
    frame_id: &str,
    project_id: &str,
) -> Result<()> {
    let owner: Option<String> = sqlx::query_scalar("SELECT project_id FROM frames WHERE id=?")
        .bind(frame_id)
        .fetch_optional(pool)
        .await?;
    if owner.as_deref() != Some(project_id) {
        anyhow::bail!("Conversation frame does not belong to the project");
    }
    Ok(())
}

async fn ensure_checkpoint_exists(pool: &sqlx::SqlitePool, checkpoint_id: &str) -> Result<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM exploration_checkpoints WHERE id=?)")
            .bind(checkpoint_id)
            .fetch_one(pool)
            .await?;
    if !exists {
        anyhow::bail!("Exploration checkpoint not found");
    }
    Ok(())
}

async fn ensure_artifact_version(
    pool: &sqlx::SqlitePool,
    artifact_id: &str,
    version_id: &str,
) -> Result<()> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM artifact_versions WHERE id=? AND artifact_id=?)",
    )
    .bind(version_id)
    .bind(artifact_id)
    .fetch_one(pool)
    .await?;
    if !exists {
        anyhow::bail!("Artifact version does not belong to the Artifact");
    }
    Ok(())
}

fn exploration_family_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ExplorationFamily> {
    Ok(ExplorationFamily {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        root_frame_id: row.try_get("root_frame_id")?,
        mainline_frame_id: row.try_get("mainline_frame_id")?,
        generation: row.try_get("generation")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn exploration_checkpoint_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ExplorationCheckpoint> {
    Ok(ExplorationCheckpoint {
        id: row.try_get("id")?,
        family_id: row.try_get("family_id")?,
        project_id: row.try_get("project_id")?,
        source_frame_id: row.try_get("source_frame_id")?,
        source_message_seq: row.try_get("source_message_seq")?,
        source_frame_head_seq: row.try_get("source_frame_head_seq")?,
        source_ui_event_seq: row.try_get("source_ui_event_seq")?,
        source_family_generation: row.try_get("source_family_generation")?,
        source_state_generation: row.try_get("source_state_generation")?,
        workspace_snapshot_id: row.try_get("workspace_snapshot_id")?,
        context_archive_id: row.try_get("context_archive_id")?,
        guard_hash: row.try_get("guard_hash")?,
        entity_hash: row.try_get("entity_hash")?,
        isolation_summary_json: row.try_get("isolation_summary_json")?,
        created_at: row.try_get("created_at")?,
    })
}

fn exploration_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Exploration> {
    let status: String = row.try_get("status")?;
    Ok(Exploration {
        id: row.try_get("id")?,
        checkpoint_id: row.try_get("checkpoint_id")?,
        frame_id: row.try_get("frame_id")?,
        name: row.try_get("name")?,
        status: ExplorationStatus::from_storage(&status)?,
        workspace_dir: row.try_get("workspace_dir")?,
        workspace_backend: row.try_get("workspace_backend")?,
        scope_generation: row.try_get("scope_generation")?,
        warnings_json: row.try_get("warnings_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn artifact_head_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ArtifactHead> {
    Ok(ArtifactHead {
        project_id: row.try_get("project_id")?,
        scope_key: row.try_get("scope_key")?,
        logical_key: row.try_get("logical_key")?,
        artifact_id: row.try_get("artifact_id")?,
        artifact_version_id: row.try_get("artifact_version_id")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn exploration_promotion_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ExplorationPromotion> {
    let status: String = row.try_get("status")?;
    Ok(ExplorationPromotion {
        id: row.try_get("id")?,
        exploration_id: row.try_get("exploration_id")?,
        expected_guard_hash: row.try_get("expected_guard_hash")?,
        status: ExplorationPromotionStatus::from_storage(&status)?,
        diff_json: row.try_get("diff_json")?,
        journal_path: row.try_get("journal_path")?,
        error: row.try_get("error")?,
        started_at: row.try_get("started_at")?,
        committed_at: row.try_get("committed_at")?,
    })
}
