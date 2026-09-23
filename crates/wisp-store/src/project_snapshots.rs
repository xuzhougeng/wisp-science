//! Cloud-drive project folders. A sync client cannot copy an open SQLite
//! database (and its WAL) consistently, so the live database stays in a
//! device-local cache and complete, immutable snapshots are published into
//! `.wisp/revisions`. The client only ever moves finished files.
//!
//! Revisions form a DAG through `parents`; there is no mutable head file for
//! the client to merge. More than one tip means two devices published from the
//! same base, which is reported as a conflict instead of being overwritten.
use super::{project_storage, ProjectSyncState, Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const WORKSPACE_TRANSPORT: &str = "workspace";
pub(super) const REVISIONS: &str = ".wisp/revisions";
pub(super) const SNAPSHOT_METADATA_VERSION: u32 = 2;
/// Descriptors are tiny and let a lagging device prove descent; snapshots are
/// full databases, so only the head and its parents keep one.
const KEPT_DESCRIPTOR_DEPTH: usize = 64;
pub const FOLDER_WAITING: &str = "project_folder_waiting:";

/// Length and modification time of the cache database and its WAL.
pub(super) type Fingerprint = Vec<Option<(u64, SystemTime)>>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SnapshotRevision {
    project_id: String,
    revision_id: String,
    #[serde(default)]
    parents: Vec<String>,
    device_id: String,
    created_at: i64,
    size: u64,
    database_sha256: String,
    /// `Store::portable_project_database_hash`; independent of file layout.
    state_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSyncOutcome {
    /// `published`, `pulled`, `up-to-date`, `recovered` or `remote-newer`.
    pub status: &'static str,
    pub revision: Option<String>,
}

struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn waiting(detail: &str) -> anyhow::Error {
    anyhow::anyhow!("{FOLDER_WAITING} {detail}")
}

fn conflict() -> anyhow::Error {
    anyhow::anyhow!(
        "Sync conflict: this device and another device both changed the project folder. No data was overwritten."
    )
}

fn read_revisions(
    workspace: &Path,
    project_id: &str,
) -> Result<BTreeMap<String, SnapshotRevision>> {
    let mut revisions = BTreeMap::new();
    let entries = match std::fs::read_dir(workspace.join(REVISIONS)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(revisions),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Cloud clients add conflict copies ("x (1).json") and partial files.
        // Only exact, canonical names are revisions.
        let Some(id) = name.strip_suffix(".json").filter(|id| canonical_id(id)) else {
            continue;
        };
        if entry.metadata()?.len() > 64 * 1024 {
            continue;
        }
        let Ok(revision) =
            serde_json::from_slice::<SnapshotRevision>(&std::fs::read(entry.path())?)
        else {
            continue;
        };
        // Parents become file names when pruning; a shared folder must not be
        // able to name a path outside `.wisp/revisions`.
        if revision.revision_id == id
            && revision.project_id == project_id
            && revision.parents.iter().all(|parent| canonical_id(parent))
        {
            revisions.insert(id.to_owned(), revision);
        }
    }
    Ok(revisions)
}

fn canonical_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.hyphenated().to_string() == id)
}

fn tips(revisions: &BTreeMap<String, SnapshotRevision>) -> Vec<&SnapshotRevision> {
    let parents: std::collections::BTreeSet<&str> = revisions
        .values()
        .flat_map(|revision| revision.parents.iter().map(String::as_str))
        .collect();
    revisions
        .values()
        .filter(|revision| !parents.contains(revision.revision_id.as_str()))
        .collect()
}

/// Every id reachable from `from` (inclusive) with its distance. Parents whose
/// descriptors were pruned are still included; they just are not expanded.
fn ancestors(
    revisions: &BTreeMap<String, SnapshotRevision>,
    from: &str,
) -> BTreeMap<String, usize> {
    let mut depths = BTreeMap::from([(from.to_owned(), 0)]);
    let mut queue = VecDeque::from([from.to_owned()]);
    while let Some(id) = queue.pop_front() {
        let depth = depths[&id];
        for parent in revisions
            .get(&id)
            .map(|r| r.parents.as_slice())
            .unwrap_or_default()
        {
            if !depths.contains_key(parent) {
                depths.insert(parent.clone(), depth + 1);
                queue.push_back(parent.clone());
            }
        }
    }
    depths
}

fn snapshot_path(workspace: &Path, revision: &str) -> PathBuf {
    workspace.join(REVISIONS).join(format!("{revision}.sqlite"))
}

fn snapshot_arrived(workspace: &Path, revision: &SnapshotRevision) -> bool {
    std::fs::metadata(snapshot_path(workspace, &revision.revision_id))
        .is_ok_and(|metadata| metadata.len() == revision.size)
}

/// Write beside the target and rename, so the folder never shows a partial file.
fn write_atomic(target: &Path, write: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    let parent = target.parent().context("revision path has no parent")?;
    let partial = parent.join(format!(".{}.partial", uuid::Uuid::new_v4()));
    let result = write(&partial)
        .and_then(|()| {
            Ok(std::fs::OpenOptions::new()
                .write(true)
                .open(&partial)?
                .sync_all()?)
        })
        .and_then(|()| Ok(std::fs::rename(&partial, target)?));
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

fn fingerprint(database: &Path) -> Fingerprint {
    ["", "-wal"]
        .iter()
        .map(|suffix| {
            let metadata = std::fs::metadata(format!("{}{suffix}", database.display())).ok()?;
            Some((metadata.len(), metadata.modified().ok()?))
        })
        .collect()
}

impl Store {
    /// Scratch and cache files live beside the application database, never
    /// in the synchronized folder.
    async fn application_dir(&self) -> Result<PathBuf> {
        anyhow::ensure!(
            self.registry.is_some() && self.project_scope.is_none(),
            "Application store required"
        );
        Ok(self
            .database_path()
            .await?
            .parent()
            .context("application database has no directory")?
            .to_path_buf())
    }

    async fn scratch_file(&self, label: &str) -> Result<TempFile> {
        let dir = self.application_dir().await?;
        Ok(TempFile(
            dir.join(format!("{label}-{}.sqlite", uuid::Uuid::new_v4())),
        ))
    }

    async fn cache_database(&self, project_id: &str) -> Result<PathBuf> {
        let dir = self.application_dir().await?.join("project-cache");
        std::fs::create_dir_all(&dir)?;
        // A fresh name per registration: a stale cache left by a removed
        // registration may hold unpublished edits and is never overwritten.
        let label: String = project_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(64)
            .collect();
        Ok(dir.join(format!("{label}-{}.sqlite", uuid::Uuid::new_v4())))
    }

    async fn registered_database(&self, project_id: &str) -> Result<PathBuf> {
        let path: String =
            sqlx::query_scalar("SELECT database_path FROM project_locations WHERE project_id=?")
                .bind(project_id)
                .fetch_one(&self.pool)
                .await
                .context("Project storage is not registered")?;
        Ok(PathBuf::from(path))
    }

    async fn folder_cursor(&self, project_id: &str) -> Result<Option<(ProjectSyncState, PathBuf)>> {
        let Some(cursor) = self
            .get_project_sync_state(project_id)
            .await?
            .filter(|state| state.transport_kind == WORKSPACE_TRANSPORT)
        else {
            return Ok(None);
        };
        let (_, workspace) = self
            .get_project(project_id)
            .await?
            .context("Project not found")?;
        Ok(Some((cursor, PathBuf::from(workspace))))
    }

    fn record_published(&self, project_id: &str, value: Fingerprint) {
        if let Some(registry) = &self.registry {
            registry
                .published
                .lock()
                .unwrap()
                .insert(project_id.to_owned(), value);
        }
    }

    /// Projects whose live database is a local cache of a cloud-drive folder.
    pub async fn folder_snapshot_projects(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT project_id FROM project_sync_state WHERE transport_kind=?")
                .bind(WORKSPACE_TRANSPORT)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// Cheap status for listings: reads descriptors and file metadata only.
    /// `unpublished` needs a cache write since the last publish on this run;
    /// before the first check after startup, the folder state is reported.
    pub async fn folder_snapshot_status(&self, project_id: &str) -> Result<Option<&'static str>> {
        let Some((cursor, workspace)) = self.folder_cursor(project_id).await? else {
            return Ok(None);
        };
        let revisions = read_revisions(&workspace, project_id)?;
        let changed = {
            let current = fingerprint(&self.registered_database(project_id).await?);
            let registry = self
                .registry
                .as_ref()
                .context("Application store required")?;
            let published = registry.published.lock().unwrap();
            published
                .get(project_id)
                .is_some_and(|last| *last != current)
        };
        let base = cursor.base_revision.as_deref();
        Ok(Some(match tips(&revisions).as_slice() {
            [] if base.is_none() => "unpublished",
            [] => "waiting",
            [tip] if Some(tip.revision_id.as_str()) == base => {
                if changed {
                    "unpublished"
                } else {
                    "saved"
                }
            }
            [tip]
                if base.is_some_and(|base| {
                    ancestors(&revisions, &tip.revision_id).contains_key(base)
                }) =>
            {
                if snapshot_arrived(&workspace, tip) {
                    "remote-newer"
                } else {
                    "waiting"
                }
            }
            _ => "conflict",
        }))
    }

    /// Move a registered project's live database out of its folder into the
    /// local cache and publish the first snapshot. Idempotent once enabled.
    pub async fn enable_folder_snapshots(
        &self,
        project_id: &str,
        device_id: &str,
    ) -> Result<FolderSyncOutcome> {
        self.application_dir().await?;
        if let Some(cursor) = self.get_project_sync_state(project_id).await? {
            anyhow::ensure!(
                cursor.transport_kind == WORKSPACE_TRANSPORT,
                "This project already uses device sync; it cannot also publish folder snapshots."
            );
            return self
                .sync_folder_snapshots(project_id, device_id, false, None)
                .await;
        }
        let (_, workspace) = self
            .get_project(project_id)
            .await?
            .context("Project not found")?;
        let workspace = PathBuf::from(workspace);
        let live = self.registered_database(project_id).await?;
        anyhow::ensure!(
            live == workspace.join(project_storage::PROJECT_DATABASE),
            "Project storage is not inside its folder"
        );
        anyhow::ensure!(
            read_revisions(&workspace, project_id)?.is_empty(),
            "The project folder already contains published versions; import the folder instead."
        );
        let source = self
            .route_project(project_id)
            .await?
            .context("Project storage is not registered")?;
        let cache = self.cache_database(project_id).await?;
        sqlx::query("VACUUM INTO ?")
            .bind(cache.to_string_lossy().as_ref())
            .execute(&source.pool)
            .await?;
        let cursor = ProjectSyncState::uninitialized(
            project_id,
            WORKSPACE_TRANSPORT,
            &workspace.to_string_lossy(),
        );
        // The cache becomes live in the same transaction that records the
        // folder cursor; the stale folder copy is removed after publishing.
        let mut tx = self.begin_write().await?;
        sqlx::query("UPDATE project_locations SET database_path=? WHERE project_id=?")
            .bind(cache.to_string_lossy().as_ref())
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO project_sync_state(project_id,transport_kind,transport_location,relay_project_id,base_manifest_json) VALUES(?,?,?,?,?)",
        )
        .bind(&cursor.project_id)
        .bind(&cursor.transport_kind)
        .bind(&cursor.transport_location)
        .bind(&cursor.relay_project_id)
        .bind(&cursor.base_manifest_json)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        let registry = self
            .registry
            .as_ref()
            .context("Application store required")?;
        if let Some(pool) = registry.pools.lock().await.remove(project_id) {
            pool.close().await;
        }
        source.pool.close().await;
        self.sync_folder_snapshots(project_id, device_id, false, None)
            .await
    }

    /// Publish local edits, pull a newer folder version when `allow_pull` and
    /// this device is clean, or report a conflict. `strategy` resolves one:
    /// `local` publishes this device's state over every tip, `remote` adopts
    /// the newest complete tip.
    pub async fn sync_folder_snapshots(
        &self,
        project_id: &str,
        device_id: &str,
        allow_pull: bool,
        strategy: Option<&str>,
    ) -> Result<FolderSyncOutcome> {
        let (mut cursor, workspace) = self
            .folder_cursor(project_id)
            .await?
            .context("This project does not publish folder snapshots")?;
        let database = self.registered_database(project_id).await?;
        let revisions = read_revisions(&workspace, project_id)?;
        let tips = tips(&revisions);
        let tip_ids: Vec<String> = tips.iter().map(|tip| tip.revision_id.clone()).collect();
        let base = cursor.base_revision.clone();
        // Taken before the export: a write during it must look unpublished.
        let before = fingerprint(&database);
        let local = self.scratch_file("project-folder").await?;
        self.export_project_database(project_id, &local.0).await?;
        let local_hash = Store::portable_project_database_hash(&local.0).await?;
        let dirty = cursor.base_state_hash.as_deref() != Some(local_hash.as_str());
        let outcome = match (strategy, tips.as_slice()) {
            (Some("local"), _) => {
                self.publish_snapshot(
                    &mut cursor,
                    &workspace,
                    &local.0,
                    &local_hash,
                    tip_ids,
                    device_id,
                )
                .await?
            }
            (Some("remote"), []) => {
                return Err(waiting("no published version is in the folder yet"))
            }
            (Some("remote"), _) => {
                let newest = tips
                    .iter()
                    .copied()
                    .max_by_key(|tip| (tip.created_at, tip.revision_id.clone()))
                    .context("no tip")?;
                self.pull_snapshot(&mut cursor, &workspace, newest).await?;
                if tips.len() > 1 {
                    // Close the fork: the adopted state now supersedes every tip.
                    let merged = self.scratch_file("project-folder").await?;
                    self.export_project_database(project_id, &merged.0).await?;
                    let hash = Store::portable_project_database_hash(&merged.0).await?;
                    self.publish_snapshot(
                        &mut cursor,
                        &workspace,
                        &merged.0,
                        &hash,
                        tip_ids,
                        device_id,
                    )
                    .await?;
                }
                self.record_published(project_id, fingerprint(&database));
                return Ok(FolderSyncOutcome {
                    status: "pulled",
                    revision: cursor.base_revision,
                });
            }
            (Some(other), _) => anyhow::bail!("Unknown sync conflict strategy: {other}"),
            (None, []) if base.is_none() => {
                self.publish_snapshot(
                    &mut cursor,
                    &workspace,
                    &local.0,
                    &local_hash,
                    Vec::new(),
                    device_id,
                )
                .await?
            }
            (None, []) => {
                return Err(waiting(
                    "published versions are missing from the project folder",
                ))
            }
            (None, [tip]) if Some(&tip.revision_id) == base.as_ref() => {
                if dirty {
                    self.publish_snapshot(
                        &mut cursor,
                        &workspace,
                        &local.0,
                        &local_hash,
                        tip_ids,
                        device_id,
                    )
                    .await?
                } else {
                    finalize_folder(&workspace, project_id);
                    FolderSyncOutcome {
                        status: "up-to-date",
                        revision: base,
                    }
                }
            }
            // Our own publish landed but recording the cursor failed.
            (None, [tip])
                if tip.device_id == device_id
                    && tip.state_hash == local_hash
                    && tip.parents == base.clone().into_iter().collect::<Vec<_>>() =>
            {
                cursor.base_revision = Some(tip.revision_id.clone());
                cursor.base_state_hash = Some(local_hash);
                cursor.last_synced_at = Some(chrono::Utc::now().timestamp());
                cursor.last_direction = Some("push".into());
                self.upsert_project_sync_state(&cursor).await?;
                FolderSyncOutcome {
                    status: "recovered",
                    revision: cursor.base_revision.clone(),
                }
            }
            (None, [tip])
                if base.as_deref().is_some_and(|base| {
                    ancestors(&revisions, &tip.revision_id).contains_key(base)
                }) =>
            {
                if tip.state_hash == local_hash {
                    // Same records already (e.g. a merge that adopted ours):
                    // advance without replacing rows or reloading sessions.
                    cursor.base_revision = Some(tip.revision_id.clone());
                    cursor.last_synced_at = Some(chrono::Utc::now().timestamp());
                    self.upsert_project_sync_state(&cursor).await?;
                    self.record_published(project_id, before);
                    return Ok(FolderSyncOutcome {
                        status: "up-to-date",
                        revision: cursor.base_revision,
                    });
                }
                if dirty {
                    return Err(conflict());
                }
                if !allow_pull {
                    return Ok(FolderSyncOutcome {
                        status: "remote-newer",
                        revision: Some(tip.revision_id.clone()),
                    });
                }
                self.pull_snapshot(&mut cursor, &workspace, tip).await?;
                self.record_published(project_id, fingerprint(&database));
                return Ok(FolderSyncOutcome {
                    status: "pulled",
                    revision: cursor.base_revision,
                });
            }
            (None, _) => return Err(conflict()),
        };
        self.record_published(project_id, before);
        Ok(outcome)
    }

    async fn publish_snapshot(
        &self,
        cursor: &mut ProjectSyncState,
        workspace: &Path,
        snapshot: &Path,
        state_hash: &str,
        parents: Vec<String>,
        device_id: &str,
    ) -> Result<FolderSyncOutcome> {
        let project_id = cursor.project_id.clone();
        std::fs::create_dir_all(workspace.join(REVISIONS))?;
        let revision = SnapshotRevision {
            project_id: project_id.clone(),
            revision_id: uuid::Uuid::new_v4().to_string(),
            parents,
            device_id: device_id.to_owned(),
            created_at: chrono::Utc::now().timestamp(),
            size: std::fs::metadata(snapshot)?.len(),
            database_sha256: project_storage::database_digest(snapshot)?,
            state_hash: state_hash.to_owned(),
        };
        // Snapshot first, descriptor last: a descriptor names a complete file,
        // although a sync client may still deliver the two in either order.
        write_atomic(
            &snapshot_path(workspace, &revision.revision_id),
            |partial| {
                std::fs::copy(snapshot, partial)?;
                Ok(())
            },
        )?;
        let descriptor = workspace
            .join(REVISIONS)
            .join(format!("{}.json", revision.revision_id));
        write_atomic(&descriptor, |partial| {
            Ok(std::fs::write(
                partial,
                serde_json::to_vec_pretty(&revision)?,
            )?)
        })?;
        cursor.base_revision = Some(revision.revision_id.clone());
        cursor.base_state_hash = Some(state_hash.to_owned());
        cursor.last_synced_at = Some(chrono::Utc::now().timestamp());
        cursor.last_direction = Some("push".into());
        self.upsert_project_sync_state(cursor).await?;
        finalize_folder(workspace, &project_id);
        let mut revisions = read_revisions(workspace, &project_id)?;
        revisions.insert(revision.revision_id.clone(), revision.clone());
        prune_revisions(workspace, &revisions, &revision.revision_id);
        Ok(FolderSyncOutcome {
            status: "published",
            revision: Some(revision.revision_id),
        })
    }

    /// Verify a local copy of the tip before replacing any project row.
    async fn verified_snapshot(
        &self,
        workspace: &Path,
        tip: &SnapshotRevision,
    ) -> Result<TempFile> {
        if !snapshot_arrived(workspace, tip) {
            return Err(waiting("the latest version has not finished downloading"));
        }
        let copy = self.scratch_file("project-folder-pull").await?;
        std::fs::copy(snapshot_path(workspace, &tip.revision_id), &copy.0)?;
        if project_storage::database_digest(&copy.0)? != tip.database_sha256 {
            return Err(waiting("the latest version is still being downloaded"));
        }
        anyhow::ensure!(
            Store::portable_project_database_hash(&copy.0).await? == tip.state_hash,
            "The published project version failed integrity verification"
        );
        Ok(copy)
    }

    async fn pull_snapshot(
        &self,
        cursor: &mut ProjectSyncState,
        workspace: &Path,
        tip: &SnapshotRevision,
    ) -> Result<()> {
        let snapshot = self.verified_snapshot(workspace, tip).await?;
        cursor.base_revision = Some(tip.revision_id.clone());
        cursor.base_state_hash = Some(tip.state_hash.clone());
        cursor.last_synced_at = Some(chrono::Utc::now().timestamp());
        cursor.last_direction = Some("pull".into());
        let project_id = cursor.project_id.clone();
        self.replace_project_database(&snapshot.0, &project_id, workspace, cursor)
            .await
    }

    /// Open a snapshot-mode folder on this device: materialize a local cache
    /// from its single tip, then register it like any other project.
    pub(super) async fn register_snapshot_folder(
        &self,
        workspace: &Path,
        project_id: &str,
    ) -> Result<String> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?)")
            .bind(project_id)
            .fetch_one(&self.pool)
            .await?;
        anyhow::ensure!(!exists, "This project is already registered");
        let revisions = read_revisions(workspace, project_id)?;
        let tip = match tips(&revisions).as_slice() {
            [tip] => (*tip).clone(),
            [] => return Err(waiting("no published version is in the folder yet")),
            _ => anyhow::bail!(
                "Sync conflict: the folder holds versions from two devices. Press Sync now on a device where the project is open, then import the folder again."
            ),
        };
        let snapshot = self.verified_snapshot(workspace, &tip).await?;
        let cache = self.cache_database(project_id).await?;
        std::fs::copy(&snapshot.0, &cache)?;
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&cache)
            .create_if_missing(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Self::migrate(&pool).await?;
        let (name, description, created_at, updated_at): (String, String, i64, i64) =
            sqlx::query_as(
                "SELECT name,description,created_at,updated_at FROM projects WHERE id=?",
            )
            .bind(project_id)
            .fetch_one(&pool)
            .await?;
        let root = workspace.to_string_lossy();
        sqlx::query("UPDATE projects SET workspace_dir=?")
            .bind(root.as_ref())
            .execute(&pool)
            .await?;
        pool.close().await;
        let mut tx = self.begin_write().await?;
        sqlx::query("INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(project_id).bind(name).bind(description).bind(root.as_ref())
            .bind(created_at).bind(updated_at)
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO project_locations(project_id,database_path) VALUES(?,?)")
            .bind(project_id)
            .bind(cache.to_string_lossy().as_ref())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        // Replacing from the verified snapshot rebinds workspace paths, retires
        // another device's in-flight runs and records the cursor atomically.
        let mut cursor =
            ProjectSyncState::uninitialized(project_id, WORKSPACE_TRANSPORT, root.as_ref());
        cursor.base_revision = Some(tip.revision_id.clone());
        cursor.base_state_hash = Some(tip.state_hash.clone());
        cursor.last_synced_at = Some(chrono::Utc::now().timestamp());
        cursor.last_direction = Some("pull".into());
        if let Err(error) = self
            .replace_project_database(&snapshot.0, project_id, workspace, &cursor)
            .await
        {
            // Without its cursor the cache would look like an ordinary
            // project that never publishes. Undo; the folder is untouched.
            let _ = self.unregister_project_storage(project_id).await;
            let _ = std::fs::remove_file(&cache);
            return Err(error);
        }
        self.record_published(project_id, fingerprint(&cache));
        Ok(project_id.to_owned())
    }
}

/// Mark the folder as snapshot storage and drop its stale live database.
/// Older Wisp versions reject version 2 instead of opening a missing file.
/// Failure (e.g. a reader still holds the old file on Windows) loses nothing:
/// the cache is live and every later sync retries.
fn finalize_folder(workspace: &Path, project_id: &str) {
    if let Err(error) = try_finalize_folder(workspace, project_id) {
        tracing::warn!(project_id, %error, "Project folder cleanup deferred");
    }
}

fn try_finalize_folder(workspace: &Path, project_id: &str) -> Result<()> {
    let metadata_path = workspace.join(project_storage::PROJECT_METADATA);
    let current = std::fs::read(&metadata_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<project_storage::ProjectMetadata>(&bytes).ok());
    if current.is_none_or(|metadata| {
        metadata.version != SNAPSHOT_METADATA_VERSION || metadata.project_id != project_id
    }) {
        let metadata = project_storage::ProjectMetadata {
            format: "wisp-project".into(),
            version: SNAPSHOT_METADATA_VERSION,
            project_id: project_id.to_owned(),
        };
        write_atomic(&metadata_path, |partial| {
            Ok(std::fs::write(
                partial,
                serde_json::to_vec_pretty(&metadata)?,
            )?)
        })?;
    }
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let path = format!(
            "{}{suffix}",
            workspace.join(project_storage::PROJECT_DATABASE).display()
        );
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    Ok(())
}

/// Keep snapshots for the head and its parents, and descriptors for a bounded
/// history. Only known ancestors are touched: a file without a descriptor may
/// be another device's publish that the sync client has not finished moving.
fn prune_revisions(workspace: &Path, revisions: &BTreeMap<String, SnapshotRevision>, head: &str) {
    for (id, depth) in ancestors(revisions, head) {
        if !revisions.contains_key(&id) {
            continue;
        }
        if depth >= 2 {
            let _ = std::fs::remove_file(snapshot_path(workspace, &id));
        }
        if depth > KEPT_DESCRIPTOR_DEPTH {
            let _ = std::fs::remove_file(workspace.join(REVISIONS).join(format!("{id}.json")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PROJECT_DATABASE, PROJECT_METADATA};
    use wisp_llm::Message;

    struct Devices {
        _root: tempfile::TempDir,
        folder: PathBuf,
        a: Store,
        b: Store,
    }

    /// Device A creates the project in a "cloud" folder and enables snapshots;
    /// device B has its own application database and opens the same folder.
    async fn two_devices() -> Devices {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("Nutstore/Study");
        let a = Store::open_application(&root.path().join("a/wisp.sqlite"))
            .await
            .unwrap();
        a.create_project("p", "Study", folder.to_str().unwrap())
            .await
            .unwrap();
        a.create_frame("f", "p", "agent", "model").await.unwrap();
        a.append_message("f", 1, &Message::user("from A"))
            .await
            .unwrap();
        assert_eq!(
            a.enable_folder_snapshots("p", "device-a")
                .await
                .unwrap()
                .status,
            "published"
        );
        let b = Store::open_application(&root.path().join("b/wisp.sqlite"))
            .await
            .unwrap();
        assert_eq!(b.register_project_folder(&folder).await.unwrap(), "p");
        Devices {
            _root: root,
            folder,
            a,
            b,
        }
    }

    async fn sync(store: &Store, device: &str) -> Result<FolderSyncOutcome> {
        store.sync_folder_snapshots("p", device, true, None).await
    }

    #[tokio::test]
    async fn enabling_moves_the_live_database_out_of_the_cloud_folder() {
        let d = two_devices().await;
        for suffix in ["", "-wal", "-shm"] {
            assert!(!Path::new(&format!(
                "{}{suffix}",
                d.folder.join(PROJECT_DATABASE).display()
            ))
            .exists());
        }
        let metadata: project_storage::ProjectMetadata =
            serde_json::from_slice(&std::fs::read(d.folder.join(PROJECT_METADATA)).unwrap())
                .unwrap();
        assert_eq!(metadata.version, SNAPSHOT_METADATA_VERSION);
        let cache = d.a.registered_database("p").await.unwrap();
        assert!(!cache.starts_with(&d.folder));
        assert_eq!(d.a.message_count("f").await.unwrap(), 1);
        assert_eq!(d.b.message_count("f").await.unwrap(), 1);
        assert_eq!(
            d.b.get_project("p").await.unwrap().unwrap().1,
            std::fs::canonicalize(&d.folder).unwrap().to_string_lossy()
        );
        // Enabling again is a no-op check, not a second root revision.
        assert_eq!(
            d.a.enable_folder_snapshots("p", "device-a")
                .await
                .unwrap()
                .status,
            "up-to-date"
        );
        assert_eq!(tips(&read_revisions(&d.folder, "p").unwrap()).len(), 1);
    }

    #[tokio::test]
    async fn devices_hand_the_project_back_and_forth() {
        let d = two_devices().await;
        assert_eq!(sync(&d.b, "device-b").await.unwrap().status, "up-to-date");
        d.b.append_message("f", 2, &Message::user("from B"))
            .await
            .unwrap();
        assert_eq!(
            d.b.folder_snapshot_status("p").await.unwrap(),
            Some("unpublished")
        );
        assert_eq!(sync(&d.b, "device-b").await.unwrap().status, "published");
        assert_eq!(
            d.b.folder_snapshot_status("p").await.unwrap(),
            Some("saved")
        );
        assert_eq!(
            d.a.folder_snapshot_status("p").await.unwrap(),
            Some("remote-newer")
        );
        // Background publishing never swaps data under an open project.
        assert_eq!(
            d.a.sync_folder_snapshots("p", "device-a", false, None)
                .await
                .unwrap()
                .status,
            "remote-newer"
        );
        assert_eq!(sync(&d.a, "device-a").await.unwrap().status, "pulled");
        assert_eq!(d.a.message_count("f").await.unwrap(), 2);
        // A pulled state is clean: no spurious revision on the next check.
        assert_eq!(sync(&d.a, "device-a").await.unwrap().status, "up-to-date");
        assert_eq!(
            d.a.folder_snapshot_status("p").await.unwrap(),
            Some("saved")
        );
    }

    #[tokio::test]
    async fn edits_on_both_devices_conflict_until_one_is_chosen() {
        let d = two_devices().await;
        d.a.append_message("f", 2, &Message::user("A offline edit"))
            .await
            .unwrap();
        d.b.append_message("f", 2, &Message::user("B edit"))
            .await
            .unwrap();
        sync(&d.b, "device-b").await.unwrap();
        let error = sync(&d.a, "device-a").await.unwrap_err().to_string();
        assert!(error.contains("Sync conflict"), "{error}");
        let text = |messages: Vec<Message>| format!("{messages:?}");
        let a_messages = text(d.a.load_messages("f").await.unwrap());
        assert!(a_messages.contains("A offline edit"));
        d.a.sync_folder_snapshots("p", "device-a", true, Some("local"))
            .await
            .unwrap();
        assert_eq!(sync(&d.b, "device-b").await.unwrap().status, "pulled");
        assert_eq!(text(d.b.load_messages("f").await.unwrap()), a_messages);
    }

    #[tokio::test]
    async fn concurrent_publishes_form_a_fork_that_resolution_closes() {
        let d = two_devices().await;
        d.a.append_message("f", 2, &Message::user("A"))
            .await
            .unwrap();
        sync(&d.a, "device-a").await.unwrap();
        // The cloud client has not delivered A's revision to B yet.
        let dir = d.folder.join(REVISIONS);
        let a_head = d.a.get_project_sync_state("p").await.unwrap().unwrap();
        let a_descriptor = dir.join(format!("{}.json", a_head.base_revision.unwrap()));
        let hidden = d.folder.join("in-transit.json");
        std::fs::rename(&a_descriptor, &hidden).unwrap();
        d.b.append_message("f", 2, &Message::user("B"))
            .await
            .unwrap();
        assert_eq!(sync(&d.b, "device-b").await.unwrap().status, "published");
        std::fs::rename(&hidden, &a_descriptor).unwrap();
        for (store, device) in [(&d.a, "device-a"), (&d.b, "device-b")] {
            assert_eq!(
                store.folder_snapshot_status("p").await.unwrap(),
                Some("conflict")
            );
            assert!(sync(store, device)
                .await
                .unwrap_err()
                .to_string()
                .contains("Sync conflict"));
        }
        assert_eq!(
            d.b.sync_folder_snapshots("p", "device-b", true, Some("remote"))
                .await
                .unwrap()
                .status,
            "pulled"
        );
        assert_eq!(tips(&read_revisions(&d.folder, "p").unwrap()).len(), 1);
        // A adopts the merge: fast-forward if it kept A's records, else pull.
        sync(&d.a, "device-a").await.unwrap();
        let text = |messages: Vec<Message>| format!("{messages:?}");
        assert_eq!(
            text(d.a.load_messages("f").await.unwrap()),
            text(d.b.load_messages("f").await.unwrap())
        );
        for (store, device) in [(&d.a, "device-a"), (&d.b, "device-b")] {
            assert_eq!(sync(store, device).await.unwrap().status, "up-to-date");
        }
    }

    #[tokio::test]
    async fn an_incomplete_download_waits_without_touching_local_records() {
        let d = two_devices().await;
        d.b.append_message("f", 2, &Message::user("B"))
            .await
            .unwrap();
        let revision = sync(&d.b, "device-b").await.unwrap().revision.unwrap();
        let snapshot = snapshot_path(&d.folder, &revision);
        let complete = std::fs::read(&snapshot).unwrap();
        std::fs::write(&snapshot, &complete[..complete.len() / 2]).unwrap();
        assert_eq!(
            d.a.folder_snapshot_status("p").await.unwrap(),
            Some("waiting")
        );
        let error = sync(&d.a, "device-a").await.unwrap_err().to_string();
        assert!(error.starts_with(FOLDER_WAITING), "{error}");
        assert_eq!(d.a.message_count("f").await.unwrap(), 1);
        // Same size, different bytes: the checksum still refuses it.
        let mut corrupt = complete.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        std::fs::write(&snapshot, corrupt).unwrap();
        assert!(sync(&d.a, "device-a")
            .await
            .unwrap_err()
            .to_string()
            .starts_with(FOLDER_WAITING));
        std::fs::write(&snapshot, complete).unwrap();
        assert_eq!(sync(&d.a, "device-a").await.unwrap().status, "pulled");
        assert_eq!(d.a.message_count("f").await.unwrap(), 2);
    }

    #[tokio::test]
    async fn only_the_head_and_its_parents_keep_snapshots() {
        let d = two_devices().await;
        let mut heads = Vec::new();
        for seq in 2..6 {
            d.a.append_message("f", seq, &Message::user("edit"))
                .await
                .unwrap();
            heads.push(sync(&d.a, "device-a").await.unwrap().revision.unwrap());
        }
        let kept: Vec<_> = heads
            .iter()
            .map(|id| snapshot_path(&d.folder, id).exists())
            .collect();
        assert_eq!(kept, [false, false, true, true]);
        assert_eq!(read_revisions(&d.folder, "p").unwrap().len(), 5);
        // A conflict copy made by a sync client is not a competing revision.
        let head = heads.last().unwrap();
        std::fs::copy(
            d.folder.join(REVISIONS).join(format!("{head}.json")),
            d.folder.join(REVISIONS).join(format!("{head} (1).json")),
        )
        .unwrap();
        assert_eq!(sync(&d.b, "device-b").await.unwrap().status, "pulled");
    }

    #[tokio::test]
    async fn a_shared_folder_cannot_steer_pruning_outside_revisions() {
        let d = two_devices().await;
        let outside = d.folder.parent().unwrap().join("thesis.sqlite");
        std::fs::write(&outside, b"not Wisp's").unwrap();
        let hostile = SnapshotRevision {
            project_id: "p".into(),
            revision_id: uuid::Uuid::new_v4().to_string(),
            parents: vec!["../../thesis".into()],
            device_id: "elsewhere".into(),
            created_at: 0,
            size: 0,
            database_sha256: String::new(),
            state_hash: String::new(),
        };
        std::fs::write(
            d.folder
                .join(REVISIONS)
                .join(format!("{}.json", hostile.revision_id)),
            serde_json::to_vec(&hostile).unwrap(),
        )
        .unwrap();
        for seq in 2..5 {
            d.a.append_message("f", seq, &Message::user("edit"))
                .await
                .unwrap();
            assert_eq!(sync(&d.a, "device-a").await.unwrap().status, "published");
        }
        assert!(outside.exists());
    }

    #[tokio::test]
    async fn an_offline_project_folder_does_not_hide_other_projects() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open_application(&root.path().join("global.sqlite"))
            .await
            .unwrap();
        for id in ["a", "b"] {
            store
                .create_project(id, id, root.path().join(id).to_str().unwrap())
                .await
                .unwrap();
        }
        std::fs::rename(root.path().join("b"), root.path().join("b-unplugged")).unwrap();
        let reopened = Store::open_application(&root.path().join("global.sqlite"))
            .await
            .unwrap();
        let ids: Vec<_> = reopened
            .list_projects()
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.0)
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(reopened.starred_project_ids().await.unwrap().is_empty());
        assert!(reopened.due_schedules(i64::MAX).await.unwrap().is_empty());
        assert!(reopened
            .create_frame("f", "b", "agent", "model")
            .await
            .unwrap_err()
            .to_string()
            .contains("unavailable"));
    }
}
