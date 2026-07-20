use super::{
    artifact_node_id, artifact_version_from_row, session_display_title, ArtifactSearchResult,
    ArtifactVersion, ResearchNode, ResearchNodeKind, Store,
};
use anyhow::Result;
use sqlx::Row;

impl Store {
    pub async fn save_artifact(
        &self,
        id: &str,
        project_id: &str,
        root_frame_id: &str,
        filename: &str,
        content_type: &str,
        storage_path: &str,
    ) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        let parent_version_id: Option<String> =
            sqlx::query_scalar("SELECT latest_version_id FROM artifacts WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
        let version_number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version_number), 0) + 1 FROM artifact_versions WHERE artifact_id=?",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO artifacts(id,project_id,root_frame_id,filename,content_type,storage_path,created_at,latest_version_id) \
             VALUES(?,?,?,?,?,?,?,NULL) \
             ON CONFLICT(id) DO UPDATE SET filename=excluded.filename, content_type=excluded.content_type, storage_path=excluded.storage_path",
        )
        .bind(id)
        .bind(project_id)
        .bind(root_frame_id)
        .bind(filename)
        .bind(content_type)
        .bind(storage_path)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let version_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO artifact_versions(\
                id,artifact_id,version_number,content_type,storage_path,parent_version_id,created_at\
             ) VALUES(?,?,?,?,?,?,?)",
        )
        .bind(&version_id)
        .bind(id)
        .bind(version_number)
        .bind(content_type)
        .bind(storage_path)
        .bind(parent_version_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE artifacts SET latest_version_id=? WHERE id=?")
            .bind(&version_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        let mut node = ResearchNode::new(
            artifact_node_id(id),
            project_id,
            ResearchNodeKind::Artifact,
            filename,
        )?;
        node.ref_id = Some(id.to_string());
        self.save_research_node(&node).await?;
        Ok(version_id)
    }

    /// Relocate the current storage target without creating a new scientific
    /// version. This is used when an isolated Agent workspace is removed after
    /// its immutable artifact bytes have been copied to durable app storage.
    pub async fn relocate_artifact_storage(&self, id: &str, storage_path: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let latest_version_id: Option<String> =
            sqlx::query_scalar("SELECT latest_version_id FROM artifacts WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
        let updated = sqlx::query("UPDATE artifacts SET storage_path=? WHERE id=?")
            .bind(storage_path)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if let Some(version_id) = latest_version_id {
            sqlx::query("UPDATE artifact_versions SET storage_path=? WHERE id=?")
                .bind(storage_path)
                .bind(version_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn list_artifact_versions(&self, artifact_id: &str) -> Result<Vec<ArtifactVersion>> {
        let rows = sqlx::query(
            "SELECT id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,\
                    parent_version_id,producing_run_id,env_snapshot_hash,created_at \
             FROM artifact_versions WHERE artifact_id=? ORDER BY version_number DESC",
        )
        .bind(artifact_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(artifact_version_from_row).collect()
    }

    pub async fn get_artifact_version(&self, version_id: &str) -> Result<Option<ArtifactVersion>> {
        let row = sqlx::query(
            "SELECT id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,\
                    parent_version_id,producing_run_id,env_snapshot_hash,created_at \
             FROM artifact_versions WHERE id=?",
        )
        .bind(version_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(artifact_version_from_row).transpose()
    }

    pub async fn set_artifact_version_provenance(
        &self,
        version_id: &str,
        producing_run_id: Option<&str>,
        env_snapshot_hash: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE artifact_versions SET producing_run_id=?, env_snapshot_hash=? WHERE id=?",
        )
        .bind(producing_run_id)
        .bind(env_snapshot_hash)
        .bind(version_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn save_artifact_dependency(
        &self,
        id: &str,
        artifact_version_id: &str,
        depends_on_version_id: &str,
        reference_name: Option<&str>,
    ) -> Result<()> {
        if artifact_version_id == depends_on_version_id {
            anyhow::bail!("An artifact version cannot depend on itself");
        }
        sqlx::query(
            "INSERT INTO artifact_dependencies(\
                id,artifact_version_id,depends_on_version_id,reference_name,created_at\
             ) VALUES(?,?,?,?,?) \
             ON CONFLICT(artifact_version_id,depends_on_version_id) DO UPDATE SET \
                reference_name=excluded.reference_name",
        )
        .bind(id)
        .bind(artifact_version_id)
        .bind(depends_on_version_id)
        .bind(reference_name)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Artifacts for one conversation frame, newest first.
    pub async fn list_artifacts(
        &self,
        root_frame_id: &str,
    ) -> Result<Vec<(String, String, String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT id, filename, content_type, storage_path, created_at FROM artifacts \
             WHERE root_frame_id=? ORDER BY created_at DESC",
        )
        .bind(root_frame_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for row in rows {
            out.push((
                row.try_get("id")?,
                row.try_get("filename")?,
                row.try_get("content_type")?,
                row.try_get("storage_path")?,
                row.try_get("created_at")?,
            ));
        }
        Ok(out)
    }

    /// Recent artifacts for one project, optionally filtered by filename.
    pub async fn search_project_artifacts(
        &self,
        project_id: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, String, String, i64)>> {
        let q = query.trim().to_lowercase();
        let rows = sqlx::query(
            "SELECT id, filename, content_type, storage_path, created_at FROM artifacts \
             WHERE project_id=? AND (?='' OR lower(filename) LIKE ?) \
             ORDER BY created_at DESC, filename ASC LIMIT ?",
        )
        .bind(project_id)
        .bind(&q)
        .bind(format!("%{q}%"))
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for row in rows {
            out.push((
                row.try_get("id")?,
                row.try_get("filename")?,
                row.try_get("content_type")?,
                row.try_get("storage_path")?,
                row.try_get("created_at")?,
            ));
        }
        Ok(out)
    }

    /// Recent artifacts across projects for composer and command-palette search.
    /// The rows intentionally carry ownership metadata: callers must validate a
    /// cross-project path against its owner's workspace, not the current one.
    pub async fn search_artifacts(
        &self,
        project_id: Option<&str>,
        query: &str,
        limit: i64,
        artifact_id: Option<&str>,
    ) -> Result<Vec<ArtifactSearchResult>> {
        let q = query.trim().to_lowercase();
        let rows = sqlx::query(
            "SELECT a.id AS id, a.filename AS filename, a.content_type AS content_type, \
                    a.storage_path AS storage_path, a.created_at AS created_at, \
                    a.project_id AS project_id, COALESCE(p.name,'') AS project_name, \
                    COALESCE(p.workspace_dir,'') AS project_root, a.root_frame_id AS frame_id, \
                    COALESCE(f.title,'') AS frame_title, \
                    (SELECT content FROM messages m WHERE m.frame_id=a.root_frame_id AND m.role='user' ORDER BY m.seq ASC LIMIT 1) AS first_user, \
                    (SELECT size_bytes FROM artifact_versions v WHERE v.id=a.latest_version_id) AS size_bytes, \
                    CASE \
                      WHEN EXISTS (SELECT 1 FROM run_artifacts ra WHERE ra.artifact_id=a.id) THEN 'output' \
                      WHEN replace(a.storage_path, '\\\\', '/') LIKE replace(p.workspace_dir, '\\\\', '/') || '/uploads/%' THEN 'upload' \
                      ELSE 'artifact' \
                    END AS origin \
             FROM artifacts a JOIN projects p ON p.id=a.project_id \
             LEFT JOIN frames f ON f.id=a.root_frame_id \
             WHERE (? IS NULL OR a.project_id=?) AND (? IS NULL OR a.id=?) \
               AND (?='' OR lower(a.filename) LIKE ?) \
             ORDER BY a.created_at DESC, a.filename ASC LIMIT ?",
        )
        .bind(project_id)
        .bind(project_id)
        .bind(artifact_id)
        .bind(artifact_id)
        .bind(&q)
        .bind(format!("%{q}%"))
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ArtifactSearchResult {
                    id: row.try_get("id")?,
                    name: row.try_get("filename")?,
                    kind: row.try_get("content_type")?,
                    path: row.try_get("storage_path")?,
                    ts: row.try_get("created_at")?,
                    project_id: row.try_get("project_id")?,
                    project_name: row.try_get("project_name")?,
                    project_root: row.try_get("project_root")?,
                    session_id: row.try_get("frame_id")?,
                    session_title: session_display_title(
                        row.try_get::<Option<String>, _>("frame_title")?,
                        row.try_get::<Option<String>, _>("first_user")?,
                    ),
                    size_bytes: row.try_get("size_bytes")?,
                    origin: row.try_get("origin")?,
                })
            })
            .collect()
    }

    /// Root sessions across projects, newest activity first, optionally matched
    /// by their display title. Empty frames never appear in the picker.

    pub async fn get_artifact(&self, id: &str) -> Result<Option<(String, String, String, String)>> {
        let row = sqlx::query(
            "SELECT filename, content_type, storage_path, root_frame_id FROM artifacts WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| {
            (
                r.try_get("filename").unwrap_or_default(),
                r.try_get("content_type").unwrap_or_default(),
                r.try_get("storage_path").unwrap_or_default(),
                r.try_get("root_frame_id").unwrap_or_default(),
            )
        }))
    }

    /// Artifact ownership plus storage information for safe cross-project
    /// preview and explicit composer attachment resolution.
    pub async fn get_artifact_detail(&self, id: &str) -> Result<Option<ArtifactSearchResult>> {
        Ok(self
            .search_artifacts(None, "", 1, Some(id))
            .await?
            .into_iter()
            .next())
    }
}
