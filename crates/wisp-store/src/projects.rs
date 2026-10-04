use super::Store;
use anyhow::Result;
use sqlx::Row;

pub use wisp_dto::ASSISTANT_PROJECT_ID;

pub fn is_assistant_project_id(id: &str) -> bool {
    id == ASSISTANT_PROJECT_ID
}

impl Store {
    pub async fn create_project(&self, id: &str, name: &str, workspace_dir: &str) -> Result<()> {
        if let Some(store) = self.route_project(id).await? {
            return Box::pin(store.create_project(id, name, workspace_dir)).await;
        }
        if self.registry.is_some() && self.project_scope.is_none() {
            std::fs::create_dir_all(workspace_dir)?;
            anyhow::ensure!(
                !std::path::Path::new(workspace_dir)
                    .join(super::PROJECT_METADATA)
                    .exists(),
                "This is an existing project folder; import it instead"
            );
        }
        let decentralized = self.decentralized_project_storage().await?;
        let now = chrono::Utc::now().timestamp();
        let mut tx = self.begin_write().await?;
        let existing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?)")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) VALUES(?,?,'',?,?,?) \
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, workspace_dir=excluded.workspace_dir, updated_at=excluded.updated_at",
        )
        .bind(id).bind(name).bind(workspace_dir).bind(now).bind(now)
        .execute(&mut *tx).await?;
        if !existing && self.registry.is_some() && self.project_scope.is_none() && decentralized {
            Box::pin(self.prepare_project_storage(id, Some(tx))).await?;
        } else {
            // Desktop startup upserts the default project. Updating a legacy
            // registration must not implicitly copy the entire database.
            tx.commit().await?;
        }
        Ok(())
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<(String, String)>> {
        if let Some(store) = self.route_project(id).await? {
            return Box::pin(store.get_project(id)).await;
        }
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT COALESCE(name,''), COALESCE(workspace_dir,'') FROM projects WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Full editable metadata for the Project Settings modal: (name, description, workspace_dir).
    pub async fn get_project_meta(&self, id: &str) -> Result<Option<(String, String, String)>> {
        if let Some(store) = self.route_project(id).await? {
            return Box::pin(store.get_project_meta(id)).await;
        }
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT COALESCE(name,''), COALESCE(description,''), COALESCE(workspace_dir,'') FROM projects WHERE id=?",
        )
        .bind(id).fetch_optional(&self.pool).await?;
        Ok(row)
    }

    /// Update a project's user-visible name and description (touches updated_at).
    pub async fn update_project(&self, id: &str, name: &str, description: &str) -> Result<()> {
        if let Some(store) = self.route_project(id).await? {
            return Box::pin(store.update_project(id, name, description)).await;
        }
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE projects SET name=?, description=?, updated_at=? WHERE id=?")
            .bind(name)
            .bind(description)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set a local pin without changing the project's activity timestamp.
    pub async fn set_project_starred(&self, id: &str, starred: bool) -> Result<()> {
        if let Some(store) = self.route_project(id).await? {
            return Box::pin(store.set_project_starred(id, starred)).await;
        }
        anyhow::ensure!(
            Self::has_column(&self.pool, "projects", "starred").await?,
            "This database needs a desktop schema upgrade before project stars can be saved. Open it with the current WebView desktop first."
        );
        let result =
            sqlx::query("UPDATE projects SET starred=? WHERE id=? AND id NOT LIKE 'assistant:%'")
                .bind(starred)
                .bind(id)
                .execute(&self.pool)
                .await?;
        anyhow::ensure!(result.rows_affected() == 1, "Project not found");
        Ok(())
    }

    pub async fn starred_project_ids(&self) -> Result<std::collections::HashSet<String>> {
        if let Some(stores) = self.available_projects().await? {
            let mut result = std::collections::HashSet::new();
            let owned: std::collections::HashSet<_> = stores
                .iter()
                .filter_map(|store| store.project_scope.clone())
                .collect();
            for store in stores {
                let stars = Box::pin(store.starred_project_ids()).await?;
                result.extend(
                    stars
                        .into_iter()
                        .filter(|id| store.project_scope.is_some() || !owned.contains(id)),
                );
            }
            return Ok(result);
        }
        // Read-only native clients may inspect a desktop database from before
        // project stars were introduced. Reading must not force a migration.
        if !Self::has_column(&self.pool, "projects", "starred").await? {
            return Ok(std::collections::HashSet::new());
        }
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM projects WHERE starred=1 AND id NOT LIKE 'assistant:%'",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().collect())
    }

    /// All projects, starred first then newest-updated first, each with its session count
    /// (root frames with a user turn or an explicit title — matches
    /// `list_sessions_page`, #888) and artifact count.
    pub async fn list_projects(
        &self,
    ) -> Result<Vec<(String, String, String, i64, i64, i64, String, i64)>> {
        if let Some(stores) = self.available_projects().await? {
            let mut rows = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for store in stores {
                for row in Box::pin(store.list_projects()).await? {
                    if seen.insert(row.0.clone()) {
                        rows.push(row);
                    }
                }
            }
            let stars = self.starred_project_ids().await?;
            rows.sort_by(|a, b| {
                stars
                    .contains(&b.0)
                    .cmp(&stars.contains(&a.0))
                    .then(b.4.cmp(&a.4))
                    .then(b.0.cmp(&a.0))
            });
            return Ok(rows);
        }
        let starred_order = if Self::has_column(&self.pool, "projects", "starred").await? {
            "p.starred DESC, "
        } else {
            ""
        };
        let sql = format!(
            "SELECT p.id AS id, COALESCE(p.name,'') AS name, COALESCE(p.workspace_dir,'') AS ws, \
                    p.created_at AS created_at, p.updated_at AS updated_at, \
                    COALESCE(p.description,'') AS description, \
                    (SELECT COUNT(*) FROM frames f WHERE f.project_id = p.id AND f.parent_frame_id = f.id \
                       AND f.exploration_id IS NULL \
                       AND {listable}) AS sessions, \
                    (SELECT COUNT(*) FROM artifacts a WHERE a.project_id = p.id \
                       AND a.exploration_id IS NULL) AS artifacts \
             FROM projects p \
             WHERE p.id NOT LIKE 'assistant:%' \
             ORDER BY {starred_order}p.updated_at DESC, p.rowid DESC",
            listable = self.session_listable_sql().await?,
        );
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await?;
        let mut out = vec![];
        for r in rows {
            out.push((
                r.try_get("id")?,
                r.try_get("name")?,
                r.try_get("ws")?,
                r.try_get("created_at")?,
                r.try_get("updated_at")?,
                r.try_get("sessions")?,
                r.try_get("description")?,
                r.try_get("artifacts")?,
            ));
        }
        Ok(out)
    }

    /// Delete a project and everything under it. Explicit child deletes (SQLite
    /// FKs are OFF by default, so declared CASCADE would not fire). Filesystem
    /// is untouched — only DB rows.
    /// ponytail: explicit cascade of known child tables; switch to
    /// `PRAGMA foreign_keys=ON` if more child tables appear.
    pub async fn delete_project(&self, id: &str) -> Result<()> {
        if self.unregister_project_storage(id).await? {
            return Ok(());
        }
        let mut tx = self.begin_write().await?;
        super::project_transfer::delete_project_children(&mut tx, id).await?;
        sqlx::query("DELETE FROM project_sync_state WHERE project_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM projects WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
