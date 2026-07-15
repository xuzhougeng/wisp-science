use super::{
    build_project_summary,
    project_transfer::{
        collect_workspace, directory_component, pick_import_parent, unique_destination,
        WorkspaceEntryKind,
    },
    AppState, ProjectSummary,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, State};
use wisp_store::{secrets::Secret, ProjectSyncState, Store};
use wisp_sync::{
    decrypt_blob, encrypt_blob, random_project_key, sha256_hex, sign_revision, verify_revision,
    CommitOutcome, CommitRequest, FileRelay, HttpRelay, SyncRevision, SyncTransport, WorkspaceFile,
    WorkspaceManifest, PROJECT_KEY_BYTES, SYNC_PROTOCOL_VERSION,
};

const MAX_SYNC_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SYNC_CHANGED_BLOBS_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SYNC_METADATA_BYTES: u64 = 192 * 1024 * 1024;
const MAX_SYNC_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
const JOIN_CODE_PREFIX: &str = "wisp-sync:";
const RELAY_TOKEN_SECRET: &str = "sync_relay_token";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectSyncResult {
    status: String,
    direction: String,
    revision: Option<String>,
    uploaded_files: usize,
    downloaded_files: usize,
    skipped_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectSyncStatus {
    configured: bool,
    transport_kind: Option<String>,
    last_synced_at: Option<i64>,
    last_direction: Option<String>,
    revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JoinCode {
    version: u32,
    project_id: String,
    project_name: String,
    project_key: String,
    transport_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay_url: Option<String>,
}

struct TempPath(PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.to_string_lossy()));
        }
    }
}

struct LocalSnapshot {
    _database: TempPath,
    metadata: Vec<u8>,
    manifest: WorkspaceManifest,
    manifest_bytes: Vec<u8>,
    state_hash: String,
    changed_blobs: Vec<(String, Vec<u8>)>,
}

fn key_secret_name(project_id: &str) -> String {
    format!("sync_project_key:{project_id}")
}

async fn read_secret(name: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || Secret::get(&name))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

async fn write_secret(name: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || Secret::set(&name, &value))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

async fn delete_secret(name: String) {
    let _ = tokio::task::spawn_blocking(move || Secret::delete(&name)).await;
}

pub(super) async fn forget_project_key(project_id: &str) {
    delete_secret(key_secret_name(project_id)).await;
}

fn encode_key(key: &[u8; PROJECT_KEY_BYTES]) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

fn decode_key(encoded: &str) -> Result<[u8; PROJECT_KEY_BYTES], String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .map_err(|_| "Invalid project sync key.".to_string())?;
    bytes
        .try_into()
        .map_err(|_| "Invalid project sync key length.".to_string())
}

async fn load_project_key(project_id: &str) -> Result<[u8; PROJECT_KEY_BYTES], String> {
    decode_key(&read_secret(key_secret_name(project_id)).await?)
}

async fn create_project_key(project_id: &str) -> Result<[u8; PROJECT_KEY_BYTES], String> {
    let key = random_project_key().map_err(|error| error.to_string())?;
    write_secret(key_secret_name(project_id), encode_key(&key)).await?;
    Ok(key)
}

fn compute_state_hash(
    metadata_fingerprint: &str,
    manifest: &WorkspaceManifest,
) -> Result<String, String> {
    // Ciphertext blob ids intentionally do not define project content. A lost
    // local cursor may re-encrypt an unchanged file with a fresh nonce; paths
    // and plaintext hashes must still compare equal.
    let mut logical_manifest = manifest.clone();
    for file in &mut logical_manifest.files {
        file.blob_id.clear();
    }
    // Excluded paths are device-local observations. A second device does not
    // materialize a large/symlinked file, so its absence must not look like a
    // project edit or cause a conflict on the source device.
    logical_manifest.skipped_paths.clear();
    let manifest = serde_json::to_vec(&logical_manifest).map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(metadata_fingerprint.len() + manifest.len() + 1);
    bytes.extend_from_slice(metadata_fingerprint.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&manifest);
    Ok(sha256_hex(&bytes))
}

fn valid_windows_component(component: &str) -> bool {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.ends_with([' ', '.'])
        || component
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*'))
    {
        return false;
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

fn valid_portable_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && path
            .split('/')
            .all(|component| valid_windows_component(component))
}

fn valid_relay_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

fn workspace_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if !valid_portable_path(relative) {
        return Err(format!("Sync contains a non-portable path: {relative}"));
    }
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        path.push(component);
    }
    Ok(path)
}

fn validate_manifest(manifest: &WorkspaceManifest) -> Result<(), String> {
    if manifest.version != SYNC_PROTOCOL_VERSION {
        return Err("Unsupported workspace sync manifest version.".into());
    }
    if manifest.files.len() > 100_000 {
        return Err("Workspace sync manifest contains too many files.".into());
    }
    let mut paths = BTreeSet::new();
    let mut folded_paths = BTreeSet::new();
    for file in &manifest.files {
        if !valid_portable_path(&file.path)
            || !valid_hash(&file.sha256)
            || !valid_hash(&file.blob_id)
            || file.size > MAX_SYNC_FILE_BYTES
            || !paths.insert(file.path.as_str())
            || !folded_paths.insert(file.path.to_lowercase())
        {
            return Err("Invalid or duplicate path in workspace sync manifest.".into());
        }
    }
    Ok(())
}

fn reserve_changed_blob_bytes(current: u64, file_size: u64, limit: u64) -> Result<u64, String> {
    // The extra 64 bytes conservatively covers the current blob envelope and tag.
    current
        .checked_add(file_size)
        .and_then(|size| size.checked_add(64))
        .filter(|size| *size <= limit)
        .ok_or_else(|| "Changed workspace files exceed the 128 MiB sync upload limit.".to_string())
}

fn build_workspace_snapshot(
    workspace: &Path,
    excluded: &Path,
    base: &WorkspaceManifest,
    key: &[u8; PROJECT_KEY_BYTES],
) -> Result<(WorkspaceManifest, Vec<(String, Vec<u8>)>), String> {
    let collected = collect_workspace(workspace, excluded)?;
    let base_files = base
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let mut manifest = WorkspaceManifest {
        version: SYNC_PROTOCOL_VERSION,
        files: Vec::new(),
        skipped_paths: collected.skipped_paths,
    };
    let mut changed_blobs = Vec::new();
    let mut changed_blob_bytes = 0_u64;
    for entry in collected.entries {
        if !matches!(entry.kind, WorkspaceEntryKind::File) {
            continue;
        }
        if !valid_portable_path(&entry.archive_path) || entry.size > MAX_SYNC_FILE_BYTES {
            manifest.skipped_paths.push(entry.archive_path);
            continue;
        }
        let bytes = std::fs::read(&entry.source)
            .map_err(|error| format!("Cannot read {}: {error}", entry.source.display()))?;
        if bytes.len() as u64 != entry.size {
            return Err(format!(
                "Workspace file changed while sync was reading it: {}",
                entry.archive_path
            ));
        }
        let plaintext_hash = sha256_hex(&bytes);
        let previous = base_files.get(entry.archive_path.as_str()).copied();
        let blob_id = if let Some(previous) = previous.filter(|file| file.sha256 == plaintext_hash)
        {
            previous.blob_id.clone()
        } else {
            changed_blob_bytes = reserve_changed_blob_bytes(
                changed_blob_bytes,
                entry.size,
                MAX_SYNC_CHANGED_BLOBS_BYTES,
            )?;
            let encrypted = encrypt_blob(key, &bytes).map_err(|error| error.to_string())?;
            let blob_id = sha256_hex(&encrypted);
            changed_blobs.push((blob_id.clone(), encrypted));
            blob_id
        };
        let executable = entry
            .mode
            .map(|mode| mode & 0o111 != 0)
            .or_else(|| previous.and_then(|file| file.executable))
            .or(Some(false));
        manifest.files.push(WorkspaceFile {
            path: entry.archive_path,
            size: entry.size,
            sha256: plaintext_hash,
            blob_id,
            executable,
        });
    }
    manifest
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    let mut folded_paths = BTreeSet::new();
    if manifest
        .files
        .iter()
        .any(|file| !folded_paths.insert(file.path.to_lowercase()))
    {
        return Err("Workspace contains file paths that collide on Windows or macOS.".into());
    }
    manifest.skipped_paths.sort();
    manifest.skipped_paths.dedup();
    Ok((manifest, changed_blobs))
}

async fn build_local_snapshot(
    store: &Store,
    app_data: &Path,
    project_id: &str,
    workspace: &Path,
    base: &WorkspaceManifest,
    key: &[u8; PROJECT_KEY_BYTES],
) -> Result<LocalSnapshot, String> {
    std::fs::create_dir_all(app_data).map_err(|error| error.to_string())?;
    let database = TempPath(app_data.join(format!("project-sync-{}.sqlite", uuid::Uuid::new_v4())));
    store
        .export_project_database(project_id, &database.0)
        .await
        .map_err(|error| error.to_string())?;
    let metadata_len = std::fs::metadata(&database.0)
        .map_err(|error| error.to_string())?
        .len();
    if metadata_len > MAX_SYNC_METADATA_BYTES {
        return Err("Project metadata is too large to synchronize.".into());
    }
    let metadata_fingerprint = Store::portable_project_database_hash(&database.0)
        .await
        .map_err(|error| error.to_string())?;
    let workspace_owned = workspace.to_path_buf();
    let excluded = database.0.clone();
    let base_owned = base.clone();
    let key_owned = *key;
    let (manifest, changed_blobs) = tokio::task::spawn_blocking(move || {
        build_workspace_snapshot(&workspace_owned, &excluded, &base_owned, &key_owned)
    })
    .await
    .map_err(|error| error.to_string())??;
    validate_manifest(&manifest)?;
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    if manifest_bytes.len() > MAX_SYNC_MANIFEST_BYTES {
        return Err("Workspace sync manifest exceeds the local size limit.".into());
    }
    // Keep the database out of memory while workspace files are read and encrypted.
    let metadata = std::fs::read(&database.0).map_err(|error| error.to_string())?;
    let state_hash = compute_state_hash(&metadata_fingerprint, &manifest)?;
    Ok(LocalSnapshot {
        _database: database,
        metadata,
        manifest,
        manifest_bytes,
        state_hash,
        changed_blobs,
    })
}

async fn transport_for(kind: &str, location: &str) -> Result<Arc<dyn SyncTransport>, String> {
    match kind {
        "relay" => {
            let token = read_secret(RELAY_TOKEN_SECRET.into()).await.map_err(|_| {
                "Configure the relay token in Settings before synchronizing.".to_string()
            })?;
            Ok(Arc::new(
                HttpRelay::new(location, token).map_err(|error| error.to_string())?,
            ))
        }
        "folder" => {
            let folder = PathBuf::from(location);
            if !folder.is_absolute() {
                return Err("The shared sync folder must be an absolute path.".into());
            }
            std::fs::create_dir_all(&folder).map_err(|error| {
                format!(
                    "Cannot create shared sync folder {}: {error}",
                    folder.display()
                )
            })?;
            Ok(Arc::new(
                FileRelay::open(folder.join("Wisp Sync"))
                    .await
                    .map_err(|error| error.to_string())?,
            ))
        }
        _ => Err("Unsupported project sync backend.".into()),
    }
}

async fn default_sync_state(store: &Store, project_id: &str) -> Result<ProjectSyncState, String> {
    let kind = store
        .get_setting("sync_backend")
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "relay".into());
    let key = if kind == "folder" {
        "sync_folder"
    } else {
        "sync_relay_url"
    };
    let location = store
        .get_setting(key)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    if location.trim().is_empty() {
        return Err(if kind == "folder" {
            "Choose a shared sync folder in Settings first.".into()
        } else {
            "Configure a sync relay URL in Settings first.".into()
        });
    }
    Ok(ProjectSyncState::uninitialized(
        project_id,
        &kind,
        location.trim(),
    ))
}

async fn put_missing(
    transport: &dyn SyncTransport,
    blob_id: &str,
    bytes: Vec<u8>,
) -> Result<bool, String> {
    if transport
        .blob_exists(blob_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(false);
    }
    transport
        .put_blob(blob_id, bytes)
        .await
        .map_err(|error| error.to_string())?;
    Ok(true)
}

async fn push_snapshot(
    store: &Store,
    transport: &dyn SyncTransport,
    key: &[u8; PROJECT_KEY_BYTES],
    mut state: ProjectSyncState,
    snapshot: LocalSnapshot,
    base_revision: Option<String>,
    device_id: &str,
) -> Result<ProjectSyncResult, String> {
    if !valid_relay_component(&state.relay_project_id) || !valid_relay_component(device_id) {
        return Err("Project or device sync identity is invalid.".into());
    }
    let mut uploaded_files = 0;
    for (blob_id, bytes) in snapshot.changed_blobs {
        if put_missing(transport, &blob_id, bytes).await? {
            uploaded_files += 1;
        }
    }
    // Consume changed blobs before allocating encrypted metadata copies.
    let metadata_encrypted = encrypt_blob(key, &snapshot.metadata).map_err(|e| e.to_string())?;
    let metadata_blob = sha256_hex(&metadata_encrypted);
    let manifest_encrypted =
        encrypt_blob(key, &snapshot.manifest_bytes).map_err(|e| e.to_string())?;
    let manifest_blob = sha256_hex(&manifest_encrypted);
    put_missing(transport, &metadata_blob, metadata_encrypted).await?;
    put_missing(transport, &manifest_blob, manifest_encrypted).await?;

    let revision_id = uuid::Uuid::new_v4().to_string();
    let mut workspace_blobs = snapshot
        .manifest
        .files
        .iter()
        .map(|file| file.blob_id.clone())
        .collect::<Vec<_>>();
    workspace_blobs.sort();
    workspace_blobs.dedup();
    let mut revision = SyncRevision {
        protocol_version: SYNC_PROTOCOL_VERSION,
        project_id: state.relay_project_id.clone(),
        revision_id: revision_id.clone(),
        parent_revision: base_revision.clone(),
        device_id: device_id.into(),
        created_at: chrono::Utc::now().timestamp(),
        metadata_blob,
        manifest_blob,
        workspace_blobs,
        state_hash: snapshot.state_hash.clone(),
        auth_tag: String::new(),
    };
    sign_revision(key, &mut revision).map_err(|error| error.to_string())?;
    match transport
        .commit(
            &state.relay_project_id,
            CommitRequest {
                base_revision,
                revision,
            },
        )
        .await
        .map_err(|error| error.to_string())?
    {
        CommitOutcome::Conflict(_) => return Err(
            "Sync conflict: another device published a revision first. No local data was changed."
                .into(),
        ),
        CommitOutcome::Committed(head) if head.revision_id == revision_id => {}
        CommitOutcome::Committed(_) => {
            return Err("Relay returned an unexpected project head.".into())
        }
    }
    state.base_revision = Some(revision_id.clone());
    state.base_state_hash = Some(snapshot.state_hash);
    state.base_manifest_json =
        serde_json::to_string(&snapshot.manifest).map_err(|error| error.to_string())?;
    state.last_synced_at = Some(chrono::Utc::now().timestamp());
    state.last_direction = Some("push".into());
    store
        .upsert_project_sync_state(&state)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ProjectSyncResult {
        status: "synced".into(),
        direction: "push".into(),
        revision: Some(revision_id),
        uploaded_files,
        downloaded_files: 0,
        skipped_paths: snapshot.manifest.skipped_paths,
    })
}

async fn download_decrypted(
    transport: &dyn SyncTransport,
    key: &[u8; PROJECT_KEY_BYTES],
    blob_id: &str,
) -> Result<Vec<u8>, String> {
    let encrypted = transport
        .get_blob(blob_id)
        .await
        .map_err(|error| error.to_string())?;
    if sha256_hex(&encrypted) != blob_id {
        return Err("Relay returned a blob whose hash does not match its id.".into());
    }
    decrypt_blob(key, &encrypted).map_err(|error| error.to_string())
}

async fn download_revision(
    transport: &dyn SyncTransport,
    relay_project_id: &str,
    revision_id: &str,
    key: &[u8; PROJECT_KEY_BYTES],
) -> Result<(SyncRevision, Vec<u8>, WorkspaceManifest, Vec<u8>), String> {
    let revision = transport
        .revision(relay_project_id, revision_id)
        .await
        .map_err(|error| error.to_string())?;
    if revision.protocol_version != SYNC_PROTOCOL_VERSION
        || revision.project_id != relay_project_id
        || revision.revision_id != revision_id
    {
        return Err("Relay revision identity or protocol version is invalid.".into());
    }
    verify_revision(key, &revision).map_err(|error| error.to_string())?;
    let metadata = download_decrypted(transport, key, &revision.metadata_blob).await?;
    if metadata.len() as u64 > MAX_SYNC_METADATA_BYTES {
        return Err("Synchronized project metadata exceeds the local size limit.".into());
    }
    let manifest_bytes = download_decrypted(transport, key, &revision.manifest_blob).await?;
    if manifest_bytes.len() > MAX_SYNC_MANIFEST_BYTES {
        return Err("Encrypted workspace manifest exceeds the local size limit.".into());
    }
    let manifest: WorkspaceManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| "Invalid encrypted workspace manifest.".to_string())?;
    validate_manifest(&manifest)?;
    let declared = revision
        .workspace_blobs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if manifest
        .files
        .iter()
        .any(|file| !declared.contains(file.blob_id.as_str()))
    {
        return Err("Revision does not declare every workspace blob.".into());
    }
    Ok((revision, metadata, manifest, manifest_bytes))
}

async fn revision_descends_from(
    transport: &dyn SyncTransport,
    relay_project_id: &str,
    head_revision: &str,
    base_revision: &str,
    key: &[u8; PROJECT_KEY_BYTES],
) -> Result<bool, String> {
    let mut current = head_revision.to_string();
    let mut seen = BTreeSet::new();
    for _ in 0..10_000 {
        if current == base_revision {
            return Ok(true);
        }
        if !seen.insert(current.clone()) {
            return Err("Remote revision history contains a cycle.".into());
        }
        let revision = transport
            .revision(relay_project_id, &current)
            .await
            .map_err(|error| error.to_string())?;
        if revision.protocol_version != SYNC_PROTOCOL_VERSION
            || revision.project_id != relay_project_id
            || revision.revision_id != current
        {
            return Err("Remote revision history has an invalid identity.".into());
        }
        verify_revision(key, &revision).map_err(|error| error.to_string())?;
        match revision.parent_revision {
            Some(parent) => current = parent,
            None => return Ok(false),
        }
    }
    Err("Remote revision history exceeds the safety limit.".into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyOperation {
    path: String,
    had_original: bool,
    replacement: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApplyMarker {
    project_id: String,
    target_revision: String,
    workspace: PathBuf,
    staging: PathBuf,
    backup: PathBuf,
    operations: Vec<ApplyOperation>,
}

struct PreparedApply {
    marker_path: PathBuf,
    marker: ApplyMarker,
}

fn marker_path(app_data: &Path, project_id: &str) -> PathBuf {
    app_data.join(format!(
        "project-sync-apply-{}.json",
        directory_component(project_id)
    ))
}

fn ensure_no_symlink_ancestors(root: &Path, relative: &str) -> Result<(), String> {
    let parts = relative.split('/').collect::<Vec<_>>();
    let mut cursor = root.to_path_buf();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        cursor.push(part);
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing to synchronize through linked directory {}.",
                    cursor.display()
                ))
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!(
                    "Workspace path component is not a directory: {}",
                    cursor.display()
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn remove_any(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            std::fs::remove_dir_all(path).map_err(|error| error.to_string())
        }
        Ok(_) => std::fs::remove_file(path).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

fn rollback_apply(marker: &ApplyMarker) -> Result<(), String> {
    let mut first_error = None;
    for operation in marker.operations.iter().rev() {
        let destination = workspace_path(&marker.workspace, &operation.path)?;
        if operation.had_original {
            let backup = workspace_path(&marker.backup, &operation.path)?;
            if backup.exists() {
                if let Err(error) = remove_any(&destination) {
                    first_error.get_or_insert(error);
                }
                if let Some(parent) = destination.parent() {
                    if let Err(error) = std::fs::create_dir_all(parent) {
                        first_error.get_or_insert(error.to_string());
                        continue;
                    }
                }
                if let Err(error) = std::fs::rename(&backup, &destination) {
                    first_error.get_or_insert(error.to_string());
                }
            }
        } else if operation.replacement {
            if let Err(error) = remove_any(&destination) {
                first_error.get_or_insert(error);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&marker.staging);
    if first_error.is_none() {
        let _ = std::fs::remove_dir_all(&marker.backup);
    }
    first_error.map_or(Ok(()), Err)
}

fn cleanup_apply(prepared: &PreparedApply) -> Result<(), String> {
    for directory in [&prepared.marker.staging, &prepared.marker.backup] {
        match std::fs::remove_dir_all(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    match std::fs::remove_file(&prepared.marker_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

async fn recover_interrupted_apply(
    store: &Store,
    app_data: &Path,
    project_id: &str,
    workspace: &Path,
) -> Result<(), String> {
    let path = marker_path(app_data, project_id);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    let marker: ApplyMarker = serde_json::from_slice(&bytes).map_err(|_| {
        "An interrupted sync marker is invalid; manual recovery is required.".to_string()
    })?;
    if marker.project_id != project_id || marker.workspace != workspace {
        return Err("An interrupted sync marker does not match this project.".into());
    }
    let committed = store
        .get_project_sync_state(project_id)
        .await
        .map_err(|error| error.to_string())?
        .and_then(|state| state.base_revision)
        .as_deref()
        == Some(marker.target_revision.as_str());
    let prepared = PreparedApply {
        marker_path: path,
        marker,
    };
    if committed {
        cleanup_apply(&prepared)
    } else {
        rollback_apply(&prepared.marker)?;
        cleanup_apply(&prepared)
    }
}

async fn prepare_workspace_apply(
    transport: &dyn SyncTransport,
    key: &[u8; PROJECT_KEY_BYTES],
    app_data: &Path,
    project_id: &str,
    target_revision: &str,
    workspace: &Path,
    base: &WorkspaceManifest,
    remote: &WorkspaceManifest,
) -> Result<(PreparedApply, usize), String> {
    let parent = workspace
        .parent()
        .ok_or_else(|| "Project workspace has no parent directory.".to_string())?;
    let token = uuid::Uuid::new_v4();
    let staging = parent.join(format!(".wisp-sync-stage-{token}"));
    let backup = parent.join(format!(".wisp-sync-backup-{token}"));
    std::fs::create_dir(&staging).map_err(|error| error.to_string())?;
    std::fs::create_dir(&backup).map_err(|error| error.to_string())?;
    let base_files = base
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let remote_files = remote
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let changed = remote
        .files
        .iter()
        .filter(|file| {
            base_files.get(file.path.as_str()).is_none_or(|old| {
                old.sha256 != file.sha256
                    || old.size != file.size
                    || old.executable != file.executable
            })
        })
        .collect::<Vec<_>>();
    let removed = base
        .files
        .iter()
        .filter(|file| !remote_files.contains_key(file.path.as_str()))
        .collect::<Vec<_>>();

    let staged_result: Result<(), String> = async {
        for file in &changed {
            let plaintext = download_decrypted(transport, key, &file.blob_id).await?;
            if plaintext.len() as u64 != file.size || sha256_hex(&plaintext) != file.sha256 {
                return Err(format!(
                    "Workspace file {} failed integrity verification.",
                    file.path
                ));
            }
            let path = workspace_path(&staging, &file.path)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            write_synced(&path, &plaintext)?;
            #[cfg(unix)]
            if let Some(executable) = file.executable {
                use std::os::unix::fs::PermissionsExt;
                let mode = if executable { 0o755 } else { 0o644 };
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(error) = staged_result {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&backup);
        return Err(error);
    }

    let mut operations = changed
        .iter()
        .map(|file| (file.path.clone(), true))
        .chain(removed.iter().map(|file| (file.path.clone(), false)))
        .collect::<Vec<_>>();
    operations.sort_by(|left, right| left.0.cmp(&right.0));
    let mut marker = ApplyMarker {
        project_id: project_id.into(),
        target_revision: target_revision.into(),
        workspace: workspace.to_path_buf(),
        staging,
        backup,
        operations: Vec::new(),
    };
    for (relative, replacement) in operations {
        if let Err(error) = ensure_no_symlink_ancestors(workspace, &relative) {
            let _ = std::fs::remove_dir_all(&marker.staging);
            let _ = std::fs::remove_dir_all(&marker.backup);
            return Err(error);
        }
        let destination = match workspace_path(workspace, &relative) {
            Ok(destination) => destination,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&marker.staging);
                let _ = std::fs::remove_dir_all(&marker.backup);
                return Err(error);
            }
        };
        let had_original = match std::fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let _ = std::fs::remove_dir_all(&marker.staging);
                let _ = std::fs::remove_dir_all(&marker.backup);
                return Err(format!("Refusing to replace linked path {relative}."));
            }
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&marker.staging);
                let _ = std::fs::remove_dir_all(&marker.backup);
                return Err(error.to_string());
            }
        };
        marker.operations.push(ApplyOperation {
            path: relative,
            had_original,
            replacement,
        });
    }
    if let Err(error) = std::fs::create_dir_all(app_data) {
        let _ = std::fs::remove_dir_all(&marker.staging);
        let _ = std::fs::remove_dir_all(&marker.backup);
        return Err(error.to_string());
    }
    let marker_path = marker_path(app_data, project_id);
    let marker_bytes = match serde_json::to_vec_pretty(&marker) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&marker.staging);
            let _ = std::fs::remove_dir_all(&marker.backup);
            return Err(error.to_string());
        }
    };
    let marker_temp = app_data.join(format!(".project-sync-marker-{}.tmp", uuid::Uuid::new_v4()));
    let write_marker = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&marker_temp)?;
        file.write_all(&marker_bytes)?;
        file.sync_all()?;
        std::fs::rename(&marker_temp, &marker_path)
    })();
    if let Err(error) = write_marker {
        let _ = std::fs::remove_file(&marker_temp);
        let _ = std::fs::remove_dir_all(&marker.staging);
        let _ = std::fs::remove_dir_all(&marker.backup);
        return Err(error.to_string());
    }

    let apply_result = (|| -> Result<(), String> {
        for operation in &marker.operations {
            let destination = workspace_path(workspace, &operation.path)?;
            if operation.had_original {
                let backup_path = workspace_path(&marker.backup, &operation.path)?;
                if let Some(parent) = backup_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                std::fs::rename(&destination, backup_path).map_err(|error| error.to_string())?;
            }
            if operation.replacement {
                let staged = workspace_path(&marker.staging, &operation.path)?;
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                std::fs::rename(staged, destination).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    })();
    if let Err(error) = apply_result {
        let rollback = rollback_apply(&marker);
        if rollback.is_ok() {
            let _ = std::fs::remove_file(&marker_path);
        }
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!(
                "{error}; workspace rollback also failed: {rollback_error}"
            )),
        };
    }
    Ok((
        PreparedApply {
            marker_path,
            marker,
        },
        changed.len(),
    ))
}

async fn pull_snapshot(
    store: &Store,
    app_data: &Path,
    workspace: &Path,
    transport: &dyn SyncTransport,
    key: &[u8; PROJECT_KEY_BYTES],
    mut state: ProjectSyncState,
    target_revision: &str,
    base_manifest: &WorkspaceManifest,
) -> Result<ProjectSyncResult, String> {
    let (revision, metadata, remote_manifest, _manifest_bytes) =
        download_revision(transport, &state.relay_project_id, target_revision, key).await?;
    let database =
        TempPath(app_data.join(format!("project-sync-pull-{}.sqlite", uuid::Uuid::new_v4())));
    std::fs::write(&database.0, metadata).map_err(|error| error.to_string())?;
    let metadata_fingerprint = Store::portable_project_database_hash(&database.0)
        .await
        .map_err(|error| error.to_string())?;
    if compute_state_hash(&metadata_fingerprint, &remote_manifest)? != revision.state_hash {
        return Err("Synchronized project state failed integrity verification.".into());
    }
    let (prepared, downloaded_files) = prepare_workspace_apply(
        transport,
        key,
        app_data,
        &state.project_id,
        target_revision,
        workspace,
        base_manifest,
        &remote_manifest,
    )
    .await?;
    state.base_revision = Some(target_revision.into());
    state.base_state_hash = Some(revision.state_hash);
    state.base_manifest_json =
        serde_json::to_string(&remote_manifest).map_err(|error| error.to_string())?;
    state.last_synced_at = Some(chrono::Utc::now().timestamp());
    state.last_direction = Some("pull".into());
    if let Err(error) = store
        .replace_project_database(&database.0, &state.project_id, workspace, &state)
        .await
    {
        let rollback = rollback_apply(&prepared.marker);
        if rollback.is_ok() {
            let _ = cleanup_apply(&prepared);
        }
        return match rollback {
            Ok(()) => Err(error.to_string()),
            Err(rollback_error) => Err(format!(
                "{error}; workspace rollback also failed: {rollback_error}"
            )),
        };
    }
    cleanup_apply(&prepared)?;
    Ok(ProjectSyncResult {
        status: "synced".into(),
        direction: "pull".into(),
        revision: Some(target_revision.into()),
        uploaded_files: 0,
        downloaded_files,
        skipped_paths: remote_manifest.skipped_paths,
    })
}

fn parse_base_manifest(state: &ProjectSyncState) -> Result<WorkspaceManifest, String> {
    let manifest: WorkspaceManifest = serde_json::from_str(&state.base_manifest_json)
        .map_err(|_| "The local sync cursor contains an invalid workspace manifest.".to_string())?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

async fn device_id(store: &Store) -> Result<String, String> {
    if let Some(id) = store
        .get_setting("sync_device_id")
        .await
        .map_err(|error| error.to_string())?
        .filter(|id| !id.trim().is_empty())
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    store
        .set_setting("sync_device_id", &id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(id)
}

fn validate_transport_workspace(state: &ProjectSyncState, workspace: &Path) -> Result<(), String> {
    if state.transport_kind != "folder" {
        return Ok(());
    }
    let relay_root = PathBuf::from(&state.transport_location).join("Wisp Sync");
    let workspace = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let relay_root = std::fs::canonicalize(&relay_root).unwrap_or(relay_root);
    if relay_root == workspace
        || relay_root.starts_with(&workspace)
        || workspace.starts_with(&relay_root)
    {
        return Err("The shared sync storage cannot be inside the project workspace.".into());
    }
    Ok(())
}

async fn sync_existing_core(
    store: &Store,
    app_data: &Path,
    workspace: &Path,
    state: ProjectSyncState,
    transport: &dyn SyncTransport,
    key: &[u8; PROJECT_KEY_BYTES],
    device_id: &str,
) -> Result<ProjectSyncResult, String> {
    recover_interrupted_apply(store, app_data, &state.project_id, workspace).await?;
    validate_transport_workspace(&state, workspace)?;
    let base_manifest = parse_base_manifest(&state)?;
    let snapshot = build_local_snapshot(
        store,
        app_data,
        &state.project_id,
        workspace,
        &base_manifest,
        key,
    )
    .await?;
    let remote_head = transport
        .head(&state.relay_project_id)
        .await
        .map_err(|error| error.to_string())?;
    // A CAS commit can reach the relay just before persisting the local cursor
    // fails (disk full, process termination). Recognize our own exact revision
    // on the next manual sync instead of reporting a false divergence.
    if let Some(head) = remote_head
        .as_ref()
        .filter(|head| state.base_revision.as_deref() != Some(head.revision_id.as_str()))
    {
        let remote_revision = transport
            .revision(&state.relay_project_id, &head.revision_id)
            .await
            .map_err(|error| error.to_string())?;
        if remote_revision.project_id != state.relay_project_id
            || remote_revision.revision_id != head.revision_id
        {
            return Err("Remote revision identity is invalid.".into());
        }
        verify_revision(key, &remote_revision).map_err(|error| error.to_string())?;
        if remote_revision.device_id == device_id
            && remote_revision.parent_revision == state.base_revision
            && remote_revision.state_hash == snapshot.state_hash
        {
            let mut recovered = state;
            recovered.base_revision = Some(head.revision_id.clone());
            recovered.base_state_hash = Some(snapshot.state_hash);
            recovered.base_manifest_json =
                serde_json::to_string(&snapshot.manifest).map_err(|error| error.to_string())?;
            recovered.last_synced_at = Some(chrono::Utc::now().timestamp());
            recovered.last_direction = Some("push".into());
            store
                .upsert_project_sync_state(&recovered)
                .await
                .map_err(|error| error.to_string())?;
            return Ok(ProjectSyncResult {
                status: "recovered".into(),
                direction: "push".into(),
                revision: Some(head.revision_id.clone()),
                uploaded_files: 0,
                downloaded_files: 0,
                skipped_paths: snapshot.manifest.skipped_paths,
            });
        }
    }
    match (state.base_revision.as_deref(), remote_head.as_ref()) {
        (None, None) => {
            push_snapshot(store, transport, key, state, snapshot, None, device_id).await
        }
        (None, Some(_)) => Err(
            "A remote project already exists. Join it instead of initializing sync from this copy."
                .into(),
        ),
        (Some(_), None) => Err(
            "The remote project head is missing. No local data was uploaded; check the selected relay or shared folder."
                .into(),
        ),
        (Some(base), Some(head)) if base == head.revision_id => {
            if state.base_state_hash.as_deref() == Some(snapshot.state_hash.as_str()) {
                let mut current = state;
                current.last_synced_at = Some(chrono::Utc::now().timestamp());
                current.last_direction = Some("none".into());
                store
                    .upsert_project_sync_state(&current)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(ProjectSyncResult {
                    status: "up-to-date".into(),
                    direction: "none".into(),
                    revision: Some(head.revision_id.clone()),
                    uploaded_files: 0,
                    downloaded_files: 0,
                    skipped_paths: snapshot.manifest.skipped_paths,
                })
            } else {
                push_snapshot(
                    store,
                    transport,
                    key,
                    state,
                    snapshot,
                    Some(head.revision_id.clone()),
                    device_id,
                )
                .await
            }
        }
        (Some(_), Some(head)) => {
            if state.base_state_hash.as_deref() != Some(snapshot.state_hash.as_str()) {
                return Err(
                    "Sync conflict: this device and another device both changed the project. No data was overwritten."
                        .into(),
                );
            }
            let base = state.base_revision.as_deref().unwrap_or_default();
            if !revision_descends_from(
                transport,
                &state.relay_project_id,
                &head.revision_id,
                base,
                key,
            )
            .await?
            {
                return Err(
                    "Sync conflict: remote history does not descend from this device's last revision. No data was overwritten."
                        .into(),
                );
            }
            pull_snapshot(
                store,
                app_data,
                workspace,
                transport,
                key,
                state,
                &head.revision_id,
                &base_manifest,
            )
            .await
        }
    }
}

async fn project_has_frame_activity(state: &AppState, project_id: &str) -> Result<bool, String> {
    let mut frames = state.running_turns.lock().await.clone();
    frames.extend(state.awaiting_confirm.lock().unwrap().iter().cloned());
    frames.extend(state.reviewing.lock().unwrap().iter().cloned());
    for frame_id in frames {
        if state
            .store
            .frame_project_id(&frame_id)
            .await
            .map_err(|error| error.to_string())?
            .as_deref()
            == Some(project_id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn begin_quiet_sync(
    state: &AppState,
    project_id: &str,
) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>, String> {
    let quiet = state
        .project_activity(project_id)
        .try_write_owned()
        .map_err(|_| {
            "Wait for every task in this project to finish before synchronizing.".to_string()
        })?;
    if project_has_frame_activity(state, project_id).await?
        || state
            .store
            .project_has_active_runs(project_id)
            .await
            .map_err(|error| error.to_string())?
        || state
            .run_manager
            .has_in_flight_project(&state.store, project_id)
            .await?
    {
        return Err(
            "Wait for every task and run in this project to finish before synchronizing.".into(),
        );
    }
    Ok(quiet)
}

async fn clear_project_runtime_cache(state: &AppState, project_id: &str, frame_ids: &[String]) {
    for frame_id in frame_ids {
        super::acp::close_frame(state, frame_id).await;
    }
    let frame_ids = frame_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    state
        .sessions
        .lock()
        .await
        .retain(|frame_id, _| !frame_ids.contains(frame_id.as_str()));
    state
        .active_frame
        .write()
        .unwrap()
        .retain(|_, frame_id| !frame_ids.contains(frame_id.as_str()));
    state
        .confirms
        .lock()
        .unwrap()
        .retain(|frame_id, _| !frame_ids.contains(frame_id.as_str()));
    let mut active = state.active.write().unwrap();
    for project in active
        .values_mut()
        .filter(|project| project.id == project_id)
    {
        project.skills = Arc::new(wisp_skills::SkillIndex::load(&super::skill_paths(
            &project.root,
        )));
        project.memory = Arc::new(wisp_core::MemoryManager::new(&project.root));
    }
}

#[tauri::command]
pub(super) async fn sync_project(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProjectSyncResult, String> {
    let _quiet = begin_quiet_sync(&state, &id).await?;
    let old_frame_ids = state
        .store
        .list_project_frame_ids(&id)
        .await
        .map_err(|error| error.to_string())?;
    let (_, workspace_dir) = state
        .store
        .get_project(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project not found.".to_string())?;
    let existing = state
        .store
        .get_project_sync_state(&id)
        .await
        .map_err(|error| error.to_string())?;
    let (sync_state, key) = match existing {
        Some(mut sync_state) => {
            if sync_state.transport_kind == "folder" {
                if let Some(folder) = state
                    .store
                    .get_setting("sync_folder")
                    .await
                    .map_err(|error| error.to_string())?
                    .filter(|folder| !folder.trim().is_empty())
                {
                    sync_state.transport_location = folder;
                }
            }
            (sync_state, load_project_key(&id).await?)
        }
        None => {
            let sync_state = default_sync_state(&state.store, &id).await?;
            let key = create_project_key(&id).await?;
            if let Err(error) = state.store.upsert_project_sync_state(&sync_state).await {
                delete_secret(key_secret_name(&id)).await;
                return Err(error.to_string());
            }
            (sync_state, key)
        }
    };
    validate_transport_workspace(&sync_state, Path::new(&workspace_dir))?;
    let transport =
        transport_for(&sync_state.transport_kind, &sync_state.transport_location).await?;
    let device_id = device_id(&state.store).await?;
    let result = sync_existing_core(
        &state.store,
        &state.app_data,
        Path::new(&workspace_dir),
        sync_state,
        transport.as_ref(),
        &key,
        &device_id,
    )
    .await?;
    if result.direction == "pull" {
        clear_project_runtime_cache(&state, &id, &old_frame_ids).await;
    }
    Ok(result)
}

#[tauri::command]
pub(super) async fn resolve_project_sync(
    state: State<'_, AppState>,
    id: String,
    strategy: String,
) -> Result<ProjectSyncResult, String> {
    if !matches!(strategy.as_str(), "local" | "remote") {
        return Err("Sync conflict strategy must be local or remote.".into());
    }
    let _quiet = begin_quiet_sync(&state, &id).await?;
    let old_frame_ids = state
        .store
        .list_project_frame_ids(&id)
        .await
        .map_err(|error| error.to_string())?;
    let (_, workspace_dir) = state
        .store
        .get_project(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project not found.".to_string())?;
    let workspace = Path::new(&workspace_dir);
    let mut sync_state = state
        .store
        .get_project_sync_state(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "This project has not been synchronized yet.".to_string())?;
    if sync_state.transport_kind == "folder" {
        if let Some(folder) = state
            .store
            .get_setting("sync_folder")
            .await
            .map_err(|error| error.to_string())?
            .filter(|folder| !folder.trim().is_empty())
        {
            sync_state.transport_location = folder;
        }
    }
    validate_transport_workspace(&sync_state, workspace)?;
    recover_interrupted_apply(&state.store, &state.app_data, &id, workspace).await?;
    let key = load_project_key(&id).await?;
    let transport =
        transport_for(&sync_state.transport_kind, &sync_state.transport_location).await?;
    let base_manifest = parse_base_manifest(&sync_state)?;
    let snapshot = build_local_snapshot(
        &state.store,
        &state.app_data,
        &id,
        workspace,
        &base_manifest,
        &key,
    )
    .await?;
    let head = transport
        .head(&sync_state.relay_project_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The remote project head is missing.".to_string())?;
    let result = if strategy == "local" {
        let device_id = device_id(&state.store).await?;
        push_snapshot(
            &state.store,
            transport.as_ref(),
            &key,
            sync_state,
            snapshot,
            Some(head.revision_id),
            &device_id,
        )
        .await?
    } else {
        // Passing the current local manifest makes explicit "Use remote"
        // remove local-only eligible files and replace every differing file.
        let current_manifest = snapshot.manifest;
        pull_snapshot(
            &state.store,
            &state.app_data,
            workspace,
            transport.as_ref(),
            &key,
            sync_state,
            &head.revision_id,
            &current_manifest,
        )
        .await?
    };
    if result.direction == "pull" {
        clear_project_runtime_cache(&state, &id, &old_frame_ids).await;
    }
    Ok(result)
}

#[tauri::command]
pub(super) async fn get_project_sync_status(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProjectSyncStatus, String> {
    let cursor = state
        .store
        .get_project_sync_state(&id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(match cursor {
        Some(cursor) => ProjectSyncStatus {
            configured: cursor.base_revision.is_some(),
            transport_kind: Some(cursor.transport_kind),
            last_synced_at: cursor.last_synced_at,
            last_direction: cursor.last_direction,
            revision: cursor.base_revision,
        },
        None => ProjectSyncStatus {
            configured: false,
            transport_kind: None,
            last_synced_at: None,
            last_direction: None,
            revision: None,
        },
    })
}

#[tauri::command]
pub(super) async fn project_sync_code(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let cursor = state
        .store
        .get_project_sync_state(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            "Synchronize this project once before copying its device code.".to_string()
        })?;
    if cursor.base_revision.is_none() {
        return Err("Synchronize this project successfully before copying its device code.".into());
    }
    let (name, _) = state
        .store
        .get_project(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Project not found.".to_string())?;
    let code = JoinCode {
        version: SYNC_PROTOCOL_VERSION,
        project_id: id.clone(),
        project_name: name,
        project_key: encode_key(&load_project_key(&id).await?),
        transport_kind: cursor.transport_kind.clone(),
        relay_url: (cursor.transport_kind == "relay").then_some(cursor.transport_location),
    };
    let bytes = serde_json::to_vec(&code).map_err(|error| error.to_string())?;
    Ok(format!(
        "{JOIN_CODE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

fn decode_join_code(raw: &str) -> Result<JoinCode, String> {
    let encoded = raw
        .trim()
        .strip_prefix(JOIN_CODE_PREFIX)
        .ok_or_else(|| "This is not a Wisp project device code.".to_string())?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "Invalid Wisp project device code.".to_string())?;
    let code: JoinCode = serde_json::from_slice(&bytes)
        .map_err(|_| "Invalid Wisp project device code.".to_string())?;
    if code.version != SYNC_PROTOCOL_VERSION
        || code.project_id.is_empty()
        || !valid_relay_component(&code.project_id)
        || !matches!(code.transport_kind.as_str(), "relay" | "folder")
    {
        return Err("Unsupported or invalid Wisp project device code.".into());
    }
    decode_key(&code.project_key)?;
    Ok(code)
}

async fn materialize_workspace(
    transport: &dyn SyncTransport,
    key: &[u8; PROJECT_KEY_BYTES],
    destination: &Path,
    manifest: &WorkspaceManifest,
) -> Result<usize, String> {
    for file in &manifest.files {
        let plaintext = download_decrypted(transport, key, &file.blob_id).await?;
        if plaintext.len() as u64 != file.size || sha256_hex(&plaintext) != file.sha256 {
            return Err(format!(
                "Workspace file {} failed integrity verification.",
                file.path
            ));
        }
        let path = workspace_path(destination, &file.path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        write_synced(&path, &plaintext)?;
        #[cfg(unix)]
        if let Some(executable) = file.executable {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(manifest.files.len())
}

#[tauri::command]
pub(super) async fn join_synced_project(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
) -> Result<Option<ProjectSummary>, String> {
    let code = decode_join_code(&code)?;
    if state
        .store
        .get_project(&code.project_id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("This project is already present on this device.".into());
    }
    let location = match code.transport_kind.as_str() {
        "relay" => code
            .relay_url
            .clone()
            .ok_or_else(|| "The device code has no relay URL.".to_string())?,
        "folder" => state
            .store
            .get_setting("sync_folder")
            .await
            .map_err(|error| error.to_string())?
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| "Choose a shared sync folder in Settings first.".to_string())?,
        _ => return Err("Unsupported sync backend.".into()),
    };
    let Some(parent) = pick_import_parent(&app).await? else {
        return Ok(None);
    };
    let destination = unique_destination(&parent, &code.project_name)?;
    let staging = parent.join(format!(".wisp-sync-join-{}", uuid::Uuid::new_v4()));
    let destination_state =
        ProjectSyncState::uninitialized(&code.project_id, &code.transport_kind, &location);
    validate_transport_workspace(&destination_state, &destination)?;
    let key = decode_key(&code.project_key)?;
    let transport = transport_for(&code.transport_kind, &location).await?;
    let head = match transport
        .head(&code.project_id)
        .await
        .map_err(|error| error.to_string())?
    {
        Some(head) => head,
        None => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(
                "The shared project is not available yet. Wait for the cloud folder or relay to finish updating."
                    .into(),
            );
        }
    };
    let (revision, metadata, manifest, _manifest_bytes) = match download_revision(
        transport.as_ref(),
        &code.project_id,
        &head.revision_id,
        &key,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    std::fs::create_dir_all(&state.app_data).map_err(|error| error.to_string())?;
    let database = TempPath(
        state
            .app_data
            .join(format!("project-sync-join-{}.sqlite", uuid::Uuid::new_v4())),
    );
    std::fs::write(&database.0, metadata).map_err(|error| error.to_string())?;
    let metadata_fingerprint = Store::portable_project_database_hash(&database.0)
        .await
        .map_err(|error| error.to_string())?;
    if compute_state_hash(&metadata_fingerprint, &manifest)? != revision.state_hash {
        return Err("Synchronized project state failed integrity verification.".into());
    }
    std::fs::create_dir(&staging).map_err(|error| error.to_string())?;
    if let Err(error) = materialize_workspace(transport.as_ref(), &key, &staging, &manifest).await {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    std::fs::rename(&staging, &destination).map_err(|error| {
        let _ = std::fs::remove_dir_all(&staging);
        format!("Cannot place synchronized project: {error}")
    })?;
    let mut sync_state =
        ProjectSyncState::uninitialized(&code.project_id, &code.transport_kind, &location);
    sync_state.base_revision = Some(head.revision_id.clone());
    sync_state.base_state_hash = Some(revision.state_hash);
    sync_state.base_manifest_json =
        serde_json::to_string(&manifest).map_err(|error| error.to_string())?;
    sync_state.last_synced_at = Some(chrono::Utc::now().timestamp());
    sync_state.last_direction = Some("pull".into());
    if let Err(error) = write_secret(key_secret_name(&code.project_id), encode_key(&key)).await {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error);
    }
    if let Err(error) = state
        .store
        .import_project_database(&database.0, &code.project_id, &destination)
        .await
    {
        delete_secret(key_secret_name(&code.project_id)).await;
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error.to_string());
    }
    if let Err(error) = state.store.upsert_project_sync_state(&sync_state).await {
        let _ = state.store.delete_project(&code.project_id).await;
        delete_secret(key_secret_name(&code.project_id)).await;
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error.to_string());
    }
    Ok(Some(build_project_summary(&state, &code.project_id).await))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProject {
        root: PathBuf,
        app_data: PathBuf,
        store: Store,
    }

    impl TestProject {
        async fn new(label: &str, project_id: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("wisp-sync-{label}-{}", uuid::Uuid::new_v4()));
            let workspace = root.join("workspace");
            let app_data = root.join("app-data");
            let database = root.join("store.sqlite");
            std::fs::create_dir_all(&workspace).unwrap();
            std::fs::create_dir_all(&app_data).unwrap();
            let store = Store::open(&database).await.unwrap();
            store
                .create_project(project_id, "Shared study", workspace.to_str().unwrap())
                .await
                .unwrap();
            Self {
                root: workspace,
                app_data,
                store,
            }
        }

        async fn close(self) {
            let base = self.root.parent().unwrap().to_path_buf();
            drop(self.store);
            let _ = std::fs::remove_dir_all(base);
        }
    }

    #[test]
    fn paths_are_valid_on_windows_and_macos() {
        assert!(valid_portable_path("results/plot 1.png"));
        assert!(!valid_portable_path("results/a:b.txt"));
        assert!(!valid_portable_path("CON/data.txt"));
        assert!(!valid_portable_path("../escape"));
        assert!(!valid_portable_path("trailing./x"));
    }

    #[test]
    fn device_code_hides_machine_local_folder_paths() {
        let code = JoinCode {
            version: 1,
            project_id: "project-1".into(),
            project_name: "Study".into(),
            project_key: encode_key(&[7; PROJECT_KEY_BYTES]),
            transport_kind: "folder".into(),
            relay_url: None,
        };
        let encoded = format!(
            "{JOIN_CODE_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&code).unwrap())
        );
        let decoded = decode_join_code(&encoded).unwrap();
        assert_eq!(decoded.project_id, "project-1");
        assert!(decoded.relay_url.is_none());
    }

    #[tokio::test]
    async fn exclusive_sync_gate_refuses_tasks_and_active_tasks_refuse_sync() {
        let gate = Arc::new(tokio::sync::RwLock::new(()));
        let task = gate.clone().try_read_owned().unwrap();
        assert!(gate.clone().try_write_owned().is_err());
        drop(task);
        let sync = gate.clone().try_write_owned().unwrap();
        assert!(gate.try_read_owned().is_err());
        drop(sync);
    }

    #[test]
    fn large_scientific_files_stay_local() {
        let root = std::env::temp_dir().join(format!("wisp-sync-large-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let large = root.join("reads.fastq");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(MAX_SYNC_FILE_BYTES + 1)
            .unwrap();
        let (manifest, blobs) = build_workspace_snapshot(
            &root,
            &root.join("not-present"),
            &WorkspaceManifest::default(),
            &[3; PROJECT_KEY_BYTES],
        )
        .unwrap();
        assert!(manifest.files.is_empty());
        assert_eq!(manifest.skipped_paths, vec!["reads.fastq"]);
        assert!(blobs.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn changed_workspace_files_have_an_aggregate_upload_limit() {
        let total = reserve_changed_blob_bytes(0, 10, 150).unwrap();
        let total = reserve_changed_blob_bytes(total, 10, 150).unwrap();
        let error = reserve_changed_blob_bytes(total, 10, 150).unwrap_err();
        assert!(error.contains("128 MiB sync upload limit"));
    }

    #[test]
    fn device_local_exclusions_and_ciphertext_ids_do_not_make_state_dirty() {
        let file = WorkspaceFile {
            path: "notes.txt".into(),
            size: 4,
            sha256: sha256_hex(b"note"),
            blob_id: sha256_hex(b"ciphertext-one"),
            executable: Some(false),
        };
        let first = WorkspaceManifest {
            version: 1,
            files: vec![file.clone()],
            skipped_paths: vec!["large.fastq".into()],
        };
        let mut second = WorkspaceManifest {
            version: 1,
            files: vec![file],
            skipped_paths: vec![],
        };
        second.files[0].blob_id = sha256_hex(b"ciphertext-two");
        assert_eq!(
            compute_state_hash("metadata", &first).unwrap(),
            compute_state_hash("metadata", &second).unwrap()
        );
    }

    #[tokio::test]
    async fn two_devices_push_pull_and_refuse_divergent_changes() {
        let project_id = "project-sync-e2e";
        let relay_parent =
            std::env::temp_dir().join(format!("wisp-sync-relay-{}", uuid::Uuid::new_v4()));
        let relay = FileRelay::open(relay_parent.join("Wisp Sync"))
            .await
            .unwrap();
        let key = [11_u8; PROJECT_KEY_BYTES];
        let first = TestProject::new("first", project_id).await;
        std::fs::write(first.root.join("notes.txt"), b"device one").unwrap();
        let state =
            ProjectSyncState::uninitialized(project_id, "folder", relay_parent.to_str().unwrap());
        let pushed = sync_existing_core(
            &first.store,
            &first.app_data,
            &first.root,
            state,
            &relay,
            &key,
            "device-one",
        )
        .await
        .unwrap();
        assert_eq!(pushed.direction, "push");
        let recovered = sync_existing_core(
            &first.store,
            &first.app_data,
            &first.root,
            ProjectSyncState::uninitialized(project_id, "folder", relay_parent.to_str().unwrap()),
            &relay,
            &key,
            "device-one",
        )
        .await
        .unwrap();
        assert_eq!(recovered.status, "recovered");
        let first_state = first
            .store
            .get_project_sync_state(project_id)
            .await
            .unwrap()
            .unwrap();
        let unchanged = sync_existing_core(
            &first.store,
            &first.app_data,
            &first.root,
            first_state.clone(),
            &relay,
            &key,
            "device-one",
        )
        .await
        .unwrap();
        assert_eq!(unchanged.direction, "none");

        let second = TestProject::new("second", project_id).await;
        second.store.delete_project(project_id).await.unwrap();
        let head = relay.head(project_id).await.unwrap().unwrap();
        let (revision, metadata, manifest, _) =
            download_revision(&relay, project_id, &head.revision_id, &key)
                .await
                .unwrap();
        materialize_workspace(&relay, &key, &second.root, &manifest)
            .await
            .unwrap();
        let imported_db = second.app_data.join("joined.sqlite");
        std::fs::write(&imported_db, metadata).unwrap();
        second
            .store
            .import_project_database(&imported_db, project_id, &second.root)
            .await
            .unwrap();
        let mut second_state =
            ProjectSyncState::uninitialized(project_id, "folder", relay_parent.to_str().unwrap());
        second_state.base_revision = Some(head.revision_id);
        second_state.base_state_hash = Some(revision.state_hash);
        second_state.base_manifest_json = serde_json::to_string(&manifest).unwrap();
        second
            .store
            .upsert_project_sync_state(&second_state)
            .await
            .unwrap();

        std::fs::write(second.root.join("notes.txt"), b"device two").unwrap();
        let pushed = sync_existing_core(
            &second.store,
            &second.app_data,
            &second.root,
            second_state,
            &relay,
            &key,
            "device-two",
        )
        .await
        .unwrap();
        assert_eq!(pushed.direction, "push");
        let pulled = sync_existing_core(
            &first.store,
            &first.app_data,
            &first.root,
            first_state,
            &relay,
            &key,
            "device-one",
        )
        .await
        .unwrap();
        assert_eq!(pulled.direction, "pull");
        assert_eq!(
            std::fs::read(first.root.join("notes.txt")).unwrap(),
            b"device two"
        );

        let first_current = first
            .store
            .get_project_sync_state(project_id)
            .await
            .unwrap()
            .unwrap();
        let second_current = second
            .store
            .get_project_sync_state(project_id)
            .await
            .unwrap()
            .unwrap();
        std::fs::write(first.root.join("notes.txt"), b"local divergence").unwrap();
        std::fs::write(second.root.join("notes.txt"), b"remote divergence").unwrap();
        sync_existing_core(
            &second.store,
            &second.app_data,
            &second.root,
            second_current,
            &relay,
            &key,
            "device-two",
        )
        .await
        .unwrap();
        let conflict = sync_existing_core(
            &first.store,
            &first.app_data,
            &first.root,
            first_current.clone(),
            &relay,
            &key,
            "device-one",
        )
        .await
        .unwrap_err();
        assert!(conflict.contains("conflict"));
        assert_eq!(
            std::fs::read(first.root.join("notes.txt")).unwrap(),
            b"local divergence"
        );
        let current_snapshot = build_local_snapshot(
            &first.store,
            &first.app_data,
            project_id,
            &first.root,
            &parse_base_manifest(&first_current).unwrap(),
            &key,
        )
        .await
        .unwrap();
        let head = relay.head(project_id).await.unwrap().unwrap();
        pull_snapshot(
            &first.store,
            &first.app_data,
            &first.root,
            &relay,
            &key,
            first_current,
            &head.revision_id,
            &current_snapshot.manifest,
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(first.root.join("notes.txt")).unwrap(),
            b"remote divergence"
        );

        first.close().await;
        second.close().await;
        let _ = std::fs::remove_dir_all(relay_parent);
    }
}
