use super::{ArtifactVersion, MessageResourceLink, Store};
use anyhow::Result;
use sqlx::Row;

impl Store {
    pub async fn replace_message_resource_links(
        &self,
        frame_id: &str,
        message_seq: i64,
        links: &[MessageResourceLink],
    ) -> Result<()> {
        let mut tx = self.begin_write().await?;
        sqlx::query("DELETE FROM message_resource_links WHERE frame_id=? AND message_seq=?")
            .bind(frame_id)
            .bind(message_seq)
            .execute(&mut *tx)
            .await?;
        for link in links {
            sqlx::query(
                "INSERT INTO message_resource_links(\
                 id,frame_id,message_seq,ordinal,original_reference,artifact_id,\
                 artifact_version_id,display_name,resource_kind,mime_type,status,error,created_at) \
                 VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&link.id)
            .bind(frame_id)
            .bind(message_seq)
            .bind(link.ordinal)
            .bind(&link.original_reference)
            .bind(link.artifact_id.as_deref())
            .bind(link.artifact_version_id.as_deref())
            .bind(&link.display_name)
            .bind(&link.resource_kind)
            .bind(&link.mime_type)
            .bind(&link.status)
            .bind(link.error.as_deref())
            .bind(link.created_at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_message_resource_links(
        &self,
        frame_id: &str,
        start_seq: i64,
        before_seq: Option<i64>,
    ) -> Result<Vec<MessageResourceLink>> {
        let rows = sqlx::query(
            "SELECT id,frame_id,message_seq,ordinal,original_reference,artifact_id,\
             artifact_version_id,display_name,resource_kind,mime_type,status,error,created_at \
             FROM message_resource_links WHERE frame_id=? AND message_seq>=? \
             AND (? IS NULL OR message_seq<?) ORDER BY message_seq,ordinal",
        )
        .bind(frame_id)
        .bind(start_seq)
        .bind(before_seq)
        .bind(before_seq)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(MessageResourceLink {
                    id: row.try_get("id")?,
                    frame_id: row.try_get("frame_id")?,
                    message_seq: row.try_get("message_seq")?,
                    ordinal: row.try_get("ordinal")?,
                    original_reference: row.try_get("original_reference")?,
                    artifact_id: row.try_get("artifact_id")?,
                    artifact_version_id: row.try_get("artifact_version_id")?,
                    display_name: row.try_get("display_name")?,
                    resource_kind: row.try_get("resource_kind")?,
                    mime_type: row.try_get("mime_type")?,
                    status: row.try_get("status")?,
                    error: row.try_get("error")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn latest_artifact_version(
        &self,
        artifact_id: &str,
    ) -> Result<Option<ArtifactVersion>> {
        let row = sqlx::query(
            "SELECT id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,\
             parent_version_id,producing_run_id,env_snapshot_hash,created_at \
             FROM artifact_versions WHERE artifact_id=? ORDER BY version_number DESC LIMIT 1",
        )
        .bind(artifact_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(super::artifact_version_from_row).transpose()
    }

    pub async fn set_artifact_version_file_metadata(
        &self,
        version_id: &str,
        size_bytes: i64,
        checksum: &str,
    ) -> Result<()> {
        sqlx::query("UPDATE artifact_versions SET size_bytes=?,checksum=? WHERE id=?")
            .bind(size_bytes)
            .bind(checksum)
            .bind(version_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
