use super::{execution_context_from_row, ExecutionContext, ExecutionContextKind, Store};
use anyhow::Result;

/// Per-conversation default analysis environment, stored in `settings`.
/// Missing key = follow the live global default; `local` = pin this chat to
/// this machine; any other value is a pinned remote context id.
pub const FRAME_DEFAULT_EXECUTION_CONTEXT_PREFIX: &str = "frame_default_execution_context:";

pub fn frame_default_execution_context_key(frame_id: &str) -> String {
    format!("{FRAME_DEFAULT_EXECUTION_CONTEXT_PREFIX}{frame_id}")
}

impl Store {
    /// Opening the database must preserve the user's Local configuration.
    pub(crate) async fn ensure_local_execution_context(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT OR IGNORE INTO execution_contexts(id,kind,label,config_json,capabilities_json,created_at,updated_at) VALUES('local','local','Local','{}','{}',?,?)")
            .bind(now).bind(now).execute(&self.pool).await?;
        Ok(())
    }

    /// Fill only unset local tool paths. Serialize with other database writes
    /// so a delayed detector cannot overwrite a user's saved interpreter.
    pub async fn save_detected_local_paths(
        &self,
        paths: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let raw: String =
            sqlx::query_scalar("SELECT config_json FROM execution_contexts WHERE id='local'")
                .fetch_one(&mut *tx)
                .await?;
        let mut config: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&raw)?;
        let mut changed = false;
        for (key, path) in paths {
            let legacy = match key.as_str() {
                "python_executable" => "python_path",
                "rscript_executable" => "rscript_path",
                _ => key,
            };
            let configured = [key.as_str(), legacy].iter().any(|name| {
                config
                    .get(*name)
                    .and_then(|value| value.as_str())
                    .is_some_and(|value| !value.trim().is_empty())
            });
            if !configured && !path.trim().is_empty() {
                config.insert(key.clone(), serde_json::Value::String(path.clone()));
                changed = true;
            }
        }
        if changed {
            sqlx::query(
                "UPDATE execution_contexts SET config_json=?,updated_at=? WHERE id='local'",
            )
            .bind(serde_json::to_string(&config)?)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn upsert_execution_context(&self, ctx: &ExecutionContext) -> Result<()> {
        ctx.validate()?;
        sqlx::query(
            "INSERT INTO execution_contexts(\
                id,kind,label,config_json,capabilities_json,last_probe_at,last_probe_status,last_probe_error,created_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
                kind=excluded.kind, label=excluded.label, config_json=excluded.config_json, \
                capabilities_json=excluded.capabilities_json, last_probe_at=excluded.last_probe_at, \
                last_probe_status=excluded.last_probe_status, last_probe_error=excluded.last_probe_error, \
                updated_at=excluded.updated_at",
        )
        .bind(&ctx.id)
        .bind(ctx.kind.as_str())
        .bind(&ctx.label)
        .bind(&ctx.config_json)
        .bind(&ctx.capabilities_json)
        .bind(ctx.last_probe_at)
        .bind(ctx.last_probe_status.as_deref())
        .bind(ctx.last_probe_error.as_deref())
        .bind(ctx.created_at)
        .bind(ctx.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_execution_context(&self, id: &str) -> Result<Option<ExecutionContext>> {
        ExecutionContextKind::from_id(id)?;
        let row = sqlx::query(
            "SELECT id,kind,label,config_json,capabilities_json,last_probe_at,last_probe_status,last_probe_error,created_at,updated_at \
             FROM execution_contexts WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(execution_context_from_row).transpose()
    }

    pub async fn list_execution_contexts(&self) -> Result<Vec<ExecutionContext>> {
        let rows = sqlx::query(
            "SELECT id,kind,label,config_json,capabilities_json,last_probe_at,last_probe_status,last_probe_error,created_at,updated_at \
             FROM execution_contexts ORDER BY CASE id WHEN 'local' THEN 0 ELSE 1 END, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(execution_context_from_row).collect()
    }

    pub async fn delete_execution_context(&self, id: &str) -> Result<()> {
        ExecutionContextKind::from_id(id)?;
        sqlx::query("DELETE FROM session_execution_contexts WHERE context_id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM execution_contexts WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_session_execution_context_enabled(
        &self,
        frame_id: &str,
        context_id: &str,
        enabled: bool,
    ) -> Result<()> {
        let context = self
            .get_execution_context(context_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Execution context not found: {context_id}"))?;
        if context.kind == ExecutionContextKind::Local {
            anyhow::bail!("Local compute is always available");
        }
        if self.frame_project_id(frame_id).await?.is_none() {
            anyhow::bail!("Session not found: {frame_id}");
        }
        if enabled {
            sqlx::query(
                "INSERT OR IGNORE INTO session_execution_contexts(frame_id,context_id,created_at) \
                 VALUES(?,?,?)",
            )
            .bind(frame_id)
            .bind(context_id)
            .bind(chrono::Utc::now().timestamp())
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query("DELETE FROM session_execution_contexts WHERE frame_id=? AND context_id=?")
                .bind(frame_id)
                .bind(context_id)
                .execute(&self.pool)
                .await?;
            // Detaching the conversation's pinned default clears it so omit
            // falls back to the live global default, without rewriting global.
            let key = frame_default_execution_context_key(frame_id);
            if self.get_setting(&key).await?.as_deref() == Some(context_id) {
                self.delete_setting(&key).await?;
            }
        }
        Ok(())
    }

    /// Stored snapshot for this conversation: `None` follows the global
    /// default, `Some("local")` pins local, any other id pins that remote.
    pub async fn session_default_execution_context(
        &self,
        frame_id: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .get_setting(&frame_default_execution_context_key(frame_id))
            .await?
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()))
    }

    pub async fn set_session_default_execution_context(
        &self,
        frame_id: &str,
        value: Option<&str>,
    ) -> Result<()> {
        let key = frame_default_execution_context_key(frame_id);
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => self.set_setting(&key, value).await,
            None => self.delete_setting(&key).await,
        }
    }

    pub async fn list_session_execution_context_ids(&self, frame_id: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT context_id FROM session_execution_contexts \
             WHERE frame_id=? ORDER BY context_id",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn session_execution_context_enabled(
        &self,
        frame_id: &str,
        context_id: &str,
    ) -> Result<bool> {
        let row: (i64,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM session_execution_contexts \
             WHERE frame_id=? AND context_id=?)",
        )
        .bind(frame_id)
        .bind(context_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0 != 0)
    }
}

#[cfg(test)]
mod local_detection_tests {
    use super::*;

    #[tokio::test]
    async fn detected_paths_fill_blanks_preserve_manual_settings_and_survive_reopen() {
        let root = std::env::temp_dir().join(format!("wisp-local-paths-{}", uuid::Uuid::new_v4()));
        let db = root.join("store.db");
        let store = Store::open(&db).await.unwrap();
        let mut local = store.get_execution_context("local").await.unwrap().unwrap();
        local.label = "My computer".into();
        local.config_json = serde_json::json!({
            "python_path": "C:\\Custom Python\\python.exe",
            "rscript_executable": "", "unrelated": true,
        })
        .to_string();
        local.capabilities_json = r#"{"cpu_count":8}"#.into();
        store.upsert_execution_context(&local).await.unwrap();
        let paths = [
            ("python_executable".into(), "/discovered/python".into()),
            ("rscript_executable".into(), "/discovered/Rscript".into()),
            ("uv_executable".into(), "/discovered/uv".into()),
            ("node_executable".into(), "/discovered/node".into()),
        ]
        .into();
        store.save_detected_local_paths(&paths).await.unwrap();
        let mut detected = store.get_execution_context("local").await.unwrap().unwrap();
        detected.updated_at = 123;
        store.upsert_execution_context(&detected).await.unwrap();
        store.save_detected_local_paths(&paths).await.unwrap();
        store
            .save_detected_local_paths(&Default::default())
            .await
            .unwrap();
        let reopened = Store::open(&db).await.unwrap();
        let saved = reopened
            .get_execution_context("local")
            .await
            .unwrap()
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(&saved.config_json).unwrap();
        assert_eq!(saved.label, "My computer");
        assert_eq!(saved.updated_at, 123, "repeat detection must be a no-op");
        assert_eq!(saved.capabilities_json, local.capabilities_json);
        assert_eq!(config["python_path"], r"C:\Custom Python\python.exe");
        assert!(config.get("python_executable").is_none());
        assert_eq!(config["rscript_executable"], "/discovered/Rscript");
        assert_eq!(config["uv_executable"], "/discovered/uv");
        assert_eq!(config["node_executable"], "/discovered/node");
        assert_eq!(config["unrelated"], true);
        drop(reopened);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
