//! Ledger of files this app placed on remote servers (run input staging,
//! transfer uploads, and harvest-persisted outputs), so retracted, replaced,
//! or no-longer-referenced files can be found and deleted instead of rotting
//! on the server. Run workdir intermediates stay off this ledger.

use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteStagingEntry {
    pub id: String,
    pub project_id: String,
    pub context_id: String,
    /// Run that placed the file (input staging run or transfer run).
    pub run_id: Option<String>,
    /// Absolute or `~/…` path on the remote server.
    pub remote_path: String,
    /// `run_input`, `transfer`, or `harvest_persist`.
    pub source: String,
    pub checksum: Option<String>,
    pub size_bytes: Option<i64>,
    pub created_at: i64,
    pub removed_at: Option<i64>,
}

impl RemoteStagingEntry {
    pub fn new(
        project_id: impl Into<String>,
        context_id: impl Into<String>,
        run_id: Option<String>,
        remote_path: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.into(),
            context_id: context_id.into(),
            run_id,
            remote_path: remote_path.into(),
            source: source.into(),
            checksum: None,
            size_bytes: None,
            created_at: chrono::Utc::now().timestamp(),
            removed_at: None,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty()
            || self.project_id.trim().is_empty()
            || self.context_id.trim().is_empty()
        {
            anyhow::bail!("Remote staging entry requires id, project, and context");
        }
        if self.remote_path.trim().is_empty() || self.remote_path.contains(['\n', '\r', '\0']) {
            anyhow::bail!("Remote staging entry requires a sane remote path");
        }
        if !matches!(
            self.source.as_str(),
            "run_input" | "transfer" | "harvest_persist"
        ) {
            anyhow::bail!("Remote staging source must be run_input, transfer, or harvest_persist");
        }
        Ok(())
    }
}

impl Store {
    pub async fn record_remote_staging(&self, entry: &RemoteStagingEntry) -> Result<()> {
        if let Some(store) = self.route_project(&entry.project_id).await? {
            return Box::pin(store.record_remote_staging(entry)).await;
        }
        entry.validate()?;
        sqlx::query(
            "INSERT INTO remote_staging(\
                id,project_id,context_id,run_id,remote_path,source,checksum,size_bytes,\
                created_at,removed_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&entry.id)
        .bind(&entry.project_id)
        .bind(&entry.context_id)
        .bind(entry.run_id.as_deref())
        .bind(&entry.remote_path)
        .bind(&entry.source)
        .bind(entry.checksum.as_deref())
        .bind(entry.size_bytes)
        .bind(entry.created_at)
        .bind(entry.removed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_remote_staging(
        &self,
        project_id: &str,
        context_id: &str,
        include_removed: bool,
    ) -> Result<Vec<RemoteStagingEntry>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_remote_staging(project_id, context_id, include_removed))
                .await;
        }
        let sql = if include_removed {
            "SELECT id,project_id,context_id,run_id,remote_path,source,checksum,size_bytes,\
             created_at,removed_at FROM remote_staging \
             WHERE project_id=? AND context_id=? ORDER BY created_at,id"
        } else {
            "SELECT id,project_id,context_id,run_id,remote_path,source,checksum,size_bytes,\
             created_at,removed_at FROM remote_staging \
             WHERE project_id=? AND context_id=? AND removed_at IS NULL ORDER BY created_at,id"
        };
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            Option<i64>,
            i64,
            Option<i64>,
        )> = sqlx::query_as(sql)
            .bind(project_id)
            .bind(context_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    project_id,
                    context_id,
                    run_id,
                    remote_path,
                    source,
                    checksum,
                    size_bytes,
                    created_at,
                    removed_at,
                )| RemoteStagingEntry {
                    id,
                    project_id,
                    context_id,
                    run_id,
                    remote_path,
                    source,
                    checksum,
                    size_bytes,
                    created_at,
                    removed_at,
                },
            )
            .collect())
    }

    pub async fn mark_remote_staging_removed(&self, ids: &[String]) -> Result<u64> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = 0;
            for store in stores {
                let value = Box::pin(store.mark_remote_staging_removed(ids)).await?;
                result += value;
            }
            return Ok(result);
        }
        let now = chrono::Utc::now().timestamp();
        let mut removed = 0;
        for id in ids {
            let updated = sqlx::query(
                "UPDATE remote_staging SET removed_at=? WHERE id=? AND removed_at IS NULL",
            )
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
            removed += updated.rows_affected();
        }
        Ok(removed)
    }

    /// Workspace cleanup deleted the run's workdir, taking its staged inputs
    /// with it.
    pub async fn mark_remote_staging_removed_for_run(&self, run_id: &str) -> Result<u64> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.mark_remote_staging_removed_for_run(run_id)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE remote_staging SET removed_at=? \
             WHERE run_id=? AND source='run_input' AND removed_at IS NULL",
        )
        .bind(now)
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected())
    }

    /// Insert unless an unremoved row already exists for the same
    /// project/context/path/source/run. Harvest retries stay idempotent.
    pub async fn ensure_remote_staging(&self, entry: &RemoteStagingEntry) -> Result<bool> {
        if let Some(store) = self.route_project(&entry.project_id).await? {
            return Box::pin(store.ensure_remote_staging(entry)).await;
        }
        entry.validate()?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM remote_staging \
             WHERE project_id=? AND context_id=? AND remote_path=? AND source=? \
               AND IFNULL(run_id,'')=IFNULL(?,'') AND removed_at IS NULL)",
        )
        .bind(&entry.project_id)
        .bind(&entry.context_id)
        .bind(&entry.remote_path)
        .bind(&entry.source)
        .bind(entry.run_id.as_deref())
        .fetch_one(&self.pool)
        .await?;
        if exists {
            return Ok(false);
        }
        self.record_remote_staging(entry).await?;
        Ok(true)
    }

    /// Live External references on this context across every project.
    /// Host disposal is alias-global, so the audit cannot be project-scoped.
    pub async fn count_external_references_on_context_all(&self, uri_prefix: &str) -> Result<i64> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = 0;
            for store in stores {
                let value =
                    Box::pin(store.count_external_references_on_context_all(uri_prefix)).await?;
                result += value;
            }
            return Ok(result);
        }
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM artifact_versions v \
             JOIN artifacts a ON a.id=v.artifact_id \
             WHERE v.materialization='external' \
             AND v.source_discarded_at IS NULL \
             AND v.storage_path LIKE ? ESCAPE '\\' \
             AND v.id=(SELECT id FROM artifact_versions latest \
                       WHERE latest.artifact_id=v.artifact_id \
                       ORDER BY latest.version_number DESC LIMIT 1)",
        )
        .bind(like_prefix(uri_prefix))
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn count_remote_staging_on_context(&self, context_id: &str) -> Result<i64> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = 0;
            for store in stores {
                let value = Box::pin(store.count_remote_staging_on_context(context_id)).await?;
                result += value;
            }
            return Ok(result);
        }
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM remote_staging \
             WHERE context_id=? AND removed_at IS NULL",
        )
        .bind(context_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Disposal audit: live External artifact references whose URI targets this
    /// context (`ssh://<alias>/…`), still the head version of their artifact.
    /// Discarded sources are no longer a reason to keep the server.
    pub async fn count_external_references_on_context(
        &self,
        project_id: &str,
        uri_prefix: &str,
    ) -> Result<i64> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.count_external_references_on_context(project_id, uri_prefix))
                .await;
        }
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM artifact_versions v \
             JOIN artifacts a ON a.id=v.artifact_id \
             WHERE a.project_id=? AND v.materialization='external' \
             AND v.source_discarded_at IS NULL \
             AND v.storage_path LIKE ? ESCAPE '\\' \
             AND v.id=(SELECT id FROM artifact_versions latest \
                       WHERE latest.artifact_id=v.artifact_id \
                       ORDER BY latest.version_number DESC LIMIT 1)",
        )
        .bind(project_id)
        .bind(like_prefix(uri_prefix))
        .fetch_one(&self.pool)
        .await?)
    }

    /// Head External URIs that still live on this server (not discarded).
    pub async fn list_live_external_uris_on_context(
        &self,
        project_id: &str,
        uri_prefix: &str,
    ) -> Result<Vec<String>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_live_external_uris_on_context(project_id, uri_prefix))
                .await;
        }
        Ok(sqlx::query_scalar(
            "SELECT v.storage_path FROM artifact_versions v \
             JOIN artifacts a ON a.id=v.artifact_id \
             WHERE a.project_id=? AND v.materialization='external' \
             AND v.source_discarded_at IS NULL \
             AND v.storage_path LIKE ? ESCAPE '\\' \
             AND v.id=(SELECT id FROM artifact_versions latest \
                       WHERE latest.artifact_id=v.artifact_id \
                       ORDER BY latest.version_number DESC LIMIT 1)",
        )
        .bind(project_id)
        .bind(like_prefix(uri_prefix))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Mark External artifact versions whose storage URI matches `uri_prefix`
    /// (typically `ssh://<alias>/`) as abandoned after the server is dropped.
    pub async fn mark_external_artifacts_source_discarded(&self, uri_prefix: &str) -> Result<u64> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = 0;
            for store in stores {
                let value =
                    Box::pin(store.mark_external_artifacts_source_discarded(uri_prefix)).await?;
                result += value;
            }
            return Ok(result);
        }
        let now = chrono::Utc::now().timestamp();
        let result = sqlx::query(
            "UPDATE artifact_versions SET source_discarded_at=? \
             WHERE source_discarded_at IS NULL AND materialization='external' \
             AND storage_path LIKE ? ESCAPE '\\'",
        )
        .bind(now)
        .bind(like_prefix(uri_prefix))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Mark exact External URIs discarded (user deleted the remote file).
    pub async fn mark_external_uris_source_discarded(&self, uris: &[String]) -> Result<u64> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = 0;
            for store in stores {
                let value = Box::pin(store.mark_external_uris_source_discarded(uris)).await?;
                result += value;
            }
            return Ok(result);
        }
        if uris.is_empty() {
            return Ok(0);
        }
        let now = chrono::Utc::now().timestamp();
        let mut marked = 0;
        for uri in uris {
            let result = sqlx::query(
                "UPDATE artifact_versions SET source_discarded_at=? \
                 WHERE source_discarded_at IS NULL AND materialization='external' \
                 AND storage_path=?",
            )
            .bind(now)
            .bind(uri)
            .execute(&self.pool)
            .await?;
            marked += result.rows_affected();
        }
        Ok(marked)
    }

    /// True when any External version of this exact URI has been discarded.
    /// Re-adding the same host alias must not resurrect abandoned references.
    pub async fn ssh_uri_source_discarded(&self, uri: &str) -> Result<bool> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = false;
            for store in stores {
                let value = Box::pin(store.ssh_uri_source_discarded(uri)).await?;
                result |= value;
            }
            return Ok(result);
        }
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifact_versions \
             WHERE materialization='external' AND storage_path=? \
               AND source_discarded_at IS NOT NULL)",
        )
        .bind(uri)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Abandon the ledger when the server itself is dropped. Files stay on the
    /// discarded machine; the project no longer claims them.
    pub async fn mark_remote_staging_removed_for_context(&self, context_id: &str) -> Result<u64> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = 0;
            for store in stores {
                let value =
                    Box::pin(store.mark_remote_staging_removed_for_context(context_id)).await?;
                result += value;
            }
            return Ok(result);
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE remote_staging SET removed_at=? \
             WHERE context_id=? AND removed_at IS NULL",
        )
        .bind(now)
        .bind(context_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected())
    }
}

fn like_prefix(uri_prefix: &str) -> String {
    format!("{}%", uri_prefix.replace('%', "\\%").replace('_', "\\_"))
}
