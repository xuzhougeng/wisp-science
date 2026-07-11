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
    pub async fn save_acp_session(&self, binding: &AcpSessionBinding) -> Result<()> {
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_acp_session(&self, frame_id: &str) -> Result<Option<AcpSessionBinding>> {
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

    pub async fn delete_acp_session(&self, frame_id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM acp_sessions WHERE frame_id=?")
            .bind(frame_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
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

        assert!(store.delete_acp_session("f1").await.unwrap());
        assert!(!store.delete_acp_session("f1").await.unwrap());
        assert!(store.get_acp_session("f1").await.unwrap().is_none());
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
