use super::*;
use super::{ensure_active_frame, workspace_manifest, ActiveProject, AppState, ArtifactInfo};
use crate::file_browser::mime_for_path;
use base64::Engine;
use tauri::State;

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

async fn register_artifact_at(
    state: &AppState,
    label: &str,
    ap: &ActiveProject,
    path: String,
    content_type: Option<String>,
) -> Result<ArtifactInfo, String> {
    let real = wisp_tools::safety::validate_file_path(&ap.root, &path)?;
    let frame_id = ensure_active_frame(state, label, ap).await?;
    let id = Uuid::new_v4().to_string();
    let filename = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let mime = content_type.unwrap_or_else(|| mime_for_path(&real).to_string());
    let storage = real.to_string_lossy().into_owned();
    state
        .store
        .save_artifact(&id, &ap.id, &frame_id, &filename, &mime, &storage)
        .await
        .map_err(|e| format!("{e}"))?;
    let ts = chrono::Utc::now().timestamp();
    Ok(ArtifactInfo {
        id,
        name: filename,
        kind: mime,
        path: storage,
        ts,
        project_id: Some(ap.id.clone()),
        project_name: None,
        session_id: Some(frame_id),
        session_title: None,
        size_bytes: None,
        origin: Some("upload".into()),
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
    let ap = state.active(window.label());
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
    register_artifact_at(&state, window.label(), &ap, rel, None).await
}

#[tauri::command]
pub(super) async fn register_artifact(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    path: String,
    content_type: Option<String>,
) -> Result<ArtifactInfo, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    register_artifact_at(&state, window.label(), &ap, path, content_type).await
}

#[tauri::command]
pub(super) async fn save_workspace_file_by_kind(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    kind: workspace_manifest::WorkspaceFileKind,
    filename: String,
    content: String,
) -> Result<String, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let path =
        workspace_manifest::save_workspace_file(&ap.root, kind, &filename, content.as_bytes())?;
    Ok(path
        .strip_prefix(&ap.root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/"))
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
        .map(|(id, name, ct, path, ts)| ArtifactInfo {
            id,
            name: name.clone(),
            kind: ct,
            path,
            ts,
            project_id: None,
            project_name: None,
            session_id: None,
            session_title: None,
            size_bytes: None,
            origin: None,
        })
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
    let ap = state.active(window.label());
    let project_id = if all_projects.unwrap_or(false) {
        None
    } else {
        project_id.as_deref().or(Some(ap.id.as_str()))
    };
    let rows = state
        .store
        .search_artifacts(
            project_id,
            query.as_deref().unwrap_or(""),
            limit.unwrap_or(12),
            None,
        )
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|a| ArtifactInfo {
            id: a.id,
            name: a.name,
            kind: a.kind,
            path: a.path,
            ts: a.ts,
            project_id: Some(a.project_id),
            project_name: Some(a.project_name),
            session_id: Some(a.session_id),
            session_title: Some(a.session_title),
            size_bytes: a.size_bytes,
            origin: Some(a.origin),
        })
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
    id: String,
) -> Result<FileContent, String> {
    let row = state
        .store
        .get_artifact_detail(&id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{id}' not found"))?;
    let root = PathBuf::from(row.project_root);
    tokio::task::spawn_blocking(move || read_file_at(&root, row.path, None))
        .await
        .map_err(|e| format!("{e}"))?
}

#[tauri::command]
pub(super) async fn read_artifact_bytes(
    state: State<'_, AppState>,
    id: String,
    max_bytes: Option<u64>,
) -> Result<Response, String> {
    let row = state
        .store
        .get_artifact_detail(&id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{id}' not found"))?;
    let root = PathBuf::from(row.project_root);
    let bytes =
        tokio::task::spawn_blocking(move || read_file_bytes_at(&root, &row.path, max_bytes))
            .await
            .map_err(|e| format!("{e}"))??;
    Ok(Response::new(bytes))
}

/// Read the immutable artifact version captured by a message resource binding.
/// Resource previews must never follow the artifact's mutable latest-version pointer.
#[tauri::command]
pub(super) async fn read_artifact_version(
    state: State<'_, AppState>,
    version_id: String,
) -> Result<FileContent, String> {
    let version = state
        .store
        .get_artifact_version(&version_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact version '{version_id}' not found"))?;
    let artifact = state
        .store
        .get_artifact_detail(&version.artifact_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{}' not found", version.artifact_id))?;
    let root = PathBuf::from(artifact.project_root);
    tokio::task::spawn_blocking(move || read_file_at(&root, version.storage_path, None))
        .await
        .map_err(|e| format!("{e}"))?
}

#[tauri::command]
pub(super) async fn read_artifact_version_bytes(
    state: State<'_, AppState>,
    version_id: String,
    max_bytes: Option<u64>,
) -> Result<Response, String> {
    let version = state
        .store
        .get_artifact_version(&version_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact version '{version_id}' not found"))?;
    let artifact = state
        .store
        .get_artifact_detail(&version.artifact_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("artifact '{}' not found", version.artifact_id))?;
    let root = PathBuf::from(artifact.project_root);
    let bytes = tokio::task::spawn_blocking(move || {
        read_file_bytes_at(&root, &version.storage_path, max_bytes)
    })
    .await
    .map_err(|e| format!("{e}"))??;
    Ok(Response::new(bytes))
}
