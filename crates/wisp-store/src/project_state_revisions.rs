use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

/// Immutable turn-boundary state used to reconstruct historical explorations.
/// Workspace bytes stay deduplicated in the exploration snapshot blob store;
/// this row owns only manifests, memberships, and stable boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStateRevision {
    pub id: String,
    pub project_id: String,
    pub frame_id: String,
    pub turn_index: i64,
    pub message_seq: i64,
    pub ui_event_seq: i64,
    pub parent_revision_id: Option<String>,
    pub workspace_snapshot_id: String,
    pub workspace_manifest_sha256: String,
    pub workspace_delta_json: String,
    pub artifact_heads_json: String,
    pub entities_json: String,
    pub run_ids_json: String,
    pub decision_ids_json: String,
    pub external_effects_json: String,
    pub context_archive_id: String,
    pub state_generation: i64,
    pub is_full: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStateRevisionSummary {
    pub frame_id: String,
    pub turn_index: i64,
}

impl Store {
    pub async fn create_project_state_revision(
        &self,
        revision: &ProjectStateRevision,
    ) -> Result<bool> {
        if let Some(store) = self.route_project(&revision.project_id).await? {
            return Box::pin(store.create_project_state_revision(revision)).await;
        }
        if revision.id.trim().is_empty()
            || revision.project_id.trim().is_empty()
            || revision.frame_id.trim().is_empty()
            || revision.workspace_snapshot_id.trim().is_empty()
            || revision.context_archive_id.trim().is_empty()
            || revision.turn_index < 0
            || revision.message_seq <= 0
            || revision.ui_event_seq < 0
            || revision.state_generation < 0
        {
            anyhow::bail!("Project state revision boundary is invalid");
        }
        if revision.workspace_manifest_sha256.len() != 64
            || !revision
                .workspace_manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            anyhow::bail!("Project state revision manifest checksum is invalid");
        }
        for (label, value) in [
            ("workspace delta", &revision.workspace_delta_json),
            ("Artifact heads", &revision.artifact_heads_json),
            ("entities", &revision.entities_json),
            ("Run ids", &revision.run_ids_json),
            ("Decision ids", &revision.decision_ids_json),
            ("external effects", &revision.external_effects_json),
        ] {
            serde_json::from_str::<serde_json::Value>(value).map_err(|error| {
                anyhow::anyhow!("Project state revision {label} is invalid: {error}")
            })?;
        }
        let owner: Option<String> = sqlx::query_scalar("SELECT project_id FROM frames WHERE id=?")
            .bind(&revision.frame_id)
            .fetch_optional(&self.pool)
            .await?;
        if owner.as_deref() != Some(revision.project_id.as_str()) {
            anyhow::bail!("Project state revision frame does not belong to its project");
        }
        let snapshot_owner: Option<String> =
            sqlx::query_scalar("SELECT project_id FROM workspace_snapshots WHERE id=?")
                .bind(&revision.workspace_snapshot_id)
                .fetch_optional(&self.pool)
                .await?;
        let archive_owner: Option<(String, String)> =
            sqlx::query_as("SELECT project_id,frame_id FROM context_archives WHERE id=?")
                .bind(&revision.context_archive_id)
                .fetch_optional(&self.pool)
                .await?;
        if snapshot_owner.as_deref() != Some(revision.project_id.as_str())
            || archive_owner.as_ref().map(|owner| (&owner.0, &owner.1))
                != Some((&revision.project_id, &revision.frame_id))
        {
            anyhow::bail!("Project state revision snapshot or archive has the wrong owner");
        }
        if sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM project_state_revisions WHERE frame_id=? AND turn_index=?",
        )
        .bind(&revision.frame_id)
        .bind(revision.turn_index)
        .fetch_one(&self.pool)
        .await?
            != 0
        {
            return Ok(false);
        }
        if let Some(parent_id) = revision.parent_revision_id.as_deref() {
            let parent: Option<(String, String, i64)> = sqlx::query_as(
                "SELECT project_id,frame_id,turn_index FROM project_state_revisions WHERE id=?",
            )
            .bind(parent_id)
            .fetch_optional(&self.pool)
            .await?;
            if !parent.as_ref().is_some_and(|parent| {
                parent.0 == revision.project_id
                    && parent.1 == revision.frame_id
                    && parent.2 < revision.turn_index
            }) {
                anyhow::bail!("Project state revision parent is not an earlier frame revision");
            }
        } else {
            let prior_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM project_state_revisions WHERE frame_id=?")
                    .bind(&revision.frame_id)
                    .fetch_one(&self.pool)
                    .await?;
            if prior_count != 0 {
                anyhow::bail!("Only the first stored project state revision may have no parent");
            }
        }
        let result = sqlx::query(
            "INSERT OR IGNORE INTO project_state_revisions(\
               id,project_id,frame_id,turn_index,message_seq,ui_event_seq,parent_revision_id,\
               workspace_snapshot_id,workspace_manifest_sha256,workspace_delta_json,\
               artifact_heads_json,entities_json,run_ids_json,decision_ids_json,\
               external_effects_json,context_archive_id,state_generation,is_full,created_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&revision.id)
        .bind(&revision.project_id)
        .bind(&revision.frame_id)
        .bind(revision.turn_index)
        .bind(revision.message_seq)
        .bind(revision.ui_event_seq)
        .bind(&revision.parent_revision_id)
        .bind(&revision.workspace_snapshot_id)
        .bind(&revision.workspace_manifest_sha256)
        .bind(&revision.workspace_delta_json)
        .bind(&revision.artifact_heads_json)
        .bind(&revision.entities_json)
        .bind(&revision.run_ids_json)
        .bind(&revision.decision_ids_json)
        .bind(&revision.external_effects_json)
        .bind(&revision.context_archive_id)
        .bind(revision.state_generation)
        .bind(i64::from(revision.is_full))
        .bind(revision.created_at)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn project_state_revision_for_turn(
        &self,
        frame_id: &str,
        turn_index: i64,
    ) -> Result<Option<ProjectStateRevision>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.project_state_revision_for_turn(frame_id, turn_index)).await;
        }
        let row =
            sqlx::query("SELECT * FROM project_state_revisions WHERE frame_id=? AND turn_index=?")
                .bind(frame_id)
                .bind(turn_index)
                .fetch_optional(&self.pool)
                .await?;
        row.map(project_state_revision_from_row).transpose()
    }

    pub async fn project_state_revision_for_boundary(
        &self,
        frame_id: &str,
        message_seq: i64,
        workspace_snapshot_id: &str,
    ) -> Result<Option<ProjectStateRevision>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.project_state_revision_for_boundary(
                frame_id,
                message_seq,
                workspace_snapshot_id,
            ))
            .await;
        }
        let row = sqlx::query(
            "SELECT * FROM project_state_revisions WHERE frame_id=? AND message_seq=? \
             AND workspace_snapshot_id=? LIMIT 1",
        )
        .bind(frame_id)
        .bind(message_seq)
        .bind(workspace_snapshot_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(project_state_revision_from_row).transpose()
    }

    pub async fn latest_project_state_revision(
        &self,
        frame_id: &str,
    ) -> Result<Option<ProjectStateRevision>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.latest_project_state_revision(frame_id)).await;
        }
        let row = sqlx::query(
            "SELECT * FROM project_state_revisions WHERE frame_id=? ORDER BY turn_index DESC LIMIT 1",
        )
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(project_state_revision_from_row).transpose()
    }

    pub async fn list_project_state_revisions(
        &self,
        frame_id: &str,
    ) -> Result<Vec<ProjectStateRevision>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.list_project_state_revisions(frame_id)).await;
        }
        let rows = sqlx::query(
            "SELECT * FROM project_state_revisions WHERE frame_id=? ORDER BY turn_index",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(project_state_revision_from_row)
            .collect()
    }

    pub async fn list_project_state_revision_summaries(
        &self,
        frame_id: &str,
        turn_start: i64,
        turn_end: i64,
    ) -> Result<Vec<ProjectStateRevisionSummary>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(
                store.list_project_state_revision_summaries(frame_id, turn_start, turn_end),
            )
            .await;
        }
        if turn_start < 0 || turn_end < turn_start || turn_end - turn_start > 200 {
            anyhow::bail!("Project state revision summary range is invalid");
        }
        Ok(sqlx::query_as::<_, (String, i64)>(
            "SELECT frame_id,turn_index FROM project_state_revisions \
             WHERE frame_id=? AND turn_index>=? AND turn_index<=? ORDER BY turn_index",
        )
        .bind(frame_id)
        .bind(turn_start)
        .bind(turn_end)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|(frame_id, turn_index)| ProjectStateRevisionSummary {
            frame_id,
            turn_index,
        })
        .collect())
    }

    /// Stable UI turn count. Unlike the model-context `messages` table, these
    /// User events survive compaction and therefore preserve historical indices.
    pub async fn frame_visual_user_turn_count(&self, frame_id: &str) -> Result<i64> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.frame_visual_user_turn_count(frame_id)).await;
        }
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM session_ui_events WHERE frame_id=? \
             AND json_extract(event_json,'$.kind')='User'",
        )
        .bind(frame_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_mainline_decision_ids(&self, project_id: &str) -> Result<Vec<String>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_mainline_decision_ids(project_id)).await;
        }
        Ok(sqlx::query_scalar(
            "SELECT id FROM research_nodes WHERE project_id=? AND exploration_id IS NULL \
             AND kind='decision' ORDER BY id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

fn project_state_revision_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ProjectStateRevision> {
    Ok(ProjectStateRevision {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        frame_id: row.try_get("frame_id")?,
        turn_index: row.try_get("turn_index")?,
        message_seq: row.try_get("message_seq")?,
        ui_event_seq: row.try_get("ui_event_seq")?,
        parent_revision_id: row.try_get("parent_revision_id")?,
        workspace_snapshot_id: row.try_get("workspace_snapshot_id")?,
        workspace_manifest_sha256: row.try_get("workspace_manifest_sha256")?,
        workspace_delta_json: row.try_get("workspace_delta_json")?,
        artifact_heads_json: row.try_get("artifact_heads_json")?,
        entities_json: row.try_get("entities_json")?,
        run_ids_json: row.try_get("run_ids_json")?,
        decision_ids_json: row.try_get("decision_ids_json")?,
        external_effects_json: row.try_get("external_effects_json")?,
        context_archive_id: row.try_get("context_archive_id")?,
        state_generation: row.try_get("state_generation")?,
        is_full: row.try_get::<i64, _>("is_full")? != 0,
        created_at: row.try_get("created_at")?,
    })
}
