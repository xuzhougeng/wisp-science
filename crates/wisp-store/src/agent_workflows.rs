use anyhow::Result;
use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentWorkflow {
    pub id: String,
    pub project_id: String,
    pub workspace_id: String,
    pub name: String,
    pub description: String,
    pub version: i64,
    pub enabled: bool,
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
        let workflow = Self {
            id: id.into(),
            project_id: project_id.into(),
            workspace_id: workspace_id.into(),
            name: name.into(),
            description: String::new(),
            version: 1,
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        workflow.validate()?;
        Ok(workflow)
    }

    fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("id", self.id.as_str()),
            ("project_id", self.project_id.as_str()),
            ("workspace_id", self.workspace_id.as_str()),
            ("name", self.name.as_str()),
        ] {
            if value.trim().is_empty() {
                anyhow::bail!("workflow {field} is required");
            }
        }
        if self.version <= 0 {
            anyhow::bail!("workflow version must be positive");
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
    pub role: String,
    pub backend: String,
    pub model: Option<String>,
    pub prompt_template: String,
    pub input_schema_json: String,
    pub output_schema_json: String,
    pub permissions_json: String,
    pub context_policy_json: String,
    pub timeout_secs: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
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
        let step = Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            position,
            agent_id: agent_id.into(),
            role: role.into(),
            backend: backend.into(),
            model: None,
            prompt_template: prompt_template.into(),
            input_schema_json: "{}".into(),
            output_schema_json: "{}".into(),
            permissions_json: "{}".into(),
            context_policy_json: "{}".into(),
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
            ("permissions_json", self.permissions_json.as_str()),
            ("context_policy_json", self.context_policy_json.as_str()),
        ] {
            if serde_json::from_str::<serde_json::Value>(value).is_err() {
                anyhow::bail!("workflow step {field} must be valid JSON");
            }
        }
        Ok(())
    }
}

fn workflow_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AgentWorkflow> {
    Ok(AgentWorkflow {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        workspace_id: row.try_get("workspace_id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        version: row.try_get("version")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
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
        role: row.try_get("role")?,
        backend: row.try_get("backend")?,
        model: row.try_get("model")?,
        prompt_template: row.try_get("prompt_template")?,
        input_schema_json: row.try_get("input_schema_json")?,
        output_schema_json: row.try_get("output_schema_json")?,
        permissions_json: row.try_get("permissions_json")?,
        context_policy_json: row.try_get("context_policy_json")?,
        timeout_secs: row.try_get("timeout_secs")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

impl super::Store {
    pub async fn create_agent_workflow(&self, workflow: &AgentWorkflow) -> Result<()> {
        workflow.validate()?;
        sqlx::query(
            "INSERT INTO agent_workflows(id,project_id,workspace_id,name,description,version,enabled,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
        )
        .bind(&workflow.id)
        .bind(&workflow.project_id)
        .bind(&workflow.workspace_id)
        .bind(&workflow.name)
        .bind(&workflow.description)
        .bind(workflow.version)
        .bind(workflow.enabled as i64)
        .bind(workflow.created_at)
        .bind(workflow.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_agent_workflow(&self, id: &str) -> Result<Option<AgentWorkflow>> {
        sqlx::query(
            "SELECT id,project_id,workspace_id,name,description,version,enabled,created_at,updated_at FROM agent_workflows WHERE id=?",
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
            "SELECT id,project_id,workspace_id,name,description,version,enabled,created_at,updated_at FROM agent_workflows WHERE project_id=? ORDER BY name,id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(workflow_from_row).collect()
    }

    pub async fn update_agent_workflow(&self, workflow: &AgentWorkflow) -> Result<bool> {
        workflow.validate()?;
        let updated = sqlx::query(
            "UPDATE agent_workflows SET project_id=?,workspace_id=?,name=?,description=?,version=?,enabled=?,updated_at=? WHERE id=?",
        )
        .bind(&workflow.project_id)
        .bind(&workflow.workspace_id)
        .bind(&workflow.name)
        .bind(&workflow.description)
        .bind(workflow.version)
        .bind(workflow.enabled as i64)
        .bind(workflow.updated_at)
        .bind(&workflow.id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn delete_agent_workflow(&self, id: &str) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM agent_workflows WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(deleted.rows_affected() == 1)
    }

    pub async fn create_agent_workflow_step(&self, step: &AgentWorkflowStep) -> Result<()> {
        step.validate()?;
        sqlx::query(
            "INSERT INTO agent_workflow_steps(id,workflow_id,position,agent_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,permissions_json,context_policy_json,timeout_secs,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&step.id)
        .bind(&step.workflow_id)
        .bind(step.position)
        .bind(&step.agent_id)
        .bind(&step.role)
        .bind(&step.backend)
        .bind(step.model.as_deref())
        .bind(&step.prompt_template)
        .bind(&step.input_schema_json)
        .bind(&step.output_schema_json)
        .bind(&step.permissions_json)
        .bind(&step.context_policy_json)
        .bind(step.timeout_secs)
        .bind(step.created_at)
        .bind(step.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_agent_workflow_step(&self, id: &str) -> Result<Option<AgentWorkflowStep>> {
        sqlx::query("SELECT id,workflow_id,position,agent_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,permissions_json,context_policy_json,timeout_secs,created_at,updated_at FROM agent_workflow_steps WHERE id=?")
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
        let rows = sqlx::query("SELECT id,workflow_id,position,agent_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,permissions_json,context_policy_json,timeout_secs,created_at,updated_at FROM agent_workflow_steps WHERE workflow_id=? ORDER BY position,id")
            .bind(workflow_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(step_from_row).collect()
    }

    pub async fn update_agent_workflow_step(&self, step: &AgentWorkflowStep) -> Result<bool> {
        step.validate()?;
        let updated = sqlx::query("UPDATE agent_workflow_steps SET workflow_id=?,position=?,agent_id=?,role=?,backend=?,model=?,prompt_template=?,input_schema_json=?,output_schema_json=?,permissions_json=?,context_policy_json=?,timeout_secs=?,updated_at=? WHERE id=?")
            .bind(&step.workflow_id).bind(step.position).bind(&step.agent_id)
            .bind(&step.role).bind(&step.backend).bind(step.model.as_deref())
            .bind(&step.prompt_template).bind(&step.input_schema_json)
            .bind(&step.output_schema_json).bind(&step.permissions_json)
            .bind(&step.context_policy_json).bind(step.timeout_secs)
            .bind(step.updated_at).bind(&step.id)
            .execute(&self.pool).await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn delete_agent_workflow_step(&self, id: &str) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM agent_workflow_steps WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(deleted.rows_affected() == 1)
    }
}
