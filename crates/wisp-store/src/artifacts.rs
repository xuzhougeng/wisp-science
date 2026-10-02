use super::{
    artifact_node_id, artifact_version_from_row, session_display_title, ArtifactCaptureTiming,
    ArtifactMaterialization, ArtifactSearchResult, ArtifactVersion, ArtifactVersionContext,
    ArtifactVersionDraft, ResearchNode, ResearchNodeKind, StateScope, Store, MAINLINE_SCOPE_KEY,
};
use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::Row;

pub fn logical_artifact_id(project_id: &str, logical_key: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(project_id.as_bytes());
    digest.update([0]);
    digest.update(logical_key.as_bytes());
    format!("artifact-{}", &hex::encode(digest.finalize())[..32])
}

pub fn scoped_logical_artifact_id(project_id: &str, scope_key: &str, logical_key: &str) -> String {
    if scope_key == MAINLINE_SCOPE_KEY {
        return logical_artifact_id(project_id, logical_key);
    }
    let mut digest = Sha256::new();
    digest.update(project_id.as_bytes());
    digest.update([0]);
    digest.update(scope_key.as_bytes());
    digest.update([0]);
    digest.update(logical_key.as_bytes());
    format!("artifact-{}", &hex::encode(digest.finalize())[..32])
}

impl Store {
    pub async fn artifact_visible_in_scope(
        &self,
        artifact_id: &str,
        scope: &StateScope,
    ) -> Result<bool> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.artifact_visible_in_scope(artifact_id, scope)).await;
        }
        scope.validate()?;
        let visible: bool =
            match scope {
                StateScope::Mainline { .. } => sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM artifacts WHERE id=? AND exploration_id IS NULL)",
                )
                .bind(artifact_id)
                .fetch_one(&self.pool)
                .await?,
                StateScope::Exploration {
                    project_id,
                    exploration_id,
                } => {
                    sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM artifacts artifact \
                 WHERE artifact.id=? AND artifact.project_id=? AND (artifact.exploration_id=? \
                   OR (artifact.exploration_id IS NULL AND EXISTS(\
                     SELECT 1 FROM explorations exploration \
                     JOIN exploration_baseline_artifact_heads baseline \
                       ON baseline.checkpoint_id=exploration.checkpoint_id \
                     WHERE exploration.id=? AND baseline.artifact_id=artifact.id))))",
                    )
                    .bind(artifact_id)
                    .bind(project_id)
                    .bind(exploration_id)
                    .bind(exploration_id)
                    .fetch_one(&self.pool)
                    .await?
                }
            };
        Ok(visible)
    }

    pub async fn artifact_path_in_scope(
        &self,
        artifact_id: &str,
        scope: &StateScope,
    ) -> Result<Option<String>> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.artifact_path_in_scope(artifact_id, scope)).await;
        }
        scope.validate()?;
        match scope {
            StateScope::Mainline { .. } => Ok(sqlx::query_scalar(
                "SELECT storage_path FROM artifacts WHERE id=? AND exploration_id IS NULL",
            )
            .bind(artifact_id)
            .fetch_optional(&self.pool)
            .await?),
            StateScope::Exploration {
                project_id,
                exploration_id,
            } => Ok(sqlx::query_scalar(
                "SELECT CASE WHEN artifact.exploration_id=? THEN artifact.storage_path \
                         ELSE baseline_version.storage_path END \
                 FROM artifacts artifact \
                 JOIN explorations exploration ON exploration.id=? \
                 LEFT JOIN exploration_baseline_artifact_heads baseline \
                   ON baseline.checkpoint_id=exploration.checkpoint_id \
                  AND baseline.artifact_id=artifact.id \
                 LEFT JOIN artifact_versions baseline_version \
                   ON baseline_version.id=baseline.artifact_version_id \
                 WHERE artifact.id=? AND artifact.project_id=? \
                   AND (artifact.exploration_id=? OR (artifact.exploration_id IS NULL \
                     AND baseline.artifact_id IS NOT NULL))",
            )
            .bind(exploration_id)
            .bind(exploration_id)
            .bind(artifact_id)
            .bind(project_id)
            .bind(exploration_id)
            .fetch_optional(&self.pool)
            .await?),
        }
    }

    pub async fn save_artifact(
        &self,
        id: &str,
        project_id: &str,
        root_frame_id: &str,
        filename: &str,
        content_type: &str,
        storage_path: &str,
    ) -> Result<String> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.save_artifact(
                id,
                project_id,
                root_frame_id,
                filename,
                content_type,
                storage_path,
            ))
            .await;
        }
        self.save_artifact_version(&ArtifactVersionDraft {
            version_id: None,
            artifact_id: id.to_string(),
            project_id: project_id.to_string(),
            root_frame_id: root_frame_id.to_string(),
            filename: filename.to_string(),
            content_type: content_type.to_string(),
            storage_path: storage_path.to_string(),
            logical_key: None,
            size_bytes: None,
            checksum: None,
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: ArtifactMaterialization::Reference,
            capture_timing: ArtifactCaptureTiming::Unknown,
        })
        .await
    }

    pub async fn save_artifact_version(&self, draft: &ArtifactVersionDraft) -> Result<String> {
        if let Some(store) = self.route_project(&draft.project_id).await? {
            return Box::pin(store.save_artifact_version(draft)).await;
        }
        self.retain_environment_snapshot(draft.env_snapshot_hash.as_deref())
            .await?;
        if draft.artifact_id.trim().is_empty()
            || draft.project_id.trim().is_empty()
            || draft.root_frame_id.trim().is_empty()
            || draft.filename.trim().is_empty()
            || draft.storage_path.trim().is_empty()
        {
            anyhow::bail!("Artifact version requires identity, project, frame, name, and storage");
        }
        if draft
            .logical_key
            .as_deref()
            .is_some_and(|key| key.trim().is_empty())
        {
            anyhow::bail!("Artifact logical_key cannot be empty");
        }
        if draft.checksum.as_deref().is_some_and(|checksum| {
            checksum.len() != 64 || !checksum.bytes().all(|b| b.is_ascii_hexdigit())
        }) {
            anyhow::bail!("Artifact checksum must be a SHA-256 hex digest");
        }

        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let frame_scope: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT project_id,exploration_id FROM frames WHERE id=?")
                .bind(&draft.root_frame_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((frame_project, exploration_id)) = frame_scope else {
            anyhow::bail!("Artifact source frame does not exist");
        };
        if frame_project != draft.project_id {
            anyhow::bail!("Artifact source frame must belong to the Artifact project");
        }
        let scope = match exploration_id.as_deref() {
            Some(exploration_id) => {
                StateScope::exploration(&draft.project_id, exploration_id.to_string())
            }
            None => StateScope::mainline(&draft.project_id),
        };
        if let Some(run_id) = draft.producing_run_id.as_deref() {
            let run_scope: Option<(String, Option<String>)> =
                sqlx::query_as("SELECT project_id,exploration_id FROM runs WHERE id=?")
                    .bind(run_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if run_scope
                .as_ref()
                .map(|(project, exploration)| (project.as_str(), exploration.as_deref()))
                != Some((draft.project_id.as_str(), exploration_id.as_deref()))
            {
                anyhow::bail!("Producing Run must belong to the Artifact state scope");
            }
        }
        if let Some(env_hash) = draft.env_snapshot_hash.as_deref() {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM env_snapshots WHERE hash=?)")
                    .bind(env_hash)
                    .fetch_one(&mut *tx)
                    .await?;
            if !exists {
                anyhow::bail!("Artifact environment snapshot does not exist");
            }
        }
        let existing: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT project_id,logical_key,exploration_id FROM artifacts WHERE id=?",
        )
        .bind(&draft.artifact_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((project_id, logical_key, existing_exploration_id)) = existing {
            if project_id != draft.project_id {
                anyhow::bail!("Artifact cannot move between projects");
            }
            if existing_exploration_id != exploration_id {
                anyhow::bail!("Artifact cannot move between state scopes");
            }
            if let (Some(existing), Some(requested)) =
                (logical_key.as_deref(), draft.logical_key.as_deref())
            {
                if existing != requested {
                    anyhow::bail!("Artifact logical identity cannot be changed");
                }
            }
        }
        let mut parent_version_id: Option<String> =
            sqlx::query_scalar("SELECT latest_version_id FROM artifacts WHERE id=?")
                .bind(&draft.artifact_id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
        if parent_version_id.is_none() {
            if let (StateScope::Exploration { exploration_id, .. }, Some(logical_key)) =
                (&scope, draft.logical_key.as_deref())
            {
                parent_version_id = sqlx::query_scalar(
                    "SELECT baseline.artifact_version_id FROM explorations exploration \
                     JOIN exploration_baseline_artifact_heads baseline \
                       ON baseline.checkpoint_id=exploration.checkpoint_id \
                     WHERE exploration.id=? AND baseline.logical_key=?",
                )
                .bind(exploration_id)
                .bind(logical_key)
                .fetch_optional(&mut *tx)
                .await?;
            }
        }
        let version_number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version_number), 0) + 1 FROM artifact_versions WHERE artifact_id=?",
        )
        .bind(&draft.artifact_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO artifacts(\
                id,project_id,root_frame_id,filename,content_type,storage_path,created_at,\
                latest_version_id,logical_key,exploration_id\
             ) VALUES(?,?,?,?,?,?,?,NULL,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
                filename=excluded.filename,content_type=excluded.content_type,\
                storage_path=excluded.storage_path,\
                logical_key=COALESCE(artifacts.logical_key,excluded.logical_key)",
        )
        .bind(&draft.artifact_id)
        .bind(&draft.project_id)
        .bind(&draft.root_frame_id)
        .bind(&draft.filename)
        .bind(&draft.content_type)
        .bind(&draft.storage_path)
        .bind(now)
        .bind(draft.logical_key.as_deref())
        .bind(exploration_id.as_deref())
        .execute(&mut *tx)
        .await?;
        let version_id = draft
            .version_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if version_id.trim().is_empty() {
            anyhow::bail!("Artifact version id cannot be empty");
        }
        sqlx::query(
            "INSERT INTO artifact_versions(\
                id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,\
                parent_version_id,producing_run_id,env_snapshot_hash,materialization,\
                capture_timing,created_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&version_id)
        .bind(&draft.artifact_id)
        .bind(version_number)
        .bind(&draft.content_type)
        .bind(&draft.storage_path)
        .bind(draft.size_bytes)
        .bind(draft.checksum.as_deref())
        .bind(parent_version_id)
        .bind(draft.producing_run_id.as_deref())
        .bind(draft.env_snapshot_hash.as_deref())
        .bind(draft.materialization.as_str())
        .bind(draft.capture_timing.as_str())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE artifacts SET latest_version_id=? WHERE id=?")
            .bind(&version_id)
            .bind(&draft.artifact_id)
            .execute(&mut *tx)
            .await?;
        if let Some(logical_key) = draft.logical_key.as_deref() {
            sqlx::query(
                "INSERT INTO artifact_heads(\
                   project_id,scope_key,logical_key,artifact_id,artifact_version_id,updated_at\
                 ) VALUES(?,?,?,?,?,?) ON CONFLICT(project_id,scope_key,logical_key) DO UPDATE SET \
                   artifact_id=excluded.artifact_id,artifact_version_id=excluded.artifact_version_id,\
                   updated_at=excluded.updated_at",
            )
            .bind(&draft.project_id)
            .bind(scope.scope_key())
            .bind(logical_key)
            .bind(&draft.artifact_id)
            .bind(&version_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        self.bump_state_generation_in_tx(&mut tx, &scope).await?;
        tx.commit().await?;

        let mut node = ResearchNode::new(
            artifact_node_id(&draft.artifact_id),
            &draft.project_id,
            ResearchNodeKind::Artifact,
            &draft.filename,
        )?;
        node.ref_id = Some(draft.artifact_id.clone());
        self.save_research_node_in_scope(&node, &scope).await?;
        Ok(version_id)
    }

    /// Relocate the current storage target without creating a new scientific
    /// version. This is used when an isolated Agent workspace is removed after
    /// its immutable artifact bytes have been copied to durable app storage.
    pub async fn relocate_artifact_storage(&self, id: &str, storage_path: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("artifacts", "id", id).await? {
            return Box::pin(store.relocate_artifact_storage(id, storage_path)).await;
        }
        let mut tx = self.begin_write().await?;
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

    pub async fn get_artifact_version(&self, version_id: &str) -> Result<Option<ArtifactVersion>> {
        if let Some(store) = self
            .route_entity("artifact_versions", "id", version_id)
            .await?
        {
            return Box::pin(store.get_artifact_version(version_id)).await;
        }
        let row = sqlx::query(
            "SELECT id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,\
                    parent_version_id,producing_run_id,env_snapshot_hash,materialization,\
                    capture_timing,created_at \
             FROM artifact_versions WHERE id=?",
        )
        .bind(version_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(artifact_version_from_row).transpose()
    }

    pub async fn list_artifact_versions_owned_by_exploration(
        &self,
        exploration_id: &str,
    ) -> Result<Vec<ArtifactVersion>> {
        if let Some(store) = self
            .route_entity("explorations", "id", exploration_id)
            .await?
        {
            return Box::pin(store.list_artifact_versions_owned_by_exploration(exploration_id))
                .await;
        }
        let rows = sqlx::query(
            "SELECT version.id,version.artifact_id,version.version_number,version.content_type,\
                    version.storage_path,version.size_bytes,version.checksum,version.parent_version_id,\
                    version.producing_run_id,version.env_snapshot_hash,version.materialization,\
                    version.capture_timing,version.created_at \
             FROM artifact_versions version JOIN artifacts artifact ON artifact.id=version.artifact_id \
             WHERE artifact.exploration_id=? ORDER BY artifact.id,version.version_number,version.id",
        )
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(artifact_version_from_row).collect()
    }

    pub async fn get_artifact_version_context(
        &self,
        version_id: &str,
    ) -> Result<Option<ArtifactVersionContext>> {
        if let Some(store) = self
            .route_entity("artifact_versions", "id", version_id)
            .await?
        {
            return Box::pin(store.get_artifact_version_context(version_id)).await;
        }
        let row = sqlx::query(
            "SELECT version.id,version.artifact_id,version.version_number,\
                    version.content_type,version.storage_path,version.size_bytes,version.checksum,\
                    version.parent_version_id,version.producing_run_id,\
                    version.env_snapshot_hash,version.materialization,version.capture_timing,\
                    version.created_at,artifact.project_id,artifact.root_frame_id,\
                    artifact.filename,artifact.logical_key,artifact.latest_version_id \
             FROM artifact_versions version \
             JOIN artifacts artifact ON artifact.id=version.artifact_id \
             WHERE version.id=?",
        )
        .bind(version_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let project_id = row.try_get("project_id")?;
            let root_frame_id = row.try_get("root_frame_id")?;
            let filename = row.try_get("filename")?;
            let logical_key = row.try_get("logical_key")?;
            let latest_version_id = row.try_get("latest_version_id")?;
            Ok(ArtifactVersionContext {
                version: artifact_version_from_row(row)?,
                project_id,
                root_frame_id,
                filename,
                logical_key,
                latest_version_id,
            })
        })
        .transpose()
    }

    pub async fn get_latest_artifact_version_context(
        &self,
        artifact_id: &str,
    ) -> Result<Option<ArtifactVersionContext>> {
        if let Some(store) = self.route_entity("artifacts", "id", artifact_id).await? {
            return Box::pin(store.get_latest_artifact_version_context(artifact_id)).await;
        }
        let version_id: Option<String> =
            sqlx::query_scalar("SELECT latest_version_id FROM artifacts WHERE id=?")
                .bind(artifact_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten();
        match version_id {
            Some(version_id) => self.get_artifact_version_context(&version_id).await,
            None => Ok(None),
        }
    }

    pub async fn set_artifact_version_provenance(
        &self,
        version_id: &str,
        producing_run_id: Option<&str>,
        env_snapshot_hash: Option<&str>,
    ) -> Result<()> {
        if let Some(store) = self
            .route_entity("artifact_versions", "id", version_id)
            .await?
        {
            return Box::pin(store.set_artifact_version_provenance(
                version_id,
                producing_run_id,
                env_snapshot_hash,
            ))
            .await;
        }
        self.retain_environment_snapshot(env_snapshot_hash).await?;
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

    /// Artifacts for one conversation frame, newest first.
    pub async fn list_artifacts(
        &self,
        root_frame_id: &str,
    ) -> Result<Vec<(String, String, String, String, i64, Option<String>)>> {
        if let Some(store) = self.route_entity("frames", "id", root_frame_id).await? {
            return Box::pin(store.list_artifacts(root_frame_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id, filename, content_type, storage_path, created_at, logical_key FROM artifacts \
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
                row.try_get("logical_key")?,
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
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.search_project_artifacts(project_id, query, limit)).await;
        }
        let q = query.trim().to_lowercase();
        let rows = sqlx::query(
            "SELECT id, filename, content_type, storage_path, created_at FROM artifacts \
             WHERE project_id=? AND exploration_id IS NULL \
               AND (?='' OR lower(filename) LIKE ?) \
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
        if let Some(project_id) = project_id {
            if let Some(store) = self.route_project(project_id).await? {
                return Box::pin(store.search_artifacts(
                    Some(project_id),
                    query,
                    limit,
                    artifact_id,
                ))
                .await;
            }
        }
        if let Some(stores) = self.routed_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                result.extend(
                    Box::pin(store.search_artifacts(project_id, query, limit, artifact_id)).await?,
                );
            }
            result.sort_by(|a, b| b.ts.cmp(&a.ts).then(a.name.cmp(&b.name)));
            result.truncate(limit.clamp(1, 100) as usize);
            return Ok(result);
        }
        let q = query.trim().to_lowercase();
        let rows = sqlx::query(
            "SELECT a.id AS id, a.filename AS filename, a.content_type AS content_type, \
                    a.storage_path AS storage_path, a.created_at AS created_at, \
                    a.project_id AS project_id, COALESCE(p.name,'') AS project_name, \
                    COALESCE(p.workspace_dir,'') AS project_root, a.root_frame_id AS frame_id, \
                    COALESCE(f.title,'') AS frame_title, \
                    (SELECT content FROM messages m WHERE m.frame_id=a.root_frame_id AND m.role='user' ORDER BY m.seq ASC LIMIT 1) AS first_user, \
                    (SELECT size_bytes FROM artifact_versions v WHERE v.id=a.latest_version_id) AS size_bytes, \
                    CASE WHEN a.logical_key LIKE 'path:%' THEN substr(a.logical_key,6) END AS logical_path, \
                    CASE \
                      WHEN EXISTS (SELECT 1 FROM run_artifacts ra WHERE ra.artifact_id=a.id) THEN 'output' \
                      WHEN a.logical_key LIKE 'path:uploads/%' \
                        OR replace(a.storage_path, '\\\\', '/') LIKE replace(p.workspace_dir, '\\\\', '/') || '/uploads/%' THEN 'upload' \
                      ELSE 'artifact' \
                    END AS origin, \
                    CASE WHEN EXISTS (\
                      SELECT 1 FROM artifact_versions discarded \
                      WHERE discarded.id=a.latest_version_id \
                        AND discarded.source_discarded_at IS NOT NULL\
                    ) THEN 1 ELSE 0 END AS source_discarded \
             FROM artifacts a JOIN projects p ON p.id=a.project_id \
             LEFT JOIN frames f ON f.id=a.root_frame_id \
             WHERE a.exploration_id IS NULL AND (? IS NULL OR a.project_id=?) \
               AND (? IS NULL OR a.id=?) \
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
                    logical_path: row.try_get("logical_path")?,
                    source_discarded: row.try_get::<i64, _>("source_discarded")? != 0,
                })
            })
            .collect()
    }

    pub async fn search_exploration_artifacts(
        &self,
        project_id: &str,
        exploration_id: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ArtifactSearchResult>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.search_exploration_artifacts(
                project_id,
                exploration_id,
                query,
                limit,
            ))
            .await;
        }
        let q = query.trim().to_lowercase();
        let rows = sqlx::query(
            "SELECT a.id AS id,a.filename AS filename,\
                    CASE WHEN a.exploration_id=view_exploration.id THEN a.content_type \
                         ELSE COALESCE(baseline_version.content_type,a.content_type) END AS content_type,\
                    CASE WHEN a.exploration_id=view_exploration.id THEN a.storage_path \
                         ELSE COALESCE(baseline_version.storage_path,a.storage_path) END AS storage_path,\
                    a.created_at AS created_at,a.project_id AS project_id,\
                    COALESCE(p.name,'') AS project_name,view_exploration.workspace_dir AS project_root,\
                    a.root_frame_id AS frame_id,\
                    COALESCE(f.title,'') AS frame_title,\
                    (SELECT content FROM messages m WHERE m.frame_id=a.root_frame_id \
                      AND m.role='user' ORDER BY m.seq LIMIT 1) AS first_user,\
                    CASE WHEN a.exploration_id=view_exploration.id \
                         THEN (SELECT size_bytes FROM artifact_versions v WHERE v.id=a.latest_version_id) \
                         ELSE baseline_version.size_bytes END AS size_bytes,\
                    CASE WHEN a.logical_key LIKE 'path:%' THEN substr(a.logical_key,6) END AS logical_path, \
                    CASE WHEN COALESCE(\
                      CASE WHEN a.exploration_id=view_exploration.id \
                           THEN (SELECT discarded.source_discarded_at FROM artifact_versions discarded \
                                 WHERE discarded.id=a.latest_version_id) \
                           ELSE baseline_version.source_discarded_at END, \
                      0) != 0 THEN 1 ELSE 0 END AS source_discarded \
             FROM artifacts a JOIN projects p ON p.id=a.project_id \
             JOIN explorations view_exploration ON view_exploration.id=? \
             LEFT JOIN exploration_baseline_artifact_heads baseline \
               ON baseline.checkpoint_id=view_exploration.checkpoint_id AND baseline.artifact_id=a.id \
             LEFT JOIN artifact_versions baseline_version \
               ON baseline_version.id=baseline.artifact_version_id \
             LEFT JOIN artifact_heads overlay \
               ON overlay.project_id=a.project_id AND overlay.scope_key=view_exploration.id \
              AND overlay.logical_key=baseline.logical_key \
             LEFT JOIN frames f ON f.id=a.root_frame_id \
             WHERE a.project_id=? AND (a.exploration_id=? OR (a.exploration_id IS NULL \
               AND baseline.artifact_id IS NOT NULL AND overlay.artifact_id IS NULL)) \
               AND (?='' OR lower(a.filename) LIKE ?) \
             ORDER BY a.created_at DESC,a.filename LIMIT ?",
        )
        .bind(exploration_id)
        .bind(project_id)
        .bind(exploration_id)
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
                    origin: "artifact".into(),
                    logical_path: row.try_get("logical_path")?,
                    source_discarded: row.try_get::<i64, _>("source_discarded")? != 0,
                })
            })
            .collect()
    }

    /// Root sessions across projects, newest activity first, optionally matched
    /// by their display title. Empty frames never appear in the picker.

    pub async fn get_artifact(&self, id: &str) -> Result<Option<(String, String, String, String)>> {
        if let Some(store) = self.route_entity("artifacts", "id", id).await? {
            return Box::pin(store.get_artifact(id)).await;
        }
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
        if let Some(store) = self.route_entity("artifacts", "id", id).await? {
            return Box::pin(store.get_artifact_detail(id)).await;
        }
        let row = sqlx::query(
            "SELECT a.id AS id,a.filename AS filename,a.content_type AS content_type,\
                    a.storage_path AS storage_path,a.created_at AS created_at,\
                    a.project_id AS project_id,COALESCE(p.name,'') AS project_name,\
                    COALESCE(e.workspace_dir,p.workspace_dir,'') AS project_root,\
                    a.root_frame_id AS frame_id,COALESCE(f.title,'') AS frame_title,\
                    (SELECT content FROM messages m WHERE m.frame_id=a.root_frame_id \
                      AND m.role='user' ORDER BY m.seq LIMIT 1) AS first_user,\
                    (SELECT size_bytes FROM artifact_versions v WHERE v.id=a.latest_version_id) AS size_bytes,\
                    CASE WHEN a.logical_key LIKE 'path:%' THEN substr(a.logical_key,6) END AS logical_path,\
                    CASE WHEN EXISTS (\
                      SELECT 1 FROM artifact_versions discarded \
                      WHERE discarded.id=a.latest_version_id \
                        AND discarded.source_discarded_at IS NOT NULL\
                    ) THEN 1 ELSE 0 END AS source_discarded, \
                    CASE \
                      WHEN EXISTS (SELECT 1 FROM run_artifacts ra WHERE ra.artifact_id=a.id) THEN 'output' \
                      WHEN a.logical_key LIKE 'path:uploads/%' \
                        OR replace(a.storage_path,'\\','/') LIKE \
                           replace(COALESCE(e.workspace_dir,p.workspace_dir),'\\','/') || '/uploads/%' \
                      THEN 'upload' ELSE 'artifact' \
                    END AS origin \
             FROM artifacts a JOIN projects p ON p.id=a.project_id \
             LEFT JOIN explorations e ON e.id=a.exploration_id \
             LEFT JOIN frames f ON f.id=a.root_frame_id WHERE a.id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
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
                logical_path: row.try_get("logical_path")?,
                source_discarded: row.try_get::<i64, _>("source_discarded")? != 0,
            })
        })
        .transpose()
    }

    /// The logical workspace identity an artifact was registered under, when
    /// one exists. This is the user-facing location; `storage_path` is an
    /// internal snapshot path under `.wisp/` and must not be shown as the
    /// document's real location.
    pub async fn artifact_location(&self, id: &str) -> Result<Option<String>> {
        if let Some(store) = self.route_entity("artifacts", "id", id).await? {
            return Box::pin(store.artifact_location(id)).await;
        }
        Ok(
            sqlx::query_scalar("SELECT logical_key FROM artifacts WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }
}
