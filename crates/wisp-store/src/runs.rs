use super::{
    artifact_node_id, run_from_row, run_node_id, validate_run_transition, ResearchEdge,
    ResearchNode, ResearchNodeKind, RunRecord, RunStatus, Store,
};
use anyhow::Result;
use sqlx::Row;

impl Store {
    pub async fn project_has_active_runs(&self, project_id: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runs WHERE project_id=? \
             AND status IN ('submitted','running','cancelling')",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn create_run(&self, run: &RunRecord) -> Result<()> {
        run.validate()?;
        sqlx::query(
            "INSERT INTO runs(\
                id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                last_polled_at,last_poll_error,progress_json,env_snapshot_json\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
        .execute(&self.pool)
        .await?;
        let mut node = ResearchNode::new(
            run_node_id(&run.id),
            &run.project_id,
            ResearchNodeKind::Run,
            &run.title,
        )?;
        node.ref_id = Some(run.id.clone());
        self.save_research_node(&node).await?;
        Ok(())
    }

    pub async fn get_run(&self, id: &str) -> Result<Option<RunRecord>> {
        let row = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json \
             FROM runs WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(run_from_row).transpose()
    }

    pub async fn list_runs_by_project(&self, project_id: &str) -> Result<Vec<RunRecord>> {
        let rows = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json \
             FROM runs WHERE project_id=? ORDER BY created_at DESC, id DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn list_active_runs(&self) -> Result<Vec<RunRecord>> {
        let rows = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                    last_polled_at,last_poll_error,progress_json,env_snapshot_json \
             FROM runs WHERE status IN ('submitted','running','cancelling') \
             ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    /// Advance a Run only if its status has not changed since validation.
    pub async fn update_run_status(&self, id: &str, status: RunStatus) -> Result<bool> {
        let run = self
            .get_run(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run not found"))?;
        validate_run_transition(run.status, status)?;
        let now = chrono::Utc::now().timestamp();
        let started_at = if status == RunStatus::Running && run.started_at.is_none() {
            Some(now)
        } else {
            run.started_at
        };
        let ended_at = if status.is_terminal() {
            Some(now)
        } else {
            run.ended_at
        };
        let updated = sqlx::query(
            "UPDATE runs SET status=?, started_at=?, ended_at=?, \
             lifecycle_owner=CASE WHEN ? THEN NULL ELSE lifecycle_owner END, \
             lifecycle_lease_until=CASE WHEN ? THEN NULL ELSE lifecycle_lease_until END \
             WHERE id=? AND status=?",
        )
        .bind(status.as_str())
        .bind(started_at)
        .bind(ended_at)
        .bind(status.is_terminal())
        .bind(status.is_terminal())
        .bind(id)
        .bind(run.status.as_str())
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn claim_run_lifecycle(
        &self,
        id: &str,
        owner: &str,
        lease_secs: i64,
    ) -> Result<bool> {
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
        let updated = sqlx::query(
            "UPDATE runs SET status='cancelling' \
             WHERE id=? AND status IN ('submitted','running')",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn release_run_lifecycle(&self, id: &str, owner: &str) -> Result<bool> {
        let updated = sqlx::query(
            "UPDATE runs SET lifecycle_owner=NULL, lifecycle_lease_until=NULL \
             WHERE id=? AND lifecycle_owner=?",
        )
        .bind(id)
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn update_run_output(
        &self,
        id: &str,
        stdout_tail: Option<&str>,
        stderr_tail: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE runs SET stdout_tail=?, stderr_tail=? WHERE id=?")
            .bind(stdout_tail)
            .bind(stderr_tail)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_run_remote_handle(
        &self,
        id: &str,
        remote_handle_json: &str,
        remote_workdir: &str,
    ) -> Result<()> {
        sqlx::query("UPDATE runs SET remote_handle_json=?, remote_workdir=? WHERE id=?")
            .bind(remote_handle_json)
            .bind(remote_workdir)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_run_remote_handle_owned(
        &self,
        id: &str,
        owner: &str,
        remote_handle_json: &str,
        remote_workdir: &str,
    ) -> Result<bool> {
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

    pub async fn record_run_poll(
        &self,
        id: &str,
        stdout_tail: Option<&str>,
        stderr_tail: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE runs SET last_polled_at=?, stdout_tail=COALESCE(?,stdout_tail), \
             stderr_tail=COALESCE(?,stderr_tail), last_poll_error=? WHERE id=?",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(stdout_tail)
        .bind(stderr_tail)
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_run_poll_owned(
        &self,
        id: &str,
        owner: &str,
        stdout_tail: Option<&str>,
        stderr_tail: Option<&str>,
        error: Option<&str>,
    ) -> Result<bool> {
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

    pub async fn finish_active_run(
        &self,
        id: &str,
        status: RunStatus,
        exit_code: Option<i64>,
    ) -> Result<bool> {
        if !status.is_terminal() {
            anyhow::bail!("finish_active_run requires a terminal status");
        }
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET status=?, started_at=COALESCE(started_at,?), ended_at=?, exit_code=?, \
             lifecycle_owner=NULL, lifecycle_lease_until=NULL \
             WHERE id=? AND status IN ('submitted','running','cancelling')",
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

    pub async fn finish_active_run_owned(
        &self,
        id: &str,
        owner: &str,
        status: RunStatus,
        exit_code: Option<i64>,
    ) -> Result<bool> {
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

    pub async fn mark_run_lost_owned(&self, id: &str, owner: &str) -> Result<bool> {
        self.finish_active_run_owned(id, owner, RunStatus::Lost, None)
            .await
    }

    pub async fn mark_run_lost(&self, id: &str) -> Result<bool> {
        self.finish_active_run(id, RunStatus::Lost, None).await
    }

    /// A desktop restart cannot safely reattach to an in-memory direct process.
    pub async fn mark_active_runs_lost(&self) -> Result<u64> {
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE runs SET status='lost', ended_at=?, lifecycle_owner=NULL, lifecycle_lease_until=NULL \
             WHERE status IN ('submitted','running','cancelling')",
        )
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected())
    }

    pub async fn finish_run(
        &self,
        id: &str,
        status: RunStatus,
        exit_code: Option<i64>,
    ) -> Result<bool> {
        if !status.is_terminal() {
            anyhow::bail!("finish_run requires a terminal status");
        }
        let run = self
            .get_run(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run not found"))?;
        validate_run_transition(run.status, status)?;
        let now = chrono::Utc::now().timestamp();
        let started_at = run.started_at.or(Some(now));
        let updated = sqlx::query(
            "UPDATE runs SET status=?, started_at=?, ended_at=?, exit_code=?, \
             lifecycle_owner=NULL, lifecycle_lease_until=NULL WHERE id=? AND status=?",
        )
        .bind(status.as_str())
        .bind(started_at)
        .bind(now)
        .bind(exit_code)
        .bind(id)
        .bind(run.status.as_str())
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

    pub async fn list_run_artifacts(&self, run_id: &str) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT artifact_id, role FROM run_artifacts WHERE run_id=? ORDER BY created_at ASC, id ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| Ok((r.try_get("artifact_id")?, r.try_get("role")?)))
            .collect()
    }
}
