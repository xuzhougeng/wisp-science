//! Durable acceptance of mobile turns. A lost reply or restart never resubmits work.
use super::Store;
use anyhow::{ensure, Result};
use sqlx::Row;

#[derive(Debug)]
pub struct ServerTurn {
    pub request_id: String,
    pub status: String,
    pub error: Option<String>,
}

impl Store {
    pub async fn server_turn_request(&self, frame: &str, id: &str) -> Result<Option<String>> {
        if let Some(store) = self.route_entity("frames", "id", frame).await? {
            return Box::pin(store.server_turn_request(frame, id)).await;
        }
        Ok(sqlx::query_scalar(
            "SELECT request_json FROM server_turns WHERE frame_id=? AND request_id=?",
        )
        .bind(frame)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// True only for a newly accepted request. Reusing an ID with changed input fails.
    pub async fn accept_server_turn(
        &self,
        frame: &str,
        id: &str,
        request_json: &str,
    ) -> Result<bool> {
        if let Some(store) = self.route_entity("frames", "id", frame).await? {
            return Box::pin(store.accept_server_turn(frame, id, request_json)).await;
        }
        let mut tx = self.begin_write().await?;
        let inserted = sqlx::query("INSERT INTO server_turns(frame_id,request_id,request_json,status,created_at) VALUES(?,?,?,'accepted',?) ON CONFLICT(frame_id,request_id) DO NOTHING")
            .bind(frame).bind(id).bind(request_json).bind(chrono::Utc::now().timestamp_millis())
            .execute(&mut *tx).await?.rows_affected() == 1;
        let saved: String = sqlx::query_scalar(
            "SELECT request_json FROM server_turns WHERE frame_id=? AND request_id=?",
        )
        .bind(frame)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            saved == request_json,
            "Request ID was already used with different input"
        );
        tx.commit().await?;
        Ok(inserted)
    }

    pub async fn finish_server_turn(
        &self,
        frame: &str,
        id: &str,
        error: Option<&str>,
    ) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", frame).await? {
            return Box::pin(store.finish_server_turn(frame, id, error)).await;
        }
        sqlx::query("UPDATE server_turns SET status=?,error=? WHERE frame_id=? AND request_id=? AND status='accepted'")
            .bind(if error.is_some() { "failed" } else { "completed" }).bind(error).bind(frame).bind(id)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn latest_server_turn(&self, frame: &str) -> Result<Option<ServerTurn>> {
        if let Some(store) = self.route_entity("frames", "id", frame).await? {
            return Box::pin(store.latest_server_turn(frame)).await;
        }
        let row = sqlx::query("SELECT request_id,status,error FROM server_turns WHERE frame_id=? ORDER BY created_at DESC,rowid DESC LIMIT 1")
            .bind(frame).fetch_optional(&self.pool).await?;
        row.map(|row| {
            Ok(ServerTurn {
                request_id: row.try_get("request_id")?,
                status: row.try_get("status")?,
                error: row.try_get("error")?,
            })
        })
        .transpose()
    }

    /// Only the exclusive server database owner calls this at startup.
    pub async fn interrupt_server_turns(&self, project: &str) -> Result<()> {
        if let Some(store) = self.route_project(project).await? {
            return Box::pin(store.interrupt_server_turns(project)).await;
        }
        sqlx::query("UPDATE server_turns SET status='interrupted',error='Server restarted during this turn. Review saved output before sending another request.' WHERE status='accepted' AND frame_id IN (SELECT id FROM frames WHERE project_id=?)")
            .bind(project).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn server_turn_migration_upgrades_existing_databases_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.sqlite");
        let store = Store::open(&path).await.unwrap();
        sqlx::query("DROP TABLE server_turns")
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(crate::SERVER_TURNS_MIGRATION)
            .execute(&store.pool)
            .await
            .unwrap();
        drop(store);
        for _ in 0..2 {
            let upgraded = Store::open(&path).await.unwrap();
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM server_turns")
                .fetch_one(&upgraded.pool)
                .await
                .unwrap();
            assert_eq!(count, 0);
            assert!(upgraded
                .schema_migrations()
                .await
                .unwrap()
                .contains(&crate::SERVER_TURNS_MIGRATION.into()));
        }
    }

    #[tokio::test]
    async fn server_turn_acceptance_survives_reopen_and_never_replays() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wisp.sqlite");
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("p", "p", dir.path().to_str().unwrap())
            .await
            .unwrap();
        store.create_frame("s", "p", "main", "test").await.unwrap();
        assert!(store
            .accept_server_turn("s", "r", "original")
            .await
            .unwrap());
        assert!(!store
            .accept_server_turn("s", "r", "original")
            .await
            .unwrap());
        assert!(store.accept_server_turn("s", "r", "changed").await.is_err());
        drop(store);
        let store = Store::open(&path).await.unwrap();
        store.interrupt_server_turns("p").await.unwrap();
        assert_eq!(
            store.latest_server_turn("s").await.unwrap().unwrap().status,
            "interrupted"
        );
        assert!(!store
            .accept_server_turn("s", "r", "original")
            .await
            .unwrap());
        assert!(store.accept_server_turn("s", "r2", "next").await.unwrap());
        store.finish_server_turn("s", "r2", None).await.unwrap();
        assert_eq!(
            store.latest_server_turn("s").await.unwrap().unwrap().status,
            "completed"
        );
    }
}
