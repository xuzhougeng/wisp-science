//! Project databases are authoritative. The application database only registers
//! their locations; it never serves as a fallback when a registered file is lost.
use super::Store;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::{collections::HashMap, path::Path, sync::Arc};
use tokio::sync::Mutex;

pub(super) struct ProjectRegistry {
    global: SqlitePool,
    pools: Mutex<HashMap<String, SqlitePool>>,
    read_only: bool,
    migrate_on_open: bool,
    entities: Mutex<HashMap<(String, String, String), String>>,
}

pub const PROJECT_DATABASE: &str = ".wisp/project.sqlite";
pub const PROJECT_METADATA: &str = ".wisp/project.json";
const PROJECT_SETTING_PREFIXES: &[&str] = &[
    "project_enabled_skills:",
    "approval_scope:",
    "tool_approvals:",
    "skip_approval_connectors:",
];

#[derive(Debug, Serialize, Deserialize)]
struct ProjectMetadata {
    format: String,
    version: u32,
    project_id: String,
}

#[derive(Serialize, Deserialize)]
struct MigrationMarker {
    project_id: String,
    application_database: String,
    database_sha256: String,
}

fn database_digest(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn write_metadata(root: &Path, id: &str) -> Result<()> {
    let metadata = ProjectMetadata {
        format: "wisp-project".into(),
        version: 1,
        project_id: id.to_owned(),
    };
    let path = root.join(PROJECT_METADATA);
    if path.exists() {
        let existing: ProjectMetadata = serde_json::from_slice(&std::fs::read(&path)?)?;
        anyhow::ensure!(
            existing.project_id == id && existing.version == 1 && existing.format == "wisp-project",
            "Conflicting project metadata"
        );
        return Ok(());
    }
    publish_metadata_file(&path, &serde_json::to_vec_pretty(&metadata)?)
}

fn publish_metadata_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let pending = path.with_file_name(format!("storage-write-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::hard_link(&pending, path)?;
    std::fs::remove_file(pending)?;
    Ok(())
}

/// Copy explicitly selected, trusted columns between independent databases.
/// Preserve SQLite value types, NULLs and all historical epochs.
pub(super) async fn insert_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    rows: Vec<sqlx::sqlite::SqliteRow>,
) -> Result<()> {
    use sqlx::{Column, TypeInfo, ValueRef};
    for row in rows {
        let columns = row
            .columns()
            .iter()
            .map(|c| format!("\"{}\"", c.name()))
            .collect::<Vec<_>>()
            .join(",");
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
            "INSERT INTO {table}({columns}) VALUES("
        ));
        let mut values = query.separated(",");
        for (i, column) in row.columns().iter().enumerate() {
            let value = row.try_get_raw(i)?;
            if value.is_null() {
                values.push_bind(None::<String>);
            } else {
                match value.type_info().name() {
                    "INTEGER" => {
                        values.push_bind(row.try_get::<i64, _>(i)?);
                    }
                    "REAL" => {
                        values.push_bind(row.try_get::<f64, _>(i)?);
                    }
                    "BLOB" => {
                        values.push_bind(row.try_get::<Vec<u8>, _>(i)?);
                    }
                    "TEXT" => {
                        values.push_bind(row.try_get::<String, _>(i)?);
                    }
                    other => anyhow::bail!("Unsupported value type {other} for {}", column.name()),
                }
            }
        }
        query.push(")").build().execute(&mut **tx).await?;
    }
    Ok(())
}

impl Store {
    /// Register an existing, self-contained workspace without requiring its
    /// original application database. Metadata and database identity must agree.
    pub async fn register_project_folder(&self, workspace: &Path) -> Result<String> {
        anyhow::ensure!(
            self.registry.is_some() && self.project_scope.is_none(),
            "Application store required"
        );
        let workspace = std::fs::canonicalize(workspace)?;
        for relative in [".wisp", PROJECT_METADATA, PROJECT_DATABASE] {
            anyhow::ensure!(
                !std::fs::symlink_metadata(workspace.join(relative))?
                    .file_type()
                    .is_symlink(),
                "Project metadata paths must not be symbolic links"
            );
        }
        let metadata_path = workspace.join(PROJECT_METADATA);
        anyhow::ensure!(
            std::fs::metadata(&metadata_path)?.len() <= 64 * 1024,
            "Project metadata is too large"
        );
        let metadata: ProjectMetadata = serde_json::from_slice(&std::fs::read(metadata_path)?)?;
        anyhow::ensure!(
            metadata.format == "wisp-project"
                && metadata.version == 1
                && !metadata.project_id.is_empty(),
            "Unsupported project metadata"
        );
        let database = workspace.join(PROJECT_DATABASE);
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM projects")
            .fetch_all(&pool)
            .await?;
        anyhow::ensure!(
            ids == [metadata.project_id.clone()],
            "Project metadata does not match its database"
        );
        let mut tx = self.begin_write().await?;
        let existing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?)")
            .bind(&metadata.project_id)
            .fetch_one(&mut *tx)
            .await?;
        anyhow::ensure!(!existing, "This project is already registered");
        Self::migrate(&pool).await?;
        let project = sqlx::query(
            "SELECT name,description,workspace_dir,created_at,updated_at FROM projects WHERE id=?",
        )
        .bind(&metadata.project_id)
        .fetch_one(&pool)
        .await?;
        let old_root: String = project.try_get("workspace_dir")?;
        rebase_workspace_paths(&pool, &old_root, &workspace).await?;
        sqlx::query("INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(&metadata.project_id).bind(project.try_get::<String,_>("name")?)
            .bind(project.try_get::<String,_>("description")?).bind(workspace.to_string_lossy().as_ref())
            .bind(project.try_get::<i64,_>("created_at")?).bind(project.try_get::<i64,_>("updated_at")?)
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO project_locations(project_id,database_path) VALUES(?,?)")
            .bind(&metadata.project_id)
            .bind(database.to_string_lossy().as_ref())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        pool.close().await;
        Ok(metadata.project_id)
    }
    /// Production entry point. Legacy projects are migrated before background
    /// workers can write. Snapshot/test stores can still use `open` explicitly.
    pub async fn open_application(path: &Path) -> Result<Self> {
        let mut store = Self::open(path).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS project_locations (project_id TEXT PRIMARY KEY, database_path TEXT NOT NULL UNIQUE)")
            .execute(&store.pool).await?;
        // Memory content is application-wide; its optional source is now a
        // cross-database reference and cannot be a SQLite foreign key.
        if !sqlx::query("PRAGMA foreign_key_list(global_memories)")
            .fetch_all(&store.pool)
            .await?
            .is_empty()
        {
            let mut tx = store.begin_write().await?;
            for sql in [
                "CREATE TABLE global_memories_registry(id TEXT PRIMARY KEY,content TEXT NOT NULL CHECK(length(trim(content))>0),source_frame_id TEXT,source_turn_index INTEGER CHECK(source_turn_index IS NULL OR source_turn_index>=0),created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL)",
                "INSERT INTO global_memories_registry SELECT * FROM global_memories",
                "DROP TABLE global_memories",
                "ALTER TABLE global_memories_registry RENAME TO global_memories",
                "CREATE INDEX ix_global_memories_updated ON global_memories(updated_at DESC,id DESC)",
            ] { sqlx::query(sql).execute(&mut *tx).await?; }
            tx.commit().await?;
        }
        store.enable_project_registry(false).await?;
        if let Some(registry) = store.registry.as_mut().and_then(Arc::get_mut) {
            registry.migrate_on_open = true;
        }
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM projects WHERE id NOT IN (SELECT project_id FROM project_locations)",
        )
        .fetch_all(&store.pool)
        .await?;
        for id in ids {
            if let Err(error) = store.migrate_project_storage(&id).await {
                // A disconnected legacy workspace must not prevent application
                // startup or destroy its only copy. Retry on the next startup.
                tracing::error!(project_id=%id, %error, "Project storage migration deferred; legacy records retained");
            }
        }
        let locations: Vec<(String, String)> =
            sqlx::query_as("SELECT project_id,database_path FROM project_locations")
                .fetch_all(&store.pool)
                .await?;
        for (id, path) in locations {
            let database = Path::new(&path);
            if let Some(root) = database.parent().and_then(Path::parent) {
                let marker = root.join(".wisp/storage-migration.json");
                if marker.exists() && database.is_file() {
                    write_metadata(root, &id)?;
                    std::fs::remove_file(marker)?;
                }
            }
        }
        Ok(store)
    }

    pub(super) async fn enable_project_registry(&mut self, read_only: bool) -> Result<()> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='project_locations')")
            .fetch_one(&self.pool).await?;
        if exists {
            self.registry = Some(Arc::new(ProjectRegistry {
                global: self.pool.clone(),
                pools: Mutex::new(HashMap::new()),
                read_only,
                migrate_on_open: false,
                entities: Mutex::new(HashMap::new()),
            }));
        }
        Ok(())
    }

    pub(super) fn route_global(&self) -> Option<Self> {
        let registry = self.registry.as_ref()?;
        self.project_scope.as_ref()?;
        Some(Self {
            pool: registry.global.clone(),
            registry: self.registry.clone(),
            project_scope: None,
        })
    }

    pub(super) async fn route_project(&self, id: &str) -> Result<Option<Self>> {
        let Some(registry) = &self.registry else {
            return Ok(None);
        };
        if let Some(scope) = &self.project_scope {
            anyhow::ensure!(
                scope == id,
                "cross-project access requires the application store"
            );
            return Ok(None);
        }
        let path: Option<String> =
            sqlx::query_scalar("SELECT database_path FROM project_locations WHERE project_id=?")
                .bind(id)
                .fetch_optional(&registry.global)
                .await?;
        let Some(path) = path else { return Ok(None) };
        anyhow::ensure!(
            Path::new(&path).is_file(),
            "Project {id} database is unavailable: {path}"
        );
        let mut pools = registry.pools.lock().await;
        if !pools.contains_key(id) {
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(false)
                .read_only(registry.read_only)
                .busy_timeout(std::time::Duration::from_secs(5));
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(4)
                .connect_with(options)
                .await?;
            if registry.migrate_on_open {
                Self::migrate(&pool).await?;
            }
            let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM projects")
                .fetch_all(&pool)
                .await?;
            anyhow::ensure!(ids == [id], "Project database identity mismatch: {id}");
            pools.insert(id.to_owned(), pool);
        }
        Ok(Some(Self {
            pool: pools[id].clone(),
            registry: self.registry.clone(),
            project_scope: Some(id.to_owned()),
        }))
    }

    pub(super) async fn routed_projects(&self) -> Result<Option<Vec<Self>>> {
        if self.registry.is_none() || self.project_scope.is_some() {
            return Ok(None);
        }
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT project_id FROM project_locations ORDER BY project_id")
                .fetch_all(&self.pool)
                .await?;
        let mut stores = Vec::new();
        for id in ids {
            if let Some(store) = self.route_project(&id).await? {
                stores.push(store);
            }
        }
        // Legacy/unregistered records may still exist after an interrupted
        // migration. They are a separate source, never a fallback for a location.
        stores.push(Self {
            pool: self.pool.clone(),
            registry: None,
            project_scope: None,
        });
        Ok(Some(stores))
    }

    pub(super) async fn route_entity(
        &self,
        table: &str,
        column: &str,
        id: &str,
    ) -> Result<Option<Self>> {
        let Some(registry) = &self.registry else {
            return Ok(None);
        };
        if self.project_scope.is_some() {
            return Ok(None);
        }
        // Identifiers are compile-time constants at the call sites, never input.
        let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column}=?)");
        let key = (table.to_owned(), column.to_owned(), id.to_owned());
        let cached = registry.entities.lock().await.get(&key).cloned();
        if let Some(project) = cached {
            if let Some(store) = self.route_project(&project).await? {
                if sqlx::query_scalar::<_, bool>(&sql)
                    .bind(id)
                    .fetch_one(&store.pool)
                    .await?
                {
                    return Ok(Some(store));
                }
            }
            registry.entities.lock().await.remove(&key);
        }
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT project_id FROM project_locations ORDER BY project_id")
                .fetch_all(&self.pool)
                .await?;
        let mut unavailable = None;
        for project in ids {
            let store = match self.route_project(&project).await {
                Ok(Some(store)) => store,
                Ok(None) => continue,
                Err(error) => {
                    unavailable = Some(error);
                    continue;
                }
            };
            if sqlx::query_scalar::<_, bool>(&sql)
                .bind(id)
                .fetch_one(&store.pool)
                .await?
            {
                registry.entities.lock().await.insert(key, project);
                return Ok(Some(store));
            }
        }
        if let Some(error) = unavailable {
            return Err(error);
        }
        Ok(None)
    }

    pub(super) async fn route_setting(&self, key: &str) -> Result<Option<Self>> {
        for prefix in super::explorations::FRAME_SETTING_PREFIXES {
            if let Some(id) = key.strip_prefix(prefix) {
                return self.route_entity("frames", "id", id).await;
            }
        }
        for prefix in PROJECT_SETTING_PREFIXES {
            if let Some(id) = key.strip_prefix(prefix) {
                return self.route_project(id).await;
            }
        }
        if let Some(rest) = key.strip_prefix("acp.config_options.") {
            if let Some((id, _)) = rest.split_once('.') {
                return self.route_entity("frames", "id", id).await;
            }
        }
        Ok(self.route_global())
    }

    pub(super) async fn retain_environment_snapshot(&self, hash: Option<&str>) -> Result<()> {
        let (Some(hash), Some(global)) = (hash, self.route_global()) else {
            return Ok(());
        };
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM env_snapshots WHERE hash=?)")
                .bind(hash)
                .fetch_one(&self.pool)
                .await?;
        if !exists {
            let rows = sqlx::query("SELECT * FROM env_snapshots WHERE hash=?")
                .bind(hash)
                .fetch_all(&global.pool)
                .await?;
            let mut tx = self.begin_write().await?;
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM env_snapshots WHERE hash=?)")
                    .bind(hash)
                    .fetch_one(&mut *tx)
                    .await?;
            if !exists {
                insert_rows(&mut tx, "env_snapshots", rows).await?;
            }
            tx.commit().await?;
        }
        Ok(())
    }

    /// Detach only the registration. The project database and files survive.
    pub(super) async fn unregister_project_storage(&self, id: &str) -> Result<bool> {
        if self.registry.is_none() || self.project_scope.is_some() {
            return Ok(false);
        }
        let mut tx = self.begin_write().await?;
        let affected = sqlx::query("DELETE FROM project_locations WHERE project_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected == 0 {
            return Ok(false);
        }
        sqlx::query("DELETE FROM project_sync_state WHERE project_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM projects WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        if let Some(registry) = &self.registry {
            registry.pools.lock().await.remove(id);
        }
        Ok(true)
    }

    /// Migrate without the portable export's runtime sanitization. Work on a
    /// private snapshot, prune other projects, validate, then publish and cut over.
    pub async fn migrate_project_storage(&self, id: &str) -> Result<()> {
        anyhow::ensure!(
            self.registry.is_some() && self.project_scope.is_none(),
            "Application store required"
        );
        if self.route_project(id).await?.is_some() {
            return Ok(());
        }
        let (_, workspace): (String, String) =
            sqlx::query_as("SELECT name,workspace_dir FROM projects WHERE id=?")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        anyhow::ensure!(
            !workspace.trim().is_empty(),
            "Project {id} has no workspace"
        );
        let root = Path::new(&workspace);
        anyhow::ensure!(
            root.is_dir(),
            "Project {id} workspace is unavailable: {workspace}"
        );
        let directory = root.join(".wisp");
        std::fs::create_dir_all(&directory)?;
        let destination = root.join(PROJECT_DATABASE);
        let metadata_path = root.join(PROJECT_METADATA);
        let marker_path = directory.join("storage-migration.json");
        let application_database = self.database_path().await?.to_string_lossy().into_owned();
        if marker_path.exists() {
            let marker: MigrationMarker = serde_json::from_slice(&std::fs::read(&marker_path)?)?;
            anyhow::ensure!(
                marker.project_id == id
                    && marker.application_database == application_database
                    && !metadata_path.exists(),
                "Unfinished project migration belongs to another source"
            );
            if destination.exists() {
                anyhow::ensure!(
                    database_digest(&destination)? == marker.database_sha256,
                    "Unfinished project database was modified; preserve it for recovery"
                );
                std::fs::remove_file(&destination)?;
            }
            std::fs::remove_file(&marker_path)?;
        }
        anyhow::ensure!(
            !destination.exists() && !metadata_path.exists(),
            "Project storage already exists; register it instead: {}",
            root.display()
        );
        let staging = Path::new(&application_database)
            .with_file_name(format!("project-migration-{}.sqlite", uuid::Uuid::new_v4()));
        // Keep legacy writers out until registration and legacy removal commit.
        let mut cutover = self.begin_write().await?;
        sqlx::query("VACUUM INTO ?")
            .bind(staging.to_string_lossy().as_ref())
            .execute(&self.pool)
            .await?;
        let local = Self::open_snapshot(&staging).await?;
        let others: Vec<String> = sqlx::query_scalar("SELECT id FROM projects WHERE id<>?")
            .bind(id)
            .fetch_all(&local.pool)
            .await?;
        for other in others {
            local.delete_project(&other).await?;
        }
        let mut tx = local.begin_write().await?;
        // These bindings reference device-owned records. Keep the identifiers
        // without duplicating device credentials/configuration into a project.
        for statement in [
            "CREATE TABLE project_plugins_local(project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,plugin_id TEXT NOT NULL,version TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),grants_json TEXT NOT NULL DEFAULT '{}',updated_at INTEGER NOT NULL,PRIMARY KEY(project_id,plugin_id))",
            "INSERT INTO project_plugins_local SELECT * FROM project_plugins",
            "DROP TABLE project_plugins",
            "ALTER TABLE project_plugins_local RENAME TO project_plugins",
            "CREATE INDEX ix_project_plugins_enabled ON project_plugins(project_id,enabled,plugin_id)",
            "CREATE TABLE session_execution_contexts_local(frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,context_id TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(frame_id,context_id))",
            "INSERT INTO session_execution_contexts_local SELECT * FROM session_execution_contexts",
            "DROP TABLE session_execution_contexts",
            "ALTER TABLE session_execution_contexts_local RENAME TO session_execution_contexts",
            "CREATE INDEX ix_session_execution_contexts_context ON session_execution_contexts(context_id)",
        ] { sqlx::query(statement).execute(&mut *tx).await?; }
        // Only frame-specific settings belong to the project. Global account,
        // provider, device and discovery configuration must not follow it.
        let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM settings")
            .fetch_all(&mut *tx)
            .await?;
        let mut project_settings = Vec::new();
        for key in keys {
            let frame = super::explorations::FRAME_SETTING_PREFIXES
                .iter()
                .find_map(|p| key.strip_prefix(p))
                .or_else(|| {
                    key.strip_prefix("acp.config_options.")
                        .and_then(|rest| rest.split_once('.').map(|(id, _)| id))
                });
            let keep = if let Some(frame) = frame {
                sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM frames WHERE id=?)")
                    .bind(frame)
                    .fetch_one(&mut *tx)
                    .await?
            } else {
                PROJECT_SETTING_PREFIXES
                    .iter()
                    .any(|prefix| key.strip_prefix(prefix) == Some(id))
            };
            if !keep {
                sqlx::query("DELETE FROM settings WHERE key=?")
                    .bind(key)
                    .execute(&mut *tx)
                    .await?;
            } else {
                project_settings.push(key);
            }
        }
        sqlx::query("DELETE FROM env_snapshots WHERE hash NOT IN (SELECT env_hash FROM execution_log WHERE env_hash IS NOT NULL UNION SELECT env_snapshot_hash FROM artifact_versions WHERE env_snapshot_hash IS NOT NULL UNION SELECT env_snapshot_hash FROM run_environment_snapshots)").execute(&mut *tx).await?;
        for table in [
            "project_locations",
            "project_sync_state",
            "global_memories",
            "external_session_cache",
            "plugin_installations",
            "execution_contexts",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&local.pool)
            .await?;
        anyhow::ensure!(
            integrity == "ok",
            "Project migration failed integrity check: {integrity}"
        );
        sqlx::query("VACUUM").execute(&local.pool).await?;
        local.pool.close().await;
        // Only a fully sanitized database enters the workspace. Publish the
        // database before committing its registration, and the manifest last.
        let prepared = directory.join(format!("project-migration-{}.sqlite", uuid::Uuid::new_v4()));
        std::fs::copy(&staging, &prepared)?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&prepared)?
            .sync_all()?;
        let marker = MigrationMarker {
            project_id: id.to_owned(),
            application_database,
            database_sha256: database_digest(&prepared)?,
        };
        publish_metadata_file(&marker_path, &serde_json::to_vec(&marker)?)?;
        std::fs::hard_link(&prepared, &destination).context("publish project database")?;
        std::fs::remove_file(prepared)?;
        sqlx::query("INSERT INTO project_locations(project_id,database_path) VALUES(?,?)")
            .bind(id)
            .bind(destination.to_string_lossy().as_ref())
            .execute(&mut *cutover)
            .await?;
        let memory_sources: Vec<(String,String)> = sqlx::query_as("SELECT id,source_frame_id FROM global_memories WHERE source_frame_id IN (SELECT id FROM frames WHERE project_id=?)")
            .bind(id).fetch_all(&mut *cutover).await?;
        super::project_transfer::delete_project_children(&mut cutover, id).await?;
        for (memory, frame) in memory_sources {
            sqlx::query("UPDATE global_memories SET source_frame_id=? WHERE id=?")
                .bind(frame)
                .bind(memory)
                .execute(&mut *cutover)
                .await?;
        }
        for key in project_settings {
            sqlx::query("DELETE FROM settings WHERE key=?")
                .bind(key)
                .execute(&mut *cutover)
                .await?;
        }
        cutover.commit().await?;
        write_metadata(root, id)?;
        std::fs::remove_file(marker_path)?;
        std::fs::remove_file(staging)?;
        Ok(())
    }
}

/// Rebind only paths under the previous workspace. Remote URIs and references
/// outside it keep their meaning; historical JSON/log contents remain immutable.
async fn rebase_workspace_paths(pool: &SqlitePool, old: &str, workspace: &Path) -> Result<()> {
    let new = workspace.to_string_lossy();
    let old = old.replace('\\', "/").trim_end_matches('/').to_owned();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    for (table, column) in [
        ("artifacts", "storage_path"),
        ("artifact_versions", "storage_path"),
        ("runs", "script_path"),
        ("runs", "logs_path"),
        ("context_archives", "storage_path"),
        ("explorations", "workspace_dir"),
        ("run_code_snapshots", "source_path"),
        ("run_code_snapshots", "storage_path"),
        ("run_inputs", "source_ref"),
        ("run_outputs", "source_path"),
        ("turn_file_undo", "path"),
        ("turn_file_undo", "before_snapshot_path"),
        ("acp_sessions", "cwd"),
    ] {
        let columns = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(&mut *tx)
            .await?;
        if !columns.iter().any(|r| r.get::<String, _>("name") == column) {
            continue;
        }
        let rows = sqlx::query(&format!(
            "SELECT rowid,{column} AS path FROM {table} WHERE {column} IS NOT NULL"
        ))
        .fetch_all(&mut *tx)
        .await?;
        for row in rows {
            let path: String = row.try_get("path")?;
            let normalized = path.replace('\\', "/");
            let windows = old.as_bytes().get(1) == Some(&b':') || old.starts_with("//");
            let (candidate, prefix) = if windows {
                (normalized.to_lowercase(), old.to_lowercase())
            } else {
                (normalized.clone(), old.clone())
            };
            if candidate == prefix || candidate.starts_with(&format!("{prefix}/")) {
                let suffix = normalized[old.len()..].trim_start_matches('/');
                let rebound = workspace.join(suffix);
                sqlx::query(&format!("UPDATE {table} SET {column}=? WHERE rowid=?"))
                    .bind(rebound.to_string_lossy().as_ref())
                    .bind(row.try_get::<i64, _>("rowid")?)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    sqlx::query("UPDATE projects SET workspace_dir=?")
        .bind(new.as_ref())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn live_writes_survive_removing_registration_and_new_global_database() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = Store::open_application(&root.path().join("global.sqlite"))
            .await
            .unwrap();
        store
            .set_setting("provider_secret", "must stay global")
            .await
            .unwrap();
        store
            .create_project("p", "Project", workspace.to_str().unwrap())
            .await
            .unwrap();
        store
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap();
        store
            .append_message("f", 1, &wisp_llm::Message::user("a durable question"))
            .await
            .unwrap();
        let memory = crate::GlobalMemory {
            id: "memory".into(),
            content: "Global preference".into(),
            source_frame_id: Some("f".into()),
            source_turn_index: Some(0),
            created_at: 1,
            updated_at: 1,
        };
        store.insert_global_memory(&memory).await.unwrap();
        assert_eq!(store.list_global_memories(10).await.unwrap(), vec![memory]);
        let local = Store::open_read_only(&workspace.join(PROJECT_DATABASE))
            .await
            .unwrap();
        assert_eq!(local.message_count("f").await.unwrap(), 1);
        assert_eq!(local.get_setting("provider_secret").await.unwrap(), None);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&store.pool)
                .await
                .unwrap(),
            0
        );
        store.delete_project("p").await.unwrap();
        let fresh = Store::open_application(&root.path().join("fresh.sqlite"))
            .await
            .unwrap();
        assert_eq!(
            fresh.register_project_folder(&workspace).await.unwrap(),
            "p"
        );
        assert_eq!(fresh.message_count("f").await.unwrap(), 1);
        fresh
            .append_message("f", 2, &wisp_llm::Message::user("after reopening"))
            .await
            .unwrap();
        assert_eq!(local.message_count("f").await.unwrap(), 2);
    }

    #[tokio::test]
    async fn migration_preserves_legacy_messages_and_project_settings() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let path = root.path().join("global.sqlite");
        let legacy = Store::open(&path).await.unwrap();
        legacy
            .create_project("p", "Project", workspace.to_str().unwrap())
            .await
            .unwrap();
        legacy
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap();
        legacy
            .append_message("f", 1, &wisp_llm::Message::user("legacy question"))
            .await
            .unwrap();
        legacy
            .set_setting("frame_plan_mode:f", "true")
            .await
            .unwrap();
        legacy
            .set_setting("project_enabled_skills:p", "[\"analysis\"]")
            .await
            .unwrap();
        legacy
            .set_setting("provider_configuration", "device only")
            .await
            .unwrap();
        legacy.set_project_starred("p", true).await.unwrap();
        let mut run = crate::RunRecord::new("run", "p", "local", "Running analysis", "shell");
        run.status = crate::RunStatus::Running;
        run.remote_handle_json = Some("{\"pid\":123}".into());
        legacy.create_run(&run).await.unwrap();
        let schedule = crate::ScheduleRecord {
            id: "schedule".into(),
            project_id: "p".into(),
            frame_id: Some("f".into()),
            name: "Follow up".into(),
            prompt: "Continue".into(),
            skill: None,
            interval_secs: 3600,
            enabled: true,
            next_run_at: 100,
            last_run_at: None,
            created_at: 1,
            updated_at: 1,
        };
        legacy.create_schedule(&schedule).await.unwrap();
        let workflow = crate::AgentWorkflow::new("workflow", "p", "workspace", "Plan").unwrap();
        legacy
            .create_agent_workflow_plan(&workflow, &[])
            .await
            .unwrap();
        legacy.pool.close().await;
        let migrated = Store::open_application(&path).await.unwrap();
        migrated.set_project_starred("p", false).await.unwrap();
        assert!(!migrated.starred_project_ids().await.unwrap().contains("p"));
        assert_eq!(migrated.message_count("f").await.unwrap(), 1);
        assert_eq!(
            migrated
                .get_setting("frame_plan_mode:f")
                .await
                .unwrap()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            migrated
                .get_setting("project_enabled_skills:p")
                .await
                .unwrap()
                .as_deref(),
            Some("[\"analysis\"]")
        );
        assert_eq!(migrated.get_run("run").await.unwrap(), Some(run));
        assert_eq!(
            migrated.get_schedule("schedule").await.unwrap(),
            Some(schedule)
        );
        assert_eq!(
            migrated.get_agent_workflow("workflow").await.unwrap(),
            Some(workflow)
        );
        let direct = Store::open_read_only(&workspace.join(PROJECT_DATABASE))
            .await
            .unwrap();
        assert_eq!(
            direct.get_setting("provider_configuration").await.unwrap(),
            None
        );
        assert!(direct.list_execution_contexts().await.unwrap().is_empty());
        assert!(workspace.join(PROJECT_METADATA).is_file());
    }

    #[tokio::test]
    async fn failed_migration_keeps_legacy_data_and_can_retry() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(workspace.join(".wisp")).unwrap();
        let path = root.path().join("global.sqlite");
        let legacy = Store::open(&path).await.unwrap();
        legacy
            .create_project("p", "P", workspace.to_str().unwrap())
            .await
            .unwrap();
        legacy
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap();
        legacy
            .append_message("f", 1, &wisp_llm::Message::user("must not be lost"))
            .await
            .unwrap();
        std::fs::write(workspace.join(PROJECT_DATABASE), b"unknown existing file").unwrap();
        let deferred = Store::open_application(&path).await.unwrap();
        assert_eq!(deferred.message_count("f").await.unwrap(), 1);
        assert_eq!(
            std::fs::read(workspace.join(PROJECT_DATABASE)).unwrap(),
            b"unknown existing file"
        );
        std::fs::remove_file(workspace.join(PROJECT_DATABASE)).unwrap();
        deferred.migrate_project_storage("p").await.unwrap();
        assert_eq!(deferred.message_count("f").await.unwrap(), 1);
        assert!(workspace.join(PROJECT_METADATA).exists());
    }

    #[tokio::test]
    async fn interrupted_publication_retries_from_legacy_and_committed_cutover_repairs_manifest() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(workspace.join(".wisp")).unwrap();
        let path = root.path().join("global.sqlite");
        let legacy = Store::open(&path).await.unwrap();
        legacy
            .create_project("p", "P", workspace.to_str().unwrap())
            .await
            .unwrap();
        legacy
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap();
        legacy
            .export_project_database("p", &workspace.join(PROJECT_DATABASE))
            .await
            .unwrap();
        // The legacy store gained data after the failed cutover. Never select
        // the older prepared file just because it exists.
        legacy
            .append_message("f", 1, &wisp_llm::Message::user("newer legacy record"))
            .await
            .unwrap();
        let marker = MigrationMarker {
            project_id: "p".into(),
            application_database: legacy
                .database_path()
                .await
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            database_sha256: database_digest(&workspace.join(PROJECT_DATABASE)).unwrap(),
        };
        let marker_path = workspace.join(".wisp/storage-migration.json");
        std::fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        let store = Store::open_application(&path).await.unwrap();
        assert_eq!(store.message_count("f").await.unwrap(), 1);
        assert!(!marker_path.exists());
        std::fs::remove_file(workspace.join(PROJECT_METADATA)).unwrap();
        std::fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        let reopened = Store::open_application(&path).await.unwrap();
        assert_eq!(reopened.message_count("f").await.unwrap(), 1);
        assert!(workspace.join(PROJECT_METADATA).exists());
        assert!(!marker_path.exists());
    }

    #[tokio::test]
    async fn independent_projects_support_concurrent_writes_search_copy_and_native_reads() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("global.sqlite");
        let store = Store::open_application(&path).await.unwrap();
        for id in ["a", "b"] {
            store
                .create_project(id, id, root.path().join(id).to_str().unwrap())
                .await
                .unwrap();
            store.create_frame(id, id, "agent", "model").await.unwrap();
        }
        let alpha = wisp_llm::Message::user("alpha");
        let beta = wisp_llm::Message::user("beta");
        let (a, b) = tokio::join!(
            store.append_message("a", 1, &alpha),
            store.append_message("b", 1, &beta),
        );
        a.unwrap();
        b.unwrap();
        store
            .append_message("b", 2, &wisp_llm::Message::user("alpha only in the body"))
            .await
            .unwrap();
        assert_eq!(
            store
                .search_sessions(None, "alpha", 10, None, None)
                .await
                .unwrap()[0]
                .id,
            "a"
        );
        store
            .create_publication("publication", "a", "Paper", "")
            .await
            .unwrap();
        store
            .create_publication_revision("revision", "publication", None, "Draft")
            .await
            .unwrap();
        store
            .begin_publication_freeze(
                "revision",
                "new-freeze-attempt",
                &crate::PublicationFreezePolicy::default(),
            )
            .await
            .unwrap();
        assert!(store
            .abort_publication_freeze("revision", "new-freeze-attempt")
            .await
            .unwrap());
        store.set_project_starred("b", true).await.unwrap();
        let projects = store.list_projects().await.unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].0, "b");
        assert_eq!(projects[0].5, 1);
        assert_eq!(
            store
                .search_sessions(None, "", 100, None, None)
                .await
                .unwrap()
                .len(),
            2
        );
        store
            .copy_session_to_project("a", "a", "b", "copy")
            .await
            .unwrap();
        assert_eq!(store.message_count("copy").await.unwrap(), 1);
        store
            .move_session_to_project("copy", "b", "a", "moved")
            .await
            .unwrap();
        assert_eq!(store.frame_project_id("copy").await.unwrap(), None);
        assert_eq!(
            store.frame_project_id("moved").await.unwrap().as_deref(),
            Some("a")
        );
        let native = Store::open_read_only(&path).await.unwrap();
        assert_eq!(native.message_count("moved").await.unwrap(), 1);
        let local = Store::open_read_only(&root.path().join("b").join(PROJECT_DATABASE))
            .await
            .unwrap();
        assert_eq!(local.frame_project_id("a").await.unwrap(), None);
        assert_eq!(
            local.frame_project_id("b").await.unwrap().as_deref(),
            Some("b")
        );
    }

    #[tokio::test]
    async fn missing_registered_database_never_creates_a_blank_replacement() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_application(&root.path().join("global.sqlite"))
            .await
            .unwrap();
        let workspace = root.path().join("workspace");
        store
            .create_project("p", "P", workspace.to_str().unwrap())
            .await
            .unwrap();
        let database = workspace.join(PROJECT_DATABASE);
        std::fs::rename(&database, workspace.join("saved.sqlite")).unwrap();
        assert!(store
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap_err()
            .to_string()
            .contains("unavailable"));
        assert!(!database.exists());
        let fresh = Store::open_application(&root.path().join("fresh.sqlite"))
            .await
            .unwrap();
        assert!(fresh.register_project_folder(&workspace).await.is_err());
        assert!(fresh.list_projects().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn export_import_and_sync_target_the_live_project_database() {
        let root = tempfile::tempdir().unwrap();
        let source = Store::open_application(&root.path().join("source.sqlite"))
            .await
            .unwrap();
        source
            .create_project("p", "P", root.path().join("source").to_str().unwrap())
            .await
            .unwrap();
        source
            .create_frame("f", "p", "agent", "model")
            .await
            .unwrap();
        source
            .append_message("f", 1, &wisp_llm::Message::user("exported"))
            .await
            .unwrap();
        let archive = root.path().join("archive.sqlite");
        source.export_project_database("p", &archive).await.unwrap();
        let destination = root.path().join("target");
        std::fs::create_dir_all(&destination).unwrap();
        let target = Store::open_application(&root.path().join("target.sqlite"))
            .await
            .unwrap();
        target
            .import_project_database(&archive, "p", &destination)
            .await
            .unwrap();
        assert_eq!(target.message_count("f").await.unwrap(), 1);
        source
            .append_message("f", 2, &wisp_llm::Message::user("synced"))
            .await
            .unwrap();
        source.export_project_database("p", &archive).await.unwrap();
        let state = super::super::ProjectSyncState::uninitialized("p", "folder", "relay");
        target
            .replace_project_database(&archive, "p", &destination, &state)
            .await
            .unwrap();
        assert_eq!(
            target.get_project_sync_state("p").await.unwrap(),
            Some(state)
        );
        assert_eq!(target.message_count("f").await.unwrap(), 2);
        let direct = Store::open_read_only(&destination.join(PROJECT_DATABASE))
            .await
            .unwrap();
        assert_eq!(direct.message_count("f").await.unwrap(), 2);
    }
}
