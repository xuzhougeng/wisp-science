use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::FromRow)]
pub struct ProjectSyncState {
    pub project_id: String,
    pub transport_kind: String,
    pub transport_location: String,
    pub relay_project_id: String,
    pub base_revision: Option<String>,
    pub base_state_hash: Option<String>,
    pub base_manifest_json: String,
    pub last_synced_at: Option<i64>,
    pub last_direction: Option<String>,
}

impl ProjectSyncState {
    pub fn uninitialized(project_id: &str, transport_kind: &str, transport_location: &str) -> Self {
        Self {
            project_id: project_id.into(),
            transport_kind: transport_kind.into(),
            transport_location: transport_location.into(),
            relay_project_id: project_id.into(),
            base_revision: None,
            base_state_hash: None,
            base_manifest_json: r#"{"version":1,"files":[],"skipped_paths":[]}"#.into(),
            last_synced_at: None,
            last_direction: None,
        }
    }
}

impl Store {
    pub async fn get_project_sync_state(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectSyncState>> {
        Ok(sqlx::query_as(
            "SELECT project_id,transport_kind,transport_location,relay_project_id,base_revision,base_state_hash,\
             base_manifest_json,last_synced_at,last_direction \
             FROM project_sync_state WHERE project_id=?",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn upsert_project_sync_state(&self, state: &ProjectSyncState) -> Result<()> {
        sqlx::query(
            "INSERT INTO project_sync_state(\
               project_id,transport_kind,transport_location,relay_project_id,base_revision,base_state_hash,\
               base_manifest_json,last_synced_at,last_direction) \
             VALUES(?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(project_id) DO UPDATE SET \
               transport_kind=excluded.transport_kind, transport_location=excluded.transport_location, \
               relay_project_id=excluded.relay_project_id, \
               base_revision=excluded.base_revision, base_state_hash=excluded.base_state_hash, \
               base_manifest_json=excluded.base_manifest_json, \
               last_synced_at=excluded.last_synced_at, last_direction=excluded.last_direction",
        )
        .bind(&state.project_id)
        .bind(&state.transport_kind)
        .bind(&state.transport_location)
        .bind(&state.relay_project_id)
        .bind(&state.base_revision)
        .bind(&state.base_state_hash)
        .bind(&state.base_manifest_json)
        .bind(state.last_synced_at)
        .bind(&state.last_direction)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_project_sync_state(&self, project_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM project_sync_state WHERE project_id=?")
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sync_cursor_roundtrip_is_project_local() {
        let path =
            std::env::temp_dir().join(format!("wisp_sync_state_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p1", "One", "/tmp/one").await.unwrap();
        let mut state = ProjectSyncState::uninitialized("p1", "relay", "https://relay.example");
        state.base_revision = Some("r1".into());
        state.base_state_hash = Some("hash".into());
        state.last_direction = Some("push".into());
        store.upsert_project_sync_state(&state).await.unwrap();
        assert_eq!(
            store.get_project_sync_state("p1").await.unwrap(),
            Some(state)
        );
        store.delete_project_sync_state("p1").await.unwrap();
        assert!(store.get_project_sync_state("p1").await.unwrap().is_none());
        store.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn sync_cursor_migration_is_idempotent_on_existing_store() {
        let path = std::env::temp_dir().join(format!(
            "wisp_sync_migration_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        sqlx::query("DROP TABLE project_sync_state")
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version='0010_project_sync_state'")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let reopened = Store::open(&path).await.unwrap();
        reopened
            .create_project("p1", "One", "/tmp/one")
            .await
            .unwrap();
        let state = ProjectSyncState::uninitialized("p1", "folder", "/tmp/sync");
        reopened.upsert_project_sync_state(&state).await.unwrap();
        assert_eq!(
            reopened.get_project_sync_state("p1").await.unwrap(),
            Some(state)
        );
        reopened.pool.close().await;
        let _ = std::fs::remove_file(path);
    }
}
