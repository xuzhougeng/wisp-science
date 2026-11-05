use crate::Store;
use anyhow::Result;
use wisp_dto::project_browser::PrivacyMode;

const ACTIVE: &str = "wisp-privacy-mode-active";
const PROJECTS: &str = "wisp-privacy-mode-projects";

fn normalize(mut mode: PrivacyMode) -> PrivacyMode {
    mode.project_ids = mode
        .project_ids
        .into_iter()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    mode.project_ids.sort_unstable();
    mode.project_ids.dedup();
    mode.active &= !mode.project_ids.is_empty();
    mode
}

impl Store {
    /// Read both halves in the same SQLite snapshot, including in read-only browsers.
    pub async fn project_privacy(&self) -> Result<PrivacyMode> {
        let global = self.route_global();
        let store = global.as_ref().unwrap_or(self);
        let (active, ids): (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT (SELECT value FROM settings WHERE key=?),(SELECT value FROM settings WHERE key=?)"
        ).bind(ACTIVE).bind(PROJECTS).fetch_one(&store.pool).await?;
        Ok(normalize(PrivacyMode {
            active: active.as_deref() == Some("1"),
            project_ids: serde_json::from_str(ids.as_deref().unwrap_or("[]"))?,
        }))
    }

    /// A legacy WebView can seed an empty store once; it must never replace a
    /// selection another client already saved (including a restored selection).
    pub async fn write_project_privacy(&self, mode: PrivacyMode, seed_only: bool) -> Result<()> {
        let global = self.route_global();
        let store = global.as_ref().unwrap_or(self);
        let mut tx = store.begin_write().await?;
        let mode = normalize(mode);
        let insert = sqlx::query(if seed_only {
            "INSERT OR IGNORE INTO settings(key,value) VALUES(?,?)"
        } else {
            "INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value"
        }).bind(PROJECTS).bind(serde_json::to_string(&mode.project_ids)?).execute(&mut *tx).await?;
        if !seed_only || insert.rows_affected() != 0 {
            if mode.active {
                sqlx::query("INSERT INTO settings(key,value) VALUES(?, '1') ON CONFLICT(key) DO UPDATE SET value='1'")
                    .bind(ACTIVE).execute(&mut *tx).await?;
            } else {
                sqlx::query("DELETE FROM settings WHERE key=?")
                    .bind(ACTIVE)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
}
