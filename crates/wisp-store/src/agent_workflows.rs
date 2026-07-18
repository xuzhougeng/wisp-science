use anyhow::Result;
use sqlx::{Row, Sqlite, Transaction};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkflowStatus {
    Draft,
    Approved,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl Default for AgentWorkflowStatus {
    fn default() -> Self {
        Self::Draft
    }
}

fn assisted_mode() -> String {
    "assisted".into()
}

fn default_max_parallel() -> i64 {
    2
}

fn default_true() -> bool {
    true
}

impl AgentWorkflowStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Approved => "approved",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "draft" => Ok(Self::Draft),
            "approved" => Ok(Self::Approved),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => anyhow::bail!("unknown agent workflow status: {value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentWorkflow {
    pub id: String,
    pub project_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub frame_id: Option<String>,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default = "assisted_mode")]
    pub mode: String,
    #[serde(default)]
    pub status: AgentWorkflowStatus,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: i64,
    #[serde(default = "default_true")]
    pub requires_confirmation: bool,
    #[serde(default = "empty_json_object")]
    pub plan_json: String,
    pub version: i64,
    pub enabled: bool,
    #[serde(default)]
    pub approved_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl AgentWorkflow {
    pub fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self> {
        let now = chrono::Utc::now().timestamp();
        let name = name.into();
        let workflow = Self {
            id: id.into(),
            project_id: project_id.into(),
            workspace_id: workspace_id.into(),
            frame_id: None,
            goal: name.clone(),
            name,
            description: String::new(),
            mode: "assisted".into(),
            status: AgentWorkflowStatus::Draft,
            max_parallel: 2,
            requires_confirmation: true,
            plan_json: "{}".into(),
            version: 1,
            enabled: true,
            approved_at: None,
            created_at: now,
            updated_at: now,
        };
        workflow.validate()?;
        if workflow.status != AgentWorkflowStatus::Draft {
            anyhow::bail!("new agent workflow plans must start as draft");
        }
        Ok(workflow)
    }

    fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("id", self.id.as_str()),
            ("project_id", self.project_id.as_str()),
            ("workspace_id", self.workspace_id.as_str()),
            ("name", self.name.as_str()),
            ("goal", self.goal.as_str()),
        ] {
            if value.trim().is_empty() {
                anyhow::bail!("workflow {field} is required");
            }
        }
        if self.version <= 0 {
            anyhow::bail!("workflow version must be positive");
        }
        if !matches!(self.mode.as_str(), "manual" | "assisted" | "automatic") {
            anyhow::bail!("workflow mode must be manual, assisted, or automatic");
        }
        if !(1..=2).contains(&self.max_parallel) {
            anyhow::bail!("workflow max_parallel must be between 1 and 2");
        }
        if !serde_json::from_str::<serde_json::Value>(&self.plan_json)
            .map(|value| value.is_object())
            .unwrap_or(false)
        {
            anyhow::bail!("workflow plan_json must be a JSON object");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentWorkflowStep {
    pub id: String,
    pub workflow_id: String,
    pub position: i64,
    pub agent_id: String,
    #[serde(default)]
    pub template_id: String,
    pub role: String,
    pub backend: String,
    pub model: Option<String>,
    pub prompt_template: String,
    pub input_schema_json: String,
    pub output_schema_json: String,
    #[serde(default = "empty_json_object")]
    pub input_contract_json: String,
    #[serde(default = "empty_json_object")]
    pub output_contract_json: String,
    pub permissions_json: String,
    pub context_policy_json: String,
    #[serde(default = "empty_json_object")]
    pub budget_json: String,
    #[serde(default = "empty_json_object")]
    pub spec_json: String,
    pub timeout_secs: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn empty_json_object() -> String {
    "{}".into()
}

impl AgentWorkflowStep {
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        position: i64,
        agent_id: impl Into<String>,
        role: impl Into<String>,
        backend: impl Into<String>,
        prompt_template: impl Into<String>,
    ) -> Result<Self> {
        let now = chrono::Utc::now().timestamp();
        let agent_id = agent_id.into();
        let step = Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            position,
            template_id: agent_id.clone(),
            agent_id,
            role: role.into(),
            backend: backend.into(),
            model: None,
            prompt_template: prompt_template.into(),
            input_schema_json: "{}".into(),
            output_schema_json: "{}".into(),
            input_contract_json: "{}".into(),
            output_contract_json: "{}".into(),
            permissions_json: "{}".into(),
            context_policy_json: "{}".into(),
            budget_json: "{}".into(),
            spec_json: "{}".into(),
            timeout_secs: None,
            created_at: now,
            updated_at: now,
        };
        step.validate()?;
        Ok(step)
    }

    fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("id", self.id.as_str()),
            ("workflow_id", self.workflow_id.as_str()),
            ("agent_id", self.agent_id.as_str()),
            ("template_id", self.template_id.as_str()),
            ("role", self.role.as_str()),
            ("backend", self.backend.as_str()),
            ("prompt_template", self.prompt_template.as_str()),
        ] {
            if value.trim().is_empty() {
                anyhow::bail!("workflow step {field} is required");
            }
        }
        if self.position < 0 {
            anyhow::bail!("workflow step position must be non-negative");
        }
        if self.timeout_secs == Some(0) || self.timeout_secs.is_some_and(|v| v < 0) {
            anyhow::bail!("workflow step timeout_secs must be positive");
        }
        for (field, value) in [
            ("input_schema_json", self.input_schema_json.as_str()),
            ("output_schema_json", self.output_schema_json.as_str()),
            ("input_contract_json", self.input_contract_json.as_str()),
            ("output_contract_json", self.output_contract_json.as_str()),
            ("permissions_json", self.permissions_json.as_str()),
            ("context_policy_json", self.context_policy_json.as_str()),
            ("budget_json", self.budget_json.as_str()),
            ("spec_json", self.spec_json.as_str()),
        ] {
            if !serde_json::from_str::<serde_json::Value>(value)
                .map(|value| value.is_object())
                .unwrap_or(false)
            {
                anyhow::bail!("workflow step {field} must be a JSON object");
            }
        }
        Ok(())
    }
}

fn workflow_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AgentWorkflow> {
    let status: String = row.try_get("status")?;
    Ok(AgentWorkflow {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        workspace_id: row.try_get("workspace_id")?,
        frame_id: row.try_get("frame_id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        goal: row.try_get("goal")?,
        mode: row.try_get("mode")?,
        status: AgentWorkflowStatus::from_storage(&status)?,
        max_parallel: row.try_get("max_parallel")?,
        requires_confirmation: row.try_get::<i64, _>("requires_confirmation")? != 0,
        plan_json: row.try_get("plan_json")?,
        version: row.try_get("version")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        approved_at: row.try_get("approved_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn step_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AgentWorkflowStep> {
    Ok(AgentWorkflowStep {
        id: row.try_get("id")?,
        workflow_id: row.try_get("workflow_id")?,
        position: row.try_get("position")?,
        agent_id: row.try_get("agent_id")?,
        template_id: row.try_get("template_id")?,
        role: row.try_get("role")?,
        backend: row.try_get("backend")?,
        model: row.try_get("model")?,
        prompt_template: row.try_get("prompt_template")?,
        input_schema_json: row.try_get("input_schema_json")?,
        output_schema_json: row.try_get("output_schema_json")?,
        input_contract_json: row.try_get("input_contract_json")?,
        output_contract_json: row.try_get("output_contract_json")?,
        permissions_json: row.try_get("permissions_json")?,
        context_policy_json: row.try_get("context_policy_json")?,
        budget_json: row.try_get("budget_json")?,
        spec_json: row.try_get("spec_json")?,
        timeout_secs: row.try_get("timeout_secs")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn validate_plan_steps(workflow_id: &str, steps: &[AgentWorkflowStep]) -> Result<()> {
    let mut positions = HashSet::new();
    let mut ids = HashSet::new();
    for step in steps {
        step.validate()?;
        if step.workflow_id != workflow_id {
            anyhow::bail!("workflow step belongs to a different workflow");
        }
        if !positions.insert(step.position) || !ids.insert(step.id.as_str()) {
            anyhow::bail!("workflow step ids and positions must be unique");
        }
    }
    Ok(())
}

async fn insert_step(tx: &mut Transaction<'_, Sqlite>, step: &AgentWorkflowStep) -> Result<()> {
    sqlx::query(
        "INSERT INTO agent_workflow_steps(id,workflow_id,position,agent_id,template_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,input_contract_json,output_contract_json,permissions_json,context_policy_json,budget_json,spec_json,timeout_secs,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&step.id)
    .bind(&step.workflow_id)
    .bind(step.position)
    .bind(&step.agent_id)
    .bind(&step.template_id)
    .bind(&step.role)
    .bind(&step.backend)
    .bind(step.model.as_deref())
    .bind(&step.prompt_template)
    .bind(&step.input_schema_json)
    .bind(&step.output_schema_json)
    .bind(&step.input_contract_json)
    .bind(&step.output_contract_json)
    .bind(&step.permissions_json)
    .bind(&step.context_policy_json)
    .bind(&step.budget_json)
    .bind(&step.spec_json)
    .bind(step.timeout_secs)
    .bind(step.created_at)
    .bind(step.updated_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn bump_draft_workflow_version(
    tx: &mut Transaction<'_, Sqlite>,
    workflow_id: &str,
) -> Result<()> {
    let updated = sqlx::query(
        "UPDATE agent_workflows SET version=version+1,updated_at=? WHERE id=? AND status='draft'",
    )
    .bind(chrono::Utc::now().timestamp())
    .bind(workflow_id)
    .execute(&mut **tx)
    .await?;
    if updated.rows_affected() != 1 {
        anyhow::bail!("agent workflow plan is missing or immutable");
    }
    Ok(())
}

impl super::Store {
    pub async fn create_agent_workflow_plan(
        &self,
        workflow: &AgentWorkflow,
        steps: &[AgentWorkflowStep],
    ) -> Result<()> {
        workflow.validate()?;
        if workflow.status != AgentWorkflowStatus::Draft {
            anyhow::bail!("new agent workflow plans must start as draft");
        }
        validate_plan_steps(&workflow.id, steps)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO agent_workflows(id,project_id,workspace_id,frame_id,name,description,goal,mode,status,max_parallel,requires_confirmation,plan_json,version,enabled,approved_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&workflow.id)
        .bind(&workflow.project_id)
        .bind(&workflow.workspace_id)
        .bind(workflow.frame_id.as_deref())
        .bind(&workflow.name)
        .bind(&workflow.description)
        .bind(&workflow.goal)
        .bind(&workflow.mode)
        .bind(workflow.status.as_str())
        .bind(workflow.max_parallel)
        .bind(workflow.requires_confirmation as i64)
        .bind(&workflow.plan_json)
        .bind(workflow.version)
        .bind(workflow.enabled as i64)
        .bind(workflow.approved_at)
        .bind(workflow.created_at)
        .bind(workflow.updated_at)
        .execute(&mut *tx)
        .await?;
        for step in steps {
            insert_step(&mut tx, step).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_agent_workflow_plan(
        &self,
        id: &str,
    ) -> Result<Option<(AgentWorkflow, Vec<AgentWorkflowStep>)>> {
        let workflow = match self.get_agent_workflow(id).await? {
            Some(workflow) => workflow,
            None => return Ok(None),
        };
        let steps = self.list_agent_workflow_steps(id).await?;
        Ok(Some((workflow, steps)))
    }

    pub async fn replace_agent_workflow_plan(
        &self,
        workflow: &AgentWorkflow,
        steps: &[AgentWorkflowStep],
        expected_version: i64,
    ) -> Result<bool> {
        workflow.validate()?;
        if workflow.status != AgentWorkflowStatus::Draft {
            anyhow::bail!("only draft agent workflow plans can be edited");
        }
        validate_plan_steps(&workflow.id, steps)?;
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE agent_workflows SET frame_id=?,name=?,description=?,goal=?,mode=?,max_parallel=?,requires_confirmation=?,plan_json=?,version=version+1,enabled=?,updated_at=? WHERE id=? AND version=? AND status='draft'",
        )
        .bind(workflow.frame_id.as_deref())
        .bind(&workflow.name)
        .bind(&workflow.description)
        .bind(&workflow.goal)
        .bind(&workflow.mode)
        .bind(workflow.max_parallel)
        .bind(workflow.requires_confirmation as i64)
        .bind(&workflow.plan_json)
        .bind(workflow.enabled as i64)
        .bind(now)
        .bind(&workflow.id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query("DELETE FROM agent_workflow_steps WHERE workflow_id=?")
            .bind(&workflow.id)
            .execute(&mut *tx)
            .await?;
        for step in steps {
            insert_step(&mut tx, step).await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn approve_agent_workflow_plan(
        &self,
        id: &str,
        expected_version: i64,
    ) -> Result<bool> {
        let now = chrono::Utc::now().timestamp();
        let updated = sqlx::query(
            "UPDATE agent_workflows SET status='approved',approved_at=?,version=version+1,updated_at=? WHERE id=? AND version=? AND status='draft'",
        )
        .bind(now)
        .bind(now)
        .bind(id)
        .bind(expected_version)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn transition_agent_workflow_status(
        &self,
        id: &str,
        from: AgentWorkflowStatus,
        to: AgentWorkflowStatus,
    ) -> Result<bool> {
        let allowed = matches!(
            (from, to),
            (AgentWorkflowStatus::Approved, AgentWorkflowStatus::Running)
                | (
                    AgentWorkflowStatus::Running,
                    AgentWorkflowStatus::Succeeded
                        | AgentWorkflowStatus::Failed
                        | AgentWorkflowStatus::Cancelled
                )
                | (
                    AgentWorkflowStatus::Failed | AgentWorkflowStatus::Cancelled,
                    AgentWorkflowStatus::Approved
                )
        );
        if !allowed {
            anyhow::bail!("invalid agent workflow transition: {from:?} -> {to:?}");
        }
        if to == AgentWorkflowStatus::Succeeded {
            let incomplete: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_workflow_steps s WHERE s.workflow_id=? AND NOT EXISTS (SELECT 1 FROM agent_workflow_attempts a WHERE a.step_id=s.id AND a.status='succeeded' AND a.attempt=(SELECT MAX(latest.attempt) FROM agent_workflow_attempts latest WHERE latest.step_id=s.id))",
            )
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
            if incomplete != 0 {
                anyhow::bail!("agent workflow cannot succeed before every step succeeds");
            }
        }
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE agent_workflows SET status=?,version=version+1,approved_at=CASE WHEN ?='approved' THEN ? ELSE approved_at END,updated_at=? WHERE id=? AND status=?",
        )
        .bind(to.as_str())
        .bind(to.as_str())
        .bind(now)
        .bind(now)
        .bind(id)
        .bind(from.as_str())
        .execute(&mut *tx)
        .await?;
        let changed = updated.rows_affected() == 1;
        if changed && to == AgentWorkflowStatus::Approved {
            sqlx::query(
                "UPDATE agent_workflow_attempts SET cancel_requested=0,updated_at=? WHERE workflow_id=? AND cancel_requested=1",
            )
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn create_agent_workflow(&self, workflow: &AgentWorkflow) -> Result<()> {
        workflow.validate()?;
        if workflow.status != AgentWorkflowStatus::Draft {
            anyhow::bail!("new agent workflows must start as draft");
        }
        sqlx::query(
            "INSERT INTO agent_workflows(id,project_id,workspace_id,frame_id,name,description,goal,mode,status,max_parallel,requires_confirmation,plan_json,version,enabled,approved_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&workflow.id)
        .bind(&workflow.project_id)
        .bind(&workflow.workspace_id)
        .bind(workflow.frame_id.as_deref())
        .bind(&workflow.name)
        .bind(&workflow.description)
        .bind(&workflow.goal)
        .bind(&workflow.mode)
        .bind(workflow.status.as_str())
        .bind(workflow.max_parallel)
        .bind(workflow.requires_confirmation as i64)
        .bind(&workflow.plan_json)
        .bind(workflow.version)
        .bind(workflow.enabled as i64)
        .bind(workflow.approved_at)
        .bind(workflow.created_at)
        .bind(workflow.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_agent_workflow(&self, id: &str) -> Result<Option<AgentWorkflow>> {
        sqlx::query(
            "SELECT id,project_id,workspace_id,frame_id,name,description,goal,mode,status,max_parallel,requires_confirmation,plan_json,version,enabled,approved_at,created_at,updated_at FROM agent_workflows WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(workflow_from_row)
        .transpose()
    }

    pub async fn list_agent_workflows(&self, project_id: &str) -> Result<Vec<AgentWorkflow>> {
        let rows = sqlx::query(
            "SELECT id,project_id,workspace_id,frame_id,name,description,goal,mode,status,max_parallel,requires_confirmation,plan_json,version,enabled,approved_at,created_at,updated_at FROM agent_workflows WHERE project_id=? ORDER BY name,id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(workflow_from_row).collect()
    }

    pub async fn update_agent_workflow(&self, workflow: &AgentWorkflow) -> Result<bool> {
        workflow.validate()?;
        let current = match self.get_agent_workflow(&workflow.id).await? {
            Some(current) => current,
            None => return Ok(false),
        };
        if current.status != AgentWorkflowStatus::Draft
            || workflow.status != AgentWorkflowStatus::Draft
        {
            anyhow::bail!("approved or running agent workflow plans are immutable");
        }
        if workflow.version < current.version {
            anyhow::bail!(
                "workflow version must not move backwards ({} < {})",
                workflow.version,
                current.version
            );
        }
        let version = workflow.version.max(current.version.saturating_add(1));
        let updated = sqlx::query(
            "UPDATE agent_workflows SET project_id=?,workspace_id=?,frame_id=?,name=?,description=?,goal=?,mode=?,status=?,max_parallel=?,requires_confirmation=?,plan_json=?,version=?,enabled=?,approved_at=?,updated_at=? WHERE id=? AND version=? AND status='draft'",
        )
        .bind(&workflow.project_id)
        .bind(&workflow.workspace_id)
        .bind(workflow.frame_id.as_deref())
        .bind(&workflow.name)
        .bind(&workflow.description)
        .bind(&workflow.goal)
        .bind(&workflow.mode)
        .bind(workflow.status.as_str())
        .bind(workflow.max_parallel)
        .bind(workflow.requires_confirmation as i64)
        .bind(&workflow.plan_json)
        .bind(version)
        .bind(workflow.enabled as i64)
        .bind(workflow.approved_at)
        .bind(chrono::Utc::now().timestamp())
        .bind(&workflow.id)
        .bind(current.version)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn delete_agent_workflow(&self, id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE agent_workflows SET status='draft' WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM agent_workflow_attempts WHERE workflow_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM agent_workflow_steps WHERE workflow_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let deleted_workflow = sqlx::query("DELETE FROM agent_workflows WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(deleted_workflow.rows_affected() == 1)
    }

    pub async fn create_agent_workflow_step(&self, step: &AgentWorkflowStep) -> Result<()> {
        step.validate()?;
        let mut tx = self.pool.begin().await?;
        bump_draft_workflow_version(&mut tx, &step.workflow_id).await?;
        sqlx::query(
            "INSERT INTO agent_workflow_steps(id,workflow_id,position,agent_id,template_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,input_contract_json,output_contract_json,permissions_json,context_policy_json,budget_json,spec_json,timeout_secs,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&step.id)
        .bind(&step.workflow_id)
        .bind(step.position)
        .bind(&step.agent_id)
        .bind(&step.template_id)
        .bind(&step.role)
        .bind(&step.backend)
        .bind(step.model.as_deref())
        .bind(&step.prompt_template)
        .bind(&step.input_schema_json)
        .bind(&step.output_schema_json)
        .bind(&step.input_contract_json)
        .bind(&step.output_contract_json)
        .bind(&step.permissions_json)
        .bind(&step.context_policy_json)
        .bind(&step.budget_json)
        .bind(&step.spec_json)
        .bind(step.timeout_secs)
        .bind(step.created_at)
        .bind(step.updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_agent_workflow_step(&self, id: &str) -> Result<Option<AgentWorkflowStep>> {
        sqlx::query("SELECT id,workflow_id,position,agent_id,template_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,input_contract_json,output_contract_json,permissions_json,context_policy_json,budget_json,spec_json,timeout_secs,created_at,updated_at FROM agent_workflow_steps WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(step_from_row)
            .transpose()
    }

    pub async fn list_agent_workflow_steps(
        &self,
        workflow_id: &str,
    ) -> Result<Vec<AgentWorkflowStep>> {
        let rows = sqlx::query("SELECT id,workflow_id,position,agent_id,template_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,input_contract_json,output_contract_json,permissions_json,context_policy_json,budget_json,spec_json,timeout_secs,created_at,updated_at FROM agent_workflow_steps WHERE workflow_id=? ORDER BY position,id")
            .bind(workflow_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(step_from_row).collect()
    }

    pub async fn update_agent_workflow_step(&self, step: &AgentWorkflowStep) -> Result<bool> {
        step.validate()?;
        let mut tx = self.pool.begin().await?;
        let current_workflow_id = sqlx::query_scalar::<_, String>(
            "SELECT workflow_id FROM agent_workflow_steps WHERE id=?",
        )
        .bind(&step.id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(current_workflow_id) = current_workflow_id else {
            tx.rollback().await?;
            return Ok(false);
        };
        if current_workflow_id != step.workflow_id {
            anyhow::bail!("workflow steps cannot be moved between plans");
        }
        bump_draft_workflow_version(&mut tx, &current_workflow_id).await?;
        let updated = sqlx::query("UPDATE agent_workflow_steps SET workflow_id=?,position=?,agent_id=?,template_id=?,role=?,backend=?,model=?,prompt_template=?,input_schema_json=?,output_schema_json=?,input_contract_json=?,output_contract_json=?,permissions_json=?,context_policy_json=?,budget_json=?,spec_json=?,timeout_secs=?,updated_at=? WHERE id=?")
            .bind(&step.workflow_id).bind(step.position).bind(&step.agent_id)
            .bind(&step.template_id).bind(&step.role).bind(&step.backend).bind(step.model.as_deref())
            .bind(&step.prompt_template).bind(&step.input_schema_json)
            .bind(&step.output_schema_json).bind(&step.input_contract_json)
            .bind(&step.output_contract_json).bind(&step.permissions_json)
            .bind(&step.context_policy_json).bind(&step.budget_json).bind(&step.spec_json)
            .bind(step.timeout_secs).bind(chrono::Utc::now().timestamp())
            .bind(&step.id)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn delete_agent_workflow_step(&self, id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let workflow_id = sqlx::query_scalar::<_, String>(
            "SELECT workflow_id FROM agent_workflow_steps WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(workflow_id) = workflow_id else {
            tx.rollback().await?;
            return Ok(false);
        };
        bump_draft_workflow_version(&mut tx, &workflow_id).await?;
        let deleted = sqlx::query("DELETE FROM agent_workflow_steps WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(deleted.rows_affected() == 1)
    }
}
