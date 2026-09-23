use super::{PluginInstallation, ProjectPlugin, Store};
use anyhow::Result;

impl Store {
    pub async fn replace_plugin_installation(&self, plugin: &PluginInstallation) -> Result<()> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.replace_plugin_installation(plugin)).await;
        }
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO plugin_installations(\
                plugin_id,version,display_name,description,author,license,source_uri,\
                install_root,archive_sha256,manifest_json,trust_state,installed_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(plugin_id,version) DO UPDATE SET \
                display_name=excluded.display_name, description=excluded.description, \
                author=excluded.author, license=excluded.license, source_uri=excluded.source_uri, \
                install_root=excluded.install_root, archive_sha256=excluded.archive_sha256, \
                manifest_json=excluded.manifest_json, trust_state=excluded.trust_state, \
                updated_at=excluded.updated_at",
        )
        .bind(&plugin.plugin_id)
        .bind(&plugin.version)
        .bind(&plugin.display_name)
        .bind(&plugin.description)
        .bind(&plugin.author)
        .bind(&plugin.license)
        .bind(&plugin.source_uri)
        .bind(&plugin.install_root)
        .bind(&plugin.archive_sha256)
        .bind(&plugin.manifest_json)
        .bind(&plugin.trust_state)
        .bind(plugin.installed_at)
        .bind(plugin.updated_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE project_plugins SET version=?,updated_at=? WHERE plugin_id=?")
            .bind(&plugin.version)
            .bind(plugin.updated_at)
            .bind(&plugin.plugin_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM plugin_installations WHERE plugin_id=? AND version<>?")
            .bind(&plugin.plugin_id)
            .bind(&plugin.version)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_plugin_installation(
        &self,
        plugin_id: &str,
        version: &str,
    ) -> Result<Option<PluginInstallation>> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.get_plugin_installation(plugin_id, version)).await;
        }
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                i64,
                i64,
            ),
        >(
            "SELECT plugin_id,version,display_name,description,author,license,source_uri,\
                    install_root,archive_sha256,manifest_json,trust_state,installed_at,updated_at \
             FROM plugin_installations WHERE plugin_id=? AND version=?",
        )
        .bind(plugin_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(plugin_installation_from_row))
    }

    pub async fn list_plugin_installations(&self) -> Result<Vec<PluginInstallation>> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.list_plugin_installations()).await;
        }
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                i64,
                i64,
            ),
        >(
            "SELECT plugin_id,version,display_name,description,author,license,source_uri,\
                    install_root,archive_sha256,manifest_json,trust_state,installed_at,updated_at \
             FROM plugin_installations ORDER BY display_name COLLATE NOCASE, version",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(plugin_installation_from_row).collect())
    }

    pub async fn delete_plugin_installation(&self, plugin_id: &str, version: &str) -> Result<()> {
        if let Some(store) = self.route_global() {
            return Box::pin(store.delete_plugin_installation(plugin_id, version)).await;
        }
        sqlx::query("DELETE FROM plugin_installations WHERE plugin_id=? AND version=?")
            .bind(plugin_id)
            .bind(version)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_project_plugin(
        &self,
        project_id: &str,
        plugin_id: &str,
        version: &str,
        enabled: bool,
        grants_json: &str,
    ) -> Result<()> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.set_project_plugin(
                project_id,
                plugin_id,
                version,
                enabled,
                grants_json,
            ))
            .await;
        }
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO project_plugins(project_id,plugin_id,version,enabled,grants_json,updated_at) \
             VALUES(?,?,?,?,?,?) \
             ON CONFLICT(project_id,plugin_id) DO UPDATE SET \
                version=excluded.version, enabled=excluded.enabled, \
                grants_json=excluded.grants_json, updated_at=excluded.updated_at",
        )
        .bind(project_id)
        .bind(plugin_id)
        .bind(version)
        .bind(enabled)
        .bind(grants_json)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_project_plugin_enabled(
        &self,
        project_id: &str,
        plugin_id: &str,
        enabled: bool,
    ) -> Result<bool> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.set_project_plugin_enabled(project_id, plugin_id, enabled))
                .await;
        }
        let result = sqlx::query(
            "UPDATE project_plugins SET enabled=?,updated_at=? \
             WHERE project_id=? AND plugin_id=?",
        )
        .bind(enabled)
        .bind(chrono::Utc::now().timestamp())
        .bind(project_id)
        .bind(plugin_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() != 0)
    }

    pub async fn list_project_plugins(&self, project_id: &str) -> Result<Vec<ProjectPlugin>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_project_plugins(project_id)).await;
        }
        let rows = sqlx::query_as::<_, (String, String, String, bool, String, i64)>(
            "SELECT project_id,plugin_id,version,enabled,grants_json,updated_at \
             FROM project_plugins WHERE project_id=? ORDER BY plugin_id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(project_id, plugin_id, version, enabled, grants_json, updated_at)| {
                    ProjectPlugin {
                        project_id,
                        plugin_id,
                        version,
                        enabled,
                        grants_json,
                        updated_at,
                    }
                },
            )
            .collect())
    }

    pub async fn list_enabled_plugin_installations(
        &self,
        project_id: &str,
    ) -> Result<Vec<PluginInstallation>> {
        if self.registry.is_some() {
            let enabled = self.list_project_plugins(project_id).await?;
            let mut result = Vec::new();
            for binding in enabled.into_iter().filter(|p| p.enabled) {
                if let Some(plugin) = self
                    .get_plugin_installation(&binding.plugin_id, &binding.version)
                    .await?
                {
                    result.push(plugin);
                }
            }
            return Ok(result);
        }
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                i64,
                i64,
            ),
        >(
            "SELECT i.plugin_id,i.version,i.display_name,i.description,i.author,i.license,\
                    i.source_uri,i.install_root,i.archive_sha256,i.manifest_json,i.trust_state,\
                    i.installed_at,i.updated_at \
             FROM plugin_installations i \
             JOIN project_plugins p ON p.plugin_id=i.plugin_id AND p.version=i.version \
             WHERE p.project_id=? AND p.enabled=1 ORDER BY i.plugin_id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(plugin_installation_from_row).collect())
    }
}

fn plugin_installation_from_row(
    row: (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        i64,
    ),
) -> PluginInstallation {
    let (
        plugin_id,
        version,
        display_name,
        description,
        author,
        license,
        source_uri,
        install_root,
        archive_sha256,
        manifest_json,
        trust_state,
        installed_at,
        updated_at,
    ) = row;
    PluginInstallation {
        plugin_id,
        version,
        display_name,
        description,
        author,
        license,
        source_uri,
        install_root,
        archive_sha256,
        manifest_json,
        trust_state,
        installed_at,
        updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &str) -> PluginInstallation {
        let now = chrono::Utc::now().timestamp();
        PluginInstallation {
            plugin_id: "motif-for-claude-science".into(),
            version: "0.2.1".into(),
            display_name: "Motif for Claude Science".into(),
            description: "Molecular biology workbench".into(),
            author: "Jacob Vogan".into(),
            license: "MIT".into(),
            source_uri: "https://example.invalid/motif.zip".into(),
            install_root: root.into(),
            archive_sha256: "a".repeat(64),
            manifest_json: r#"{"schema":"wisp.plugin.v1","skills":[],"mcp_servers":[]}"#.into(),
            trust_state: "checksum_verified".into(),
            installed_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn plugin_installation_and_project_enable_roundtrip() {
        let db = std::env::temp_dir().join(format!("wisp_plugins_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&db).await.unwrap();
        store
            .create_project("project", "Project", "")
            .await
            .unwrap();
        let plugin = fixture("/plugins/motif/0.2.1");
        store.replace_plugin_installation(&plugin).await.unwrap();
        store
            .set_project_plugin(
                "project",
                &plugin.plugin_id,
                &plugin.version,
                true,
                r#"{"tools":"ask"}"#,
            )
            .await
            .unwrap();

        assert_eq!(
            store
                .get_plugin_installation(&plugin.plugin_id, &plugin.version)
                .await
                .unwrap(),
            Some(plugin.clone())
        );
        let enabled = store
            .list_enabled_plugin_installations("project")
            .await
            .unwrap();
        assert_eq!(enabled, vec![plugin.clone()]);

        let mut replacement = fixture("/plugins/motif/0.3.0");
        replacement.version = "0.3.0".into();
        replacement.description = "Updated workbench".into();
        store
            .replace_plugin_installation(&replacement)
            .await
            .unwrap();
        let bindings = store.list_project_plugins("project").await.unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].version, replacement.version);
        assert!(bindings[0].enabled);
        assert_eq!(bindings[0].grants_json, r#"{"tools":"ask"}"#);
        assert!(store
            .get_plugin_installation(&plugin.plugin_id, &plugin.version)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            store.list_plugin_installations().await.unwrap(),
            vec![replacement.clone()]
        );
        assert_eq!(
            store
                .list_enabled_plugin_installations("project")
                .await
                .unwrap(),
            vec![replacement.clone()]
        );

        assert!(store
            .set_project_plugin_enabled("project", &replacement.plugin_id, false)
            .await
            .unwrap());
        assert!(store
            .list_enabled_plugin_installations("project")
            .await
            .unwrap()
            .is_empty());

        store
            .delete_plugin_installation(&replacement.plugin_id, &replacement.version)
            .await
            .unwrap();
        assert!(store
            .list_project_plugins("project")
            .await
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_file(db);
    }
}
