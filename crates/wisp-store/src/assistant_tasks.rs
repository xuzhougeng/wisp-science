//! The research assistant's plan: dated items the researcher asked it to
//! remember, each optionally handed to a project conversation. They outlive
//! the assistant's context compaction, so a plan is never only in the chat.
//! Global data: always the application database, never a project's.

use super::Store;
use anyhow::{ensure, Result};
use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantTask {
    pub id: String,
    /// Local calendar day the item is planned for, `YYYY-MM-DD`.
    pub day: String,
    pub title: String,
    pub project_id: Option<String>,
    /// The project conversation the item was dispatched to.
    pub session_id: Option<String>,
    /// `open`, `done` or `dropped`.
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub const ASSISTANT_TASK_STATUSES: &[&str] = &["open", "done", "dropped"];

impl Store {
    pub async fn add_assistant_task(&self, task: &AssistantTask) -> Result<()> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.add_assistant_task(task)).await;
        }
        ensure!(
            ASSISTANT_TASK_STATUSES.contains(&task.status.as_str()),
            "Unknown task status"
        );
        sqlx::query(
            "INSERT INTO assistant_tasks(id,day,title,project_id,session_id,status,created_at,updated_at) \
             VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(&task.id)
        .bind(&task.day)
        .bind(task.title.trim())
        .bind(task.project_id.as_deref())
        .bind(task.session_id.as_deref())
        .bind(&task.status)
        .bind(task.created_at)
        .bind(task.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Items planned for `[from_day, until_day]`, plus earlier items still
    /// open — yesterday's unfinished plan belongs on today's list.
    pub async fn assistant_tasks(
        &self,
        from_day: &str,
        until_day: &str,
    ) -> Result<Vec<AssistantTask>> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.assistant_tasks(from_day, until_day)).await;
        }
        let rows = sqlx::query(
            "SELECT id,day,title,project_id,session_id,status,created_at,updated_at FROM assistant_tasks \
             WHERE (day>=? AND day<=?) OR (day<? AND status='open') \
             ORDER BY day, created_at, id LIMIT 200",
        )
        .bind(from_day)
        .bind(until_day)
        .bind(from_day)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AssistantTask {
                    id: row.try_get("id")?,
                    day: row.try_get("day")?,
                    title: row.try_get("title")?,
                    project_id: row.try_get("project_id")?,
                    session_id: row.try_get("session_id")?,
                    status: row.try_get("status")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }

    /// `false` when no item has this id.
    pub async fn set_assistant_task_status(
        &self,
        id: &str,
        status: &str,
        now: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.set_assistant_task_status(id, status, now)).await;
        }
        ensure!(
            ASSISTANT_TASK_STATUSES.contains(&status),
            "Unknown task status"
        );
        let result = sqlx::query("UPDATE assistant_tasks SET status=?, updated_at=? WHERE id=?")
            .bind(status)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Hand an existing item to a project conversation. `false` when no item
    /// has this id.
    pub async fn link_assistant_task(
        &self,
        id: &str,
        project_id: &str,
        session_id: &str,
        now: i64,
    ) -> Result<bool> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.link_assistant_task(id, project_id, session_id, now)).await;
        }
        let result = sqlx::query(
            "UPDATE assistant_tasks SET project_id=?, session_id=?, updated_at=? WHERE id=?",
        )
        .bind(project_id)
        .bind(session_id)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, day: &str, status: &str) -> AssistantTask {
        AssistantTask {
            id: id.into(),
            day: day.into(),
            title: format!("item {id}"),
            project_id: None,
            session_id: None,
            status: status.into(),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn todays_list_carries_unfinished_items_forward() {
        let path =
            std::env::temp_dir().join(format!("wisp-assistant-{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store
            .add_assistant_task(&task("old-open", "2026-09-30", "open"))
            .await
            .unwrap();
        store
            .add_assistant_task(&task("old-done", "2026-09-30", "done"))
            .await
            .unwrap();
        store
            .add_assistant_task(&task("today", "2026-10-02", "open"))
            .await
            .unwrap();
        store
            .add_assistant_task(&task("later", "2026-10-05", "open"))
            .await
            .unwrap();

        let ids: Vec<_> = store
            .assistant_tasks("2026-10-02", "2026-10-02")
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["old-open", "today"]);

        assert!(store
            .set_assistant_task_status("old-open", "done", 2)
            .await
            .unwrap());
        assert!(!store
            .set_assistant_task_status("missing", "done", 2)
            .await
            .unwrap());
        assert!(store
            .set_assistant_task_status("today", "finished", 2)
            .await
            .is_err());
        let ids: Vec<_> = store
            .assistant_tasks("2026-10-02", "2026-10-02")
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["today"]);
        let _ = std::fs::remove_file(path);
    }
}
