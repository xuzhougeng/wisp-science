use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkflowAttemptStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Blocked,
}

impl AgentWorkflowAttemptStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "blocked" => Ok(Self::Blocked),
            _ => anyhow::bail!("unknown agent workflow attempt status: {value}"),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Blocked
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorkflowAttempt {
    pub id: String,
    pub workflow_id: String,
    pub step_id: String,
    pub attempt: i64,
    pub request_id: String,
    pub backend: String,
    pub status: AgentWorkflowAttemptStatus,
    pub request_json: String,
    pub response_json: Option<String>,
    pub output_json: String,
    pub artifact_ids_json: String,
    pub evidence_json: String,
    pub error: Option<String>,
    pub agent_session_id: Option<String>,
    pub child_frame_id: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tool_calls: i64,
    pub cost_microunits: i64,
    pub cancel_requested: bool,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl AgentWorkflowAttempt {
    pub fn queued(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        step_id: impl Into<String>,
        attempt: i64,
        request_id: impl Into<String>,
        backend: impl Into<String>,
        request_json: impl Into<String>,
    ) -> Result<Self> {
        let now = chrono::Utc::now().timestamp();
        let value = Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            step_id: step_id.into(),
            attempt,
            request_id: request_id.into(),
            backend: backend.into(),
            status: AgentWorkflowAttemptStatus::Queued,
            request_json: request_json.into(),
            response_json: None,
            output_json: "{}".into(),
            artifact_ids_json: "[]".into(),
            evidence_json: "[]".into(),
            error: None,
            agent_session_id: None,
            child_frame_id: None,
            input_tokens: 0,
            output_tokens: 0,
            tool_calls: 0,
            cost_microunits: 0,
            cancel_requested: false,
            started_at: None,
            finished_at: None,
            created_at: now,
            updated_at: now,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("id", self.id.as_str()),
            ("workflow_id", self.workflow_id.as_str()),
            ("step_id", self.step_id.as_str()),
            ("request_id", self.request_id.as_str()),
            ("backend", self.backend.as_str()),
        ] {
            if value.trim().is_empty() {
                anyhow::bail!("agent workflow attempt {field} is required");
            }
        }
        if self.attempt <= 0 {
            anyhow::bail!("agent workflow attempt number must be positive");
        }
        for (field, value, array) in [
            ("request_json", self.request_json.as_str(), false),
            ("output_json", self.output_json.as_str(), false),
            ("artifact_ids_json", self.artifact_ids_json.as_str(), true),
            ("evidence_json", self.evidence_json.as_str(), true),
        ] {
            let parsed = serde_json::from_str::<serde_json::Value>(value)
                .map_err(|_| anyhow::anyhow!("agent workflow attempt {field} must be JSON"))?;
            if (array && !parsed.is_array()) || (!array && !parsed.is_object()) {
                anyhow::bail!("agent workflow attempt {field} has the wrong JSON shape");
            }
        }
        if let Some(response) = &self.response_json {
            serde_json::from_str::<serde_json::Value>(response).map_err(|_| {
                anyhow::anyhow!("agent workflow attempt response_json must be JSON")
            })?;
        }
        if self.status == AgentWorkflowAttemptStatus::Succeeded
            && (self.response_json.is_none() || self.error.is_some())
        {
            anyhow::bail!("succeeded agent workflow attempts require a response and no error");
        }
        if self.status == AgentWorkflowAttemptStatus::Failed
            && self.error.as_deref().is_none_or(str::is_empty)
        {
            anyhow::bail!("failed agent workflow attempts require an error");
        }
        Ok(())
    }
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AgentWorkflowAttempt> {
    Ok(AgentWorkflowAttempt {
        id: row.try_get("id")?,
        workflow_id: row.try_get("workflow_id")?,
        step_id: row.try_get("step_id")?,
        attempt: row.try_get("attempt")?,
        request_id: row.try_get("request_id")?,
        backend: row.try_get("backend")?,
        status: AgentWorkflowAttemptStatus::from_storage(&row.try_get::<String, _>("status")?)?,
        request_json: row.try_get("request_json")?,
        response_json: row.try_get("response_json")?,
        output_json: row.try_get("output_json")?,
        artifact_ids_json: row.try_get("artifact_ids_json")?,
        evidence_json: row.try_get("evidence_json")?,
        error: row.try_get("error")?,
        agent_session_id: row.try_get("agent_session_id")?,
        child_frame_id: row.try_get("child_frame_id")?,
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        tool_calls: row.try_get("tool_calls")?,
        cost_microunits: row.try_get("cost_microunits")?,
        cancel_requested: row.try_get::<i64, _>("cancel_requested")? != 0,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const SELECT_ATTEMPT: &str = "SELECT id,workflow_id,step_id,attempt,request_id,backend,status,request_json,response_json,output_json,artifact_ids_json,evidence_json,error,agent_session_id,child_frame_id,input_tokens,output_tokens,tool_calls,cost_microunits,cancel_requested,started_at,finished_at,created_at,updated_at FROM agent_workflow_attempts";

impl Store {
    pub async fn create_agent_workflow_attempt(
        &self,
        attempt: &AgentWorkflowAttempt,
    ) -> Result<()> {
        attempt.validate()?;
        if attempt.status != AgentWorkflowAttemptStatus::Queued {
            anyhow::bail!("new agent workflow attempts must start queued");
        }
        let mut tx = self.pool.begin().await?;
        let runnable: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_workflow_steps s JOIN agent_workflows w ON w.id=s.workflow_id WHERE s.id=? AND s.workflow_id=? AND w.status='running'",
        )
        .bind(&attempt.step_id)
        .bind(&attempt.workflow_id)
        .fetch_one(&mut *tx)
        .await?;
        if runnable != 1 {
            anyhow::bail!("agent workflow attempt step is missing, mismatched, or not running");
        }
        sqlx::query(
            "INSERT INTO agent_workflow_attempts(id,workflow_id,step_id,attempt,request_id,backend,status,request_json,response_json,output_json,artifact_ids_json,evidence_json,error,agent_session_id,child_frame_id,input_tokens,output_tokens,tool_calls,cost_microunits,cancel_requested,started_at,finished_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&attempt.id)
        .bind(&attempt.workflow_id)
        .bind(&attempt.step_id)
        .bind(attempt.attempt)
        .bind(&attempt.request_id)
        .bind(&attempt.backend)
        .bind(attempt.status.as_str())
        .bind(&attempt.request_json)
        .bind(attempt.response_json.as_deref())
        .bind(&attempt.output_json)
        .bind(&attempt.artifact_ids_json)
        .bind(&attempt.evidence_json)
        .bind(attempt.error.as_deref())
        .bind(attempt.agent_session_id.as_deref())
        .bind(attempt.child_frame_id.as_deref())
        .bind(attempt.input_tokens)
        .bind(attempt.output_tokens)
        .bind(attempt.tool_calls)
        .bind(attempt.cost_microunits)
        .bind(attempt.cancel_requested as i64)
        .bind(attempt.started_at)
        .bind(attempt.finished_at)
        .bind(attempt.created_at)
        .bind(attempt.updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_agent_workflow_attempt(
        &self,
        id: &str,
    ) -> Result<Option<AgentWorkflowAttempt>> {
        sqlx::query(&format!("{SELECT_ATTEMPT} WHERE id=?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }

    pub async fn list_agent_workflow_attempts(
        &self,
        workflow_id: &str,
    ) -> Result<Vec<AgentWorkflowAttempt>> {
        let rows = sqlx::query(&format!(
            "{SELECT_ATTEMPT} WHERE workflow_id=? ORDER BY created_at,step_id,attempt"
        ))
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }

    pub async fn next_agent_workflow_attempt_number(&self, step_id: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(MAX(attempt),0)+1 FROM agent_workflow_attempts WHERE step_id=?",
        )
        .bind(step_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn latest_agent_workflow_step_session(
        &self,
        step_id: &str,
    ) -> Result<Option<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT agent_session_id,child_frame_id FROM agent_workflow_attempts WHERE step_id=? AND agent_session_id IS NOT NULL AND child_frame_id IS NOT NULL ORDER BY attempt DESC LIMIT 1",
        )
        .bind(step_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_agent_workflow_attempt(
        &self,
        attempt: &AgentWorkflowAttempt,
        expected_status: AgentWorkflowAttemptStatus,
    ) -> Result<bool> {
        attempt.validate()?;
        validate_transition(expected_status, attempt.status)?;
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE agent_workflow_attempts SET status=?,response_json=?,output_json=?,artifact_ids_json=?,evidence_json=?,error=?,agent_session_id=?,child_frame_id=?,input_tokens=?,output_tokens=?,tool_calls=?,cost_microunits=?,cancel_requested=?,started_at=?,finished_at=?,updated_at=? WHERE id=? AND status=?",
        )
        .bind(attempt.status.as_str())
        .bind(attempt.response_json.as_deref())
        .bind(&attempt.output_json)
        .bind(&attempt.artifact_ids_json)
        .bind(&attempt.evidence_json)
        .bind(attempt.error.as_deref())
        .bind(attempt.agent_session_id.as_deref())
        .bind(attempt.child_frame_id.as_deref())
        .bind(attempt.input_tokens)
        .bind(attempt.output_tokens)
        .bind(attempt.tool_calls)
        .bind(attempt.cost_microunits)
        .bind(attempt.cancel_requested as i64)
        .bind(attempt.started_at)
        .bind(attempt.finished_at)
        .bind(now)
        .bind(&attempt.id)
        .bind(expected_status.as_str())
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn request_agent_workflow_attempt_cancel(&self, id: &str) -> Result<bool> {
        let updated = sqlx::query(
            "UPDATE agent_workflow_attempts SET cancel_requested=1,updated_at=? WHERE id=? AND status IN ('queued','running')",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn request_agent_workflow_cancel(&self, workflow_id: &str) -> Result<u64> {
        let updated = sqlx::query(
            "UPDATE agent_workflow_attempts SET cancel_requested=1,updated_at=? WHERE workflow_id=? AND status IN ('queued','running')",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(workflow_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected())
    }

    pub async fn agent_workflow_cancel_requested(&self, workflow_id: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_workflow_attempts WHERE workflow_id=? AND cancel_requested=1",
        )
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn set_running_agent_workflow_attempt_provenance(
        &self,
        request_id: &str,
        agent_session_id: Option<&str>,
        child_frame_id: &str,
    ) -> Result<bool> {
        let updated = sqlx::query(
            "UPDATE agent_workflow_attempts SET agent_session_id=?,child_frame_id=?,updated_at=? WHERE request_id=? AND status='running'",
        )
        .bind(agent_session_id)
        .bind(child_frame_id)
        .bind(chrono::Utc::now().timestamp())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn fail_agent_workflow_execution(
        &self,
        workflow_id: &str,
        error: &str,
    ) -> Result<(u64, bool)> {
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        let attempts = sqlx::query(
            "UPDATE agent_workflow_attempts SET status='failed',error=COALESCE(error,?),finished_at=COALESCE(finished_at,?),updated_at=? WHERE workflow_id=? AND status IN ('queued','running')",
        )
        .bind(error)
        .bind(now)
        .bind(now)
        .bind(workflow_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let workflow = sqlx::query(
            "UPDATE agent_workflows SET status='failed',version=version+1,updated_at=? WHERE id=? AND status='running'",
        )
        .bind(now)
        .bind(workflow_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        tx.commit().await?;
        Ok((attempts, workflow))
    }

    pub async fn recover_interrupted_agent_workflows(&self) -> Result<(u64, u64)> {
        let now = chrono::Utc::now().timestamp();
        let reason =
            "The application stopped before this Agent execution reached a terminal state.";
        let mut tx = self.pool.begin().await?;
        let attempts = sqlx::query(
            "UPDATE agent_workflow_attempts SET status='failed',error=COALESCE(error,?),finished_at=COALESCE(finished_at,?),updated_at=? WHERE status IN ('queued','running') AND workflow_id IN (SELECT id FROM agent_workflows WHERE status='running')",
        )
        .bind(reason)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let workflows = sqlx::query(
            "UPDATE agent_workflows SET status='failed',version=version+1,updated_at=? WHERE status='running'",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok((attempts, workflows))
    }
}

fn validate_transition(
    from: AgentWorkflowAttemptStatus,
    to: AgentWorkflowAttemptStatus,
) -> Result<()> {
    let allowed = matches!(
        (from, to),
        (
            AgentWorkflowAttemptStatus::Queued,
            AgentWorkflowAttemptStatus::Running
                | AgentWorkflowAttemptStatus::Cancelled
                | AgentWorkflowAttemptStatus::Blocked
        ) | (
            AgentWorkflowAttemptStatus::Running,
            AgentWorkflowAttemptStatus::Succeeded
                | AgentWorkflowAttemptStatus::Failed
                | AgentWorkflowAttemptStatus::Cancelled
        )
    );
    if !allowed {
        anyhow::bail!("invalid agent workflow attempt transition: {from:?} -> {to:?}");
    }
    Ok(())
}
