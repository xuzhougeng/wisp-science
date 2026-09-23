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
        if let Some(store) = self.route_global() {
            return Box::pin(store.ensure_local_execution_context()).await;
        }
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
        if let Some(store) = self.route_global() {
            return Box::pin(store.save_detected_local_paths(paths)).await;
        }
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

    /// Apply explicit edits atomically without changing unrelated Local settings.
    /// Empty values remove overrides so subsequent detection can fill them again.
    pub async fn save_local_environment_paths(
        &self,
        paths: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.save_local_environment_paths(paths)).await;
        }
        for (key, value) in paths {
            anyhow::ensure!(
                matches!(
                    key.as_str(),
                    "python_executable"
                        | "rscript_executable"
                        | "uv_executable"
                        | "node_executable"
                        | "npm_executable"
                        | "sci_executable"
                        | "pixi_executable"
                ),
                "Unknown local tool path: {key}"
            );
            anyhow::ensure!(
                !value.chars().any(char::is_control),
                "Invalid local tool path: {key}"
            );
        }
        let mut tx = self.begin_write().await?;
        let raw: String =
            sqlx::query_scalar("SELECT config_json FROM execution_contexts WHERE id='local'")
                .fetch_one(&mut *tx)
                .await?;
        let mut config: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&raw)?;
        for (key, value) in paths {
            match key.as_str() {
                "python_executable" => {
                    config.remove("python_path");
                }
                "rscript_executable" => {
                    config.remove("rscript_path");
                }
                _ => {}
            }
            let value = value.trim();
            if value.is_empty() {
                config.remove(key);
            } else {
                config.insert(key.clone(), value.into());
            }
        }
        sqlx::query("UPDATE execution_contexts SET config_json=?,updated_at=? WHERE id='local'")
            .bind(serde_json::to_string(&config)?)
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn upsert_execution_context(&self, ctx: &ExecutionContext) -> Result<()> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.upsert_execution_context(ctx)).await;
        }
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
        if let Some(store) = self.route_global() {
            return Box::pin(store.get_execution_context(id)).await;
        }
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
        if let Some(store) = self.route_global() {
            return Box::pin(store.list_execution_contexts()).await;
        }
        let rows = sqlx::query(
            "SELECT id,kind,label,config_json,capabilities_json,last_probe_at,last_probe_status,last_probe_error,created_at,updated_at \
             FROM execution_contexts ORDER BY CASE id WHEN 'local' THEN 0 ELSE 1 END, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(execution_context_from_row).collect()
    }

    pub async fn delete_execution_context(&self, id: &str) -> Result<()> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.delete_execution_context(id)).await;
        }
        ExecutionContextKind::from_id(id)?;
        if let Some(stores) = self.routed_projects().await? {
            for store in stores {
                sqlx::query("DELETE FROM session_execution_contexts WHERE context_id=?")
                    .bind(id)
                    .execute(&store.pool)
                    .await?;
                sqlx::query("DELETE FROM settings WHERE key LIKE 'frame_default_execution_context:%' AND value=?")
                    .bind(id).execute(&store.pool).await?;
            }
        }
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
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(
                store.set_session_execution_context_enabled(frame_id, context_id, enabled),
            )
            .await;
        }
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
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.session_default_execution_context(frame_id)).await;
        }
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
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.set_session_default_execution_context(frame_id, value)).await;
        }
        let key = frame_default_execution_context_key(frame_id);
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => self.set_setting(&key, value).await,
            None => self.delete_setting(&key).await,
        }
    }

    pub async fn list_session_execution_context_ids(&self, frame_id: &str) -> Result<Vec<String>> {
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.list_session_execution_context_ids(frame_id)).await;
        }
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
        if let Some(store) = self.route_entity("frames", "id", frame_id).await? {
            return Box::pin(store.session_execution_context_enabled(frame_id, context_id)).await;
        }
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
    async fn manual_paths_replace_overrides_atomically_and_survive_detection_and_reopen() {
        let root = std::env::temp_dir().join(format!("wisp-manual-paths-{}", uuid::Uuid::new_v4()));
        let db = root.join("store.db");
        let store = Store::open(&db).await.unwrap();
        let mut local = store.get_execution_context("local").await.unwrap().unwrap();
        local.config_json = serde_json::json!({
            "python_path": "/old/python", "rscript_path": "/old/Rscript",
            "node_executable": "/keep/node", "unrelated": true,
        })
        .to_string();
        local.capabilities_json = r#"{"cpu_count":8}"#.into();
        store.upsert_execution_context(&local).await.unwrap();
        let edits = [
            (
                "python_executable".into(),
                r"  C:\Custom Python\python.exe  ".into(),
            ),
            ("rscript_executable".into(), "".into()),
            ("uv_executable".into(), "/custom/uv".into()),
            ("npm_executable".into(), r"C:\Node\npm.cmd".into()),
            ("sci_executable".into(), "/custom/sci".into()),
            ("pixi_executable".into(), "/custom/pixi".into()),
        ]
        .into();
        store.save_local_environment_paths(&edits).await.unwrap();
        let saved = store.get_execution_context("local").await.unwrap().unwrap();
        for bad in ["unknown", "python_executable"] {
            let invalid = [
                ("node_executable".into(), "/should-not-save".into()),
                (bad.into(), "invalid\npath".into()),
            ]
            .into();
            assert!(store.save_local_environment_paths(&invalid).await.is_err());
            assert_eq!(
                store
                    .get_execution_context("local")
                    .await
                    .unwrap()
                    .unwrap()
                    .config_json,
                saved.config_json
            );
        }
        store
            .save_detected_local_paths(
                &[
                    ("python_executable".into(), "/detected/python".into()),
                    ("rscript_executable".into(), "/detected/Rscript".into()),
                ]
                .into(),
            )
            .await
            .unwrap();
        let reopened = Store::open(&db).await.unwrap();
        let result = reopened
            .get_execution_context("local")
            .await
            .unwrap()
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(&result.config_json).unwrap();
        assert_eq!(config["python_executable"], r"C:\Custom Python\python.exe");
        assert_eq!(config["rscript_executable"], "/detected/Rscript");
        assert_eq!(config["node_executable"], "/keep/node");
        assert_eq!(config["npm_executable"], r"C:\Node\npm.cmd");
        assert_eq!(config["uv_executable"], "/custom/uv");
        assert_eq!(config["sci_executable"], "/custom/sci");
        assert_eq!(config["pixi_executable"], "/custom/pixi");
        assert_eq!(config["unrelated"], true);
        assert!(config.get("python_path").is_none());
        assert!(config.get("rscript_path").is_none());
        assert_eq!(result.capabilities_json, local.capabilities_json);
        drop(reopened);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

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
