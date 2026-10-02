use super::{AgentWorkflowAttemptStatus, RunRecord, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

pub const METHOD_SEARCH_ACTIVITY: &str = "method_search";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorkflowRunActivity {
    pub attempt_id: String,
    pub run_id: String,
    pub activity: String,
    pub state_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl AgentWorkflowRunActivity {
    pub fn new(
        attempt_id: impl Into<String>,
        run_id: impl Into<String>,
        activity: impl Into<String>,
    ) -> Result<Self> {
        let now = chrono::Utc::now().timestamp();
        let value = Self {
            attempt_id: attempt_id.into(),
            run_id: run_id.into(),
            activity: activity.into(),
            state_json: "{}".into(),
            created_at: now,
            updated_at: now,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<()> {
        if self.attempt_id.trim().is_empty() || self.run_id.trim().is_empty() {
            anyhow::bail!("workflow Run activity requires attempt_id and run_id");
        }
        if self.activity != METHOD_SEARCH_ACTIVITY {
            anyhow::bail!("unsupported workflow Run activity: {}", self.activity);
        }
        if !serde_json::from_str::<serde_json::Value>(&self.state_json)
            .is_ok_and(|value| value.is_object())
        {
            anyhow::bail!("workflow Run activity state_json must be a JSON object");
        }
        Ok(())
    }
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AgentWorkflowRunActivity> {
    let value = AgentWorkflowRunActivity {
        attempt_id: row.try_get("attempt_id")?,
        run_id: row.try_get("run_id")?,
        activity: row.try_get("activity")?,
        state_json: row.try_get("state_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    };
    value.validate()?;
    Ok(value)
}

const SELECT_ACTIVITY: &str = "SELECT attempt_id,run_id,activity,state_json,created_at,updated_at FROM agent_workflow_run_activities";

impl Store {
    /// Atomically insert a draft Run, link it to a running Workflow attempt,
    /// and release the attempt's Agent slot by moving it to `waiting_run`.
    pub async fn create_agent_workflow_run_activity(
        &self,
        run: &RunRecord,
        activity: &AgentWorkflowRunActivity,
    ) -> Result<()> {
        if let Some(store) = self.route_project(&run.project_id).await? {
            return Box::pin(store.create_agent_workflow_run_activity(run, activity)).await;
        }
        self.create_agent_workflow_run_activity_inner(run, activity, None)
            .await
    }

    /// Method-search initialization extends the Run/link/waiting transition
    /// with its durable checkpoint row in the same SQLite transaction.
    pub async fn create_method_search_workflow_run_activity(
        &self,
        run: &RunRecord,
        activity: &AgentWorkflowRunActivity,
        state: &super::MethodSearchRunState,
    ) -> Result<()> {
        if let Some(store) = self.route_project(&run.project_id).await? {
            return Box::pin(
                store.create_method_search_workflow_run_activity(run, activity, state),
            )
            .await;
        }
        state.validate()?;
        if state.run_id != run.id {
            anyhow::bail!("method-search state does not match the linked Run");
        }
        self.create_agent_workflow_run_activity_inner(run, activity, Some(state))
            .await
    }

    async fn create_agent_workflow_run_activity_inner(
        &self,
        run: &RunRecord,
        activity: &AgentWorkflowRunActivity,
        method_state: Option<&super::MethodSearchRunState>,
    ) -> Result<()> {
        run.validate()?;
        activity.validate()?;
        if run.id != activity.run_id {
            anyhow::bail!("workflow Run activity link does not match the Run id");
        }
        if run.status != super::RunStatus::Draft {
            anyhow::bail!("workflow Run activities must create a draft Run");
        }
        if run.kind != METHOD_SEARCH_ACTIVITY || activity.activity != METHOD_SEARCH_ACTIVITY {
            anyhow::bail!("workflow Run activity kind does not match method_search");
        }

        let environment = serde_json::from_str::<serde_json::Value>(&run.env_snapshot_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        let (environment_json, environment_hash) = super::canonical_json_sha256(&environment);
        let packages_json = environment
            .get("packages")
            .map(super::canonical_json)
            .unwrap_or_else(|| "[]".into());
        let command = run.command.as_deref().unwrap_or_default();
        let command_checksum = {
            use sha2::{Digest, Sha256};
            let mut digest = Sha256::new();
            digest.update(command.as_bytes());
            hex::encode(digest.finalize())
        };

        let mut tx = self.begin_write().await?;
        let ownership = sqlx::query(
            "SELECT a.status,w.project_id,s.task_kind,s.activity_json \
             FROM agent_workflow_attempts a \
             JOIN agent_workflows w ON w.id=a.workflow_id \
             JOIN agent_workflow_steps s ON s.id=a.step_id AND s.workflow_id=a.workflow_id \
             WHERE a.id=?",
        )
        .bind(&activity.attempt_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("workflow Run activity attempt does not exist"))?;
        if ownership.try_get::<String, _>("status")? != "running" {
            anyhow::bail!("workflow Run activity attempt must be running before it can wait");
        }
        if ownership.try_get::<String, _>("project_id")? != run.project_id {
            anyhow::bail!("workflow Run activity and Run must belong to the same project");
        }
        if ownership.try_get::<String, _>("task_kind")? != "run_activity" {
            anyhow::bail!("workflow attempt step is not a Run activity");
        }
        let activity_spec: serde_json::Value =
            serde_json::from_str(&ownership.try_get::<String, _>("activity_json")?)?;
        if activity_spec
            .get("activity")
            .and_then(serde_json::Value::as_str)
            != Some(activity.activity.as_str())
        {
            anyhow::bail!("workflow Run activity does not match its approved step");
        }

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
        .execute(&mut *tx)
        .await?;
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
        .bind(command_checksum)
        .bind(Option::<&str>::None)
        .bind(Option::<&str>::None)
        .bind(Option::<&str>::None)
        .bind(run.created_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO env_snapshots(hash,env_name,packages_json,snapshot_json,hash_algorithm,created_at) \
             VALUES(?,?,?,?,?,?) ON CONFLICT(hash) DO UPDATE SET \
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
        sqlx::query(
            "INSERT INTO agent_workflow_run_activities(attempt_id,run_id,activity,state_json,created_at,updated_at) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(&activity.attempt_id)
        .bind(&activity.run_id)
        .bind(&activity.activity)
        .bind(&activity.state_json)
        .bind(activity.created_at)
        .bind(activity.updated_at)
        .execute(&mut *tx)
        .await?;
        if let Some(state) = method_state {
            let exact_spec: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM artifact_versions version \
                 JOIN artifacts artifact ON artifact.id=version.artifact_id \
                 WHERE version.id=? AND artifact.project_id=? AND version.checksum=?)",
            )
            .bind(&state.spec_artifact_version_id)
            .bind(&run.project_id)
            .bind(&state.spec_sha256)
            .fetch_one(&mut *tx)
            .await?;
            if !exact_spec {
                anyhow::bail!(
                    "method-search spec must be an exact ArtifactVersion in the Run project"
                );
            }
            sqlx::query(
                "INSERT INTO method_search_runs(run_id,spec_artifact_version_id,spec_sha256,activity_version,checkpoint_json,control_state,result_status,created_at,updated_at) \
                 VALUES(?,?,?,?,?,?,?,?,?)",
            )
            .bind(&state.run_id)
            .bind(&state.spec_artifact_version_id)
            .bind(&state.spec_sha256)
            .bind(state.activity_version)
            .bind(&state.checkpoint_json)
            .bind(&state.control_state)
            .bind(state.result_status.as_deref())
            .bind(state.created_at)
            .bind(state.updated_at)
            .execute(&mut *tx)
            .await?;
        }
        let updated = sqlx::query(
            "UPDATE agent_workflow_attempts SET status='waiting_run',delegation_slot_yielded=0,updated_at=? \
             WHERE id=? AND status='running'",
        )
        .bind(activity.updated_at)
        .bind(&activity.attempt_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("workflow Run activity attempt changed before it could wait");
        }
        tx.commit().await?;

        let mut node = super::ResearchNode::new(
            super::run_node_id(&run.id),
            &run.project_id,
            super::ResearchNodeKind::Run,
            &run.title,
        )?;
        node.ref_id = Some(run.id.clone());
        self.save_research_node(&node).await?;
        Ok(())
    }

    pub async fn get_agent_workflow_run_activity(
        &self,
        attempt_id: &str,
    ) -> Result<Option<AgentWorkflowRunActivity>> {
        if let Some(store) = self
            .route_entity("agent_workflow_attempts", "id", attempt_id)
            .await?
        {
            return Box::pin(store.get_agent_workflow_run_activity(attempt_id)).await;
        }
        sqlx::query(&format!("{SELECT_ACTIVITY} WHERE attempt_id=?"))
            .bind(attempt_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }

    pub async fn get_agent_workflow_run_activity_by_run(
        &self,
        run_id: &str,
    ) -> Result<Option<AgentWorkflowRunActivity>> {
        if let Some(store) = self.route_entity("runs", "id", run_id).await? {
            return Box::pin(store.get_agent_workflow_run_activity_by_run(run_id)).await;
        }
        sqlx::query(&format!("{SELECT_ACTIVITY} WHERE run_id=?"))
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }

    pub async fn list_agent_workflow_run_activities(
        &self,
        workflow_id: &str,
    ) -> Result<Vec<AgentWorkflowRunActivity>> {
        if let Some(store) = self
            .route_entity("agent_workflows", "id", workflow_id)
            .await?
        {
            return Box::pin(store.list_agent_workflow_run_activities(workflow_id)).await;
        }
        let rows = sqlx::query(&format!(
            "{SELECT_ACTIVITY} WHERE attempt_id IN (SELECT id FROM agent_workflow_attempts WHERE workflow_id=?) ORDER BY created_at,attempt_id"
        ))
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }

    pub async fn update_agent_workflow_run_activity_state(
        &self,
        attempt_id: &str,
        state_json: &str,
    ) -> Result<bool> {
        if let Some(store) = self
            .route_entity("agent_workflow_attempts", "id", attempt_id)
            .await?
        {
            return Box::pin(
                store.update_agent_workflow_run_activity_state(attempt_id, state_json),
            )
            .await;
        }
        if !serde_json::from_str::<serde_json::Value>(state_json)
            .is_ok_and(|value| value.is_object())
        {
            anyhow::bail!("workflow Run activity state_json must be a JSON object");
        }
        let updated = sqlx::query(
            "UPDATE agent_workflow_run_activities SET state_json=?,updated_at=? WHERE attempt_id=?",
        )
        .bind(state_json)
        .bind(chrono::Utc::now().timestamp())
        .bind(attempt_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Terminalize a waiting activity from the authoritative linked Run. The
    /// compare-and-set makes repeated reconciliation idempotent.
    pub async fn reconcile_agent_workflow_run_activity(
        &self,
        attempt_id: &str,
    ) -> Result<Option<AgentWorkflowAttemptStatus>> {
        if let Some(store) = self
            .route_entity("agent_workflow_attempts", "id", attempt_id)
            .await?
        {
            return Box::pin(store.reconcile_agent_workflow_run_activity(attempt_id)).await;
        }
        let Some(row) = sqlx::query(
            "SELECT r.status,r.id FROM agent_workflow_run_activities link \
             JOIN runs r ON r.id=link.run_id WHERE link.attempt_id=?",
        )
        .bind(attempt_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let run_status = super::RunStatus::from_storage(&row.try_get::<String, _>("status")?)?;
        if !run_status.is_terminal() {
            return Ok(Some(AgentWorkflowAttemptStatus::WaitingRun));
        }
        let (attempt_status, error) = match run_status {
            super::RunStatus::Succeeded => (AgentWorkflowAttemptStatus::Succeeded, None),
            super::RunStatus::Cancelled => (
                AgentWorkflowAttemptStatus::Cancelled,
                Some("Linked Run was cancelled."),
            ),
            super::RunStatus::Failed | super::RunStatus::TimedOut | super::RunStatus::Lost => (
                AgentWorkflowAttemptStatus::Failed,
                Some("Linked Run did not complete successfully."),
            ),
            _ => unreachable!("terminal Run status matched above"),
        };
        let now = chrono::Utc::now().timestamp();
        let run_id: String = row.try_get("id")?;
        let response = serde_json::json!({"run_id": run_id, "run_status": run_status.as_str()});
        let changed = sqlx::query(
            "UPDATE agent_workflow_attempts SET status=?,response_json=?,output_json=?,error=?,finished_at=?,updated_at=? \
             WHERE id=? AND status='waiting_run'",
        )
        .bind(attempt_status.as_str())
        .bind(response.to_string())
        .bind(response.to_string())
        .bind(error)
        .bind(now)
        .bind(now)
        .bind(attempt_id)
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() == 0 {
            return Ok(self
                .get_agent_workflow_attempt(attempt_id)
                .await?
                .map(|attempt| attempt.status));
        }
        Ok(Some(attempt_status))
    }
}
