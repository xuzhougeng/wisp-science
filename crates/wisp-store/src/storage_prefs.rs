//! Per-(project × execution context) storage locations: where uploads land on
//! a server, where run workdirs are created, and where retrieved outputs are
//! placed inside the project.

use super::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextStoragePrefs {
    pub project_id: String,
    pub context_id: String,
    /// Remote directory for uploaded data: absolute, `~/…`, or HOME-relative.
    pub remote_data_root: String,
    /// HOME-relative root for run workdirs (default `.wisp-science/runs`).
    pub remote_workdir_root: String,
    /// Project-relative directory where retrieved outputs land.
    pub local_results_dir: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// These paths are interpolated into remote shell scripts and transfer
/// destinations, so the accepted charset is deliberately conservative.
fn validate_path_charset(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{field} is required");
    }
    if value.len() > 512 {
        anyhow::bail!("{field} must be at most 512 characters");
    }
    if let Some(bad) = value
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || "._-/~".contains(*c)))
    {
        anyhow::bail!("{field} contains an unsupported character '{bad}'");
    }
    if value.split('/').any(|part| part == "..") {
        anyhow::bail!("{field} must not contain '..'");
    }
    if value.ends_with('/') {
        anyhow::bail!("{field} must not end with '/'");
    }
    Ok(())
}

pub fn validate_remote_data_root(value: &str) -> Result<()> {
    validate_path_charset("remote_data_root", value)?;
    if value.contains('~') && !value.starts_with("~/") {
        anyhow::bail!("remote_data_root may only use '~' as a leading '~/'");
    }
    Ok(())
}

pub fn validate_remote_workdir_root(value: &str) -> Result<()> {
    validate_path_charset("remote_workdir_root", value)?;
    if value.starts_with('/') || value.contains('~') {
        anyhow::bail!("remote_workdir_root must be a HOME-relative path");
    }
    Ok(())
}

pub fn validate_local_results_dir(value: &str) -> Result<()> {
    validate_path_charset("local_results_dir", value)?;
    if value.starts_with('/') || value.contains('~') {
        anyhow::bail!("local_results_dir must be a project-relative path");
    }
    Ok(())
}

impl ContextStoragePrefs {
    pub fn validate(&self) -> Result<()> {
        if self.project_id.trim().is_empty() || self.context_id.trim().is_empty() {
            anyhow::bail!("Storage preferences require a project and context");
        }
        validate_remote_data_root(&self.remote_data_root)?;
        validate_remote_workdir_root(&self.remote_workdir_root)?;
        validate_local_results_dir(&self.local_results_dir)?;
        Ok(())
    }
}

impl Store {
    pub async fn get_context_storage_prefs(
        &self,
        project_id: &str,
        context_id: &str,
    ) -> Result<Option<ContextStoragePrefs>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.get_context_storage_prefs(project_id, context_id)).await;
        }
        let row: Option<(String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT remote_data_root,remote_workdir_root,local_results_dir,created_at,updated_at \
             FROM context_storage_prefs WHERE project_id=? AND context_id=?",
        )
        .bind(project_id)
        .bind(context_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(remote_data_root, remote_workdir_root, local_results_dir, created_at, updated_at)| {
                ContextStoragePrefs {
                    project_id: project_id.into(),
                    context_id: context_id.into(),
                    remote_data_root,
                    remote_workdir_root,
                    local_results_dir,
                    created_at,
                    updated_at,
                }
            },
        ))
    }

    pub async fn upsert_context_storage_prefs(&self, prefs: &ContextStoragePrefs) -> Result<()> {
        if let Some(store) = self.route_project(&prefs.project_id).await? {
            return Box::pin(store.upsert_context_storage_prefs(prefs)).await;
        }
        prefs.validate()?;
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO context_storage_prefs(\
                project_id,context_id,remote_data_root,remote_workdir_root,local_results_dir,\
                created_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?) \
             ON CONFLICT(project_id,context_id) DO UPDATE SET \
                remote_data_root=excluded.remote_data_root, \
                remote_workdir_root=excluded.remote_workdir_root, \
                local_results_dir=excluded.local_results_dir, \
                updated_at=excluded.updated_at",
        )
        .bind(&prefs.project_id)
        .bind(&prefs.context_id)
        .bind(&prefs.remote_data_root)
        .bind(&prefs.remote_workdir_root)
        .bind(&prefs.local_results_dir)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
