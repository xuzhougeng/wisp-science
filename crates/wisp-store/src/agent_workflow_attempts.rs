use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, Transaction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentWorkflowAttemptStart {
    Started(AgentWorkflowAttempt),
    Busy,
    Stopped(String),
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct BudgetReservation {
    max_tokens: Option<u64>,
    max_tool_calls: Option<u64>,
    max_cost_microunits: Option<u64>,
}

impl BudgetReservation {
    fn from_json(raw: &str) -> Result<Self> {
        Ok(serde_json::from_str(raw)?)
    }

    fn add(self, other: Self) -> Self {
        Self {
            max_tokens: Some(
                self.max_tokens
                    .unwrap_or_default()
                    .saturating_add(other.max_tokens.unwrap_or_default()),
            ),
            max_tool_calls: Some(
                self.max_tool_calls
                    .unwrap_or_default()
                    .saturating_add(other.max_tool_calls.unwrap_or_default()),
            ),
            max_cost_microunits: Some(
                self.max_cost_microunits
                    .unwrap_or_default()
                    .saturating_add(other.max_cost_microunits.unwrap_or_default()),
            ),
        }
    }

    fn exceeds(self, limits: &super::AgentDelegationRootLimits) -> bool {
        self.max_tokens.unwrap_or_default() > limits.max_tokens
            || self.max_tool_calls.unwrap_or_default() > limits.max_tool_calls
            || self.max_cost_microunits.unwrap_or_default() > limits.max_cost_microunits
    }
}

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
    #[serde(default)]
    pub root_workflow_id: String,
    #[serde(default)]
    pub parent_attempt_id: Option<String>,
    #[serde(default = "default_attempt_depth")]
    pub depth: i64,
    #[serde(default)]
    pub allow_delegation: bool,
    #[serde(default)]
    pub delegation_slot_yielded: bool,
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

fn default_attempt_depth() -> i64 {
    1
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
        let workflow_id = workflow_id.into();
        let value = Self {
            id: id.into(),
            root_workflow_id: workflow_id.clone(),
            workflow_id,
            step_id: step_id.into(),
            parent_attempt_id: None,
            depth: 1,
            allow_delegation: false,
            delegation_slot_yielded: false,
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
        if self.root_workflow_id.trim().is_empty() {
            anyhow::bail!("agent workflow attempt root_workflow_id is required");
        }
        if !(1..=super::MAX_ROOT_AGENT_DEPTH.into()).contains(&self.depth) {
            anyhow::bail!("agent workflow attempt depth is outside the supported range");
        }
        if self.depth == 1 && self.parent_attempt_id.is_some() {
            anyhow::bail!("root Agent attempts cannot have a parent attempt");
        }
        if self.depth > 1 && self.parent_attempt_id.as_deref().is_none_or(str::is_empty) {
            anyhow::bail!("nested Agent attempts require a parent attempt");
        }
        if self.delegation_slot_yielded
            && (self.status != AgentWorkflowAttemptStatus::Running || !self.allow_delegation)
        {
            anyhow::bail!("only a running delegation-capable attempt may yield its slot");
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
        root_workflow_id: row.try_get("root_workflow_id")?,
        parent_attempt_id: row.try_get("parent_attempt_id")?,
        depth: row.try_get("depth")?,
        allow_delegation: row.try_get::<i64, _>("allow_delegation")? != 0,
        delegation_slot_yielded: row.try_get::<i64, _>("delegation_slot_yielded")? != 0,
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

const SELECT_ATTEMPT: &str = "SELECT id,workflow_id,step_id,root_workflow_id,parent_attempt_id,depth,allow_delegation,delegation_slot_yielded,attempt,request_id,backend,status,request_json,response_json,output_json,artifact_ids_json,evidence_json,error,agent_session_id,child_frame_id,input_tokens,output_tokens,tool_calls,cost_microunits,cancel_requested,started_at,finished_at,created_at,updated_at FROM agent_workflow_attempts";

async fn lock_root_workflow(
    tx: &mut Transaction<'_, Sqlite>,
    root_workflow_id: &str,
) -> Result<()> {
    let updated = sqlx::query("UPDATE agent_workflows SET updated_at=updated_at WHERE id=?")
        .bind(root_workflow_id)
        .execute(&mut **tx)
        .await?;
    if updated.rows_affected() != 1 {
        anyhow::bail!("root Agent workflow does not exist");
    }
    Ok(())
}

async fn active_budget_reservations(
    tx: &mut Transaction<'_, Sqlite>,
    root_workflow_id: &str,
) -> Result<BudgetReservation> {
    let rows = sqlx::query(
        "SELECT s.budget_json FROM agent_workflow_attempts a \
         JOIN agent_workflow_steps s ON s.id=a.step_id \
         WHERE a.root_workflow_id=? AND a.status IN ('queued','running')",
    )
    .bind(root_workflow_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.iter()
        .try_fold(BudgetReservation::default(), |sum, row| {
            let raw: String = row.try_get("budget_json")?;
            Ok(sum.add(BudgetReservation::from_json(&raw)?))
        })
}

async fn registered_budget(
    tx: &mut Transaction<'_, Sqlite>,
    root_workflow_id: &str,
) -> Result<BudgetReservation> {
    let rows = sqlx::query(
        "SELECT s.budget_json FROM agent_workflow_steps s JOIN agent_workflows w \
         ON w.id=s.workflow_id WHERE w.root_workflow_id=?",
    )
    .bind(root_workflow_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.iter()
        .try_fold(BudgetReservation::default(), |sum, row| {
            let raw: String = row.try_get("budget_json")?;
            Ok(sum.add(BudgetReservation::from_json(&raw)?))
        })
}

async fn terminal_usage(
    tx: &mut Transaction<'_, Sqlite>,
    root_workflow_id: &str,
) -> Result<BudgetReservation> {
    let row = sqlx::query(
        "SELECT COALESCE(SUM(input_tokens+output_tokens),0) AS tokens,\
         COALESCE(SUM(tool_calls),0) AS tools,COALESCE(SUM(cost_microunits),0) AS cost \
         FROM agent_workflow_attempts WHERE root_workflow_id=? \
         AND status IN ('succeeded','failed','cancelled','blocked')",
    )
    .bind(root_workflow_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(BudgetReservation {
        max_tokens: Some(u64::try_from(row.try_get::<i64, _>("tokens")?).unwrap_or(u64::MAX)),
        max_tool_calls: Some(u64::try_from(row.try_get::<i64, _>("tools")?).unwrap_or(u64::MAX)),
        max_cost_microunits: Some(
            u64::try_from(row.try_get::<i64, _>("cost")?).unwrap_or(u64::MAX),
        ),
    })
}

async fn step_budget(
    tx: &mut Transaction<'_, Sqlite>,
    workflow_id: &str,
    step_id: &str,
) -> Result<Option<BudgetReservation>> {
    let raw = sqlx::query_scalar::<_, String>(
        "SELECT budget_json FROM agent_workflow_steps WHERE id=? AND workflow_id=?",
    )
    .bind(step_id)
    .bind(workflow_id)
    .fetch_optional(&mut **tx)
    .await?;
    raw.as_deref().map(BudgetReservation::from_json).transpose()
}

fn root_deadline_exceeded(created_at: i64, wall_time_secs: u64, now: i64) -> bool {
    now >= created_at.saturating_add(i64::try_from(wall_time_secs).unwrap_or(i64::MAX))
}

impl Store {
    pub async fn create_agent_workflow_attempt(
        &self,
        attempt: &AgentWorkflowAttempt,
    ) -> Result<()> {
        attempt.validate()?;
        if attempt.status != AgentWorkflowAttemptStatus::Queued {
            anyhow::bail!("new agent workflow attempts must start queued");
        }
        let mut tx = self.begin_write().await?;
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
            "INSERT INTO agent_workflow_attempts(id,workflow_id,step_id,root_workflow_id,parent_attempt_id,depth,allow_delegation,delegation_slot_yielded,attempt,request_id,backend,status,request_json,response_json,output_json,artifact_ids_json,evidence_json,error,agent_session_id,child_frame_id,input_tokens,output_tokens,tool_calls,cost_microunits,cancel_requested,started_at,finished_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&attempt.id)
        .bind(&attempt.workflow_id)
        .bind(&attempt.step_id)
        .bind(&attempt.root_workflow_id)
        .bind(attempt.parent_attempt_id.as_deref())
        .bind(attempt.depth)
        .bind(attempt.allow_delegation as i64)
        .bind(attempt.delegation_slot_yielded as i64)
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

    /// Atomically reserve one root-wide concurrency slot and persist a running
    /// attempt. `Busy` is transient; `Stopped` means no backend child may be
    /// created for this request.
    pub async fn try_create_started_agent_workflow_attempt(
        &self,
        mut attempt: AgentWorkflowAttempt,
    ) -> Result<AgentWorkflowAttemptStart> {
        attempt.validate()?;
        if attempt.status != AgentWorkflowAttemptStatus::Queued {
            anyhow::bail!("new agent workflow attempts must start queued");
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        lock_root_workflow(&mut tx, &attempt.root_workflow_id).await?;
        let workflow = sqlx::query(
            "SELECT w.root_workflow_id,w.parent_attempt_id,w.depth,w.status,\
             r.status AS root_status,r.created_at AS root_created_at,r.root_limits_json,\
             s.spec_json FROM agent_workflows w \
             JOIN agent_workflows r ON r.id=w.root_workflow_id \
             JOIN agent_workflow_steps s ON s.workflow_id=w.id AND s.id=? WHERE w.id=?",
        )
        .bind(&attempt.step_id)
        .bind(&attempt.workflow_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(workflow) = workflow else {
            tx.rollback().await?;
            return Ok(AgentWorkflowAttemptStart::Stopped(
                "Agent workflow step is missing or mismatched".into(),
            ));
        };
        let workflow_root: String = workflow.try_get("root_workflow_id")?;
        let workflow_parent: Option<String> = workflow.try_get("parent_attempt_id")?;
        let workflow_depth: i64 = workflow.try_get("depth")?;
        let workflow_status: String = workflow.try_get("status")?;
        let root_status: String = workflow.try_get("root_status")?;
        let root_created_at: i64 = workflow.try_get("root_created_at")?;
        let root_limits: super::AgentDelegationRootLimits =
            serde_json::from_str(&workflow.try_get::<String, _>("root_limits_json")?)?;
        let stored_spec: serde_json::Value =
            serde_json::from_str(&workflow.try_get::<String, _>("spec_json")?)?;
        let stored_allow_delegation = stored_spec
            .get("allow_delegation")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let expected_depth = workflow_depth.saturating_add(1);
        if workflow_root != attempt.root_workflow_id
            || workflow_parent != attempt.parent_attempt_id
            || expected_depth != attempt.depth
            || stored_allow_delegation != attempt.allow_delegation
        {
            tx.rollback().await?;
            return Ok(AgentWorkflowAttemptStart::Stopped(
                "Agent attempt lineage or delegation authority does not match its approved step"
                    .into(),
            ));
        }
        if workflow_status != "running" || root_status != "running" {
            tx.rollback().await?;
            return Ok(AgentWorkflowAttemptStart::Stopped(
                "Root Agent workflow is no longer running".into(),
            ));
        }
        if attempt.allow_delegation && attempt.depth >= i64::from(root_limits.max_depth) {
            tx.rollback().await?;
            return Ok(AgentWorkflowAttemptStart::Stopped(
                "Delegation authority cannot cross the root depth limit".into(),
            ));
        }
        if let Some(parent_attempt_id) = attempt.parent_attempt_id.as_deref() {
            let parent_running: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_workflow_attempts WHERE id=? \
                 AND root_workflow_id=? AND status='running' AND cancel_requested=0",
            )
            .bind(parent_attempt_id)
            .bind(&attempt.root_workflow_id)
            .fetch_one(&mut *tx)
            .await?;
            if parent_running != 1 {
                tx.rollback().await?;
                return Ok(AgentWorkflowAttemptStart::Stopped(
                    "Parent Agent attempt is no longer running".into(),
                ));
            }
        }
        let cancel_requested: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_workflow_attempts WHERE root_workflow_id=? \
             AND cancel_requested=1",
        )
        .bind(&attempt.root_workflow_id)
        .fetch_one(&mut *tx)
        .await?;
        if cancel_requested > 0
            || root_deadline_exceeded(root_created_at, root_limits.wall_time_secs, now)
        {
            sqlx::query(
                "UPDATE agent_workflow_attempts SET cancel_requested=1,updated_at=? \
                 WHERE root_workflow_id=? AND status IN ('queued','running')",
            )
            .bind(now)
            .bind(&attempt.root_workflow_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(AgentWorkflowAttemptStart::Stopped(
                "Root Agent workflow was cancelled or exceeded its wall-clock limit".into(),
            ));
        }
        let active_slots: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_workflow_attempts WHERE root_workflow_id=? \
             AND status IN ('queued','running') AND delegation_slot_yielded=0",
        )
        .bind(&attempt.root_workflow_id)
        .fetch_one(&mut *tx)
        .await?;
        if active_slots >= i64::from(root_limits.max_parallel) {
            tx.rollback().await?;
            return Ok(AgentWorkflowAttemptStart::Busy);
        }
        let Some(proposed) = step_budget(&mut tx, &attempt.workflow_id, &attempt.step_id).await?
        else {
            tx.rollback().await?;
            return Ok(AgentWorkflowAttemptStart::Stopped(
                "Agent workflow step budget is missing".into(),
            ));
        };
        let reserved = terminal_usage(&mut tx, &attempt.root_workflow_id)
            .await?
            .add(active_budget_reservations(&mut tx, &attempt.root_workflow_id).await?)
            .add(proposed);
        if reserved.exceeds(&root_limits) {
            sqlx::query(
                "UPDATE agent_workflow_attempts SET cancel_requested=1,updated_at=? \
                 WHERE root_workflow_id=? AND status IN ('queued','running')",
            )
            .bind(now)
            .bind(&attempt.root_workflow_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(AgentWorkflowAttemptStart::Stopped(
                "Root Agent workflow budget is exhausted".into(),
            ));
        }

        attempt.status = AgentWorkflowAttemptStatus::Running;
        attempt.started_at = Some(now);
        attempt.updated_at = now;
        attempt.validate()?;
        sqlx::query(
            "INSERT INTO agent_workflow_attempts(id,workflow_id,step_id,root_workflow_id,parent_attempt_id,depth,allow_delegation,delegation_slot_yielded,attempt,request_id,backend,status,request_json,response_json,output_json,artifact_ids_json,evidence_json,error,agent_session_id,child_frame_id,input_tokens,output_tokens,tool_calls,cost_microunits,cancel_requested,started_at,finished_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&attempt.id)
        .bind(&attempt.workflow_id)
        .bind(&attempt.step_id)
        .bind(&attempt.root_workflow_id)
        .bind(attempt.parent_attempt_id.as_deref())
        .bind(attempt.depth)
        .bind(attempt.allow_delegation as i64)
        .bind(attempt.delegation_slot_yielded as i64)
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
        Ok(AgentWorkflowAttemptStart::Started(attempt))
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

    pub async fn get_agent_workflow_attempt_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<AgentWorkflowAttempt>> {
        sqlx::query(&format!("{SELECT_ATTEMPT} WHERE request_id=?"))
            .bind(request_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }

    pub async fn running_agent_workflow_attempt_for_child_frame(
        &self,
        child_frame_id: &str,
    ) -> Result<Option<AgentWorkflowAttempt>> {
        sqlx::query(&format!(
            "{SELECT_ATTEMPT} WHERE child_frame_id=? AND status='running' \
             ORDER BY created_at DESC LIMIT 1"
        ))
        .bind(child_frame_id)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(from_row)
        .transpose()
    }

    pub async fn list_child_agent_workflow_ids(
        &self,
        parent_attempt_id: &str,
    ) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM agent_workflows WHERE parent_attempt_id=? ORDER BY created_at,id",
        )
        .bind(parent_attempt_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn agent_workflow_attempt_has_delegation_capacity(
        &self,
        attempt_id: &str,
    ) -> Result<bool> {
        let mut tx = self.begin_write().await?;
        let row = sqlx::query(
            "SELECT a.root_workflow_id,a.depth,a.status,a.allow_delegation,a.cancel_requested,\
             r.status AS root_status,r.created_at,r.root_limits_json FROM agent_workflow_attempts a \
             JOIN agent_workflows r ON r.id=a.root_workflow_id WHERE a.id=?",
        )
        .bind(attempt_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let root_workflow_id: String = row.try_get("root_workflow_id")?;
        let limits: super::AgentDelegationRootLimits =
            serde_json::from_str(&row.try_get::<String, _>("root_limits_json")?)?;
        let depth: i64 = row.try_get("depth")?;
        if row.try_get::<String, _>("status")? != "running"
            || row.try_get::<i64, _>("allow_delegation")? == 0
            || row.try_get::<i64, _>("cancel_requested")? != 0
            || row.try_get::<String, _>("root_status")? != "running"
            || depth >= i64::from(limits.max_depth)
            || root_deadline_exceeded(
                row.try_get("created_at")?,
                limits.wall_time_secs,
                chrono::Utc::now().timestamp(),
            )
        {
            return Ok(false);
        }
        let cancelled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_workflow_attempts WHERE root_workflow_id=? \
             AND cancel_requested=1",
        )
        .bind(&root_workflow_id)
        .fetch_one(&mut *tx)
        .await?;
        let tasks: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_workflow_steps s JOIN agent_workflows w \
             ON w.id=s.workflow_id WHERE w.root_workflow_id=?",
        )
        .bind(&root_workflow_id)
        .fetch_one(&mut *tx)
        .await?;
        let other_slots: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_workflow_attempts WHERE root_workflow_id=? AND id<>? \
             AND status IN ('queued','running') AND delegation_slot_yielded=0",
        )
        .bind(&root_workflow_id)
        .bind(attempt_id)
        .fetch_one(&mut *tx)
        .await?;
        let budget = registered_budget(&mut tx, &root_workflow_id).await?;
        Ok(cancelled == 0
            && tasks < i64::from(limits.max_tasks)
            && other_slots < i64::from(limits.max_parallel)
            && !budget.exceeds(&limits)
            && budget.max_tokens.unwrap_or_default() < limits.max_tokens
            && budget.max_tool_calls.unwrap_or_default() < limits.max_tool_calls
            && budget.max_cost_microunits.unwrap_or_default() < limits.max_cost_microunits)
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
            "UPDATE agent_workflow_attempts SET status=?,response_json=?,output_json=?,artifact_ids_json=?,evidence_json=?,error=?,agent_session_id=?,child_frame_id=?,input_tokens=?,output_tokens=?,tool_calls=?,cost_microunits=?,cancel_requested=?,delegation_slot_yielded=?,started_at=?,finished_at=?,updated_at=? WHERE id=? AND status=?",
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
        .bind(attempt.delegation_slot_yielded as i64)
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
        let root_workflow_id = sqlx::query_scalar::<_, String>(
            "SELECT root_workflow_id FROM agent_workflow_attempts WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        match root_workflow_id {
            Some(root_workflow_id) => Ok(self
                .request_agent_workflow_cancel(&root_workflow_id)
                .await?
                > 0),
            None => Ok(false),
        }
    }

    pub async fn request_agent_workflow_cancel(&self, workflow_id: &str) -> Result<u64> {
        let root_workflow_id = sqlx::query_scalar::<_, String>(
            "SELECT root_workflow_id FROM agent_workflows WHERE id=?",
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or_else(|| workflow_id.to_string());
        let updated = sqlx::query(
            "UPDATE agent_workflow_attempts SET cancel_requested=1,updated_at=? \
             WHERE root_workflow_id=? AND status IN ('queued','running')",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(root_workflow_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected())
    }

    pub async fn agent_workflow_cancel_requested(&self, workflow_id: &str) -> Result<bool> {
        let root = sqlx::query(
            "SELECT root_workflow_id,created_at,root_limits_json,status FROM agent_workflows \
             WHERE id=?",
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(root) = root else {
            return Ok(false);
        };
        let root_workflow_id: String = root.try_get("root_workflow_id")?;
        let root = sqlx::query(
            "SELECT created_at,root_limits_json,status FROM agent_workflows WHERE id=?",
        )
        .bind(&root_workflow_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(root) = root else {
            return Ok(true);
        };
        let limits: super::AgentDelegationRootLimits =
            serde_json::from_str(&root.try_get::<String, _>("root_limits_json")?)?;
        let now = chrono::Utc::now().timestamp();
        let expired =
            root_deadline_exceeded(root.try_get("created_at")?, limits.wall_time_secs, now);
        if expired {
            let _ = self
                .request_agent_workflow_cancel(&root_workflow_id)
                .await?;
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_workflow_attempts WHERE root_workflow_id=? \
             AND cancel_requested=1",
        )
        .bind(&root_workflow_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(expired || count > 0)
    }

    /// Yielding lets a synchronously waiting parent stop consuming one global
    /// concurrency slot. Reacquisition returns `false` while another descendant
    /// still owns the slot.
    pub async fn set_agent_workflow_attempt_delegation_slot_yielded(
        &self,
        attempt_id: &str,
        yielded: bool,
    ) -> Result<bool> {
        let mut tx = self.begin_write().await?;
        let row = sqlx::query(
            "SELECT a.root_workflow_id,a.status,a.allow_delegation,a.cancel_requested,\
             r.root_limits_json,r.status AS root_status FROM agent_workflow_attempts a \
             JOIN agent_workflows r ON r.id=a.root_workflow_id WHERE a.id=?",
        )
        .bind(attempt_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(false);
        };
        let root_workflow_id: String = row.try_get("root_workflow_id")?;
        lock_root_workflow(&mut tx, &root_workflow_id).await?;
        let running = row.try_get::<String, _>("status")? == "running";
        let allowed = row.try_get::<i64, _>("allow_delegation")? != 0;
        let cancelled = row.try_get::<i64, _>("cancel_requested")? != 0
            || row.try_get::<String, _>("root_status")? != "running";
        if !running || !allowed {
            tx.rollback().await?;
            return Ok(false);
        }
        if !yielded && !cancelled {
            let limits: super::AgentDelegationRootLimits =
                serde_json::from_str(&row.try_get::<String, _>("root_limits_json")?)?;
            let occupied: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_workflow_attempts WHERE root_workflow_id=? \
                 AND id<>? AND status IN ('queued','running') AND delegation_slot_yielded=0",
            )
            .bind(&root_workflow_id)
            .bind(attempt_id)
            .fetch_one(&mut *tx)
            .await?;
            if occupied >= i64::from(limits.max_parallel) {
                tx.rollback().await?;
                return Ok(false);
            }
        }
        let updated = sqlx::query(
            "UPDATE agent_workflow_attempts SET delegation_slot_yielded=?,updated_at=? \
             WHERE id=? AND status='running' AND allow_delegation=1",
        )
        .bind(yielded as i64)
        .bind(chrono::Utc::now().timestamp())
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(updated.rows_affected() == 1)
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
        let mut tx = self.begin_write().await?;
        let attempts = sqlx::query(
            "UPDATE agent_workflow_attempts SET status='failed',error=COALESCE(error,?),\
             delegation_slot_yielded=0,finished_at=COALESCE(finished_at,?),updated_at=? \
             WHERE workflow_id=? AND status IN ('queued','running') \
             AND workflow_id IN (SELECT id FROM agent_workflows WHERE status='running')",
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
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "UPDATE agent_workflow_deliveries SET resume_status='interrupted',\
             resume_error=COALESCE(resume_error,?),updated_at=? WHERE resume_status='running'",
        )
        .bind("The application stopped while the parent conversation was auto-resuming.")
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let attempts = sqlx::query(
            "UPDATE agent_workflow_attempts SET status='failed',error=COALESCE(error,?),\
             delegation_slot_yielded=0,finished_at=COALESCE(finished_at,?),updated_at=? \
             WHERE status IN ('queued','running') \
             AND workflow_id IN (SELECT id FROM agent_workflows WHERE status='running')",
        )
        .bind(reason)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let mut workflows = sqlx::query(
            "UPDATE agent_workflows SET status='failed',version=version+1,updated_at=? WHERE status='running'",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // A background generation is reserved before its task is spawned. If
        // shutdown lands in that small window, fail it explicitly on restart;
        // never silently launch an approved external process later.
        workflows += sqlx::query(
            "UPDATE agent_workflows SET status='failed',version=version+1,updated_at=? \
             WHERE status='approved' AND EXISTS (\
               SELECT 1 FROM agent_workflow_deliveries d \
               WHERE d.workflow_id=agent_workflows.id AND d.result_json IS NULL)",
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
