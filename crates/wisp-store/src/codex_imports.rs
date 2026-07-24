//! Mapping between imported Codex CLI rollouts and Wisp frames (#464).
//! Keyed by the Codex thread id so re-imports are idempotent. Deliberately
//! not `acp_sessions`: that binding routes a frame's sends through the ACP
//! runtime and blocks rewind, which must not happen to imported transcripts.

use super::Store;
use anyhow::Result;

impl Store {
    /// The frame a Codex rollout was already imported into, if any.
    pub async fn find_codex_import(&self, codex_session_id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT frame_id FROM codex_imports WHERE codex_session_id=?")
                .bind(codex_session_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Record (or refresh) the rollout → frame mapping after an import.
    pub async fn record_codex_import(
        &self,
        codex_session_id: &str,
        frame_id: &str,
        source_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO codex_imports(codex_session_id,frame_id,source_path,created_at,updated_at) \
             VALUES(?,?,?,?,?) \
             ON CONFLICT(codex_session_id) DO UPDATE SET \
             frame_id=excluded.frame_id, source_path=excluded.source_path, \
             updated_at=excluded.updated_at",
        )
        .bind(codex_session_id)
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
    use crate::CODEX_IMPORTS_MIGRATION;

    async fn store_with_frame() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_codex_imports_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("p", "Project", "/workspace")
            .await
            .unwrap();
        store.create_frame("f1", "p", "Codex", "m").await.unwrap();
        (store, path)
    }

    #[tokio::test]
    async fn codex_import_round_trips_and_cascades_on_delete() {
        let (store, path) = store_with_frame().await;
        assert_eq!(store.find_codex_import("codex-1").await.unwrap(), None);
        store
            .record_codex_import("codex-1", "f1", "/tmp/rollout.jsonl")
            .await
            .unwrap();
        assert_eq!(
            store.find_codex_import("codex-1").await.unwrap(),
            Some("f1".to_string())
        );
        // Upsert keeps a single row per Codex thread.
        store
            .record_codex_import("codex-1", "f1", "/tmp/rollout2.jsonl")
            .await
            .unwrap();
        assert_eq!(
            store.find_codex_import("codex-1").await.unwrap(),
            Some("f1".to_string())
        );

        // Deleting the Wisp session frees the Codex id for re-import.
        store.delete_session("f1", "p").await.unwrap();
        assert_eq!(store.find_codex_import("codex-1").await.unwrap(), None);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn codex_imports_migration_is_idempotent() {
        let (store, path) = store_with_frame().await;
        store
            .record_codex_import("codex-1", "f1", "/tmp/rollout.jsonl")
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(CODEX_IMPORTS_MIGRATION)
            .execute(&store.pool)
            .await
            .unwrap();
        drop(store);

        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(
            reopened.find_codex_import("codex-1").await.unwrap(),
            Some("f1".to_string())
        );
        assert!(reopened
            .schema_migrations()
            .await
            .unwrap()
            .contains(&CODEX_IMPORTS_MIGRATION.to_string()));
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
