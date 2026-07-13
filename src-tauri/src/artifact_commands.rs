use super::{ensure_active_frame, workspace_manifest, ActiveProject, AppState, ArtifactInfo};
use crate::file_browser::mime_for_path;
use base64::Engine;
use tauri::State;
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
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|e| format!("invalid base64: {e}"))?;
    let cap = 32 * 1024 * 1024;
    if bytes.len() > cap {
        return Err(format!("file exceeds {cap} byte limit"));
    }
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
}
