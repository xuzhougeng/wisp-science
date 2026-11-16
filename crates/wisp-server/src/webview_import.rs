//! Bringing the WebView desktop's projects to this host. The WebView's
//! database is never written: its projects are listed through a read-only
//! connection, and to bring one over that connection first copies the whole
//! database, as one consistent file, into this host's data folder, from
//! which everything is then read. A project the WebView keeps in its own
//! database is copied from it and goes on using its folder where it is; a
//! project that keeps its database in its folder is opened where it is, as
//! a folder package is, so both desktops then share it.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use anyhow::{ensure, Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::Server;

/// The folder in this host's data folder the copy is made in; emptied
/// before and after every import.
const SCRATCH: &str = "webview-import";

/// A project of the WebView desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewProject {
    pub id: String,
    pub name: String,
    /// Its folder, where the WebView has it.
    pub folder: String,
    /// Its conversations; `None` when it keeps its own database, which is
    /// not opened to count them.
    pub conversations: Option<usize>,
    /// It keeps its database in its folder.
    pub own_database: bool,
    pub starred: bool,
    /// A project with its id is already here.
    pub here: bool,
    /// Its folder is on this computer.
    pub folder_found: bool,
}

/// How bringing one project over went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewImport {
    pub id: String,
    pub name: String,
    /// Why it was not brought over; `None` when it was.
    pub error: Option<String>,
}

/// A connection that only reads the WebView's database.
async fn read_only(database: &Path) -> Result<SqlitePool> {
    ensure!(
        database.is_file(),
        "No WebView database at {}",
        database.display()
    );
    let options = SqliteConnectOptions::new()
        .filename(database)
        .read_only(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .context("Cannot read the WebView's database")
}

async fn has_table(pool: &SqlitePool, table: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
    )
    .bind(table)
    .fetch_one(pool)
    .await?)
}

/// The projects in a WebView database, its hidden ones left out, starred
/// first and then by name. Nothing is written.
async fn projects(pool: &SqlitePool) -> Result<Vec<WebViewProject>> {
    ensure!(
        has_table(pool, "projects").await?,
        "This is not a Wisp database"
    );
    let own: HashSet<String> = if has_table(pool, "project_locations").await? {
        sqlx::query_scalar("SELECT project_id FROM project_locations")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };
    let starred: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('projects') WHERE name='starred')",
    )
    .fetch_one(pool)
    .await?;
    let rows: Vec<(String, String, String, bool)> = sqlx::query_as(&format!(
        "SELECT id, COALESCE(name,''), COALESCE(workspace_dir,''), {} FROM projects \
         WHERE id NOT LIKE 'assistant:%' AND id NOT LIKE 'scratch:%'",
        if starred { "starred" } else { "0" }
    ))
    .fetch_all(pool)
    .await?;
    let mut found = Vec::with_capacity(rows.len());
    for (id, name, folder, starred) in rows {
        let own_database = own.contains(&id);
        // Conversations as the list counts them: one begun, or named.
        let conversations = if own_database {
            None
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM frames f WHERE f.project_id=? AND f.parent_frame_id=f.id \
                 AND f.exploration_id IS NULL AND (f.title IS NOT NULL OR EXISTS( \
                 SELECT 1 FROM messages m WHERE m.frame_id=f.id AND m.role='user'))",
            )
            .bind(&id)
            .fetch_one(pool)
            .await
            .ok()
            .map(|count| count.max(0) as usize)
        };
        found.push(WebViewProject {
            folder_found: !folder.is_empty() && Path::new(&folder).is_dir(),
            id,
            name,
            folder,
            conversations,
            own_database,
            starred,
            here: false,
        });
    }
    found.sort_by(|a, b| {
        b.starred
            .cmp(&a.starred)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(found)
}

/// Brings one project over from the copy of the WebView's database.
async fn bring(
    store: &wisp_store::Store,
    copy: &wisp_store::Store,
    data: &Path,
    scratch: &Path,
    project: &WebViewProject,
) -> Result<()> {
    ensure!(
        store.get_project(&project.id).await?.is_none(),
        "It is already here"
    );
    let folder = PathBuf::from(&project.folder);
    ensure!(
        !project.folder.is_empty() && folder.is_dir(),
        "Its folder is not on this computer: {}",
        project.folder
    );
    let folder = dunce::canonicalize(&folder)?;
    ensure!(
        !folder.starts_with(data),
        "Keep project files separate from the host data directory"
    );
    if project.own_database {
        crate::project_archive::import_project_folder(store, data, &folder, None)
            .await
            .map_err(anyhow::Error::msg)?;
    } else {
        let records = scratch.join(format!("project-{}.sqlite", uuid::Uuid::new_v4()));
        copy.export_project_database(&project.id, &records).await?;
        store
            .import_project_database(&records, &project.id, &folder)
            .await?;
        let _ = std::fs::remove_file(&records);
    }
    if project.starred {
        let _ = store.set_project_starred(&project.id, true).await;
    }
    Ok(())
}

impl Server {
    /// The WebView desktop's projects in its database at `database`, read
    /// without changing it, each marked when it is already here.
    pub async fn webview_projects(&self, database: &Path) -> Result<Vec<WebViewProject>> {
        let pool = read_only(database).await?;
        let found = projects(&pool).await;
        pool.close().await;
        let mut found = found?;
        for project in &mut found {
            project.here = self.0.store.get_project(&project.id).await?.is_some();
        }
        Ok(found)
    }

    /// Brings the WebView desktop's projects `ids` over from its database
    /// at `database`, which is not changed: one kept in the WebView's
    /// database is copied and goes on using its folder where it is, and
    /// one that keeps its database in its folder is opened there. The reply
    /// says how each went; one that fails does not stop the rest.
    pub async fn import_webview_projects(
        &self,
        database: &Path,
        ids: &[String],
    ) -> Result<Vec<WebViewImport>> {
        // Exclusive, as any import is: no turn starts meanwhile.
        let _admission = self.0.admission.write().await;
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        let data = &self.0.config.data;
        let scratch = data.join(SCRATCH);
        // What an earlier import could not remove goes first.
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch)?;
        let copy = scratch.join("wisp.sqlite");
        let pool = read_only(database).await?;
        let copied = sqlx::query("VACUUM INTO ?")
            .bind(copy.to_string_lossy().as_ref())
            .execute(&pool)
            .await;
        pool.close().await;
        copied.context("Cannot copy the WebView's database")?;

        // The copy forgets where projects keep their own databases, so
        // nothing read from it reaches into their folders.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&copy))
            .await?;
        let listed = async {
            let found = projects(&pool).await?;
            if has_table(&pool, "project_locations").await? {
                sqlx::query("DELETE FROM project_locations")
                    .execute(&pool)
                    .await?;
            }
            anyhow::Ok(found)
        }
        .await;
        pool.close().await;
        let found = listed?;
        let records = wisp_store::Store::open(&copy)
            .await
            .context("Cannot read the copy of the WebView's database")?;

        let mut done = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(project) = found.iter().find(|project| &project.id == id) else {
                done.push(WebViewImport {
                    id: id.clone(),
                    name: String::new(),
                    error: Some("The WebView has no such project".into()),
                });
                continue;
            };
            let result = bring(&self.0.store, &records, data, &scratch, project).await;
            done.push(WebViewImport {
                id: project.id.clone(),
                name: project.name.clone(),
                error: result.err().map(|error| format!("{error:#}")),
            });
        }
        drop(records);
        let _ = std::fs::remove_dir_all(&scratch);
        Ok(done)
    }
}
