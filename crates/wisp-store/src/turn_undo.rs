use super::{artifact_node_id, Store, TurnFileUndo};
use anyhow::Result;
use sqlx::Row;

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub async fn save_turn_file_undo(
        &self,
        frame_id: &str,
        user_message_seq: i64,
        path: &str,
        before_exists: bool,
        before_snapshot_path: Option<&str>,
        before_checksum: Option<&str>,
        after_checksum: Option<&str>,
        reversible: bool,
        reason: Option<&str>,
    ) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.save_turn_file_undo(
                frame_id,
                user_message_seq,
                path,
                before_exists,
                before_snapshot_path,
                before_checksum,
                after_checksum,
                reversible,
                reason,
            ))
            .await;
        }
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO turn_file_undo(\
             frame_id,user_message_seq,path,before_exists,before_snapshot_path,\
             before_checksum,after_checksum,reversible,reason,created_at,updated_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(frame_id,user_message_seq,path) DO UPDATE SET \
             after_checksum=excluded.after_checksum,\
             reversible=CASE WHEN turn_file_undo.reversible=1 \
                 AND excluded.after_checksum IS NOT NULL \
                 THEN 1 ELSE 0 END,\
             reason=CASE WHEN turn_file_undo.reversible=1 \
                 AND excluded.after_checksum IS NOT NULL THEN NULL \
                 WHEN turn_file_undo.reversible=0 THEN turn_file_undo.reason \
                 ELSE excluded.reason END,\
             updated_at=excluded.updated_at",
        )
        .bind(frame_id)
        .bind(user_message_seq)
        .bind(path)
        .bind(before_exists)
        .bind(before_snapshot_path)
        .bind(before_checksum)
        .bind(after_checksum)
        .bind(reversible)
        .bind(reason)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_turn_file_undo(
        &self,
        frame_id: &str,
        user_message_seq: i64,
    ) -> Result<Vec<TurnFileUndo>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.list_turn_file_undo(frame_id, user_message_seq)).await;
        }
        let rows = sqlx::query(
            "SELECT frame_id,user_message_seq,path,before_exists,before_snapshot_path,\
             before_checksum,after_checksum,reversible,reason \
             FROM turn_file_undo WHERE frame_id=? AND user_message_seq=? ORDER BY path",
        )
        .bind(frame_id)
        .bind(user_message_seq)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(TurnFileUndo {
                    frame_id: row.try_get("frame_id")?,
                    user_message_seq: row.try_get("user_message_seq")?,
                    path: row.try_get("path")?,
                    before_exists: row.try_get("before_exists")?,
                    before_snapshot_path: row.try_get("before_snapshot_path")?,
                    before_checksum: row.try_get("before_checksum")?,
                    after_checksum: row.try_get("after_checksum")?,
                    reversible: row.try_get("reversible")?,
                    reason: row.try_get("reason")?,
                })
            })
            .collect()
    }

    /// Artifact versions first materialized by messages after `keep`.
    pub async fn list_owned_message_artifacts(
        &self,
        frame_id: &str,
        keep: i64,
    ) -> Result<Vec<(String, String)>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.list_owned_message_artifacts(frame_id, keep)).await;
        }
        let rows = sqlx::query(
            "SELECT DISTINCT owned.display_name,owned.mime_type \
             FROM message_resource_links owned \
             WHERE owned.frame_id=? AND owned.message_seq>? AND owned.created_version=1 \
             AND NOT EXISTS (SELECT 1 FROM message_resource_links other \
                 WHERE other.artifact_version_id=owned.artifact_version_id \
                 AND NOT(other.frame_id=? AND other.message_seq>?)) \
             AND NOT EXISTS (SELECT 1 FROM artifact_dependencies dependency \
                 WHERE dependency.artifact_version_id=owned.artifact_version_id \
                 OR dependency.depends_on_version_id=owned.artifact_version_id) \
             AND NOT EXISTS (SELECT 1 FROM run_artifacts run_artifact \
                 WHERE run_artifact.artifact_id=owned.artifact_id) \
             AND NOT EXISTS (SELECT 1 FROM run_inputs input \
                 WHERE input.artifact_version_id=owned.artifact_version_id) \
             AND NOT EXISTS (SELECT 1 FROM run_outputs output \
                 WHERE output.artifact_version_id=owned.artifact_version_id) \
             AND NOT EXISTS (SELECT 1 FROM evidence_bindings binding \
                 JOIN artifact_versions version ON version.id=binding.artifact_version_id \
                 WHERE version.artifact_id=owned.artifact_id) \
             ORDER BY owned.display_name",
        )
        .bind(frame_id)
        .bind(keep)
        .bind(frame_id)
        .bind(keep)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| Ok((row.try_get("display_name")?, row.try_get("mime_type")?)))
            .collect()
    }

    /// Truncate one turn and roll back only artifact versions that its resource
    /// bindings created. Workspace files are restored by the caller first.
    pub async fn truncate_messages_for_undo(&self, frame_id: &str, keep: i64) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.truncate_messages_for_undo(frame_id, keep)).await;
        }
        let mut tx = self.begin_write().await?;
        let owned = sqlx::query(
            "SELECT DISTINCT l.artifact_id,l.artifact_version_id,l.created_artifact,\
             v.version_number FROM message_resource_links l \
             JOIN artifact_versions v ON v.id=l.artifact_version_id \
             WHERE l.frame_id=? AND l.message_seq>? \
             AND l.created_version=1 AND l.artifact_id IS NOT NULL \
             AND l.artifact_version_id IS NOT NULL \
             ORDER BY l.artifact_id,v.version_number DESC",
        )
        .bind(frame_id)
        .bind(keep)
        .fetch_all(&mut *tx)
        .await?;

        for row in owned {
            let artifact_id: String = row.try_get("artifact_id")?;
            let version_id: String = row.try_get("artifact_version_id")?;
            let created_artifact: bool = row.try_get("created_artifact")?;
            let remaining_links: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM message_resource_links \
                 WHERE artifact_version_id=? AND NOT(frame_id=? AND message_seq>?)",
            )
            .bind(&version_id)
            .bind(frame_id)
            .bind(keep)
            .fetch_one(&mut *tx)
            .await?;
            let external_refs: i64 = sqlx::query_scalar(
                "SELECT (SELECT COUNT(*) FROM artifact_dependencies \
                    WHERE artifact_version_id=? OR depends_on_version_id=?) + \
                    (SELECT COUNT(*) FROM run_artifacts WHERE artifact_id=?) + \
                    (SELECT COUNT(*) FROM run_inputs WHERE artifact_version_id=?) + \
                    (SELECT COUNT(*) FROM run_outputs WHERE artifact_version_id=?) + \
                    (SELECT COUNT(*) FROM evidence_bindings binding \
                        JOIN artifact_versions version ON version.id=binding.artifact_version_id \
                        WHERE version.artifact_id=?)",
            )
            .bind(&version_id)
            .bind(&version_id)
            .bind(&artifact_id)
            .bind(&version_id)
            .bind(&version_id)
            .bind(&artifact_id)
            .fetch_one(&mut *tx)
            .await?;
            if remaining_links != 0 || external_refs != 0 {
                continue;
            }

            let parent: Option<String> =
                sqlx::query_scalar("SELECT parent_version_id FROM artifact_versions WHERE id=?")
                    .bind(&version_id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .flatten();
            if let Some(parent_id) = parent {
                sqlx::query(
                    "UPDATE artifacts SET latest_version_id=?,\
                     storage_path=(SELECT storage_path FROM artifact_versions WHERE id=?),\
                     content_type=(SELECT content_type FROM artifact_versions WHERE id=?) \
                     WHERE id=? AND latest_version_id=?",
                )
                .bind(&parent_id)
                .bind(&parent_id)
                .bind(&parent_id)
                .bind(&artifact_id)
                .bind(&version_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query("DELETE FROM artifact_versions WHERE id=?")
                    .bind(&version_id)
                    .execute(&mut *tx)
                    .await?;
            } else if created_artifact {
                let node_id = artifact_node_id(&artifact_id);
                sqlx::query("DELETE FROM research_edges WHERE source_id=? OR target_id=?")
                    .bind(&node_id)
                    .bind(&node_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM research_nodes WHERE id=?")
                    .bind(&node_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM artifact_versions WHERE id=?")
                    .bind(&version_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM artifacts WHERE id=?")
                    .bind(&artifact_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }

        crate::sessions::reconcile_session_branches_after_truncate(&mut tx, frame_id, keep).await?;
        Store::truncate_message_rows(&mut tx, frame_id, keep).await?;
        tx.commit().await?;
        Ok(())
    }
}
