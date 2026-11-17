//! Durable outcomes of automatic project-database recovery (#1635).
//!
//! `recover_project_database` returns a one-shot error naming what happened,
//! but a background entry point (project list, sync, maintenance) only logs
//! it. Each outcome is therefore also written here, to the application
//! database rather than the project database that was just replaced, and stays
//! pending until the user acknowledges it.
use super::Store;
use anyhow::Result;
use sqlx::{Row, SqlitePool};
use wisp_dto::{ProjectDatabaseRecovery, ProjectDatabaseRecoveryKind};

const COLUMNS: &str = "id,project_id,occurred_at,outcome,copy_name,damaged_path,backup_dir";

/// Write one outcome to `pool`, which must be the application database.
pub(super) async fn record(
    pool: &SqlitePool,
    project_id: &str,
    kind: ProjectDatabaseRecoveryKind,
    copy_name: Option<&str>,
    damaged_path: &str,
    backup_dir: Option<&str>,
) -> Result<i64> {
    let id = sqlx::query(
        "INSERT INTO project_database_recoveries\
         (project_id,occurred_at,outcome,copy_name,damaged_path,backup_dir) VALUES(?,?,?,?,?,?)",
    )
    .bind(project_id)
    .bind(chrono::Utc::now().timestamp())
    .bind(kind.as_str())
    .bind(copy_name)
    .bind(damaged_path)
    .bind(backup_dir)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

fn from_row(row: sqlx::sqlite::SqliteRow) -> Result<ProjectDatabaseRecovery> {
    let outcome: String = row.try_get("outcome")?;
    Ok(ProjectDatabaseRecovery {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        occurred_at: row.try_get("occurred_at")?,
        kind: ProjectDatabaseRecoveryKind::parse(&outcome)
            .ok_or_else(|| anyhow::anyhow!("Unknown recovery outcome: {outcome}"))?,
        copy_name: row.try_get("copy_name")?,
        damaged_path: row.try_get("damaged_path")?,
        backup_dir: row.try_get("backup_dir")?,
    })
}

impl Store {
    /// The application database, never a project database: the outcome must
    /// not depend on the database whose damage it reports.
    fn recovery_pool(&self) -> &SqlitePool {
        self.registry
            .as_ref()
            .map_or(&self.pool, |registry| &registry.global)
    }

    /// Unacknowledged outcomes for `project_id`, oldest first.
    pub async fn pending_project_database_recoveries(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProjectDatabaseRecovery>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM project_database_recoveries \
             WHERE project_id=? AND acknowledged_at IS NULL ORDER BY occurred_at,id"
        ))
        .bind(project_id)
        .fetch_all(self.recovery_pool())
        .await?
        .into_iter()
        .map(from_row)
        .collect()
    }

    /// Mark one outcome seen. Returns whether a pending outcome of that
    /// project was acknowledged; repeating it changes nothing.
    pub async fn acknowledge_project_database_recovery(
        &self,
        project_id: &str,
        id: i64,
    ) -> Result<bool> {
        let done = sqlx::query(
            "UPDATE project_database_recoveries SET acknowledged_at=? \
             WHERE id=? AND project_id=? AND acknowledged_at IS NULL",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(id)
        .bind(project_id)
        .execute(self.recovery_pool())
        .await?
        .rows_affected();
        Ok(done > 0)
    }
}
