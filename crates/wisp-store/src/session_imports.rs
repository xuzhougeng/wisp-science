//! Mapping between imported wisp session-export archives and Wisp frames.
//! Keyed by the exporting side's session id so re-imports are idempotent:
//! re-importing the same archive fast-forwards the existing frame instead of
//! creating a duplicate session. Mirrors `codex_imports`.

use super::Store;
use anyhow::Result;
use std::collections::HashSet;
use wisp_llm::{Message, Role};

/// One validated conversation recovered from a workspace context archive.
/// The caller owns archive parsing and deduplication; the store owns the
/// all-or-nothing project/frame/message insertion.
#[derive(Clone)]
pub struct RecoveredWorkspaceSession {
    pub source_session_id: String,
    pub source_path: String,
    pub messages: Vec<Message>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Store {
    /// Register an existing workspace and import all recovered conversations
    /// in one transaction. A failed frame/message/provenance insert leaves no
    /// half-created project in the live database.
    pub async fn create_recovered_workspace_project(
        &self,
        project_id: &str,
        name: &str,
        workspace_dir: &str,
        model_id: &str,
        sessions: &[RecoveredWorkspaceSession],
    ) -> Result<Vec<String>> {
        if project_id.trim().is_empty() || name.trim().is_empty() || workspace_dir.trim().is_empty()
        {
            anyhow::bail!("recovered project identity is incomplete");
        }
        if sessions.is_empty() {
            anyhow::bail!("no recoverable sessions were provided");
        }
        let mut source_ids = HashSet::new();
        for session in sessions {
            if session.source_session_id.trim().is_empty()
                || session.messages.is_empty()
                || !session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::User)
            {
                anyhow::bail!("recovered session is incomplete");
            }
            if !source_ids.insert(session.source_session_id.as_str()) {
                anyhow::bail!("recovered session ids are not unique");
            }
        }

        let now = chrono::Utc::now().timestamp();
        let folder_id = uuid::Uuid::new_v4().to_string();
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) \
             VALUES(?,?,'',?,?,?)",
        )
        .bind(project_id)
        .bind(name.trim())
        .bind(workspace_dir)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO folders(id,project_id,name,created_at,updated_at) VALUES(?,?,?,?,?)",
        )
        .bind(&folder_id)
        .bind(project_id)
        .bind("Recovered")
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        let mut frame_ids = Vec::with_capacity(sessions.len());
        for session in sessions {
            let frame_id = uuid::Uuid::new_v4().to_string();
            let created_at = if session.created_at > 0 {
                session.created_at
            } else {
                now
            };
            let updated_at = if session.updated_at >= created_at {
                session.updated_at
            } else {
                created_at
            };
            sqlx::query(
                "INSERT INTO frames(\
                   id,parent_frame_id,root_frame_id,agent_name,status,project_id,folder_id,model,\
                   input_tokens,output_tokens,created_at,updated_at,completed_at\
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,NULL)",
            )
            .bind(&frame_id)
            .bind(&frame_id)
            .bind(&frame_id)
            .bind("OPERON")
            .bind("running")
            .bind(project_id)
            .bind(&folder_id)
            .bind(model_id)
            .bind(0i64)
            .bind(0i64)
            .bind(created_at)
            .bind(updated_at)
            .execute(&mut *tx)
            .await?;
            for (index, message) in session.messages.iter().enumerate() {
                super::sessions::insert_message_row(&mut *tx, &frame_id, index as i64 + 1, message)
                    .await?;
            }
            sqlx::query(
                "INSERT INTO session_imports(\
                   source_session_id,frame_id,source_path,created_at,updated_at\
                 ) VALUES(?,?,?,?,?)",
            )
            .bind(&session.source_session_id)
            .bind(&frame_id)
            .bind(&session.source_path)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            frame_ids.push(frame_id);
        }
        tx.commit().await?;
        Ok(frame_ids)
    }

    /// The frame a session archive was already imported into, if any.
    pub async fn find_session_import(&self, source_session_id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT frame_id FROM session_imports WHERE source_session_id=?")
                .bind(source_session_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Record (or refresh) the source session → frame mapping after an import.
    pub async fn record_session_import(
        &self,
        source_session_id: &str,
        frame_id: &str,
        source_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO session_imports(source_session_id,frame_id,source_path,created_at,updated_at) \
             VALUES(?,?,?,?,?) \
             ON CONFLICT(source_session_id) DO UPDATE SET \
             frame_id=excluded.frame_id, source_path=excluded.source_path, \
             updated_at=excluded.updated_at",
        )
        .bind(source_session_id)
        .bind(frame_id)
        .bind(source_path)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SESSION_IMPORTS_MIGRATION;

    async fn store_with_frame() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_session_imports_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("p", "Project", "/workspace")
            .await
            .unwrap();
        store.create_frame("f1", "p", "wisp", "m").await.unwrap();
        (store, path)
    }

    #[tokio::test]
    async fn session_import_round_trips_and_cascades_on_delete() {
        let (store, path) = store_with_frame().await;
        assert_eq!(store.find_session_import("src-1").await.unwrap(), None);
        store
            .record_session_import("src-1", "f1", "/tmp/wisp-session-src-1.zip")
            .await
            .unwrap();
        assert_eq!(
            store.find_session_import("src-1").await.unwrap(),
            Some("f1".to_string())
        );
        // Upsert keeps a single row per source session.
        store
            .record_session_import("src-1", "f1", "/tmp/other.zip")
            .await
            .unwrap();
        assert_eq!(
            store.find_session_import("src-1").await.unwrap(),
            Some("f1".to_string())
        );

        // Deleting the Wisp session frees the source id for re-import.
        store.delete_session("f1", "p").await.unwrap();
        assert_eq!(store.find_session_import("src-1").await.unwrap(), None);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn session_imports_migration_is_idempotent() {
        let (store, path) = store_with_frame().await;
        store
            .record_session_import("src-1", "f1", "/tmp/wisp-session-src-1.zip")
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(SESSION_IMPORTS_MIGRATION)
            .execute(&store.pool)
            .await
            .unwrap();
        drop(store);

        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(
            reopened.find_session_import("src-1").await.unwrap(),
            Some("f1".to_string())
        );
        assert!(reopened
            .schema_migrations()
            .await
            .unwrap()
            .contains(&SESSION_IMPORTS_MIGRATION.to_string()));
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn recovered_workspace_project_is_inserted_atomically() {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_workspace_recovery_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        let sessions = vec![
            RecoveredWorkspaceSession {
                source_session_id: "workspace:p:source-1".into(),
                source_path: ".wisp/history/one.json".into(),
                messages: vec![
                    Message::user("first question"),
                    Message::assistant("answer"),
                ],
                created_at: 10,
                updated_at: 20,
            },
            RecoveredWorkspaceSession {
                source_session_id: "workspace:p:source-2".into(),
                source_path: ".wisp/history/two.json".into(),
                messages: vec![Message::user("second question")],
                created_at: 30,
                updated_at: 30,
            },
        ];

        let frames = store
            .create_recovered_workspace_project(
                "p",
                "Recovered study",
                "/workspace",
                "model",
                &sessions,
            )
            .await
            .unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(store.list_sessions("p").await.unwrap().len(), 2);
        assert_eq!(store.message_count(&frames[0]).await.unwrap(), 2);
        assert_eq!(
            store
                .find_session_import("workspace:p:source-1")
                .await
                .unwrap()
                .as_deref(),
            Some(frames[0].as_str())
        );

        // This conflict happens after the transaction has inserted the new
        // project, folder, frame, and messages. The unique provenance failure
        // must roll the entire transaction back.
        let error = store
            .create_recovered_workspace_project(
                "other",
                "Broken",
                "/other",
                "model",
                &[sessions[0].clone()],
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("UNIQUE"));
        assert!(store.get_project("other").await.unwrap().is_none());

        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
