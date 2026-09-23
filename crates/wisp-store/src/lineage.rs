use super::{
    artifact_version_from_row, ArtifactDependency, ArtifactVersion, ExternalResource, LineageBasis,
    LineageConfidence, RunCodeSnapshot, RunInput, RunOutput, StateScope, Store,
};
use anyhow::Result;
use sqlx::Row;

impl Store {
    pub async fn save_run_input(&self, input: &RunInput) -> Result<()> {
        if let Some(store) = self.route_entity("runs", "id", &input.run_id).await? {
            return Box::pin(store.save_run_input(input)).await;
        }
        if input.id.trim().is_empty()
            || input.run_id.trim().is_empty()
            || input.source_ref.trim().is_empty()
            || input.role.trim().is_empty()
        {
            anyhow::bail!("Run input requires id, Run, source reference, and role");
        }
        if input.artifact_version_id.is_some() && input.external_resource_id.is_some() {
            anyhow::bail!("Run input cannot bind both an ArtifactVersion and ExternalResource");
        }
        if input.artifact_version_id.is_none()
            && input.external_resource_id.is_none()
            && input.confidence == LineageConfidence::Exact
        {
            anyhow::bail!("Unresolved Run input cannot have exact confidence");
        }
        let run_scope: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT project_id,exploration_id FROM runs WHERE id=?")
                .bind(&input.run_id)
                .fetch_optional(&self.pool)
                .await?;
        let Some((project_id, exploration_id)) = run_scope else {
            anyhow::bail!("Run input requires an existing Run");
        };
        let artifact_valid = match input.artifact_version_id.as_deref() {
            None => true,
            Some(version_id) => match exploration_id.as_deref() {
                None => {
                    sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM artifact_versions version \
                     JOIN artifacts artifact ON artifact.id=version.artifact_id \
                     WHERE version.id=? AND artifact.project_id=? \
                       AND artifact.exploration_id IS NULL)",
                    )
                    .bind(version_id)
                    .bind(&project_id)
                    .fetch_one(&self.pool)
                    .await?
                }
                Some(exploration_id) => {
                    sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM artifact_versions version \
                     JOIN artifacts artifact ON artifact.id=version.artifact_id \
                     WHERE version.id=? AND artifact.project_id=? \
                       AND (artifact.exploration_id=? OR (artifact.exploration_id IS NULL \
                         AND EXISTS(SELECT 1 FROM explorations exploration \
                           JOIN exploration_baseline_artifact_heads baseline \
                             ON baseline.checkpoint_id=exploration.checkpoint_id \
                           WHERE exploration.id=? \
                             AND baseline.artifact_version_id=version.id))))",
                    )
                    .bind(version_id)
                    .bind(&project_id)
                    .bind(exploration_id)
                    .bind(exploration_id)
                    .fetch_one(&self.pool)
                    .await?
                }
            },
        };
        let external_valid = match input.external_resource_id.as_deref() {
            None => true,
            Some(resource_id) => match exploration_id.as_deref() {
                None => {
                    sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM external_resources \
                     WHERE id=? AND project_id=? AND exploration_id IS NULL)",
                    )
                    .bind(resource_id)
                    .bind(&project_id)
                    .fetch_one(&self.pool)
                    .await?
                }
                Some(exploration_id) => {
                    sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM external_resources resource \
                     WHERE resource.id=? AND resource.project_id=? \
                       AND (resource.exploration_id=? OR (resource.exploration_id IS NULL \
                         AND EXISTS(SELECT 1 FROM explorations exploration \
                           JOIN exploration_baseline_entities baseline \
                             ON baseline.checkpoint_id=exploration.checkpoint_id \
                           WHERE exploration.id=? \
                             AND baseline.entity_kind='external_resource' \
                             AND baseline.entity_id=resource.id))))",
                    )
                    .bind(resource_id)
                    .bind(&project_id)
                    .bind(exploration_id)
                    .bind(exploration_id)
                    .fetch_one(&self.pool)
                    .await?
                }
            },
        };
        let valid = artifact_valid && external_valid;
        if !valid {
            anyhow::bail!("Run input source must belong to the Run project");
        }
        let existing_run: Option<String> =
            sqlx::query_scalar("SELECT run_id FROM run_inputs WHERE id=?")
                .bind(&input.id)
                .fetch_optional(&self.pool)
                .await?;
        if existing_run
            .as_deref()
            .is_some_and(|run_id| run_id != input.run_id)
        {
            anyhow::bail!("Run input cannot move between Runs");
        }
        sqlx::query(
            "INSERT INTO run_inputs(\
               id,run_id,artifact_version_id,external_resource_id,source_ref,role,required,\
               basis,confidence,created_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               artifact_version_id=excluded.artifact_version_id,\
               external_resource_id=excluded.external_resource_id,\
               source_ref=excluded.source_ref,role=excluded.role,required=excluded.required,\
               basis=excluded.basis,confidence=excluded.confidence",
        )
        .bind(&input.id)
        .bind(&input.run_id)
        .bind(input.artifact_version_id.as_deref())
        .bind(input.external_resource_id.as_deref())
        .bind(&input.source_ref)
        .bind(&input.role)
        .bind(input.required)
        .bind(input.basis.as_str())
        .bind(input.confidence.as_str())
        .bind(input.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_run_inputs(&self, run_id: &str) -> Result<Vec<RunInput>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.list_run_inputs(run_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,run_id,artifact_version_id,external_resource_id,source_ref,role,\
                    required,basis,confidence,created_at \
             FROM run_inputs WHERE run_id=? ORDER BY created_at,id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let basis: String = row.try_get("basis")?;
                let confidence: String = row.try_get("confidence")?;
                Ok(RunInput {
                    id: row.try_get("id")?,
                    run_id: row.try_get("run_id")?,
                    artifact_version_id: row.try_get("artifact_version_id")?,
                    external_resource_id: row.try_get("external_resource_id")?,
                    source_ref: row.try_get("source_ref")?,
                    role: row.try_get("role")?,
                    required: row.try_get("required")?,
                    basis: LineageBasis::from_storage(&basis)?,
                    confidence: LineageConfidence::from_storage(&confidence)?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn save_run_output(&self, output: &RunOutput) -> Result<()> {
        if let Some(store) = self.route_entity("runs", "id", &output.run_id).await? {
            return Box::pin(store.save_run_output(output)).await;
        }
        if output.id.trim().is_empty()
            || output.run_id.trim().is_empty()
            || output.artifact_version_id.trim().is_empty()
            || output.role.trim().is_empty()
            || output.logical_output_key.trim().is_empty()
            || output.source_path.trim().is_empty()
        {
            anyhow::bail!("Run output requires exact source, role, logical key, and path");
        }
        let producing_run: Option<Option<String>> = sqlx::query_scalar(
            "SELECT v.producing_run_id \
             FROM artifact_versions v \
             JOIN artifacts a ON a.id=v.artifact_id \
             JOIN runs r ON r.id=? AND r.project_id=a.project_id \
             WHERE v.id=?",
        )
        .bind(&output.run_id)
        .bind(&output.artifact_version_id)
        .fetch_optional(&self.pool)
        .await?;
        match producing_run {
            Some(Some(existing)) if existing != output.run_id => {
                anyhow::bail!("ArtifactVersion is already attributed to another Run")
            }
            None => anyhow::bail!("Run output source must belong to the Run project"),
            _ => {}
        }
        let existing_run: Option<String> =
            sqlx::query_scalar("SELECT run_id FROM run_outputs WHERE id=?")
                .bind(&output.id)
                .fetch_optional(&self.pool)
                .await?;
        if existing_run
            .as_deref()
            .is_some_and(|run_id| run_id != output.run_id)
        {
            anyhow::bail!("Run output cannot move between Runs");
        }
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "UPDATE artifact_versions SET producing_run_id=? \
             WHERE id=? AND producing_run_id IS NULL",
        )
        .bind(&output.run_id)
        .bind(&output.artifact_version_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO run_outputs(\
               id,run_id,artifact_version_id,role,logical_output_key,source_path,created_at\
             ) VALUES(?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               artifact_version_id=excluded.artifact_version_id,role=excluded.role,\
               logical_output_key=excluded.logical_output_key,source_path=excluded.source_path",
        )
        .bind(&output.id)
        .bind(&output.run_id)
        .bind(&output.artifact_version_id)
        .bind(&output.role)
        .bind(&output.logical_output_key)
        .bind(&output.source_path)
        .bind(output.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_run_outputs(&self, run_id: &str) -> Result<Vec<RunOutput>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.list_run_outputs(run_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,run_id,artifact_version_id,role,logical_output_key,source_path,created_at \
             FROM run_outputs WHERE run_id=? ORDER BY created_at,id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(RunOutput {
                    id: row.try_get("id")?,
                    run_id: row.try_get("run_id")?,
                    artifact_version_id: row.try_get("artifact_version_id")?,
                    role: row.try_get("role")?,
                    logical_output_key: row.try_get("logical_output_key")?,
                    source_path: row.try_get("source_path")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn save_artifact_dependency(
        &self,
        id: &str,
        artifact_version_id: &str,
        depends_on_version_id: &str,
        reference_name: Option<&str>,
        basis: LineageBasis,
        confidence: LineageConfidence,
    ) -> Result<()> {
        if let Some(store) = self
            .route_entity("artifact_versions", "id", artifact_version_id)
            .await?
        {
            return Box::pin(store.save_artifact_dependency(
                id,
                artifact_version_id,
                depends_on_version_id,
                reference_name,
                basis,
                confidence,
            ))
            .await;
        }
        if id.trim().is_empty()
            || artifact_version_id.trim().is_empty()
            || depends_on_version_id.trim().is_empty()
        {
            anyhow::bail!("Artifact dependency requires identity and exact versions");
        }
        if artifact_version_id == depends_on_version_id {
            anyhow::bail!("ArtifactVersion cannot depend on itself");
        }
        let same_project: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
               SELECT 1 \
               FROM artifact_versions output \
               JOIN artifacts output_artifact ON output_artifact.id=output.artifact_id \
               JOIN artifact_versions input ON input.id=? \
               JOIN artifacts input_artifact ON input_artifact.id=input.artifact_id \
               WHERE output.id=? AND output_artifact.project_id=input_artifact.project_id\
             )",
        )
        .bind(depends_on_version_id)
        .bind(artifact_version_id)
        .fetch_one(&self.pool)
        .await?;
        if !same_project {
            anyhow::bail!("Artifact dependency versions must belong to one project");
        }
        let introduces_cycle: bool = sqlx::query_scalar(
            "WITH RECURSIVE dependencies(version_id) AS (\
               SELECT depends_on_version_id FROM artifact_dependencies \
               WHERE artifact_version_id=? \
               UNION \
               SELECT edge.depends_on_version_id FROM artifact_dependencies edge \
               JOIN dependencies parent ON edge.artifact_version_id=parent.version_id\
             ) \
             SELECT EXISTS(SELECT 1 FROM dependencies WHERE version_id=?)",
        )
        .bind(depends_on_version_id)
        .bind(artifact_version_id)
        .fetch_one(&self.pool)
        .await?;
        if introduces_cycle {
            anyhow::bail!("Artifact dependency would create a cycle");
        }
        sqlx::query(
            "INSERT INTO artifact_dependencies(\
               id,artifact_version_id,depends_on_version_id,reference_name,basis,confidence,\
               created_at\
             ) VALUES(?,?,?,?,?,?,?) \
             ON CONFLICT(artifact_version_id,depends_on_version_id) DO UPDATE SET \
               reference_name=excluded.reference_name,basis=excluded.basis,\
               confidence=excluded.confidence",
        )
        .bind(id)
        .bind(artifact_version_id)
        .bind(depends_on_version_id)
        .bind(reference_name)
        .bind(basis.as_str())
        .bind(confidence.as_str())
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_artifact_dependencies(
        &self,
        artifact_version_id: &str,
    ) -> Result<Vec<ArtifactDependency>> {
        if let Some(store) = self
            .route_entity("artifact_versions", "id", artifact_version_id)
            .await?
        {
            return Box::pin(store.list_artifact_dependencies(artifact_version_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,artifact_version_id,depends_on_version_id,reference_name,basis,\
                    confidence,created_at \
             FROM artifact_dependencies WHERE artifact_version_id=? ORDER BY created_at,id",
        )
        .bind(artifact_version_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let basis: String = row.try_get("basis")?;
                let confidence: String = row.try_get("confidence")?;
                Ok(ArtifactDependency {
                    id: row.try_get("id")?,
                    artifact_version_id: row.try_get("artifact_version_id")?,
                    depends_on_version_id: row.try_get("depends_on_version_id")?,
                    reference_name: row.try_get("reference_name")?,
                    basis: LineageBasis::from_storage(&basis)?,
                    confidence: LineageConfidence::from_storage(&confidence)?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn save_run_code_snapshot(&self, code: &RunCodeSnapshot) -> Result<()> {
        if let Some(store) = self.route_entity("runs", "id", &code.run_id).await? {
            return Box::pin(store.save_run_code_snapshot(code)).await;
        }
        if code.id.trim().is_empty()
            || code.run_id.trim().is_empty()
            || code.source_kind.trim().is_empty()
            || code.checksum.len() != 64
            || !code.checksum.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            anyhow::bail!("Run code snapshot requires Run, kind, and SHA-256");
        }
        let run_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE id=?)")
            .bind(&code.run_id)
            .fetch_one(&self.pool)
            .await?;
        if !run_exists {
            anyhow::bail!("Run code snapshot requires an existing Run");
        }
        let existing_run: Option<String> =
            sqlx::query_scalar("SELECT run_id FROM run_code_snapshots WHERE id=?")
                .bind(&code.id)
                .fetch_optional(&self.pool)
                .await?;
        if existing_run
            .as_deref()
            .is_some_and(|run_id| run_id != code.run_id)
        {
            anyhow::bail!("Run code snapshot cannot move between Runs");
        }
        sqlx::query(
            "INSERT INTO run_code_snapshots(\
               id,run_id,source_kind,source_path,source_text,checksum,storage_path,\
               git_commit,dirty_patch,created_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               source_kind=excluded.source_kind,source_path=excluded.source_path,\
               source_text=excluded.source_text,checksum=excluded.checksum,\
               storage_path=excluded.storage_path,git_commit=excluded.git_commit,\
               dirty_patch=excluded.dirty_patch",
        )
        .bind(&code.id)
        .bind(&code.run_id)
        .bind(&code.source_kind)
        .bind(code.source_path.as_deref())
        .bind(&code.source_text)
        .bind(&code.checksum)
        .bind(code.storage_path.as_deref())
        .bind(code.git_commit.as_deref())
        .bind(code.dirty_patch.as_deref())
        .bind(code.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_run_code_snapshots(&self, run_id: &str) -> Result<Vec<RunCodeSnapshot>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.list_run_code_snapshots(run_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,run_id,source_kind,source_path,source_text,checksum,storage_path,\
                    git_commit,dirty_patch,created_at \
             FROM run_code_snapshots WHERE run_id=? ORDER BY created_at,id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(RunCodeSnapshot {
                    id: row.try_get("id")?,
                    run_id: row.try_get("run_id")?,
                    source_kind: row.try_get("source_kind")?,
                    source_path: row.try_get("source_path")?,
                    source_text: row.try_get("source_text")?,
                    checksum: row.try_get("checksum")?,
                    storage_path: row.try_get("storage_path")?,
                    git_commit: row.try_get("git_commit")?,
                    dirty_patch: row.try_get("dirty_patch")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn get_run_output_version(
        &self,
        run_id: &str,
        logical_output_key: &str,
    ) -> Result<Option<ArtifactVersion>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.get_run_output_version(run_id, logical_output_key)).await;
        }
        let row = sqlx::query(
            "SELECT v.id,v.artifact_id,v.version_number,v.content_type,v.storage_path,\
                    v.size_bytes,v.checksum,v.parent_version_id,v.producing_run_id,\
                    v.env_snapshot_hash,v.materialization,v.capture_timing,v.created_at \
             FROM run_outputs output \
             JOIN artifact_versions v ON v.id=output.artifact_version_id \
             WHERE output.run_id=? AND output.logical_output_key=? \
             ORDER BY output.created_at DESC LIMIT 1",
        )
        .bind(run_id)
        .bind(logical_output_key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(artifact_version_from_row).transpose()
    }

    pub async fn save_external_resource(&self, resource: &ExternalResource) -> Result<()> {
        if let Some(store) = self.route_project(&resource.project_id).await? {
            return Box::pin(store.save_external_resource(resource)).await;
        }
        self.save_external_resource_in_scope(resource, &StateScope::mainline(&resource.project_id))
            .await
    }

    pub async fn save_external_resource_in_scope(
        &self,
        resource: &ExternalResource,
        scope: &StateScope,
    ) -> Result<()> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.save_external_resource_in_scope(resource, scope)).await;
        }
        if resource.id.trim().is_empty()
            || resource.project_id.trim().is_empty()
            || resource.kind.trim().is_empty()
            || resource.uri.trim().is_empty()
            || !matches!(
                resource.visibility.as_str(),
                "public" | "restricted" | "private"
            )
        {
            anyhow::bail!("External resource requires identity, URI, kind, and visibility");
        }
        if resource.checksum.as_deref().is_some_and(|checksum| {
            checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            anyhow::bail!("External resource checksum must be a SHA-256 hex digest");
        }
        if scope.project_id() != resource.project_id {
            anyhow::bail!("External resource scope does not belong to its project");
        }
        let exploration_id = match scope {
            StateScope::Mainline { .. } => None,
            StateScope::Exploration { exploration_id, .. } => Some(exploration_id.as_str()),
        };
        let project_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?)")
                .bind(&resource.project_id)
                .fetch_one(&self.pool)
                .await?;
        if !project_exists {
            anyhow::bail!("External resource requires an existing project");
        }
        let existing_scope: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT project_id,exploration_id FROM external_resources WHERE id=?")
                .bind(&resource.id)
                .fetch_optional(&self.pool)
                .await?;
        if existing_scope
            .as_ref()
            .is_some_and(|(project_id, existing)| {
                project_id != &resource.project_id || existing.as_deref() != exploration_id
            })
        {
            anyhow::bail!("External resource cannot move between state scopes");
        }
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO external_resources(\
               id,project_id,kind,uri,version,checksum,size_bytes,license,visibility,\
               access_instructions,accessed_at,created_at,updated_at,exploration_id\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
               kind=excluded.kind,uri=excluded.uri,version=excluded.version,\
               checksum=excluded.checksum,size_bytes=excluded.size_bytes,\
               license=excluded.license,visibility=excluded.visibility,\
               access_instructions=excluded.access_instructions,\
               accessed_at=excluded.accessed_at,updated_at=excluded.updated_at",
        )
        .bind(&resource.id)
        .bind(&resource.project_id)
        .bind(&resource.kind)
        .bind(&resource.uri)
        .bind(resource.version.as_deref())
        .bind(resource.checksum.as_deref())
        .bind(resource.size_bytes)
        .bind(resource.license.as_deref())
        .bind(&resource.visibility)
        .bind(resource.access_instructions.as_deref())
        .bind(resource.accessed_at)
        .bind(resource.created_at)
        .bind(resource.updated_at)
        .bind(exploration_id)
        .execute(&mut *tx)
        .await?;
        self.bump_state_generation_in_tx(&mut tx, scope).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_external_resource(&self, id: &str) -> Result<Option<ExternalResource>> {
        if let Some(store) = self.route_entity("external_resources", "id", id).await? {
            return Box::pin(store.get_external_resource(id)).await;
        }
        let row = sqlx::query(
            "SELECT id,project_id,kind,uri,version,checksum,size_bytes,license,visibility,\
                    access_instructions,accessed_at,created_at,updated_at \
             FROM external_resources WHERE id=? AND exploration_id IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ExternalResource {
                id: row.try_get("id")?,
                project_id: row.try_get("project_id")?,
                kind: row.try_get("kind")?,
                uri: row.try_get("uri")?,
                version: row.try_get("version")?,
                checksum: row.try_get("checksum")?,
                size_bytes: row.try_get("size_bytes")?,
                license: row.try_get("license")?,
                visibility: row.try_get("visibility")?,
                access_instructions: row.try_get("access_instructions")?,
                accessed_at: row.try_get("accessed_at")?,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .transpose()
    }

    pub async fn get_external_resource_in_scope(
        &self,
        id: &str,
        scope: &StateScope,
    ) -> Result<Option<ExternalResource>> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.get_external_resource_in_scope(id, scope)).await;
        }
        let StateScope::Exploration {
            project_id,
            exploration_id,
        } = scope
        else {
            return self.get_external_resource(id).await;
        };
        let row = sqlx::query(
            "SELECT resource.id,resource.project_id,resource.kind,resource.uri,resource.version,\
                    resource.checksum,resource.size_bytes,resource.license,resource.visibility,\
                    resource.access_instructions,resource.accessed_at,resource.created_at,\
                    resource.updated_at FROM external_resources resource \
             WHERE resource.id=? AND resource.project_id=? \
               AND (resource.exploration_id=? OR (resource.exploration_id IS NULL AND EXISTS(\
                 SELECT 1 FROM explorations exploration \
                 JOIN exploration_baseline_entities baseline \
                   ON baseline.checkpoint_id=exploration.checkpoint_id \
                 WHERE exploration.id=? AND baseline.entity_kind='external_resource' \
                   AND baseline.entity_id=resource.id)))",
        )
        .bind(id)
        .bind(project_id)
        .bind(exploration_id)
        .bind(exploration_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ExternalResource {
                id: row.try_get("id")?,
                project_id: row.try_get("project_id")?,
                kind: row.try_get("kind")?,
                uri: row.try_get("uri")?,
                version: row.try_get("version")?,
                checksum: row.try_get("checksum")?,
                size_bytes: row.try_get("size_bytes")?,
                license: row.try_get("license")?,
                visibility: row.try_get("visibility")?,
                access_instructions: row.try_get("access_instructions")?,
                accessed_at: row.try_get("accessed_at")?,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .transpose()
    }

    pub async fn list_external_resources_owned_by_exploration(
        &self,
        exploration_id: &str,
    ) -> Result<Vec<ExternalResource>> {
        if let Some(store) = self
            .route_entity("explorations", "id", exploration_id)
            .await?
        {
            return Box::pin(store.list_external_resources_owned_by_exploration(exploration_id))
                .await;
        }
        let rows = sqlx::query(
            "SELECT id,project_id,kind,uri,version,checksum,size_bytes,license,visibility,\
                    access_instructions,accessed_at,created_at,updated_at \
             FROM external_resources WHERE exploration_id=? ORDER BY created_at,id",
        )
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ExternalResource {
                    id: row.try_get("id")?,
                    project_id: row.try_get("project_id")?,
                    kind: row.try_get("kind")?,
                    uri: row.try_get("uri")?,
                    version: row.try_get("version")?,
                    checksum: row.try_get("checksum")?,
                    size_bytes: row.try_get("size_bytes")?,
                    license: row.try_get("license")?,
                    visibility: row.try_get("visibility")?,
                    access_instructions: row.try_get("access_instructions")?,
                    accessed_at: row.try_get("accessed_at")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }
}
