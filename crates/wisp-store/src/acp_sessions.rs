use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpSessionBinding {
    pub frame_id: String,
    pub agent_profile_id: String,
    pub profile_fingerprint: String,
    pub agent_session_id: String,
    pub cwd: String,
    pub protocol_version: i64,
    pub agent_info_json: String,
    pub capabilities_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn from_row(row: sqlx::sqlite::SqliteRow) -> Result<AcpSessionBinding> {
    Ok(AcpSessionBinding {
        frame_id: row.try_get("frame_id")?,
        agent_profile_id: row.try_get("agent_profile_id")?,
        profile_fingerprint: row.try_get("profile_fingerprint")?,
        agent_session_id: row.try_get("agent_session_id")?,
        cwd: row.try_get("cwd")?,
        protocol_version: row.try_get("protocol_version")?,
        agent_info_json: row.try_get("agent_info_json")?,
        capabilities_json: row.try_get("capabilities_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

impl Store {
    /// A user's choice before ACP has returned a remote session handle. This
    /// is separate from a binding: it must not pretend the agent connected.
    pub async fn frame_acp_agent_selection(&self, frame_id: &str) -> Result<Option<String>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.frame_acp_agent_selection(frame_id)).await;
        }
        if !Self::has_column(&self.pool, "frames", "acp_agent_selection").await? {
            return Ok(None);
        }
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT acp_agent_selection FROM frames WHERE id=?",
        )
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    /// Only an empty root conversation can acquire a choice. Repeating the
    /// same choice is idempotent; switching engines requires a new session.
    pub async fn set_frame_acp_agent_selection(
        &self,
        frame_id: &str,
        project_id: &str,
        agent: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            !agent.trim().is_empty() && agent == agent.trim(),
            "ACP profile is required"
        );
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.set_frame_acp_agent_selection(frame_id, project_id, agent))
                .await;
        }
        let changed = sqlx::query("UPDATE frames SET acp_agent_selection=? WHERE id=? AND project_id=? AND parent_frame_id=id AND exploration_id IS NULL AND (acp_agent_selection IS NULL OR acp_agent_selection=?) AND NOT EXISTS(SELECT 1 FROM messages WHERE frame_id=frames.id) AND NOT EXISTS(SELECT 1 FROM acp_sessions WHERE frame_id=frames.id)")
            .bind(agent).bind(frame_id).bind(project_id).bind(agent).execute(&self.pool).await?;
        anyhow::ensure!(
            changed.rows_affected() == 1,
            "ACP selection requires an empty conversation in this project"
        );
        Ok(())
    }

    pub async fn save_acp_session(&self, binding: &AcpSessionBinding) -> Result<()> {
        if let Some(store) = self.route_entity("frames", "id", &binding.frame_id).await? {
            return Box::pin(store.save_acp_session(binding)).await;
        }
        let mut tx = self.begin_write().await?;
        let selected = sqlx::query_scalar::<_, Option<String>>(
            "SELECT acp_agent_selection FROM frames WHERE id=?",
        )
        .bind(&binding.frame_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        anyhow::ensure!(
            selected
                .as_deref()
                .is_none_or(|id| id == binding.agent_profile_id),
            "ACP binding does not match the saved agent selection"
        );
        sqlx::query(
            "INSERT INTO acp_sessions(\
             frame_id,agent_profile_id,profile_fingerprint,agent_session_id,cwd,\
             protocol_version,agent_info_json,capabilities_json,created_at,updated_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(frame_id) DO UPDATE SET \
             agent_profile_id=excluded.agent_profile_id, \
             profile_fingerprint=excluded.profile_fingerprint, \
             agent_session_id=excluded.agent_session_id, cwd=excluded.cwd, \
             protocol_version=excluded.protocol_version, \
             agent_info_json=excluded.agent_info_json, \
             capabilities_json=excluded.capabilities_json, \
             updated_at=excluded.updated_at",
        )
        .bind(&binding.frame_id)
        .bind(&binding.agent_profile_id)
        .bind(&binding.profile_fingerprint)
        .bind(&binding.agent_session_id)
        .bind(&binding.cwd)
        .bind(binding.protocol_version)
        .bind(&binding.agent_info_json)
        .bind(&binding.capabilities_json)
        .bind(binding.created_at)
        .bind(binding.updated_at)
        .execute(&mut *tx)
        .await?;
        // Commit the connection and retire the provisional choice together.
        sqlx::query("UPDATE frames SET acp_agent_selection=NULL WHERE id=?")
            .bind(&binding.frame_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_acp_session(&self, frame_id: &str) -> Result<Option<AcpSessionBinding>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.get_acp_session(frame_id)).await;
        }
        let row = sqlx::query(
            "SELECT frame_id,agent_profile_id,profile_fingerprint,agent_session_id,cwd,\
             protocol_version,agent_info_json,capabilities_json,created_at,updated_at \
             FROM acp_sessions WHERE frame_id=?",
        )
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(from_row).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ACP_SESSIONS_MIGRATION;

    async fn store_with_frames() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_acp_sessions_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("p", "Project", "/workspace")
            .await
            .unwrap();
        for frame_id in ["f1", "f2"] {
            store
                .create_frame(frame_id, "p", "OPERON", "acp")
                .await
                .unwrap();
        }
        (store, path)
    }

    fn binding(frame_id: &str, session_id: &str) -> AcpSessionBinding {
        AcpSessionBinding {
            frame_id: frame_id.into(),
            agent_profile_id: "agent-profile".into(),
            profile_fingerprint: "sha256:test".into(),
            agent_session_id: session_id.into(),
            cwd: "/workspace".into(),
            protocol_version: 1,
            agent_info_json: r#"{"name":"fake-agent"}"#.into(),
            capabilities_json: r#"{"resumeSession":true}"#.into(),
            created_at: 10,
            updated_at: 10,
        }
    }

    #[tokio::test]
    async fn pending_choice_survives_restart_without_faking_a_binding_or_user_turn() {
        let (store, path) = store_with_frames().await;
        store
            .set_frame_acp_agent_selection("f1", "p", "agent-profile")
            .await
            .unwrap();
        store
            .set_frame_acp_agent_selection("f1", "p", "agent-profile")
            .await
            .unwrap();
        assert!(store
            .set_frame_acp_agent_selection("f1", "other", "agent-profile")
            .await
            .is_err());
        assert!(store
            .set_frame_acp_agent_selection("f1", "p", "changed")
            .await
            .is_err());
        assert!(store
            .set_frame_acp_agent_selection("missing", "p", "agent-profile")
            .await
            .is_err());
        assert!(store
            .set_frame_acp_agent_selection("f2", "p", " ")
            .await
            .is_err());
        drop(store);
        let store = Store::open(&path).await.unwrap();
        assert_eq!(
            store
                .frame_acp_agent_selection("f1")
                .await
                .unwrap()
                .as_deref(),
            Some("agent-profile")
        );
        assert!(store.get_acp_session("f1").await.unwrap().is_none());
        assert!(store.load_messages("f1").await.unwrap().is_empty());
        let sessions = store.list_sessions("p").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].0, "f1");
        assert_eq!(store.list_projects().await.unwrap()[0].5, 1);
        assert!(store
            .list_recent_sessions_detail(5)
            .await
            .unwrap()
            .is_empty());
        store.set_session_pinned("f1", "p", true).await.unwrap();
        assert_eq!(store.list_pinned_sessions("p").await.unwrap()[0].0, "f1");
        store
            .append_message("f2", 1, &wisp_llm::Message::user("existing HTTP history"))
            .await
            .unwrap();
        assert!(store
            .set_frame_acp_agent_selection("f2", "p", "agent-profile")
            .await
            .is_err());
        store.delete_session("f1", "p").await.unwrap();
        assert!(store
            .frame_acp_agent_selection("f1")
            .await
            .unwrap()
            .is_none());
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn confirmed_binding_retires_choice_only_after_the_transaction_succeeds() {
        let (store, path) = store_with_frames().await;
        store
            .set_frame_acp_agent_selection("f1", "p", "agent-profile")
            .await
            .unwrap();
        store
            .set_frame_acp_agent_selection("f2", "p", "agent-profile")
            .await
            .unwrap();
        let mut conflicting = binding("f1", "wrong-profile");
        conflicting.agent_profile_id = "different-agent".into();
        assert!(store.save_acp_session(&conflicting).await.is_err());
        assert!(store.get_acp_session("f1").await.unwrap().is_none());
        assert_eq!(
            store
                .frame_acp_agent_selection("f1")
                .await
                .unwrap()
                .as_deref(),
            Some("agent-profile")
        );
        store
            .save_acp_session(&binding("f1", "connected"))
            .await
            .unwrap();
        assert!(store
            .frame_acp_agent_selection("f1")
            .await
            .unwrap()
            .is_none());
        // A crash after connecting but before the first user message must
        // not hide the now-bound empty conversation from the sidebar.
        assert_eq!(store.list_sessions("p").await.unwrap().len(), 2);

        assert!(store
            .save_acp_session(&binding("f2", "connected"))
            .await
            .is_err());
        assert_eq!(
            store
                .frame_acp_agent_selection("f2")
                .await
                .unwrap()
                .as_deref(),
            Some("agent-profile")
        );
        assert!(store.get_acp_session("f2").await.unwrap().is_none());
        assert!(store
            .set_frame_acp_agent_selection("f1", "p", "agent-profile")
            .await
            .is_err());
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn old_read_only_database_keeps_history_without_migrating_and_writable_open_repairs_column(
    ) {
        let (store, path) = store_with_frames().await;
        store
            .append_message("f1", 1, &wisp_llm::Message::user("saved history"))
            .await
            .unwrap();
        sqlx::query("ALTER TABLE frames DROP COLUMN acp_agent_selection")
            .execute(&store.pool)
            .await
            .unwrap();
        drop(store);
        let reader = Store::open_read_only(&path).await.unwrap();
        assert!(reader
            .frame_acp_agent_selection("f1")
            .await
            .unwrap()
            .is_none());
        assert_eq!(reader.list_sessions("p").await.unwrap().len(), 1);
        assert_eq!(reader.list_projects().await.unwrap()[0].5, 1);
        assert!(
            !Store::has_column(&reader.pool, "frames", "acp_agent_selection")
                .await
                .unwrap()
        );
        drop(reader);
        let upgraded = Store::open(&path).await.unwrap();
        upgraded
            .set_frame_acp_agent_selection("f2", "p", "agent-profile")
            .await
            .unwrap();
        drop(upgraded);
        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(
            reopened
                .frame_acp_agent_selection("f2")
                .await
                .unwrap()
                .as_deref(),
            Some("agent-profile")
        );
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn pending_choice_follows_session_copy_and_project_snapshot_roundtrip() {
        let (store, path) = store_with_frames().await;
        let temporary = tempfile::tempdir().unwrap();
        store
            .create_project("target", "Target", "/target")
            .await
            .unwrap();
        store
            .set_frame_acp_agent_selection("f1", "p", "agent-profile")
            .await
            .unwrap();
        store
            .copy_session_to_project("f1", "p", "target", "copied")
            .await
            .unwrap();
        assert_eq!(
            store
                .frame_acp_agent_selection("copied")
                .await
                .unwrap()
                .as_deref(),
            Some("agent-profile")
        );
        assert!(store.get_acp_session("copied").await.unwrap().is_none());
        let snapshot = temporary.path().join("export.sqlite");
        store.export_project_database("p", &snapshot).await.unwrap();
        let imported = Store::open(&temporary.path().join("import.sqlite"))
            .await
            .unwrap();
        imported
            .import_project_database(&snapshot, "p", &temporary.path().join("workspace"))
            .await
            .unwrap();
        assert_eq!(
            imported
                .frame_acp_agent_selection("f1")
                .await
                .unwrap()
                .as_deref(),
            Some("agent-profile")
        );
        assert_eq!(imported.list_sessions("p").await.unwrap().len(), 1);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn pending_choice_routes_to_workspace_storage_and_cross_project_copy() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("application.sqlite");
        let store = Store::open_application(&path).await.unwrap();
        for project in ["p", "q"] {
            let workspace = temporary.path().join(project);
            std::fs::create_dir(&workspace).unwrap();
            store
                .create_project(project, project, workspace.to_str().unwrap())
                .await
                .unwrap();
        }
        store
            .create_frame("draft", "p", "OPERON", "http-default")
            .await
            .unwrap();
        store
            .set_frame_acp_agent_selection("draft", "p", "chosen-agent")
            .await
            .unwrap();
        store
            .copy_session_to_project("draft", "p", "q", "copy")
            .await
            .unwrap();
        drop(store);
        let reopened = Store::open_application(&path).await.unwrap();
        for id in ["draft", "copy"] {
            assert_eq!(
                reopened
                    .frame_acp_agent_selection(id)
                    .await
                    .unwrap()
                    .as_deref(),
                Some("chosen-agent")
            );
            assert!(reopened.get_acp_session(id).await.unwrap().is_none());
        }
        assert!(reopened
            .set_frame_acp_agent_selection("draft", "q", "chosen-agent")
            .await
            .is_err());
        assert_eq!(
            reopened
                .search_sessions(Some("p"), "", 10, None, None)
                .await
                .unwrap()[0]
                .id,
            "draft"
        );
        assert_eq!(reopened.list_sessions("q").await.unwrap()[0].0, "copy");
        reopened
            .move_session_to_project("copy", "q", "p", "moved")
            .await
            .unwrap();
        assert!(reopened
            .frame_acp_agent_selection("copy")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            reopened
                .frame_acp_agent_selection("moved")
                .await
                .unwrap()
                .as_deref(),
            Some("chosen-agent")
        );
        assert!(reopened.list_sessions("q").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn acp_session_round_trips_and_updates() {
        let (store, path) = store_with_frames().await;
        let original = binding("f1", "session-1");
        store.save_acp_session(&original).await.unwrap();
        assert_eq!(
            store.get_acp_session("f1").await.unwrap(),
            Some(original.clone())
        );

        let mut updated = original;
        updated.cwd = "/workspace/renamed".into();
        updated.capabilities_json = r#"{"loadSession":true}"#.into();
        updated.created_at = 999;
        updated.updated_at = 20;
        store.save_acp_session(&updated).await.unwrap();
        let loaded = store.get_acp_session("f1").await.unwrap().unwrap();
        assert_eq!(loaded.cwd, "/workspace/renamed");
        assert_eq!(loaded.capabilities_json, r#"{"loadSession":true}"#);
        assert_eq!(loaded.created_at, 10);
        assert_eq!(loaded.updated_at, 20);

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn acp_session_enforces_frame_and_agent_session_identity() {
        let (store, path) = store_with_frames().await;
        store
            .save_acp_session(&binding("f1", "session-1"))
            .await
            .unwrap();
        assert!(store
            .save_acp_session(&binding("f2", "session-1"))
            .await
            .is_err());
        assert!(store
            .save_acp_session(&binding("missing", "session-2"))
            .await
            .is_err());

        store.delete_session("f1", "p").await.unwrap();
        assert!(store.get_acp_session("f1").await.unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn acp_session_migration_is_idempotent() {
        let (store, path) = store_with_frames().await;
        let original = binding("f1", "session-1");
        store.save_acp_session(&original).await.unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(ACP_SESSIONS_MIGRATION)
            .execute(&store.pool)
            .await
            .unwrap();
        drop(store);

        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(
            reopened.get_acp_session("f1").await.unwrap(),
            Some(original)
        );
        assert!(reopened
            .schema_migrations()
            .await
            .unwrap()
            .contains(&ACP_SESSIONS_MIGRATION.to_string()));
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
