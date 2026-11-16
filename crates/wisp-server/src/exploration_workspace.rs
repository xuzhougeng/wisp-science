//! Persistent, non-Git workspace snapshots for exploration branches, copied
//! from the WebView's `src-tauri/src/exploration_workspace.rs`.
//!
//! Snapshot blobs and materialized workspaces live under the application's
//! data directory. Writable workspaces are never hard-linked to the project or
//! blob store, and traversal never follows project symlinks.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::project_archive::workspace_scan::{
    scan_workspace, WorkspaceNode, WorkspaceNodeKind, WorkspaceScanOptions, MAX_WORKSPACE_ENTRIES,
};

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const REFERENCES_MANIFEST: &str = ".wisp/exploration-references.json";
const DEFAULT_BLOB_LIMIT: u64 = 32 * 1024 * 1024;
const DEFAULT_BLOB_BUDGET: u64 = 64 * 1024 * 1024;
const DEFAULT_BLOB_COUNT_LIMIT: usize = 4_096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SnapshotMaterialization {
    Blob,
    Reference,
    RemoteReference,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkspaceSnapshotEntry {
    pub path: String,
    pub size_bytes: u64,
    pub checksum: Option<String>,
    pub executable: bool,
    pub materialization: SnapshotMaterialization,
    pub reference_uri: Option<String>,
    pub recoverable: bool,
    pub modified_unix_millis: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkspaceSnapshot {
    pub schema_version: u32,
    pub id: String,
    pub project_key: String,
    pub source_root: String,
    pub manifest_sha256: String,
    pub entries: Vec<WorkspaceSnapshotEntry>,
    pub warnings: Vec<String>,
    pub created_at: i64,
}

impl WorkspaceSnapshot {
    fn calculate_manifest_sha256(&self) -> Result<String, String> {
        let mut unhashed = self.clone();
        unhashed.manifest_sha256.clear();
        let encoded = serde_json::to_vec(&unhashed).map_err(|error| error.to_string())?;
        Ok(hex::encode(Sha256::digest(encoded)))
    }

    fn verify_manifest(&self) -> Result<(), String> {
        if self.schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(format!(
                "unsupported workspace snapshot schema {}",
                self.schema_version
            ));
        }
        if self.calculate_manifest_sha256()? != self.manifest_sha256 {
            return Err("workspace snapshot manifest checksum mismatch".into());
        }
        validate_component("snapshot id", &self.id)?;
        validate_component("project key", &self.project_key)?;
        validate_entries(&self.entries)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MaterializedWorkspace {
    pub exploration_id: String,
    pub project_key: String,
    pub snapshot_id: String,
    pub root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FileDeltaKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileDelta {
    pub path: String,
    pub kind: FileDeltaKind,
    pub before: Option<WorkspaceSnapshotEntry>,
    pub after: Option<WorkspaceSnapshotEntry>,
}

#[async_trait]
pub(crate) trait ExplorationWorkspaceBackend: Send + Sync {
    async fn checkpoint(&self, project_root: &Path) -> Result<WorkspaceSnapshot, String>;
    async fn materialize(
        &self,
        snapshot: &WorkspaceSnapshot,
        exploration_id: &str,
    ) -> Result<MaterializedWorkspace, String>;
    async fn diff(&self, base: &WorkspaceSnapshot, root: &Path) -> Result<Vec<FileDelta>, String>;
    async fn dispose(&self, workspace: &MaterializedWorkspace) -> Result<(), String>;
}

#[derive(Clone, Debug)]
pub(crate) struct PersistentExplorationWorkspace {
    app_data_root: PathBuf,
    blob_limit: u64,
    blob_budget: u64,
    blob_count_limit: usize,
    max_entries: usize,
}

impl PersistentExplorationWorkspace {
    pub(crate) fn new(app_data_root: PathBuf) -> Self {
        Self {
            app_data_root,
            blob_limit: DEFAULT_BLOB_LIMIT,
            blob_budget: DEFAULT_BLOB_BUDGET,
            blob_count_limit: DEFAULT_BLOB_COUNT_LIMIT,
            max_entries: MAX_WORKSPACE_ENTRIES,
        }
    }

    #[cfg(test)]
    fn with_limits(app_data_root: PathBuf, blob_limit: u64, max_entries: usize) -> Self {
        Self {
            app_data_root,
            blob_limit,
            blob_budget: u64::MAX,
            blob_count_limit: usize::MAX,
            max_entries,
        }
    }

    #[cfg(test)]
    fn with_snapshot_limits(
        app_data_root: PathBuf,
        blob_limit: u64,
        blob_budget: u64,
        blob_count_limit: usize,
    ) -> Self {
        Self {
            app_data_root,
            blob_limit,
            blob_budget,
            blob_count_limit,
            max_entries: MAX_WORKSPACE_ENTRIES,
        }
    }

    pub(crate) fn load_snapshot(&self, snapshot_id: &str) -> Result<WorkspaceSnapshot, String> {
        validate_component("snapshot id", snapshot_id)?;
        let path = self.manifest_root()?.join(format!("{snapshot_id}.json"));
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect workspace snapshot manifest: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("workspace snapshot manifest is not a regular file".into());
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("cannot read workspace snapshot manifest: {error}"))?;
        let snapshot: WorkspaceSnapshot =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if snapshot.id != snapshot_id {
            return Err("workspace snapshot manifest id mismatch".into());
        }
        snapshot.verify_manifest()?;
        Ok(snapshot)
    }

    /// Remove a resolved snapshot's app-data manifest. Callers must first
    /// verify that the snapshot no longer has a database owner.
    pub(crate) fn remove_snapshot_manifest(&self, snapshot_id: &str) -> Result<(), String> {
        validate_component("snapshot id", snapshot_id)?;
        let path = self.manifest_root()?.join(format!("{snapshot_id}.json"));
        remove_owned_regular_file(&path, "workspace snapshot manifest")
    }

    /// Remove an unreferenced content-addressed snapshot blob. Callers must
    /// compute references from the surviving database-owned manifests first.
    pub(crate) fn remove_snapshot_blob(&self, checksum: &str) -> Result<(), String> {
        let path = self.blob_path(checksum)?;
        remove_owned_regular_file(&path, "workspace snapshot blob")?;
        if let Some(prefix) = path.parent() {
            let _ = std::fs::remove_dir(prefix);
        }
        Ok(())
    }

    /// Retry removal of containers that were safely moved out of their
    /// canonical location but remained locked during a previous disposal.
    /// Only app-owned names with two valid UUID/components are eligible.
    pub(crate) fn sweep_disposal_quarantine(&self) -> Result<(), String> {
        let explorations_root = self.explorations_root()?;
        let mut errors = Vec::new();
        for project_entry in
            std::fs::read_dir(&explorations_root).map_err(|error| error.to_string())?
        {
            let project_entry = match project_entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };
            let project_name = project_entry.file_name();
            let Some(project_name) = project_name.to_str() else {
                continue;
            };
            if validate_component("project key", project_name).is_err() {
                continue;
            }
            let metadata = match std::fs::symlink_metadata(project_entry.path()) {
                Ok(metadata) => metadata,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(project_entry.path()) {
                Ok(entries) => entries,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        errors.push(error.to_string());
                        continue;
                    }
                };
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if !is_disposal_quarantine_name(name) {
                    continue;
                }
                let metadata = match std::fs::symlink_metadata(entry.path()) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        errors.push(error.to_string());
                        continue;
                    }
                };
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    errors.push(format!(
                        "refusing non-directory exploration quarantine: {}",
                        entry.path().display()
                    ));
                    continue;
                }
                if let Err(error) = std::fs::remove_dir_all(entry.path()) {
                    errors.push(format!("{}: {error}", entry.path().display()));
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "some exploration quarantine directories could not be removed: {}",
                errors.join("; ")
            ))
        }
    }

    fn scan_options(&self, cancel: Option<Arc<AtomicBool>>) -> WorkspaceScanOptions {
        WorkspaceScanOptions {
            excluded_relative_prefixes: vec![
                ".wisp/explorations".into(),
                ".wisp/history".into(),
                ".wisp/artifacts".into(),
                ".wisp/tool-output".into(),
                ".wisp/tmp".into(),
                ".wisp/undo".into(),
                REFERENCES_MANIFEST.into(),
            ],
            excluded_directory_names: vec![
                ".git".into(),
                ".hg".into(),
                ".svn".into(),
                ".pixi".into(),
                ".venv".into(),
                "venv".into(),
                "node_modules".into(),
                "target".into(),
                "__pycache__".into(),
                ".cache".into(),
                ".mypy_cache".into(),
                ".pytest_cache".into(),
                ".ruff_cache".into(),
            ],
            use_ignore_files: true,
            cancel,
            max_entries: self.max_entries,
            ..WorkspaceScanOptions::default()
        }
    }

    fn manifest_root(&self) -> Result<PathBuf, String> {
        secure_directory(&self.app_data_root, &["exploration-snapshots", "manifests"])
    }

    fn blob_root(&self) -> Result<PathBuf, String> {
        secure_directory(
            &self.app_data_root,
            &["exploration-snapshots", "blobs", "sha256"],
        )
    }

    fn explorations_root(&self) -> Result<PathBuf, String> {
        secure_directory(&self.app_data_root, &["explorations"])
    }

    fn capture_blob(
        &self,
        source: &Path,
        expected_size: u64,
        blob_root: &Path,
        temp_root: &Path,
        cancel: Option<&AtomicBool>,
    ) -> Result<String, String> {
        ensure_not_cancelled(cancel)?;
        let temp_path = temp_root.join(uuid::Uuid::new_v4().to_string());
        let source_before = std::fs::metadata(source).map_err(|error| error.to_string())?;
        if !source_before.is_file() || source_before.len() != expected_size {
            return Err(format!(
                "workspace file changed while checkpointing: {}",
                source.display()
            ));
        }

        let result = (|| {
            let mut input = BufReader::new(File::open(source).map_err(|error| error.to_string())?);
            let output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|error| error.to_string())?;
            let mut output = BufWriter::new(output);
            let mut digest = Sha256::new();
            let mut copied = 0u64;
            let mut buffer = [0u8; 128 * 1024];
            loop {
                ensure_not_cancelled(cancel)?;
                let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
                if read == 0 {
                    break;
                }
                copied = copied.saturating_add(read as u64);
                digest.update(&buffer[..read]);
                output
                    .write_all(&buffer[..read])
                    .map_err(|error| error.to_string())?;
            }
            output.flush().map_err(|error| error.to_string())?;
            output
                .get_ref()
                .sync_all()
                .map_err(|error| error.to_string())?;
            let source_after = std::fs::metadata(source).map_err(|error| error.to_string())?;
            if copied != expected_size || source_after.len() != expected_size {
                return Err(format!(
                    "workspace file changed while checkpointing: {}",
                    source.display()
                ));
            }

            let checksum = hex::encode(digest.finalize());
            let prefix = secure_child_directory(blob_root, &checksum[..2])?;
            let destination = prefix.join(&checksum);
            if destination.exists() {
                verify_blob(&destination, &checksum, expected_size, cancel)?;
                std::fs::remove_file(&temp_path).map_err(|error| error.to_string())?;
            } else {
                match std::fs::rename(&temp_path, &destination) {
                    Ok(()) => {}
                    Err(_) if destination.exists() => {
                        verify_blob(&destination, &checksum, expected_size, cancel)?;
                        std::fs::remove_file(&temp_path).map_err(|error| error.to_string())?;
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            Ok(checksum)
        })();
        if result.is_err() && temp_path.exists() {
            let _ = std::fs::remove_file(&temp_path);
        }
        result
    }

    fn blob_path(&self, checksum: &str) -> Result<PathBuf, String> {
        validate_checksum(checksum)?;
        Ok(self.blob_root()?.join(&checksum[..2]).join(checksum))
    }

    fn persist_manifest(&self, snapshot: &WorkspaceSnapshot) -> Result<(), String> {
        let manifest_root = self.manifest_root()?;
        let destination = manifest_root.join(format!("{}.json", snapshot.id));
        let temporary =
            manifest_root.join(format!(".{}.{}.tmp", snapshot.id, uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(snapshot).map_err(|error| error.to_string())?;
        let result = (|| {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| error.to_string())?;
            output
                .write_all(&bytes)
                .map_err(|error| error.to_string())?;
            output.sync_all().map_err(|error| error.to_string())?;
            std::fs::rename(&temporary, &destination).map_err(|error| error.to_string())
        })();
        if result.is_err() && temporary.exists() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }

    fn snapshot_entries(
        &self,
        nodes: Vec<WorkspaceNode>,
        capture_blobs: bool,
        previous: Option<&WorkspaceSnapshot>,
        cancel: Option<&AtomicBool>,
    ) -> Result<(Vec<WorkspaceSnapshotEntry>, Vec<String>), String> {
        reject_case_collisions(&nodes)?;
        let previous_entries = previous
            .map(|snapshot| {
                snapshot
                    .entries
                    .iter()
                    .map(|entry| (entry.path.as_str(), entry))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let mut selected_blob_paths = HashSet::new();
        let mut selected_blob_bytes = 0u64;
        let mut selected_blob_count = 0usize;
        let mut blob_candidates = nodes
            .iter()
            .filter(|node| {
                node.kind == WorkspaceNodeKind::File
                    && node.size_bytes <= self.blob_limit
                    && !has_windows_reserved_component(&node.relative_path)
            })
            .collect::<Vec<_>>();
        blob_candidates.sort_by(|left, right| {
            left.size_bytes
                .cmp(&right.size_bytes)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        // Keep existing strong entries stable before admitting new files. This
        // prevents an alphabetically earlier addition from reshuffling the
        // aggregate snapshot budget on every turn. Within each group, smallest
        // files go first so an initial snapshot recovers the most files.
        for preserve_existing in [true, false] {
            for node in &blob_candidates {
                if (previous_entries
                    .get(node.relative_path.as_str())
                    .is_some_and(|entry| entry.materialization == SnapshotMaterialization::Blob)
                    != preserve_existing)
                    || selected_blob_count >= self.blob_count_limit
                    || selected_blob_bytes
                        .checked_add(node.size_bytes)
                        .is_none_or(|size| size > self.blob_budget)
                {
                    continue;
                }
                selected_blob_paths.insert(node.relative_path.clone());
                selected_blob_bytes += node.size_bytes;
                selected_blob_count += 1;
            }
        }

        let blob_storage = if capture_blobs {
            Some((
                self.blob_root()?,
                secure_directory(
                    &self.app_data_root,
                    &["exploration-snapshots", "blobs", "tmp"],
                )?,
            ))
        } else {
            None
        };
        let mut entries = Vec::new();
        let mut warnings = Vec::new();
        let mut oversized_references = 0usize;
        let mut budget_references = 0usize;
        for node in nodes {
            ensure_not_cancelled(cancel)?;
            if node.kind == WorkspaceNodeKind::Directory {
                continue;
            }
            let modified_unix_millis = node.modified_unix_millis;
            let executable = node.mode.is_some_and(|mode| mode & 0o111 != 0);
            let entry = if has_windows_reserved_component(&node.relative_path) {
                warnings.push(format!(
                    "{} is not portable to Windows and was not materialized",
                    node.relative_path
                ));
                WorkspaceSnapshotEntry {
                    path: node.relative_path,
                    size_bytes: node.size_bytes,
                    checksum: None,
                    executable,
                    materialization: SnapshotMaterialization::Unsupported,
                    reference_uri: Some(node.path.to_string_lossy().into_owned()),
                    recoverable: false,
                    modified_unix_millis,
                }
            } else {
                match node.kind {
                    WorkspaceNodeKind::File
                        if selected_blob_paths.contains(node.relative_path.as_str()) =>
                    {
                        let checksum = if capture_blobs {
                            match self.reusable_blob_checksum(
                                previous_entries.get(node.relative_path.as_str()).copied(),
                                &node,
                                blob_storage
                                    .as_ref()
                                    .expect("blob storage exists when capture is enabled")
                                    .0
                                    .as_path(),
                            )? {
                                Some(checksum) => checksum,
                                None => self.capture_blob(
                                    &node.path,
                                    node.size_bytes,
                                    &blob_storage.as_ref().unwrap().0,
                                    &blob_storage.as_ref().unwrap().1,
                                    cancel,
                                )?,
                            }
                        } else {
                            hash_file(&node.path, node.size_bytes, cancel)?
                        };
                        WorkspaceSnapshotEntry {
                            path: node.relative_path,
                            size_bytes: node.size_bytes,
                            checksum: Some(checksum),
                            executable,
                            materialization: SnapshotMaterialization::Blob,
                            reference_uri: None,
                            recoverable: true,
                            modified_unix_millis,
                        }
                    }
                    WorkspaceNodeKind::File => {
                        if node.size_bytes > self.blob_limit {
                            oversized_references += 1;
                        } else {
                            budget_references += 1;
                        }
                        WorkspaceSnapshotEntry {
                            path: node.relative_path,
                            size_bytes: node.size_bytes,
                            checksum: None,
                            executable,
                            materialization: SnapshotMaterialization::Reference,
                            reference_uri: Some(node.path.to_string_lossy().into_owned()),
                            recoverable: false,
                            modified_unix_millis,
                        }
                    }
                    WorkspaceNodeKind::Symlink => {
                        let target = std::fs::read_link(&node.path)
                            .map(|target| target.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| node.path.to_string_lossy().into_owned());
                        warnings.push(format!(
                            "{} is a symlink and was not followed",
                            node.relative_path
                        ));
                        WorkspaceSnapshotEntry {
                            path: node.relative_path,
                            size_bytes: node.size_bytes,
                            checksum: None,
                            executable: false,
                            materialization: SnapshotMaterialization::Unsupported,
                            reference_uri: Some(target),
                            recoverable: false,
                            modified_unix_millis,
                        }
                    }
                    WorkspaceNodeKind::Other => {
                        warnings.push(format!(
                            "{} is not a regular file and was not materialized",
                            node.relative_path
                        ));
                        WorkspaceSnapshotEntry {
                            path: node.relative_path,
                            size_bytes: node.size_bytes,
                            checksum: None,
                            executable: false,
                            materialization: SnapshotMaterialization::Unsupported,
                            reference_uri: Some(node.path.to_string_lossy().into_owned()),
                            recoverable: false,
                            modified_unix_millis,
                        }
                    }
                    WorkspaceNodeKind::Directory => unreachable!(),
                }
            };
            entries.push(entry);
        }
        if oversized_references > 0 {
            warnings.push(format!(
                "{oversized_references} files exceed the per-file snapshot limit and remain weak external references"
            ));
        }
        if budget_references > 0 {
            warnings.push(format!(
                "{budget_references} files exceed the aggregate snapshot budget and remain weak external references"
            ));
        }
        validate_entries(&entries)?;
        Ok((entries, warnings))
    }

    fn reusable_blob_checksum(
        &self,
        previous: Option<&WorkspaceSnapshotEntry>,
        node: &WorkspaceNode,
        blob_root: &Path,
    ) -> Result<Option<String>, String> {
        let Some(previous) = previous else {
            return Ok(None);
        };
        if previous.materialization != SnapshotMaterialization::Blob
            || previous.size_bytes != node.size_bytes
            || previous.modified_unix_millis.is_none()
            || previous.modified_unix_millis != node.modified_unix_millis
        {
            return Ok(None);
        }
        let Some(checksum) = previous.checksum.as_deref() else {
            return Ok(None);
        };
        let blob = blob_root.join(&checksum[..2]).join(checksum);
        match std::fs::symlink_metadata(blob) {
            Ok(metadata)
                if !metadata.file_type().is_symlink()
                    && metadata.is_file()
                    && metadata.len() == node.size_bytes =>
            {
                Ok(Some(checksum.to_string()))
            }
            Ok(_) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(crate) async fn checkpoint_incremental(
        &self,
        project_root: &Path,
        previous: Option<&WorkspaceSnapshot>,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<WorkspaceSnapshot, String> {
        let backend = self.clone();
        let project_root = project_root.to_path_buf();
        let previous = previous.cloned();
        tokio::task::spawn_blocking(move || {
            backend.checkpoint_sync(&project_root, previous.as_ref(), cancel)
        })
        .await
        .map_err(|error| format!("workspace checkpoint task failed: {error}"))?
    }

    fn checkpoint_sync(
        &self,
        project_root: &Path,
        previous: Option<&WorkspaceSnapshot>,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<WorkspaceSnapshot, String> {
        ensure_not_cancelled(cancel.as_deref())?;
        let canonical_root = dunce::canonicalize(project_root)
            .map_err(|error| format!("cannot resolve project root: {error}"))?;
        let source_root = canonical_root.to_string_lossy().into_owned();
        let project_key = hex::encode(Sha256::digest(source_root.as_bytes()))[..16].to_string();
        if let Some(previous) = previous {
            previous.verify_manifest()?;
        }
        let previous = previous.filter(|snapshot| {
            snapshot.project_key == project_key && snapshot.source_root == source_root
        });
        let nodes = scan_workspace(&canonical_root, &self.scan_options(cancel.clone()))?;
        let (entries, warnings) =
            self.snapshot_entries(nodes, true, previous, cancel.as_deref())?;
        ensure_not_cancelled(cancel.as_deref())?;
        let mut snapshot = WorkspaceSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            project_key,
            source_root,
            manifest_sha256: String::new(),
            entries,
            warnings,
            created_at: chrono::Utc::now().timestamp(),
        };
        snapshot.manifest_sha256 = snapshot.calculate_manifest_sha256()?;
        snapshot.verify_manifest()?;
        ensure_not_cancelled(cancel.as_deref())?;
        self.persist_manifest(&snapshot)?;
        Ok(snapshot)
    }

    fn materialize_entry(
        &self,
        workspace_root: &Path,
        entry: &WorkspaceSnapshotEntry,
    ) -> Result<(), String> {
        if entry.materialization != SnapshotMaterialization::Blob {
            return Ok(());
        }
        let checksum = entry
            .checksum
            .as_deref()
            .ok_or_else(|| format!("snapshot blob {} has no checksum", entry.path))?;
        let blob = self.blob_path(checksum)?;
        verify_blob(&blob, checksum, entry.size_bytes, None)?;
        let destination = safe_join(workspace_root, &entry.path)?;
        let parent = destination
            .parent()
            .ok_or_else(|| "workspace destination has no parent".to_string())?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        copy_file_isolated(&blob, &destination)?;
        set_executable(&destination, entry.executable)?;
        Ok(())
    }
}

#[async_trait]
impl ExplorationWorkspaceBackend for PersistentExplorationWorkspace {
    async fn checkpoint(&self, project_root: &Path) -> Result<WorkspaceSnapshot, String> {
        self.checkpoint_incremental(project_root, None, None).await
    }

    async fn materialize(
        &self,
        snapshot: &WorkspaceSnapshot,
        exploration_id: &str,
    ) -> Result<MaterializedWorkspace, String> {
        snapshot.verify_manifest()?;
        validate_component("exploration id", exploration_id)?;
        let explorations_root = self.explorations_root()?;
        let project_root = secure_child_directory(&explorations_root, &snapshot.project_key)?;
        let final_container = project_root.join(exploration_id);
        if final_container.exists() {
            return Err(format!(
                "exploration workspace already exists: {}",
                final_container.display()
            ));
        }
        let staging_container = project_root.join(format!(
            ".{exploration_id}.creating-{}",
            uuid::Uuid::new_v4()
        ));
        let workspace_root = staging_container.join("workspace");
        std::fs::create_dir(&staging_container).map_err(|error| error.to_string())?;
        let result = (|| {
            std::fs::create_dir(&workspace_root).map_err(|error| error.to_string())?;
            for entry in &snapshot.entries {
                self.materialize_entry(&workspace_root, entry)?;
            }
            let references = snapshot
                .entries
                .iter()
                .filter(|entry| entry.materialization != SnapshotMaterialization::Blob)
                .cloned()
                .collect::<Vec<_>>();
            let references_path = safe_join(&workspace_root, REFERENCES_MANIFEST)?;
            std::fs::create_dir_all(
                references_path
                    .parent()
                    .ok_or_else(|| "reference manifest has no parent".to_string())?,
            )
            .map_err(|error| error.to_string())?;
            let bytes = serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 1,
                "snapshot_id": snapshot.id,
                "references": references,
            }))
            .map_err(|error| error.to_string())?;
            std::fs::write(&references_path, bytes).map_err(|error| error.to_string())?;
            std::fs::rename(&staging_container, &final_container).map_err(|error| error.to_string())
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_dir_all(&staging_container);
            return Err(error);
        }
        Ok(MaterializedWorkspace {
            exploration_id: exploration_id.into(),
            project_key: snapshot.project_key.clone(),
            snapshot_id: snapshot.id.clone(),
            root: final_container.join("workspace"),
        })
    }

    async fn diff(&self, base: &WorkspaceSnapshot, root: &Path) -> Result<Vec<FileDelta>, String> {
        let backend = self.clone();
        let base = base.clone();
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            base.verify_manifest()?;
            let canonical_root = dunce::canonicalize(root)
                .map_err(|error| format!("cannot resolve exploration workspace: {error}"))?;
            let nodes = scan_workspace(&canonical_root, &backend.scan_options(None))?;
            let (entries, _) = backend.snapshot_entries(nodes, false, Some(&base), None)?;
            Ok(diff_entries(&base.entries, &entries, false))
        })
        .await
        .map_err(|error| format!("workspace diff task failed: {error}"))?
    }

    async fn dispose(&self, workspace: &MaterializedWorkspace) -> Result<(), String> {
        validate_component("exploration id", &workspace.exploration_id)?;
        validate_component("project key", &workspace.project_key)?;
        let explorations_root = self.explorations_root()?;
        let expected_container = explorations_root
            .join(&workspace.project_key)
            .join(&workspace.exploration_id);
        let expected_workspace = expected_container.join("workspace");
        if workspace.root != expected_workspace {
            return Err("refusing to dispose a path outside the exploration workspace root".into());
        }
        ensure_real_directory(&explorations_root)?;
        ensure_real_directory(
            expected_container
                .parent()
                .ok_or_else(|| "exploration container has no parent".to_string())?,
        )?;
        ensure_real_directory(&expected_container)?;
        ensure_real_directory(&expected_workspace)?;

        let quarantine = expected_container.parent().unwrap().join(format!(
            ".{}.disposing-{}",
            workspace.exploration_id,
            uuid::Uuid::new_v4()
        ));
        std::fs::rename(&expected_container, &quarantine).map_err(|error| {
            format!("cannot quarantine exploration workspace before disposal: {error}")
        })?;
        std::fs::remove_dir_all(&quarantine).map_err(|error| {
            format!(
                "exploration workspace is quarantined at {} but cleanup failed: {error}",
                quarantine.display()
            )
        })
    }
}

fn validate_entries(entries: &[WorkspaceSnapshotEntry]) -> Result<(), String> {
    let mut previous: Option<&str> = None;
    for entry in entries {
        validate_relative_path(&entry.path)?;
        if previous.is_some_and(|previous| previous >= entry.path.as_str()) {
            return Err("workspace snapshot entries must be uniquely sorted".into());
        }
        if entry.materialization == SnapshotMaterialization::Blob {
            let checksum = entry
                .checksum
                .as_deref()
                .ok_or_else(|| format!("blob entry {} has no checksum", entry.path))?;
            validate_checksum(checksum)?;
            if !entry.recoverable || entry.reference_uri.is_some() {
                return Err(format!(
                    "blob entry {} has invalid isolation metadata",
                    entry.path
                ));
            }
        }
        previous = Some(&entry.path);
    }
    Ok(())
}

fn remove_owned_regular_file(path: &Path, label: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!("{label} is not a regular file"));
            }
            std::fs::remove_file(path).map_err(|error| format!("cannot remove {label}: {error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect {label}: {error}")),
    }
}

fn entries_equivalent(before: &WorkspaceSnapshotEntry, after: &WorkspaceSnapshotEntry) -> bool {
    before.path == after.path
        && before.size_bytes == after.size_bytes
        && before.checksum == after.checksum
        && before.executable == after.executable
        && before.materialization == after.materialization
        && before.recoverable == after.recoverable
        && match before.materialization {
            SnapshotMaterialization::Blob => true,
            // A local weak reference can be copied into an exploration
            // workspace before it is used. Its absolute path then necessarily
            // changes from the mainline root to the private workspace root;
            // the stable identity is its workspace-relative path plus the
            // weak size/mtime fingerprint checked above and here.
            SnapshotMaterialization::Reference => {
                before.modified_unix_millis == after.modified_unix_millis
            }
            // Remote and unsupported references do not have a safe local
            // workspace-relative identity, so their target remains part of
            // the fingerprint.
            SnapshotMaterialization::RemoteReference | SnapshotMaterialization::Unsupported => {
                before.reference_uri == after.reference_uri
                    && before.modified_unix_millis == after.modified_unix_millis
            }
        }
}

fn diff_entries(
    before_entries: &[WorkspaceSnapshotEntry],
    after_entries: &[WorkspaceSnapshotEntry],
    include_reference_deletions: bool,
) -> Vec<FileDelta> {
    let before = before_entries
        .iter()
        .cloned()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let after = after_entries
        .iter()
        .cloned()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut paths = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let mut deltas = Vec::new();
    for path in paths {
        match (before.get(&path), after.get(&path)) {
            (None, Some(after)) => deltas.push(FileDelta {
                path,
                kind: FileDeltaKind::Added,
                before: None,
                after: Some(after.clone()),
            }),
            (Some(before), None)
                if include_reference_deletions
                    || before.materialization == SnapshotMaterialization::Blob =>
            {
                deltas.push(FileDelta {
                    path,
                    kind: FileDeltaKind::Deleted,
                    before: Some(before.clone()),
                    after: None,
                })
            }
            (Some(_), None) => {}
            (Some(before), Some(after)) if !entries_equivalent(before, after) => {
                deltas.push(FileDelta {
                    path,
                    kind: FileDeltaKind::Modified,
                    before: Some(before.clone()),
                    after: Some(after.clone()),
                })
            }
            _ => {}
        }
    }
    deltas
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.contains('\\') || Path::new(path).is_absolute() {
        return Err(format!("unsafe workspace-relative path: {path}"));
    }
    for component in Path::new(path).components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!("unsafe workspace-relative path: {path}"));
        }
    }
    Ok(())
}

fn validate_component(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(format!("invalid {label}"));
    }
    Ok(())
}

fn is_disposal_quarantine_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some((exploration_id, disposal_id)) = rest.split_once(".disposing-") else {
        return false;
    };
    validate_component("exploration id", exploration_id).is_ok()
        && uuid::Uuid::parse_str(exploration_id).is_ok()
        && uuid::Uuid::parse_str(disposal_id).is_ok()
}

fn validate_checksum(checksum: &str) -> Result<(), String> {
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid SHA-256 checksum".into());
    }
    Ok(())
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    validate_relative_path(relative)?;
    Ok(root.join(relative))
}

fn reject_case_collisions(nodes: &[WorkspaceNode]) -> Result<(), String> {
    let mut seen = HashMap::<String, &str>::new();
    for node in nodes {
        let folded = node.relative_path.to_lowercase();
        if let Some(previous) = seen.insert(folded, &node.relative_path) {
            if previous != node.relative_path {
                return Err(format!(
                    "workspace contains a case-insensitive path collision: {previous} and {}",
                    node.relative_path
                ));
            }
        }
    }
    Ok(())
}

fn has_windows_reserved_component(path: &str) -> bool {
    path.split('/').any(|component| {
        let stem = component
            .trim_end_matches([' ', '.'])
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
                })
    })
}

fn ensure_not_cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
        Err("workspace snapshot cancelled".into())
    } else {
        Ok(())
    }
}

fn hash_file(
    path: &Path,
    expected_size: u64,
    cancel: Option<&AtomicBool>,
) -> Result<String, String> {
    let mut input = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let mut digest = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        ensure_not_cancelled(cancel)?;
        let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        size = size.saturating_add(read as u64);
        digest.update(&buffer[..read]);
    }
    if size != expected_size {
        return Err(format!(
            "workspace file changed while hashing: {}",
            path.display()
        ));
    }
    Ok(hex::encode(digest.finalize()))
}

fn verify_blob(
    path: &Path,
    expected_checksum: &str,
    expected_size: u64,
    cancel: Option<&AtomicBool>,
) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("workspace snapshot blob is not a regular file".into());
    }
    let actual = hash_file(path, expected_size, cancel)?;
    if actual != expected_checksum {
        return Err("workspace snapshot blob checksum mismatch or corruption".into());
    }
    Ok(())
}

fn secure_directory(root: &Path, components: &[&str]) -> Result<PathBuf, String> {
    if !root.exists() {
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    }
    ensure_real_directory(root)?;
    let mut current = root.to_path_buf();
    for component in components {
        validate_component("storage directory component", component)?;
        current = secure_child_directory(&current, component)?;
    }
    Ok(current)
}

fn secure_child_directory(parent: &Path, component: &str) -> Result<PathBuf, String> {
    validate_component("storage directory component", component)?;
    ensure_real_directory(parent)?;
    let child = parent.join(component);
    match std::fs::symlink_metadata(&child) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(format!(
            "storage path is not a real directory: {}",
            child.display()
        )),
        Ok(_) => Ok(child),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::create_dir(&child) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
            ensure_real_directory(&child)?;
            Ok(child)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("path is not a real directory: {}", path.display()));
    }
    Ok(())
}

fn copy_file_isolated(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err(format!(
            "workspace destination already exists: {}",
            destination.display()
        ));
    }
    #[cfg(target_os = "macos")]
    if try_clonefile(source, destination) {
        return Ok(());
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if try_reflink(source, destination) {
        return Ok(());
    }
    if destination.exists() {
        std::fs::remove_file(destination).map_err(|error| error.to_string())?;
    }
    let filename = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "workspace destination filename is not valid Unicode".to_string())?;
    let temporary =
        destination.with_file_name(format!(".{filename}.copy-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        std::fs::copy(source, &temporary).map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, destination).map_err(|error| error.to_string())
    })();
    if result.is_err() && temporary.exists() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(target_os = "macos")]
fn try_clonefile(source: &Path, destination: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn clonefile(
            source: *const std::os::raw::c_char,
            destination: *const std::os::raw::c_char,
            flags: u32,
        ) -> i32;
    }
    let Ok(source) = CString::new(source.as_os_str().as_bytes()) else {
        return false;
    };
    let Ok(destination_value) = CString::new(destination.as_os_str().as_bytes()) else {
        return false;
    };
    let cloned = unsafe { clonefile(source.as_ptr(), destination_value.as_ptr(), 0) } == 0;
    if !cloned && destination.exists() {
        let _ = std::fs::remove_file(destination);
    }
    cloned
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn try_reflink(source: &Path, destination: &Path) -> bool {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn ioctl(
            fd: std::os::raw::c_int,
            request: std::os::raw::c_ulong,
            ...
        ) -> std::os::raw::c_int;
    }
    const FICLONE: std::os::raw::c_ulong = 0x4004_9409;
    let Ok(source) = File::open(source) else {
        return false;
    };
    let Ok(destination_file) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
    else {
        return false;
    };
    let cloned = unsafe { ioctl(destination_file.as_raw_fd(), FICLONE, source.as_raw_fd()) } == 0;
    drop(destination_file);
    if !cloned {
        let _ = std::fs::remove_file(destination);
    }
    cloned
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    let mut mode = metadata.permissions().mode();
    if executable {
        mode |= 0o100;
    } else {
        mode &= !0o111;
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let base = std::env::temp_dir().join(format!("wx_{label}_{}", &id[..8]));
        let project = base.join("project");
        let app_data = base.join("app-data");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();
        (base, project, app_data)
    }

    #[tokio::test]
    async fn two_materializations_are_independent_and_diff_is_scoped() {
        let (base, project, app_data) = roots("independent");
        std::fs::create_dir_all(project.join("data set")).unwrap();
        std::fs::write(project.join("data set/结果.txt"), b"baseline").unwrap();
        std::fs::write(project.join("untracked.txt"), b"untracked").unwrap();
        let backend = PersistentExplorationWorkspace::new(app_data);
        let snapshot = backend.checkpoint(&project).await.unwrap();
        let first = backend.materialize(&snapshot, "first").await.unwrap();
        let second = backend.materialize(&snapshot, "second").await.unwrap();

        std::fs::write(first.root.join("data set/结果.txt"), b"first").unwrap();
        std::fs::write(first.root.join("only-first.txt"), b"new").unwrap();
        std::fs::remove_file(first.root.join("untracked.txt")).unwrap();
        assert_eq!(
            std::fs::read(second.root.join("data set/结果.txt")).unwrap(),
            b"baseline"
        );
        assert!(second.root.join("untracked.txt").exists());
        assert!(!second.root.join("only-first.txt").exists());

        let diff = backend.diff(&snapshot, &first.root).await.unwrap();
        assert_eq!(
            diff.iter()
                .map(|delta| (&delta.path, &delta.kind))
                .collect::<Vec<_>>(),
            vec![
                (&"data set/结果.txt".to_string(), &FileDeltaKind::Modified),
                (&"only-first.txt".to_string(), &FileDeltaKind::Added),
                (&"untracked.txt".to_string(), &FileDeltaKind::Deleted),
            ]
        );
        assert!(backend
            .diff(&snapshot, &second.root)
            .await
            .unwrap()
            .is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let source_inode = std::fs::metadata(project.join("untracked.txt"))
                .unwrap()
                .ino();
            let workspace_inode = std::fs::metadata(second.root.join("untracked.txt"))
                .unwrap()
                .ino();
            assert_ne!(source_inode, workspace_inode);
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn references_are_explicit_and_internal_paths_are_excluded() {
        let (base, project, app_data) = roots("references");
        #[cfg(not(unix))]
        let _ = &app_data;
        std::fs::create_dir_all(project.join("nested/.git")).unwrap();
        std::fs::create_dir_all(project.join("nested/.pixi/cache")).unwrap();
        std::fs::create_dir_all(project.join("private")).unwrap();
        std::fs::write(project.join("nested/.git/secret"), b"hidden").unwrap();
        std::fs::write(project.join("nested/.pixi/cache/package"), b"hidden").unwrap();
        std::fs::write(project.join(".gitignore"), b"ignored.txt\n").unwrap();
        std::fs::write(project.join(".wispignore"), b"private/\n").unwrap();
        std::fs::write(project.join("ignored.txt"), b"hidden").unwrap();
        std::fs::write(project.join("private/secret.txt"), b"hidden").unwrap();
        std::fs::write(project.join("large.bin"), b"0123456789").unwrap();
        std::fs::write(project.join("CON.txt"), b"reserved").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("large.bin", project.join("large-link")).unwrap();
            let _listener =
                std::os::unix::net::UnixListener::bind(project.join("agent.sock")).unwrap();
            let backend = PersistentExplorationWorkspace::with_limits(app_data, 4, 100);
            let snapshot = backend.checkpoint(&project).await.unwrap();
            assert!(!snapshot
                .entries
                .iter()
                .any(|entry| entry.path.contains(".git/")));
            assert!(!snapshot
                .entries
                .iter()
                .any(|entry| entry.path.contains(".pixi/")));
            assert!(!snapshot
                .entries
                .iter()
                .any(|entry| entry.path == "ignored.txt" || entry.path.starts_with("private/")));
            assert_eq!(
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.path == "large.bin")
                    .unwrap()
                    .materialization,
                SnapshotMaterialization::Reference
            );
            assert_eq!(
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.path == "large-link")
                    .unwrap()
                    .materialization,
                SnapshotMaterialization::Unsupported
            );
            assert_eq!(
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.path == "agent.sock")
                    .unwrap()
                    .materialization,
                SnapshotMaterialization::Unsupported
            );
            assert_eq!(
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.path == "CON.txt")
                    .unwrap()
                    .materialization,
                SnapshotMaterialization::Unsupported
            );
            let workspace = backend.materialize(&snapshot, "refs").await.unwrap();
            assert!(!workspace.root.join("large.bin").exists());
            assert!(workspace.root.join(REFERENCES_MANIFEST).is_file());
            assert!(backend
                .diff(&snapshot, &workspace.root)
                .await
                .unwrap()
                .is_empty());
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn persisted_manifests_reload_and_corrupt_blobs_fail_closed() {
        let (base, project, app_data) = roots("reload");
        std::fs::write(project.join("result.txt"), b"verified").unwrap();
        let backend = PersistentExplorationWorkspace::new(app_data.clone());
        let snapshot = backend.checkpoint(&project).await.unwrap();
        let reopened = PersistentExplorationWorkspace::new(app_data);
        let loaded = reopened.load_snapshot(&snapshot.id).unwrap();
        assert_eq!(loaded, snapshot);
        let checksum = loaded.entries[0].checksum.as_deref().unwrap();
        std::fs::write(reopened.blob_path(checksum).unwrap(), b"corrupt").unwrap();
        let error = reopened.materialize(&loaded, "corrupt").await.unwrap_err();
        assert!(error.contains("checksum") || error.contains("changed while hashing"));
        assert!(!base
            .join("app-data/explorations")
            .join(&loaded.project_key)
            .join("corrupt")
            .exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn dispose_rejects_forged_paths_and_removes_only_the_branch() {
        let (base, project, app_data) = roots("dispose");
        std::fs::write(project.join("keep.txt"), b"keep").unwrap();
        let backend = PersistentExplorationWorkspace::new(app_data);
        let snapshot = backend.checkpoint(&project).await.unwrap();
        let workspace = backend.materialize(&snapshot, "branch").await.unwrap();
        let forged = MaterializedWorkspace {
            root: project.clone(),
            ..workspace.clone()
        };
        assert!(backend.dispose(&forged).await.is_err());
        assert!(project.join("keep.txt").exists());
        backend.dispose(&workspace).await.unwrap();
        assert!(!workspace.root.exists());
        assert!(project.join("keep.txt").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn resolved_snapshot_storage_removal_is_scoped_and_idempotent() {
        let (base, project, app_data) = roots("snapshot_cleanup");
        std::fs::write(project.join("keep.txt"), b"keep").unwrap();
        let backend = PersistentExplorationWorkspace::new(app_data);
        let snapshot = backend.checkpoint(&project).await.unwrap();
        let checksum = snapshot.entries[0].checksum.as_deref().unwrap();
        let manifest = backend
            .manifest_root()
            .unwrap()
            .join(format!("{}.json", snapshot.id));
        let blob = backend.blob_path(checksum).unwrap();

        backend.remove_snapshot_manifest(&snapshot.id).unwrap();
        backend.remove_snapshot_blob(checksum).unwrap();
        assert!(!manifest.exists());
        assert!(!blob.exists());
        assert!(project.join("keep.txt").exists());
        backend.remove_snapshot_manifest(&snapshot.id).unwrap();
        backend.remove_snapshot_blob(checksum).unwrap();
        assert!(backend.remove_snapshot_manifest("../escape").is_err());
        assert!(backend.remove_snapshot_blob("not-a-checksum").is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn startup_sweep_removes_only_strict_disposal_quarantines() {
        let (base, _project, app_data) = roots("quarantine");
        let backend = PersistentExplorationWorkspace::new(app_data);
        let project_root = backend
            .explorations_root()
            .unwrap()
            .join("0123456789abcdef");
        std::fs::create_dir_all(&project_root).unwrap();
        let exploration_id = uuid::Uuid::new_v4();
        let disposal_id = uuid::Uuid::new_v4();
        let quarantine = project_root.join(format!(".{exploration_id}.disposing-{disposal_id}"));
        let creating = project_root.join(format!(
            ".{exploration_id}.creating-{}",
            uuid::Uuid::new_v4()
        ));
        let forged = project_root.join(format!(".{exploration_id}.disposing-not-a-uuid"));
        std::fs::create_dir_all(quarantine.join("workspace")).unwrap();
        std::fs::create_dir_all(&creating).unwrap();
        std::fs::create_dir_all(&forged).unwrap();
        std::fs::write(quarantine.join("workspace/result.txt"), b"private").unwrap();

        backend.sweep_disposal_quarantine().unwrap();

        assert!(!quarantine.exists());
        assert!(creating.exists());
        assert!(forged.exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn entry_limit_and_case_collisions_fail_deterministically() {
        let (base, project, app_data) = roots("validation");
        let collision_nodes = vec![
            WorkspaceNode {
                path: project.join("Alpha"),
                relative_path: "Alpha".into(),
                kind: WorkspaceNodeKind::File,
                size_bytes: 1,
                mode: None,
                modified_unix_millis: None,
            },
            WorkspaceNode {
                path: project.join("alpha"),
                relative_path: "alpha".into(),
                kind: WorkspaceNodeKind::File,
                size_bytes: 1,
                mode: None,
                modified_unix_millis: None,
            },
        ];
        let error = reject_case_collisions(&collision_nodes).unwrap_err();
        assert!(error.contains("case-insensitive path collision"));

        std::fs::write(project.join("Alpha"), b"a").unwrap();
        std::fs::write(project.join("beta"), b"b").unwrap();
        let limited = PersistentExplorationWorkspace::with_limits(app_data, 1024, 1);
        let error = limited.checkpoint(&project).await.unwrap_err();
        assert!(error.contains("more than 1 entries"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn aggregate_budget_keeps_snapshot_capture_bounded() {
        let (base, project, app_data) = roots("budget");
        std::fs::write(project.join("a.txt"), b"aaaa").unwrap();
        std::fs::write(project.join("b.txt"), b"bbbb").unwrap();
        std::fs::write(project.join("c.txt"), b"cccc").unwrap();
        let backend = PersistentExplorationWorkspace::with_snapshot_limits(app_data, 10, 5, 10);
        let snapshot = backend.checkpoint(&project).await.unwrap();

        assert_eq!(
            snapshot
                .entries
                .iter()
                .filter(|entry| entry.materialization == SnapshotMaterialization::Blob)
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt"]
        );
        assert_eq!(
            snapshot
                .entries
                .iter()
                .filter(|entry| entry.materialization == SnapshotMaterialization::Reference)
                .count(),
            2
        );
        assert!(snapshot
            .warnings
            .iter()
            .any(|warning| warning.contains("2 files exceed the aggregate snapshot budget")));
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn incremental_checkpoint_keeps_existing_strong_entries_stable() {
        let (base, project, app_data) = roots("incremental-budget");
        std::fs::write(project.join("b.txt"), b"bbbb").unwrap();
        std::fs::write(project.join("c.txt"), b"cccc").unwrap();
        let backend = PersistentExplorationWorkspace::with_snapshot_limits(app_data, 10, 5, 10);
        let first = backend.checkpoint(&project).await.unwrap();
        assert_eq!(
            first
                .entries
                .iter()
                .find(|entry| entry.path == "b.txt")
                .unwrap()
                .materialization,
            SnapshotMaterialization::Blob
        );

        // A new path that sorts first must not evict an unchanged strong file.
        std::fs::write(project.join("a.txt"), b"aa").unwrap();
        let second = backend
            .checkpoint_incremental(&project, Some(&first), None)
            .await
            .unwrap();
        assert_eq!(
            second
                .entries
                .iter()
                .find(|entry| entry.path == "b.txt")
                .unwrap()
                .materialization,
            SnapshotMaterialization::Blob
        );
        assert_eq!(
            second
                .entries
                .iter()
                .find(|entry| entry.path == "a.txt")
                .unwrap()
                .materialization,
            SnapshotMaterialization::Reference
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn checkpoint_observes_cancellation_before_scanning() {
        let (base, project, app_data) = roots("cancel");
        std::fs::write(project.join("file.txt"), b"content").unwrap();
        let backend = PersistentExplorationWorkspace::new(app_data);
        let cancel = Arc::new(AtomicBool::new(true));

        let error = backend
            .checkpoint_incremental(&project, None, Some(cancel))
            .await
            .unwrap_err();
        assert!(error.contains("workspace snapshot cancelled"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn referenced_file_fingerprint_ignores_workspace_root_but_includes_modified_time() {
        let before = WorkspaceSnapshotEntry {
            path: "large.bin".into(),
            size_bytes: DEFAULT_BLOB_LIMIT + 1,
            checksum: None,
            executable: false,
            materialization: SnapshotMaterialization::Reference,
            reference_uri: Some("/project/large.bin".into()),
            recoverable: false,
            modified_unix_millis: Some(10),
        };
        let mut after = before.clone();
        after.reference_uri = Some("/private/exploration/workspace/large.bin".into());
        assert!(entries_equivalent(&before, &after));

        after.modified_unix_millis = Some(11);
        assert!(!entries_equivalent(&before, &after));
    }

    #[test]
    fn unsupported_reference_fingerprint_still_includes_its_target() {
        let before = WorkspaceSnapshotEntry {
            path: "large-link".into(),
            size_bytes: 0,
            checksum: None,
            executable: false,
            materialization: SnapshotMaterialization::Unsupported,
            reference_uri: Some("first-target".into()),
            recoverable: false,
            modified_unix_millis: Some(10),
        };
        let mut after = before.clone();
        after.reference_uri = Some("second-target".into());
        assert!(!entries_equivalent(&before, &after));
    }
}
