use super::{
    artifact_node_id, canonical_json, canonical_json_sha256, run_from_row, run_node_id,
    run_summary_from_row, ResearchEdge, ResearchNode, ResearchNodeKind, RunRecord, RunStatus,
    RunSummary, StateScope, Store,
};
use anyhow::Result;
use sha2::{Digest, Sha256};

impl Store {
    pub async fn project_has_active_runs(&self, project_id: &str) -> Result<bool> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.project_has_active_runs(project_id)).await;
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runs WHERE project_id=? \
             AND exploration_id IS NULL AND status IN ('submitted','running','cancelling')",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn exploration_has_active_runs(&self, exploration_id: &str) -> Result<bool> {
        if let Some(store) = self
            .route_entity("explorations", "id", exploration_id)
            .await?
        {
            return Box::pin(store.exploration_has_active_runs(exploration_id)).await;
        }
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE exploration_id=? \
             AND status IN ('submitted','running','cancelling'))",
        )
        .bind(exploration_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn create_run(&self, run: &RunRecord) -> Result<()> {
        if let Some(store) = self.route_project(&run.project_id).await? {
            return Box::pin(store.create_run(run)).await;
        }
        run.validate()?;
        let environment = serde_json::from_str::<serde_json::Value>(&run.env_snapshot_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        let (environment_json, environment_hash) = canonical_json_sha256(&environment);
        let packages_json = environment
            .get("packages")
            .map(canonical_json)
            .unwrap_or_else(|| "[]".into());
        let mut tx = self.begin_write().await?;
        let exploration_id = if let Some(frame_id) = run.frame_id.as_deref() {
            let frame_scope: Option<(String, Option<String>)> =
                sqlx::query_as("SELECT project_id,exploration_id FROM frames WHERE id=?")
                    .bind(frame_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some((frame_project, exploration_id)) = frame_scope else {
                anyhow::bail!("Run frame does not exist");
            };
            if frame_project != run.project_id {
                anyhow::bail!("Run frame must belong to the Run project");
            }
            exploration_id
        } else {
            None
        };
        let scope = match exploration_id.as_deref() {
            Some(exploration_id) => {
                StateScope::exploration(&run.project_id, exploration_id.to_string())
            }
            None => StateScope::mainline(&run.project_id),
        };
        sqlx::query(
            "INSERT INTO runs(\
                id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                last_polled_at,last_poll_error,progress_json,env_snapshot_json,exploration_id\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&run.id)
        .bind(&run.project_id)
        .bind(run.frame_id.as_deref())
        .bind(&run.context_id)
        .bind(&run.title)
        .bind(&run.kind)
        .bind(run.status.as_str())
        .bind(run.command.as_deref())
        .bind(run.script_path.as_deref())
        .bind(&run.input_refs_json)
        .bind(&run.output_specs_json)
        .bind(run.created_at)
        .bind(run.started_at)
        .bind(run.ended_at)
        .bind(run.exit_code)
        .bind(run.stdout_tail.as_deref())
        .bind(run.stderr_tail.as_deref())
        .bind(run.remote_workdir.as_deref())
        .bind(run.remote_handle_json.as_deref())
        .bind(run.timeout_secs)
        .bind(run.last_polled_at)
        .bind(run.last_poll_error.as_deref())
        .bind(&run.progress_json)
        .bind(&run.env_snapshot_json)
        .bind(exploration_id.as_deref())
        .execute(&mut *tx)
        .await?;
        let command = run.command.as_deref().unwrap_or_default();
        let mut digest = Sha256::new();
        digest.update(command.as_bytes());
        sqlx::query(
            "INSERT INTO run_code_snapshots(\
               id,run_id,source_kind,source_path,source_text,checksum,storage_path,\
               git_commit,dirty_patch,created_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(format!("run-code:{}:command", run.id))
        .bind(&run.id)
        .bind("command")
        .bind(run.script_path.as_deref())
        .bind(command)
        .bind(hex::encode(digest.finalize()))
        .bind(Option::<&str>::None)
        .bind(Option::<&str>::None)
        .bind(Option::<&str>::None)
        .bind(run.created_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO env_snapshots(\
               hash,env_name,packages_json,snapshot_json,hash_algorithm,created_at\
             ) VALUES(?,?,?,?,?,?) \
             ON CONFLICT(hash) DO UPDATE SET \
               snapshot_json=excluded.snapshot_json,hash_algorithm='sha256'",
        )
        .bind(&environment_hash)
        .bind(&run.context_id)
        .bind(packages_json)
        .bind(environment_json)
        .bind("sha256")
        .bind(run.created_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO run_environment_snapshots(run_id,env_snapshot_hash) VALUES(?,?)")
            .bind(&run.id)
            .bind(environment_hash)
            .execute(&mut *tx)
            .await?;
        if let Some(exploration_id) = exploration_id.as_deref() {
            let recoverability = if run.context_id == "local" {
                "local_reversible"
            } else {
                "external_irreversible"
            };
            sqlx::query(
                "INSERT INTO exploration_effects(\
                   id,exploration_id,effect_kind,recoverability,target_summary,metadata_json,created_at\
                 ) VALUES(?,?,?,?,?,?,?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(exploration_id)
            .bind("run")
            .bind(recoverability)
            .bind(&run.context_id)
            .bind(serde_json::json!({ "run_id": &run.id }).to_string())
            .bind(run.created_at)
            .execute(&mut *tx)
            .await?;
        }
        self.bump_state_generation_in_tx(&mut tx, &scope).await?;
        tx.commit().await?;
        let mut node = ResearchNode::new(
            run_node_id(&run.id),
            &run.project_id,
            ResearchNodeKind::Run,
            &run.title,
        )?;
        node.ref_id = Some(run.id.clone());
        self.save_research_node_in_scope(&node, &scope).await?;
        Ok(())
    }

    pub async fn get_run(&self, id: &str) -> Result<Option<RunRecord>> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.get_run(id)).await;
        }
        let row = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json,harvested_at,cleaned_at,cleanup_error,logs_path \
             FROM runs WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(run_from_row).transpose()
    }

    pub async fn run_state_scope(&self, id: &str) -> Result<Option<StateScope>> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.run_state_scope(id)).await;
        }
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT project_id,exploration_id FROM runs WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(
            row.map(|(project_id, exploration_id)| match exploration_id {
                Some(exploration_id) => StateScope::exploration(project_id, exploration_id),
                None => StateScope::mainline(project_id),
            }),
        )
    }

    pub async fn run_visible_in_scope(&self, id: &str, scope: &StateScope) -> Result<bool> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.run_visible_in_scope(id, scope)).await;
        }
        scope.validate()?;
        Ok(match scope {
            StateScope::Mainline { project_id } => {
                sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM runs \
                 WHERE id=? AND project_id=? AND exploration_id IS NULL)",
                )
                .bind(id)
                .bind(project_id)
                .fetch_one(&self.pool)
                .await?
            }
            StateScope::Exploration {
                project_id,
                exploration_id,
            } => {
                sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM runs run WHERE run.id=? AND run.project_id=? \
                 AND (run.exploration_id=? OR (run.exploration_id IS NULL AND EXISTS(\
                   SELECT 1 FROM explorations exploration \
                   JOIN exploration_baseline_entities baseline \
                     ON baseline.checkpoint_id=exploration.checkpoint_id \
                   WHERE exploration.id=? AND baseline.entity_kind='run' \
                     AND baseline.entity_id=run.id))))",
                )
                .bind(id)
                .bind(project_id)
                .bind(exploration_id)
                .bind(exploration_id)
                .fetch_one(&self.pool)
                .await?
            }
        })
    }

    pub async fn list_runs_by_project(&self, project_id: &str) -> Result<Vec<RunRecord>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_runs_by_project(project_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json,harvested_at,cleaned_at,cleanup_error,logs_path \
             FROM runs WHERE project_id=? AND exploration_id IS NULL \
             ORDER BY created_at DESC, id DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn list_runs_in_scope(&self, scope: &StateScope) -> Result<Vec<RunRecord>> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.list_runs_in_scope(scope)).await;
        }
        let StateScope::Exploration {
            project_id,
            exploration_id,
        } = scope
        else {
            return self.list_runs_by_project(scope.project_id()).await;
        };
        let rows = sqlx::query(
            "SELECT run.id,run.project_id,run.frame_id,run.context_id,run.title,run.kind,run.status,\
                    run.command,run.script_path,run.input_refs_json,run.output_specs_json,run.created_at,\
                    run.started_at,run.ended_at,run.exit_code,run.stdout_tail,run.stderr_tail,\
                    run.remote_workdir,run.remote_handle_json,run.timeout_secs,run.last_polled_at,\
                    run.last_poll_error,run.progress_json,run.env_snapshot_json,run.harvested_at,run.cleaned_at,run.cleanup_error,run.logs_path FROM runs run \
             WHERE run.project_id=? AND (run.exploration_id=? OR (run.exploration_id IS NULL \
               AND EXISTS(SELECT 1 FROM explorations exploration \
                 JOIN exploration_baseline_entities baseline \
                   ON baseline.checkpoint_id=exploration.checkpoint_id \
                 WHERE exploration.id=? AND baseline.entity_kind='run' \
                   AND baseline.entity_id=run.id))) ORDER BY run.created_at DESC,run.id DESC",
        )
        .bind(project_id)
        .bind(exploration_id)
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn list_run_summaries_in_scope(&self, scope: &StateScope) -> Result<Vec<RunSummary>> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.list_run_summaries_in_scope(scope)).await;
        }
        let columns = "run.id,run.frame_id,run.context_id,run.title,run.kind,run.status,\
            run.created_at,run.started_at,run.ended_at,run.exit_code,run.remote_workdir,\
            run.timeout_secs,run.last_polled_at,substr(run.last_poll_error,1,2048) AS last_poll_error,\
            run.progress_json,run.harvested_at,run.cleaned_at,run.cleanup_error,printf('%d:%s:%s|%d:%s:%s',\
              length(CAST(coalesce(run.stdout_tail,'') AS BLOB)),substr(coalesce(run.stdout_tail,''),1,64),\
              substr(coalesce(run.stdout_tail,''),-128),\
              length(CAST(coalesce(run.stderr_tail,'') AS BLOB)),\
              substr(coalesce(run.stderr_tail,''),1,64),substr(coalesce(run.stderr_tail,''),-128)) \
              AS output_fingerprint";
        let rows = match scope {
            StateScope::Mainline { project_id } => {
                let sql = format!(
                    "SELECT {columns} FROM runs run WHERE run.project_id=? \
                     AND run.exploration_id IS NULL ORDER BY run.created_at DESC,run.id DESC"
                );
                sqlx::query(&sql)
                    .bind(project_id)
                    .fetch_all(&self.pool)
                    .await?
            }
            StateScope::Exploration {
                project_id,
                exploration_id,
            } => {
                let sql = format!(
                    "SELECT {columns} FROM runs run WHERE run.project_id=? \
                     AND (run.exploration_id=? OR (run.exploration_id IS NULL AND EXISTS(\
                       SELECT 1 FROM explorations exploration \
                       JOIN exploration_baseline_entities baseline \
                         ON baseline.checkpoint_id=exploration.checkpoint_id \
                       WHERE exploration.id=? AND baseline.entity_kind='run' \
                         AND baseline.entity_id=run.id))) \
                     ORDER BY run.created_at DESC,run.id DESC"
                );
                sqlx::query(&sql)
                    .bind(project_id)
                    .bind(exploration_id)
                    .bind(exploration_id)
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        rows.into_iter().map(run_summary_from_row).collect()
    }

    pub async fn list_runs_owned_by_exploration(
        &self,
        exploration_id: &str,
    ) -> Result<Vec<RunRecord>> {
        if let Some(store) = self
            .route_entity("explorations", "id", exploration_id)
            .await?
        {
            return Box::pin(store.list_runs_owned_by_exploration(exploration_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json,harvested_at,cleaned_at,cleanup_error,logs_path \
             FROM runs WHERE exploration_id=? ORDER BY created_at,id",
        )
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn list_uncleaned_runs_for_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<RunRecord>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_uncleaned_runs_for_project(project_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json,harvested_at,cleaned_at,cleanup_error,logs_path \
             FROM runs WHERE project_id=? AND cleaned_at IS NULL \
             AND remote_handle_json IS NOT NULL \
             AND status IN ('succeeded','failed','cancelled','timed_out','lost') \
             ORDER BY created_at, id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn list_active_runs_for_project(&self, project_id: &str) -> Result<Vec<RunRecord>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_active_runs_for_project(project_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json,harvested_at,cleaned_at,cleanup_error,logs_path \
             FROM runs WHERE project_id=? AND status IN ('submitted','running','cancelling') \
             ORDER BY created_at, id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn list_active_runs_for_context(&self, context_id: &str) -> Result<Vec<RunRecord>> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                let value = Box::pin(store.list_active_runs_for_context(context_id)).await?;
                result.extend(value);
            }
            result.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
            return Ok(result);
        }
        let rows = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json,harvested_at,cleaned_at,cleanup_error,logs_path \
             FROM runs WHERE context_id=? AND status IN ('submitted','running','cancelling') \
             ORDER BY created_at, id",
        )
        .bind(context_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn count_active_runs_on_context(&self, context_id: &str) -> Result<i64> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = 0;
            for store in stores {
                let value = Box::pin(store.count_active_runs_on_context(context_id)).await?;
                result += value;
            }
            return Ok(result);
        }
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM runs \
             WHERE context_id=? AND status IN ('submitted','running','cancelling')",
        )
        .bind(context_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_active_runs(&self) -> Result<Vec<RunRecord>> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                let value = Box::pin(store.list_active_runs()).await?;
                result.extend(value);
            }
            result.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
            return Ok(result);
        }
        let rows = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json,harvested_at,cleaned_at,cleanup_error,logs_path \
             FROM runs WHERE status IN ('submitted','running','cancelling') \
             ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn claim_run_lifecycle(
        &self,
        id: &str,
        owner: &str,
        lease_secs: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.claim_run_lifecycle(id, owner, lease_secs)).await;
        }
        if owner.is_empty() || lease_secs <= 0 {
            anyhow::bail!("Run lifecycle lease requires an owner and positive duration");
        }
        let now = chrono::Utc::now().timestamp();
        let lease_until = now.saturating_add(lease_secs);
        let updated = sqlx::query(
            "UPDATE runs SET lifecycle_owner=?, lifecycle_lease_until=? \
             WHERE id=? AND status IN ('submitted','running','cancelling') \
             AND (lifecycle_owner IS NULL OR lifecycle_lease_until IS NULL \
                  OR lifecycle_lease_until<=? \
                  OR (lifecycle_owner=? AND lifecycle_lease_until>?))",
        )
        .bind(owner)
        .bind(lease_until)
        .bind(id)
        .bind(now)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn renew_run_lifecycle(
        &self,
        id: &str,
        owner: &str,
        lease_secs: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.renew_run_lifecycle(id, owner, lease_secs)).await;
        }
        if owner.is_empty() || lease_secs <= 0 {
            anyhow::bail!("Run lifecycle lease requires an owner and positive duration");
        }
        let lease_until = chrono::Utc::now().timestamp().saturating_add(lease_secs);
        let updated = sqlx::query(
            "UPDATE runs SET lifecycle_lease_until=? \
             WHERE id=? AND lifecycle_owner=? \
             AND lifecycle_lease_until>? \
             AND status IN ('submitted','running','cancelling')",
        )
        .bind(lease_until)
        .bind(id)
        .bind(owner)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Atomically make a newly created draft Run active and assign its lifecycle owner.
    pub async fn activate_run_lifecycle(
        &self,
        id: &str,
        status: RunStatus,
        owner: &str,
        lease_secs: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.activate_run_lifecycle(id, status, owner, lease_secs)).await;
        }
        if !matches!(status, RunStatus::Submitted | RunStatus::Running) {
            anyhow::bail!("Run activation requires submitted or running status");
        }
        if owner.is_empty() || lease_secs <= 0 {
            anyhow::bail!("Run lifecycle lease requires an owner and positive duration");
        }
        let now = chrono::Utc::now().timestamp();
        let started_at = (status == RunStatus::Running).then_some(now);
        let updated = sqlx::query(
            "UPDATE runs SET status=?, started_at=?, lifecycle_owner=?, lifecycle_lease_until=? \
             WHERE id=? AND status='draft' AND lifecycle_owner IS NULL",
        )
        .bind(status.as_str())
        .bind(started_at)
        .bind(owner)
        .bind(now.saturating_add(lease_secs))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Request cancellation without taking ownership away from the active lifecycle.
    pub async fn request_run_cancellation(&self, id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.request_run_cancellation(id)).await;
        }
        let updated = sqlx::query(
            "UPDATE runs SET status='cancelling' \
             WHERE id=? AND status IN ('draft','submitted','running','paused')",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn set_run_remote_handle_owned(
        &self,
        id: &str,
        owner: &str,
        remote_handle_json: &str,
        remote_workdir: &str,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.set_run_remote_handle_owned(
                id,
                owner,
                remote_handle_json,
                remote_workdir,
            ))
            .await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET remote_handle_json=?, remote_workdir=? \
             WHERE id=? AND lifecycle_owner=? AND lifecycle_lease_until>? \
             AND status IN ('submitted','running','cancelling')",
        )
        .bind(remote_handle_json)
        .bind(remote_workdir)
        .bind(id)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn record_run_poll_owned(
        &self,
        id: &str,
        owner: &str,
        stdout_tail: Option<&str>,
        stderr_tail: Option<&str>,
        error: Option<&str>,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.record_run_poll_owned(
                id,
                owner,
                stdout_tail,
                stderr_tail,
                error,
            ))
            .await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET last_polled_at=?, stdout_tail=COALESCE(?,stdout_tail), \
             stderr_tail=COALESCE(?,stderr_tail), last_poll_error=? \
             WHERE id=? AND lifecycle_owner=? AND lifecycle_lease_until>? \
             AND status IN ('submitted','running','cancelling')",
        )
        .bind(now)
        .bind(stdout_tail)
        .bind(stderr_tail)
        .bind(error)
        .bind(id)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn update_run_output_owned(
        &self,
        id: &str,
        owner: &str,
        stdout_tail: Option<&str>,
        stderr_tail: Option<&str>,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.update_run_output_owned(id, owner, stdout_tail, stderr_tail))
                .await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET stdout_tail=?, stderr_tail=? \
             WHERE id=? AND lifecycle_owner=? AND lifecycle_lease_until>? \
             AND status IN ('submitted','running','cancelling')",
        )
        .bind(stdout_tail)
        .bind(stderr_tail)
        .bind(id)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn update_run_progress_owned(
        &self,
        id: &str,
        owner: &str,
        progress: &super::RunProgress,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.update_run_progress_owned(id, owner, progress)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let progress_json = serde_json::to_string(progress)?;
        let updated = sqlx::query(
            "UPDATE runs SET progress_json=? \
             WHERE id=? AND lifecycle_owner=? AND lifecycle_lease_until>? \
             AND status IN ('submitted','running','cancelling')",
        )
        .bind(progress_json)
        .bind(id)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn transition_run_to_running_owned(&self, id: &str, owner: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.transition_run_to_running_owned(id, owner)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET status='running', started_at=COALESCE(started_at,?) \
             WHERE id=? AND status='submitted' AND lifecycle_owner=? \
             AND lifecycle_lease_until>?",
        )
        .bind(now)
        .bind(id)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn finish_active_run_owned(
        &self,
        id: &str,
        owner: &str,
        status: RunStatus,
        exit_code: Option<i64>,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.finish_active_run_owned(id, owner, status, exit_code)).await;
        }
        if !status.is_terminal() {
            anyhow::bail!("finish_active_run requires a terminal status");
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET status=?, started_at=COALESCE(started_at,?), ended_at=?, exit_code=?, \
             lifecycle_owner=NULL, lifecycle_lease_until=NULL \
             WHERE id=? AND lifecycle_owner=? AND lifecycle_lease_until>? \
             AND status IN ('submitted','running','cancelling')",
        )
        .bind(status.as_str())
        .bind(now)
        .bind(now)
        .bind(exit_code)
        .bind(id)
        .bind(owner)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Force a Cancelling Run to a terminal status without holding the lease.
    /// Used when a second cancel must unstick a wedged cancel/poll RPC.
    pub async fn force_finish_cancelling_run(
        &self,
        id: &str,
        status: RunStatus,
        exit_code: Option<i64>,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.force_finish_cancelling_run(id, status, exit_code)).await;
        }
        if !status.is_terminal() {
            anyhow::bail!("force_finish_cancelling_run requires a terminal status");
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET status=?, started_at=COALESCE(started_at,?), ended_at=?, exit_code=?, \
             lifecycle_owner=NULL, lifecycle_lease_until=NULL \
             WHERE id=? AND status='cancelling'",
        )
        .bind(status.as_str())
        .bind(now)
        .bind(now)
        .bind(exit_code)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn mark_run_lost_owned(&self, id: &str, owner: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.mark_run_lost_owned(id, owner)).await;
        }
        self.finish_active_run_owned(id, owner, RunStatus::Lost, None)
            .await
    }

    /// Record that this Run's declared outputs were registered (and, for
    /// remote Runs, downloaded and checksum-verified). Idempotent.
    pub async fn mark_run_harvested(&self, id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.mark_run_harvested(id)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated =
            sqlx::query("UPDATE runs SET harvested_at=? WHERE id=? AND harvested_at IS NULL")
                .bind(now)
                .bind(id)
                .execute(&self.pool)
                .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Per-project retention windows (days) for automatic server reclamation:
    /// succeeded+harvested run workspaces, failed/cancelled/timed-out run
    /// workspaces, and orphaned ledgered remote files (uploads and persisted
    /// outputs no live run or artifact references). NULL disables the
    /// respective sweep.
    pub async fn project_run_retention(
        &self,
        project_id: &str,
    ) -> Result<(Option<i64>, Option<i64>, Option<i64>)> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.project_run_retention(project_id)).await;
        }
        let row: Option<(Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT run_retention_days, failed_run_retention_days, orphan_file_retention_days \
             FROM projects WHERE id=?",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.unwrap_or((None, None, None)))
    }

    pub async fn set_project_run_retention(
        &self,
        project_id: &str,
        run_retention_days: Option<i64>,
        failed_run_retention_days: Option<i64>,
        orphan_file_retention_days: Option<i64>,
    ) -> Result<()> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.set_project_run_retention(
                project_id,
                run_retention_days,
                failed_run_retention_days,
                orphan_file_retention_days,
            ))
            .await;
        }
        for value in [
            run_retention_days,
            failed_run_retention_days,
            orphan_file_retention_days,
        ]
        .into_iter()
        .flatten()
        {
            if !(1..=3650).contains(&value) {
                anyhow::bail!("Retention windows must be between 1 and 3650 days");
            }
        }
        let updated = sqlx::query(
            "UPDATE projects SET run_retention_days=?, failed_run_retention_days=?, \
             orphan_file_retention_days=?, updated_at=? WHERE id=?",
        )
        .bind(run_retention_days)
        .bind(failed_run_retention_days)
        .bind(orphan_file_retention_days)
        .bind(chrono::Utc::now().timestamp())
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Project not found");
        }
        Ok(())
    }

    /// (project, SSH context, cutoff) pairs whose unremoved staging entries
    /// are old enough for the opt-in orphan-file sweep to inspect. State
    /// classification (active/replaced/orphan) happens at sweep time.
    pub async fn list_orphan_gc_contexts(&self, now: i64) -> Result<Vec<(String, String, i64)>> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                let value = Box::pin(store.list_orphan_gc_contexts(now)).await?;
                result.extend(value);
            }
            result.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            return Ok(result);
        }
        Ok(sqlx::query_as(
            "SELECT DISTINCT s.project_id, s.context_id, \
                    ? - p.orphan_file_retention_days*86400 AS cutoff \
             FROM remote_staging s JOIN projects p ON p.id=s.project_id \
             WHERE p.orphan_file_retention_days IS NOT NULL \
             AND s.removed_at IS NULL AND s.context_id LIKE 'ssh:%' \
             AND s.created_at < ? - p.orphan_file_retention_days*86400 \
             ORDER BY s.project_id, s.context_id",
        )
        .bind(now)
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Runs whose server workspaces are due for automatic retention cleanup:
    /// succeeded runs only after their outputs were harvested (or none were
    /// declared), failed/cancelled/timed-out/lost runs on their own window.
    pub async fn list_runs_due_for_retention(&self, now: i64) -> Result<Vec<RunRecord>> {
        if let Some(stores) = self.routed_projects().await? {
            let mut result = Vec::new();
            for store in stores {
                let value = Box::pin(store.list_runs_due_for_retention(now)).await?;
                result.extend(value);
            }
            result.sort_by(|a, b| a.ended_at.cmp(&b.ended_at).then(a.id.cmp(&b.id)));
            return Ok(result);
        }
        let rows = sqlx::query(
            "SELECT r.id,r.project_id,r.frame_id,r.context_id,r.title,r.kind,r.status,r.command,\
                    r.script_path,r.input_refs_json,r.output_specs_json,r.created_at,r.started_at,\
                    r.ended_at,r.exit_code,r.stdout_tail,r.stderr_tail,r.remote_workdir,\
                    r.remote_handle_json,r.timeout_secs,r.last_polled_at,r.last_poll_error,\
                    r.progress_json,r.env_snapshot_json,r.harvested_at,r.cleaned_at,r.cleanup_error,r.logs_path \
             FROM runs r JOIN projects p ON p.id=r.project_id \
             WHERE r.cleaned_at IS NULL AND r.remote_handle_json IS NOT NULL \
             AND r.ended_at IS NOT NULL \
             AND r.kind IN ('ssh_direct','local_detached') \
             AND ((r.status='succeeded' AND p.run_retention_days IS NOT NULL \
                   AND (r.harvested_at IS NOT NULL OR r.output_specs_json='[]') \
                   AND r.ended_at < ? - p.run_retention_days*86400) \
               OR (r.status IN ('failed','cancelled','timed_out','lost') \
                   AND p.failed_run_retention_days IS NOT NULL \
                   AND r.ended_at < ? - p.failed_run_retention_days*86400)) \
             ORDER BY r.ended_at, r.id",
        )
        .bind(now)
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    /// Record that this Run's server-side workspace was deleted. Idempotent;
    /// clears any earlier cleanup error.
    /// Record where the run's full logs were saved inside the project
    /// workspace (pulled back before cleanup deletes the server workdir).
    pub async fn mark_run_logs_saved(&self, id: &str, logs_path: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.mark_run_logs_saved(id, logs_path)).await;
        }
        let updated = sqlx::query("UPDATE runs SET logs_path=? WHERE id=? AND logs_path IS NULL")
            .bind(logs_path)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Record that the user closed this Run's results-review prompt so it is
    /// never auto-opened again. Manual review stays available. Idempotent:
    /// returns false when already dismissed.
    pub async fn mark_run_review_dismissed(&self, id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.mark_run_review_dismissed(id)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET review_dismissed_at=? WHERE id=? AND review_dismissed_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Whether the user already closed this Run's results-review prompt.
    pub async fn run_review_dismissed(&self, id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.run_review_dismissed(id)).await;
        }
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT review_dismissed_at FROM runs WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some_and(|(dismissed,)| dismissed.is_some()))
    }

    pub async fn mark_run_cleaned(&self, id: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.mark_run_cleaned(id)).await;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET cleaned_at=?, cleanup_error=NULL WHERE id=? AND cleaned_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Surface a failed workspace cleanup so the user can retry.
    pub async fn record_run_cleanup_error(&self, id: &str, error: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.record_run_cleanup_error(id, error)).await;
        }
        let updated =
            sqlx::query("UPDATE runs SET cleanup_error=? WHERE id=? AND cleaned_at IS NULL")
                .bind(error)
                .bind(id)
                .execute(&self.pool)
                .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Surface a harvest failure on an already-terminal Run so the UI and
    /// retry tooling can see why outputs were not registered.
    pub async fn record_run_harvest_error(&self, id: &str, error: &str) -> Result<bool> {
        if let Some(store) = self.route_entity("runs", "id", id).await? {
            return Box::pin(store.record_run_harvest_error(id, error)).await;
        }
        let updated = sqlx::query("UPDATE runs SET last_poll_error=? WHERE id=?")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn save_run_artifact_link(
        &self,
        id: &str,
        run_id: &str,
        artifact_id: &str,
        role: &str,
    ) -> Result<()> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.save_run_artifact_link(id, run_id, artifact_id, role)).await;
        }
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO run_artifacts(id,run_id,artifact_id,role,created_at) VALUES(?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET run_id=excluded.run_id, artifact_id=excluded.artifact_id, role=excluded.role",
        )
        .bind(id)
        .bind(run_id)
        .bind(artifact_id)
        .bind(role)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let project_id: Option<String> = sqlx::query_scalar(
            "SELECT r.project_id FROM runs r JOIN artifacts a ON a.id=? \
             WHERE r.id=? AND a.project_id=r.project_id",
        )
        .bind(artifact_id)
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        let project_id = project_id.ok_or_else(|| {
            anyhow::anyhow!("Run and artifact must exist in the same project before linking")
        })?;
        self.save_research_edge(&ResearchEdge::new(
            format!("run-artifact:{run_id}:{artifact_id}"),
            project_id,
            run_node_id(run_id),
            artifact_node_id(artifact_id),
            "produced",
        )?)
        .await?;
        Ok(())
    }
}
