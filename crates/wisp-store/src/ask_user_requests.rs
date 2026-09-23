//! The ACP bridge's `ask_user` handshake rows.
//!
//! The bridge MCP server runs in a separate process (launched by the ACP
//! agent), so SQLite is the only channel it shares with the host: the bridge
//! INSERTs a pending row and polls it; the host's turn loop surfaces pendings
//! to the UI, `respond_ask_user` writes the answer, and the bridge consumes
//! (deletes) it. Rows that outlive their turn are expired, never resolved —
//! the reload path renders them as dead cards.

use super::Store;
use anyhow::Result;
use std::collections::HashSet;

/// What the bridge's poll loop sees.
#[derive(Debug, PartialEq, Eq)]
pub enum AskUserPoll {
    Pending,
    Answered(String),
    /// Row missing or expired: the question can no longer be answered.
    Gone,
}

impl Store {
    pub async fn insert_ask_user_request(
        &self,
        request_id: &str,
        frame_id: &str,
        payload_json: &str,
    ) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.insert_ask_user_request(request_id, frame_id, payload_json))
                .await;
        }
        sqlx::query(
            "INSERT INTO ask_user_requests(request_id,frame_id,payload_json,status,created_at) \
             VALUES(?,?,?,'pending',?)",
        )
        .bind(request_id)
        .bind(frame_id)
        .bind(payload_json)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Bridge side: poll for the answer, consuming the row once it has one.
    pub async fn poll_ask_user_answer(&self, request_id: &str) -> Result<AskUserPoll> {
        if let Some(store) = self
            .route_entity("ask_user_requests", "request_id", request_id)
            .await?
        {
            return Box::pin(store.poll_ask_user_answer(request_id)).await;
        }
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT status, answer FROM ask_user_requests WHERE request_id=?")
                .bind(request_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(match row {
            Some((status, Some(answer))) if status == "answered" => {
                sqlx::query("DELETE FROM ask_user_requests WHERE request_id=?")
                    .bind(request_id)
                    .execute(&self.pool)
                    .await?;
                AskUserPoll::Answered(answer)
            }
            Some((status, _)) if status == "pending" => AskUserPoll::Pending,
            _ => AskUserPoll::Gone,
        })
    }

    /// Host side: the pendings the turn loop has to surface to the UI.
    pub async fn pending_ask_user_requests(&self, frame_id: &str) -> Result<Vec<(String, String)>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.pending_ask_user_requests(frame_id)).await;
        }
        Ok(sqlx::query_as(
            "SELECT request_id, payload_json FROM ask_user_requests \
             WHERE frame_id=? AND status='pending' ORDER BY created_at, rowid",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// `respond_ask_user`: answer a still-pending request. False when the row
    /// is gone or already answered/expired — the caller reports "no longer
    /// pending" instead of silently double-writing.
    pub async fn answer_ask_user_request(&self, request_id: &str, answer: &str) -> Result<bool> {
        if let Some(store) = self
            .route_entity("ask_user_requests", "request_id", request_id)
            .await?
        {
            return Box::pin(store.answer_ask_user_request(request_id, answer)).await;
        }
        let updated = sqlx::query(
            "UPDATE ask_user_requests SET status='answered', answer=?, answered_at=? \
             WHERE request_id=? AND status='pending'",
        )
        .bind(answer)
        .bind(chrono::Utc::now().timestamp())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() > 0)
    }

    /// Expire every pending row of a frame except the ones in `keep` (still
    /// live in the host's pending map). Returns what was expired so the
    /// caller can settle the on-screen cards.
    pub async fn expire_ask_user_requests_except(
        &self,
        frame_id: &str,
        keep: &HashSet<String>,
    ) -> Result<Vec<(String, String)>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.expire_ask_user_requests_except(frame_id, keep)).await;
        }
        let mut expired = Vec::new();
        for (request_id, payload_json) in self.pending_ask_user_requests(frame_id).await? {
            if keep.contains(&request_id) {
                continue;
            }
            sqlx::query(
                "UPDATE ask_user_requests SET status='expired' \
                 WHERE request_id=? AND status='pending'",
            )
            .bind(&request_id)
            .execute(&self.pool)
            .await?;
            expired.push((request_id, payload_json));
        }
        Ok(expired)
    }

    /// Reload: every surviving row of a frame as `(request_id, payload_json,
    /// status)`, oldest first. Answered rows are transient (the bridge deletes
    /// them on consumption) but render correctly if a load catches one.
    pub async fn ask_user_rows_for_frame(
        &self,
        frame_id: &str,
    ) -> Result<Vec<(String, String, String)>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.ask_user_rows_for_frame(frame_id)).await;
        }
        Ok(sqlx::query_as(
            "SELECT request_id, payload_json, status FROM ask_user_requests \
             WHERE frame_id=? ORDER BY created_at, rowid",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store_with_frame() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_ask_user_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("p", "Project", "/workspace")
            .await
            .unwrap();
        store.create_frame("f1", "p", "Wisp", "m").await.unwrap();
        (store, path)
    }

    #[tokio::test]
    async fn answer_lifecycle_resolves_once_and_consumes() {
        let (store, path) = store_with_frame().await;
        store
            .insert_ask_user_request("ask-1", "f1", r#"{"question":"Deploy?"}"#)
            .await
            .unwrap();

        assert_eq!(
            store.pending_ask_user_requests("f1").await.unwrap(),
            vec![("ask-1".into(), r#"{"question":"Deploy?"}"#.into())]
        );
        assert_eq!(
            store.poll_ask_user_answer("ask-1").await.unwrap(),
            AskUserPoll::Pending
        );

        assert!(store.answer_ask_user_request("ask-1", "yes").await.unwrap());
        assert!(
            !store
                .answer_ask_user_request("ask-1", "again")
                .await
                .unwrap(),
            "a second answer is refused"
        );

        assert_eq!(
            store.poll_ask_user_answer("ask-1").await.unwrap(),
            AskUserPoll::Answered("yes".into())
        );
        assert_eq!(
            store.poll_ask_user_answer("ask-1").await.unwrap(),
            AskUserPoll::Gone,
            "consumption deletes the row"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn expiry_keeps_live_requests_and_kills_the_rest() {
        let (store, path) = store_with_frame().await;
        store
            .insert_ask_user_request("live", "f1", "{}")
            .await
            .unwrap();
        store
            .insert_ask_user_request("dead", "f1", "{}")
            .await
            .unwrap();

        let keep: HashSet<String> = ["live".to_string()].into();
        let expired = store
            .expire_ask_user_requests_except("f1", &keep)
            .await
            .unwrap();
        assert_eq!(expired, vec![("dead".into(), "{}".into())]);

        assert_eq!(
            store.poll_ask_user_answer("dead").await.unwrap(),
            AskUserPoll::Gone
        );
        assert!(
            !store.answer_ask_user_request("dead", "late").await.unwrap(),
            "expired rows refuse answers"
        );
        assert_eq!(
            store.poll_ask_user_answer("live").await.unwrap(),
            AskUserPoll::Pending
        );

        let rows = store.ask_user_rows_for_frame("f1").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("live".into(), "{}".into(), "pending".into()));
        assert_eq!(rows[1], ("dead".into(), "{}".into(), "expired".into()));
        let _ = std::fs::remove_file(&path);
    }
}
