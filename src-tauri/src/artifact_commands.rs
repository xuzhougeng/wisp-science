use super::*;
use super::{ensure_active_frame, ActiveProject, AppState, ArtifactInfo};
use crate::file_browser::mime_for_path;
use base64::Engine;
use tauri::{AppHandle, State};

const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
const MAX_UPLOAD_BASE64_BYTES: usize = MAX_UPLOAD_BYTES.div_ceil(3) * 4;

fn validate_upload_base64_len(len: usize) -> Result<(), String> {
    if len > MAX_UPLOAD_BASE64_BYTES {
        return Err(format!("file exceeds {MAX_UPLOAD_BYTES} byte limit"));
    }
    Ok(())
}

fn decode_upload_data(data_base64: &str) -> Result<Vec<u8>, String> {
    let encoded = data_base64.trim();
    validate_upload_base64_len(encoded.len())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| format!("invalid base64: {e}"))?;
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(format!("file exceeds {MAX_UPLOAD_BYTES} byte limit"));
    }
    Ok(bytes)
}
#[cfg(test)]
use uuid::Uuid;

fn sanitize_upload_name(name: &str) -> Result<String, String> {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "invalid filename".to_string())?;
    if base.is_empty() || base == "." || base == ".." || base.contains('\0') {
        return Err("invalid filename".into());
    }
    Ok(base.to_string())
}

fn unique_upload_path(root: &std::path::Path, dir: &str, name: &str) -> std::path::PathBuf {
    let mut path = root.join(dir).join(name);
    if !path.exists() {
        return path;
    }
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str());
    for i in 1..1000 {
        let candidate = match ext {
            Some(e) => format!("{stem}_{i}.{e}"),
            None => format!("{stem}_{i}"),
        };
        path = root.join(dir).join(&candidate);
        if !path.exists() {
            return path;
        }
    }
    root.join(dir).join(name)
}

fn existing_artifact_path(
    root: &std::path::Path,
    path: &str,
) -> Result<std::path::PathBuf, String> {
    let real = wisp_tools::safety::validate_file_path(root, path)?;
    if !real.is_file() {
        return Err(format!("path '{path}' is not an existing file"));
    }
    Ok(real)
}

/// Convert an artifact's stored logical key into the path users should see in
/// the file browser. `path:` and `source:` keys are workspace paths; anything
/// else (internal run keys, remote URIs, legacy rows) keeps the storage path
/// as a fallback rather than inventing a user-facing location.
fn user_artifact_location(logical_key: Option<&str>, storage_path: &str) -> String {
    logical_key
        .and_then(|key| {
            key.strip_prefix("path:")
                .or_else(|| key.strip_prefix("source:"))
        })
        .filter(|path| !path.is_empty())
        .unwrap_or(storage_path)
        .to_string()
}

/// Product artifact surfaces never expose dot-hidden files or Wisp's private
/// snapshot directories. Apply the rule to every path component, including a
/// path inside a remote URI; `.` and `..` are path syntax rather than names.
fn artifact_path_is_hidden(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    let components = match normalized.split_once("://") {
        Some((_, authority_and_path)) => authority_and_path
            .split_once('/')
            .map(|(_, path)| path)
            .unwrap_or(""),
        None => normalized.as_str(),
    };
    components
        .split('/')
        .filter(|component| !component.is_empty())
        .any(|component| component.starts_with('.') && component != "." && component != "..")
}

fn artifact_info_is_visible(info: &ArtifactInfo) -> bool {
    !artifact_path_is_hidden(
        info.logical_path
            .as_deref()
            .or(info.location.as_deref())
            .unwrap_or(&info.path),
    )
}

async fn register_artifact_at(
    state: &AppState,
    label: &str,
    ap: &ActiveProject,
    path: String,
    content_type: Option<String>,
    origin: &'static str,
) -> Result<ArtifactInfo, String> {
    let scope = match state.active_frame(label) {
        Some(frame_id) => state
            .store
            .frame_state_scope(&frame_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Artifact conversation no longer exists".to_string())?,
        None => wisp_store::StateScope::mainline(ap.id.clone()),
    };
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let real = existing_artifact_path(&ap.root, &path)?;
    let snapshot_source = {
        let source = std::path::PathBuf::from(&path);
        if source.is_absolute() {
            source
        } else {
            ap.root.join(source)
        }
    };
    let frame_id = ensure_active_frame(state, label, ap).await?;
    let filename = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let mime = content_type.unwrap_or_else(|| mime_for_path(&real).to_string());
    let source_path = real
        .strip_prefix(&ap.root)
        .unwrap_or(&real)
        .to_string_lossy()
        .replace('\\', "/");
    let logical_key = format!("path:{source_path}");
    let id = wisp_store::scoped_logical_artifact_id(&ap.id, scope.scope_key(), &logical_key);
    let captured = crate::snapshot_store::capture_file(
        &ap.root,
        &snapshot_source,
        crate::snapshot_store::SnapshotPolicy::UpTo(crate::snapshot_store::DEFAULT_SNAPSHOT_LIMIT),
    )?;
    let size_bytes =
        i64::try_from(captured.size_bytes).map_err(|_| "artifact is too large".to_string())?;
    state
        .store
        .save_artifact_version(&wisp_store::ArtifactVersionDraft {
            version_id: None,
            artifact_id: id.clone(),
            project_id: ap.id.clone(),
            root_frame_id: frame_id.clone(),
            filename: filename.clone(),
            content_type: mime.clone(),
            storage_path: captured.storage_path.clone(),
            logical_key: Some(logical_key),
            size_bytes: Some(size_bytes),
            checksum: Some(captured.checksum),
            producing_run_id: None,
            env_snapshot_hash: None,
            materialization: captured.materialization,
            capture_timing: wisp_store::ArtifactCaptureTiming::AtCreation,
        })
        .await
        .map_err(|e| format!("{e}"))?;
    let ts = chrono::Utc::now().timestamp();
    Ok(ArtifactInfo {
        id,
        name: filename,
        kind: mime,
        path: captured.storage_path,
        location: Some(source_path.clone()),
        ts,
        project_id: Some(ap.id.clone()),
        project_name: None,
        session_id: Some(frame_id),
        session_title: None,
        size_bytes: Some(size_bytes),
        origin: Some(origin.into()),
        logical_path: Some(source_path),
        source_discarded: false,
    })
}

#[tauri::command]
pub(super) async fn upload_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    filename: String,
    data_base64: String,
) -> Result<ArtifactInfo, String> {
    let name = sanitize_upload_name(&filename)?;
    let bytes = decode_upload_data(&data_base64)?;
    let (ap, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let upload_dir = ap.root.join("uploads");
    tokio::fs::create_dir_all(&upload_dir)
        .await
        .map_err(|e| format!("{e}"))?;
    let dest = unique_upload_path(&ap.root, "uploads", &name);
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| format!("{e}"))?;
    let rel = dest
        .strip_prefix(&ap.root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| dest.to_string_lossy().into_owned());
    register_artifact_at(&state, window.label(), &ap, rel, None, "upload").await
}

#[tauri::command]
pub(super) async fn register_artifact(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    path: String,
    content_type: Option<String>,
) -> Result<ArtifactInfo, String> {
    let (ap, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    crate::exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let _project_activity = state.begin_project_activity(&ap.id)?;
    register_artifact_at(&state, window.label(), &ap, path, content_type, "artifact").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_names_drop_parent_paths_and_reject_special_names() {
        assert_eq!(
            sanitize_upload_name("some/path/data.csv").unwrap(),
            "data.csv"
        );
        for name in ["", ".", ".."] {
            assert!(sanitize_upload_name(name).is_err());
        }
    }

    #[test]
    fn upload_size_is_rejected_before_base64_decode() {
        assert!(validate_upload_base64_len(MAX_UPLOAD_BASE64_BYTES).is_ok());
        assert!(validate_upload_base64_len(MAX_UPLOAD_BASE64_BYTES + 1).is_err());
        assert_eq!(decode_upload_data("aGVsbG8=").unwrap(), b"hello");
    }

    #[test]
    fn artifact_registration_requires_an_existing_file() {
        let root = std::env::temp_dir().join(format!("wisp_register_artifact_{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("results")).unwrap();
        let root = dunce::canonicalize(root).unwrap();
        std::fs::write(root.join("results/report.csv"), b"a,b\n1,2\n").unwrap();

        assert_eq!(
            existing_artifact_path(&root, "results/report.csv").unwrap(),
            root.join("results/report.csv")
        );
        assert!(existing_artifact_path(&root, "results").is_err());
        assert!(existing_artifact_path(&root, "missing.csv").is_err());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn user_artifact_location_uses_workspace_keys_and_falls_back_to_storage() {
        let storage = ".wisp/artifacts/sha256/aa/report.md";
        assert_eq!(
            user_artifact_location(Some("path:results/report.md"), storage),
            "results/report.md"
        );
        assert_eq!(
            user_artifact_location(Some("source:notes/claim.md"), storage),
            "notes/claim.md"
        );
        assert_eq!(
            user_artifact_location(Some("method-search-run:run:spec"), storage),
            storage
        );
        assert_eq!(user_artifact_location(None, storage), storage);
    }

    #[test]
    fn artifact_surfaces_hide_dot_files_at_every_path_depth() {
        for path in [
            ".wisp/artifacts/sha256/aa/result.csv",
            "results/.cache/result.csv",
            r"results\.private\result.csv",
            "ssh://gpu/home/research/.cache/result.csv",
        ] {
            assert!(artifact_path_is_hidden(path), "expected hidden: {path}");
        }
        for path in [
            "results/figures/result.png",
            "../results/result.csv",
            "ssh://gpu/home/research/result.csv",
        ] {
            assert!(!artifact_path_is_hidden(path), "expected visible: {path}");
        }
    }
}
#[tauri::command]
pub(super) async fn list_artifacts(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
) -> Result<Vec<ArtifactInfo>, String> {
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => Some(id.to_string()),
        None => state.active_frame(window.label()),
    };
    let Some(fid) = frame_id else {
        return Ok(vec![]);
    };
    let rows = state
        .store
        .list_artifacts(&fid)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|(id, name, ct, path, ts, logical_key)| {
            let location = user_artifact_location(logical_key.as_deref(), &path);
            let logical_path = logical_key
                .as_deref()
                .and_then(|key| key.strip_prefix("path:"))
                .map(str::to_owned);
            ArtifactInfo {
                id,
                name,
                kind: ct,
                path,
                location: Some(location),
                ts,
                project_id: None,
                project_name: None,
                session_id: None,
                session_title: None,
                size_bytes: None,
                origin: None,
                logical_path,
                source_discarded: false,
            }
        })
        .filter(artifact_info_is_visible)
        .collect())
}

#[tauri::command]
pub(super) async fn search_artifacts(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    query: Option<String>,
    limit: Option<i64>,
    project_id: Option<String>,
    all_projects: Option<bool>,
) -> Result<Vec<ArtifactInfo>, String> {
    let (ap, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    let rows = match &scope {
        wisp_store::StateScope::Exploration { exploration_id, .. } => {
            if all_projects.unwrap_or(false) || project_id.as_deref().is_some_and(|id| id != ap.id)
            {
                return Err(
                    "exploration_scope_violation: cross-project Artifact search is disabled inside an exploration."
                        .into(),
                );
            }
            state
                .store
                .search_exploration_artifacts(
                    &ap.id,
                    exploration_id,
                    query.as_deref().unwrap_or(""),
                    limit.unwrap_or(12),
                )
                .await
                .map_err(|e| format!("{e}"))?
        }
        wisp_store::StateScope::Mainline { .. } => {
            let project_id = if all_projects.unwrap_or(false) {
                None
            } else {
                project_id.as_deref().or(Some(ap.id.as_str()))
            };
            state
                .store
                .search_artifacts(
                    project_id,
                    query.as_deref().unwrap_or(""),
                    limit.unwrap_or(12),
                    None,
                )
                .await
                .map_err(|e| format!("{e}"))?
        }
    };
    Ok(rows
        .into_iter()
        .map(|a| {
            let location = a.logical_path.clone().unwrap_or_else(|| a.path.clone());
            ArtifactInfo {
                id: a.id,
                name: a.name,
                kind: a.kind,
                path: a.path,
                location: Some(location),
                ts: a.ts,
                project_id: Some(a.project_id),
                project_name: Some(a.project_name),
                session_id: Some(a.session_id),
                session_title: Some(a.session_title),
                size_bytes: a.size_bytes,
                origin: Some(a.origin),
                logical_path: a.logical_path,
                source_discarded: a.source_discarded,
            }
        })
        .filter(artifact_info_is_visible)
        .collect())
}

/// Given candidate artifact file paths (as they appear in chat), return the
/// subset that can't be previewed: resolved against the project root and
/// missing on disk, or outside the root. The UI drops these so a stale
/// intermediate file doesn't linger as an artifact that 404s on click (#41).
#[tauri::command]
pub(super) fn missing_files(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let ap = state.active(window.label());
    Ok(paths
        .into_iter()
        .filter(|p| {
            wisp_tools::safety::validate_file_path(&ap.root, p)
                .map(|real| !real.exists())
                .unwrap_or(true)
        })
        .collect())
}

#[tauri::command]
pub(super) async fn read_artifact(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<FileContent, String> {
    let (working_project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    if !state
        .store
        .artifact_visible_in_scope(&id, &scope)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!("artifact '{id}' not found in the active state"));
    }
    let mut row = state
        .store
        .get_artifact_detail(&id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{id}' not found"))?;
    row.path = state
        .store
        .artifact_path_in_scope(&id, &scope)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("artifact '{id}' not found in the active state"))?;
    let root = if matches!(&scope, wisp_store::StateScope::Exploration { .. }) {
        working_project.root
    } else {
        PathBuf::from(row.project_root)
    };
    tokio::task::spawn_blocking(move || read_file_at(&root, row.path, None))
        .await
        .map_err(|e| format!("{e}"))?
}

#[tauri::command]
pub(super) async fn read_artifact_bytes(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    max_bytes: Option<u64>,
) -> Result<Response, String> {
    let (working_project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    if !state
        .store
        .artifact_visible_in_scope(&id, &scope)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!("artifact '{id}' not found in the active state"));
    }
    let mut row = state
        .store
        .get_artifact_detail(&id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{id}' not found"))?;
    row.path = state
        .store
        .artifact_path_in_scope(&id, &scope)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("artifact '{id}' not found in the active state"))?;
    let root = if matches!(&scope, wisp_store::StateScope::Exploration { .. }) {
        working_project.root
    } else {
        PathBuf::from(row.project_root)
    };
    let bytes =
        tokio::task::spawn_blocking(move || read_file_bytes_at(&root, &row.path, max_bytes))
            .await
            .map_err(|e| format!("{e}"))??;
    Ok(Response::new(bytes))
}

/// Save an immutable Artifact snapshot without exposing its private storage
/// path to the UI. This is the download counterpart of `read_artifact`.
#[tauri::command]
pub(super) async fn download_artifact(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (working_project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    if !state
        .store
        .artifact_visible_in_scope(&id, &scope)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!("artifact '{id}' not found in the active state"));
    }
    let artifact = state
        .store
        .get_artifact_detail(&id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("artifact '{id}' not found"))?;
    let storage_path = state
        .store
        .artifact_path_in_scope(&id, &scope)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("artifact '{id}' not found in the active state"))?;
    if storage_path.starts_with("ssh://") {
        crate::run_context::remote_files::refuse_if_source_discarded(&state.store, &storage_path)
            .await?;
    }
    let root = if matches!(&scope, wisp_store::StateScope::Exploration { .. }) {
        working_project.root
    } else {
        PathBuf::from(artifact.project_root)
    };
    let source = wisp_tools::safety::validate_file_path(&root, &storage_path)?;
    if !source.is_file() {
        return Err(format!(
            "artifact '{}' is no longer readable",
            artifact.name
        ));
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&artifact.name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(destination) = rx.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    let destination = PathBuf::from(destination.to_string());
    tokio::fs::copy(source, &destination)
        .await
        .map_err(|error| format!("copy failed: {error}"))?;
    Ok(Some(destination.display().to_string()))
}

/// Read the immutable artifact version captured by a message resource binding.
/// Resource previews must never follow the artifact's mutable latest-version pointer.
#[tauri::command]
pub(super) async fn read_artifact_version(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    version_id: String,
) -> Result<FileContent, String> {
    let version = state
        .store
        .get_artifact_version(&version_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact version '{version_id}' not found"))?;
    let (working_project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    if !state
        .store
        .artifact_visible_in_scope(&version.artifact_id, &scope)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "artifact version '{version_id}' not found in the active state"
        ));
    }
    let artifact = state
        .store
        .get_artifact_detail(&version.artifact_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{}' not found", version.artifact_id))?;
    let root = if matches!(&scope, wisp_store::StateScope::Exploration { .. }) {
        working_project.root
    } else {
        PathBuf::from(artifact.project_root)
    };
    tokio::task::spawn_blocking(move || read_file_at(&root, version.storage_path, None))
        .await
        .map_err(|e| format!("{e}"))?
}

#[tauri::command]
pub(super) async fn read_artifact_version_bytes(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    version_id: String,
    max_bytes: Option<u64>,
) -> Result<Response, String> {
    let version = state
        .store
        .get_artifact_version(&version_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact version '{version_id}' not found"))?;
    let (working_project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    if !state
        .store
        .artifact_visible_in_scope(&version.artifact_id, &scope)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "artifact version '{version_id}' not found in the active state"
        ));
    }
    let artifact = state
        .store
        .get_artifact_detail(&version.artifact_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{}' not found", version.artifact_id))?;
    let root = if matches!(&scope, wisp_store::StateScope::Exploration { .. }) {
        working_project.root
    } else {
        PathBuf::from(artifact.project_root)
    };
    let bytes = tokio::task::spawn_blocking(move || {
        read_file_bytes_at(&root, &version.storage_path, max_bytes)
    })
    .await
    .map_err(|e| format!("{e}"))??;
    Ok(Response::new(bytes))
}

/// Save the immutable artifact version behind an `artifact-version:` preview
/// path. This is the download counterpart of `read_artifact_version`: it must
/// never follow the artifact's mutable latest-version pointer, so previews
/// pinned by branch/exploration views download the exact bytes they display.
#[tauri::command]
pub(super) async fn download_artifact_version(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    version_id: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let version = state
        .store
        .get_artifact_version(&version_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact version '{version_id}' not found"))?;
    let (working_project, scope) =
        crate::exploration_commands::working_project_for_active_frame(&state, window.label())
            .await?;
    if !state
        .store
        .artifact_visible_in_scope(&version.artifact_id, &scope)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "artifact version '{version_id}' not found in the active state"
        ));
    }
    let artifact = state
        .store
        .get_artifact_detail(&version.artifact_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{}' not found", version.artifact_id))?;
    let root = if matches!(&scope, wisp_store::StateScope::Exploration { .. }) {
        working_project.root
    } else {
        PathBuf::from(artifact.project_root)
    };
    let source = wisp_tools::safety::validate_file_path(&root, &version.storage_path)?;
    if !source.is_file() {
        return Err(format!(
            "artifact '{}' is no longer readable",
            artifact.name
        ));
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&artifact.name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(destination) = rx.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    let destination = PathBuf::from(destination.to_string());
    tokio::fs::copy(source, &destination)
        .await
        .map_err(|error| format!("copy failed: {error}"))?;
    Ok(Some(destination.display().to_string()))
}
