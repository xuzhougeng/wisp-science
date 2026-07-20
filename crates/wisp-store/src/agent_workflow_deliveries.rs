use anyhow::Result;
use sqlx::Row;

use super::Store;

pub const AGENT_WORKFLOW_COMPLETION_TOOL: &str = "delegate_tasks_completion";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentWorkflowDelivery {
    pub id: String,
    pub workflow_id: String,
    pub frame_id: String,
    pub generation: i64,
    pub auto_resume: bool,
    pub result_json: Option<String>,
    pub message_seq: Option<i64>,
    pub delivered_at: Option<i64>,
    pub resume_status: String,
    pub resume_error: Option<String>,
    pub presented_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AgentWorkflowDelivery> {
    Ok(AgentWorkflowDelivery {
        id: row.try_get("id")?,
        workflow_id: row.try_get("workflow_id")?,
        frame_id: row.try_get("frame_id")?,
        generation: row.try_get("generation")?,
        auto_resume: row.try_get::<i64, _>("auto_resume")? != 0,
        result_json: row.try_get("result_json")?,
        message_seq: row.try_get("message_seq")?,
        delivered_at: row.try_get("delivered_at")?,
        resume_status: row.try_get("resume_status")?,
        resume_error: row.try_get("resume_error")?,
        presented_at: row.try_get("presented_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const SELECT_DELIVERY: &str = "SELECT id,workflow_id,frame_id,generation,auto_resume,result_json,message_seq,delivered_at,resume_status,resume_error,presented_at,created_at,updated_at FROM agent_workflow_deliveries";

impl Store {
    /// Reserve one durable background-execution generation before any child
    /// starts. Attempt numbers and older delivery generations both participate
    /// so a retry that was interrupted before its first attempt still gets a
    /// distinct completion record.
    pub async fn create_agent_workflow_delivery(
        &self,
        workflow_id: &str,
        auto_resume: bool,
    ) -> Result<AgentWorkflowDelivery> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        let resume_status = if auto_resume { "pending" } else { "disabled" };
        let inserted = sqlx::query(
            "INSERT INTO agent_workflow_deliveries(\
             id,workflow_id,frame_id,generation,auto_resume,result_json,message_seq,delivered_at,\
             resume_status,resume_error,presented_at,created_at,updated_at) \
             SELECT ?,w.id,w.frame_id,\
               MAX(\
                 COALESCE((SELECT MAX(a.attempt) FROM agent_workflow_attempts a WHERE a.workflow_id=w.id),0),\
                 COALESCE((SELECT MAX(d.generation) FROM agent_workflow_deliveries d WHERE d.workflow_id=w.id),0)\
               )+1,?,NULL,NULL,NULL,?,NULL,NULL,?,? \
             FROM agent_workflows w WHERE w.id=? AND w.status='approved' AND w.frame_id IS NOT NULL",
        )
        .bind(&id)
        .bind(auto_resume as i64)
        .bind(resume_status)
        .bind(now)
        .bind(now)
        .bind(workflow_id)
        .execute(&self.pool)
        .await?;
        if inserted.rows_affected() != 1 {
            anyhow::bail!("background Agent workflow must be approved and own a conversation");
        }
        self.get_agent_workflow_delivery(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("background Agent delivery disappeared after creation"))
    }

    pub async fn get_agent_workflow_delivery(
        &self,
        id: &str,
    ) -> Result<Option<AgentWorkflowDelivery>> {
        sqlx::query(&format!("{SELECT_DELIVERY} WHERE id=?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }

    pub async fn list_agent_workflow_deliveries(
        &self,
        workflow_id: &str,
    ) -> Result<Vec<AgentWorkflowDelivery>> {
        let rows = sqlx::query(&format!(
            "{SELECT_DELIVERY} WHERE workflow_id=? ORDER BY generation"
        ))
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }

    pub async fn complete_agent_workflow_delivery(
        &self,
        id: &str,
        result_json: &str,
    ) -> Result<bool> {
        if !serde_json::from_str::<serde_json::Value>(result_json)
            .is_ok_and(|value| value.is_object())
        {
            anyhow::bail!("Agent workflow delivery result must be a JSON object");
        }
        let updated = sqlx::query(
            "UPDATE agent_workflow_deliveries SET result_json=?,updated_at=? \
             WHERE id=? AND result_json IS NULL",
        )
        .bind(result_json)
        .bind(chrono::Utc::now().timestamp())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn list_incomplete_agent_workflow_deliveries(
        &self,
    ) -> Result<Vec<AgentWorkflowDelivery>> {
        let rows = sqlx::query(
            "SELECT d.id,d.workflow_id,d.frame_id,d.generation,d.auto_resume,d.result_json,\
             d.message_seq,d.delivered_at,d.resume_status,d.resume_error,d.presented_at,\
             d.created_at,d.updated_at FROM agent_workflow_deliveries d \
             JOIN agent_workflows w ON w.id=d.workflow_id \
             WHERE d.result_json IS NULL AND w.status IN ('succeeded','failed','cancelled') \
             ORDER BY d.created_at,d.id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }

    pub async fn list_ready_agent_workflow_delivery_frames(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT frame_id FROM agent_workflow_deliveries \
             WHERE result_json IS NOT NULL \
               AND (delivered_at IS NULL OR resume_status='pending') \
             ORDER BY frame_id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Append every ready completion as an internal conversation message and
    /// mark it delivered in the same transaction. The non-standard persisted
    /// role is decoded as a user-context turn for providers but is not counted
    /// as user-authored history by session queries.
    pub async fn deliver_agent_workflow_completions(
        &self,
        frame_id: &str,
    ) -> Result<Vec<AgentWorkflowDelivery>> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(&format!(
            "{SELECT_DELIVERY} WHERE frame_id=? AND result_json IS NOT NULL \
             AND delivered_at IS NULL ORDER BY created_at,id"
        ))
        .bind(frame_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut delivered = Vec::with_capacity(rows.len());
        for row in rows {
            let mut item = from_row(&row)?;
            let seq: i64 =
                sqlx::query_scalar("SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE frame_id=?")
                    .bind(frame_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let result = item
                .result_json
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("ready Agent delivery has no result"))?;
            let content = serde_json::to_string(&wisp_llm::Content::text(result))?;
            sqlx::query(
                "INSERT INTO messages(id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name) \
                 VALUES(?,?,?,'internal',?,NULL,?,?,NULL,?,NULL)",
            )
            .bind(format!("agent-delivery-{}", item.id))
            .bind(frame_id)
            .bind(seq)
            .bind(content)
            .bind(&item.id)
            .bind(AGENT_WORKFLOW_COMPLETION_TOOL)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *tx)
            .await?;
            let now = chrono::Utc::now().timestamp();
            let updated = sqlx::query(
                "UPDATE agent_workflow_deliveries SET message_seq=?,delivered_at=?,updated_at=? \
                 WHERE id=? AND delivered_at IS NULL",
            )
            .bind(seq)
            .bind(now)
            .bind(now)
            .bind(&item.id)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                anyhow::bail!("Agent workflow completion lost its delivery claim");
            }
            item.message_seq = Some(seq);
            item.delivered_at = Some(now);
            item.updated_at = now;
            delivered.push(item);
        }
        tx.commit().await?;
        Ok(delivered)
    }

    /// Claim every delivered auto-resume request for one idle conversation.
    /// One synthesized parent turn may consume several batches that completed
    /// together, but each delivery can be claimed only once.
    pub async fn claim_agent_workflow_auto_resumes(
        &self,
        frame_id: &str,
    ) -> Result<Vec<AgentWorkflowDelivery>> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(&format!(
            "{SELECT_DELIVERY} WHERE frame_id=? AND delivered_at IS NOT NULL \
             AND resume_status='pending' ORDER BY created_at,id"
        ))
        .bind(frame_id)
        .fetch_all(&mut *tx)
        .await?;
        let claimed = rows.iter().map(from_row).collect::<Result<Vec<_>>>()?;
        let now = chrono::Utc::now().timestamp();
        for item in &claimed {
            let updated = sqlx::query(
                "UPDATE agent_workflow_deliveries SET resume_status='running',updated_at=? \
                 WHERE id=? AND resume_status='pending'",
            )
            .bind(now)
            .bind(&item.id)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                anyhow::bail!("Agent workflow auto-resume lost its claim");
            }
        }
        tx.commit().await?;
        Ok(claimed)
    }

    pub async fn finish_agent_workflow_auto_resumes(
        &self,
        ids: &[String],
        success: bool,
        error: Option<&str>,
    ) -> Result<u64> {
        let status = if success { "succeeded" } else { "failed" };
        let now = chrono::Utc::now().timestamp();
        let mut changed = 0;
        let mut tx = self.pool.begin().await?;
        for id in ids {
            changed += sqlx::query(
                "UPDATE agent_workflow_deliveries SET resume_status=?,resume_error=?,presented_at=?,updated_at=? \
                 WHERE id=? AND resume_status='running'",
            )
            .bind(status)
            .bind(error)
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn list_unpresented_agent_workflow_deliveries(
        &self,
        frame_id: &str,
    ) -> Result<Vec<AgentWorkflowDelivery>> {
        let rows = sqlx::query(&format!(
            "{SELECT_DELIVERY} WHERE frame_id=? AND delivered_at IS NOT NULL \
             AND presented_at IS NULL ORDER BY created_at,id"
        ))
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }

    pub async fn mark_agent_workflow_deliveries_presented(&self, ids: &[String]) -> Result<u64> {
        let now = chrono::Utc::now().timestamp();
        let mut changed = 0;
        let mut tx = self.pool.begin().await?;
        for id in ids {
            changed += sqlx::query(
                "UPDATE agent_workflow_deliveries SET presented_at=?,updated_at=? \
                 WHERE id=? AND presented_at IS NULL",
            )
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
        Ok(changed)
    }
}
