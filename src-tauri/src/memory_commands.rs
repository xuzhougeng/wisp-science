//! Memory Commands split out of lib.rs; shared state/helpers stay in the crate root.

use super::*;

#[derive(Serialize, Clone)]
pub(super) struct MemoryView {
    enabled: bool,
    today_file: String,
    files: Vec<MemoryFile>,
}

#[tauri::command]
pub(super) fn list_memory(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active(window.label());
    Ok(list_memory_files(&ap.memory))
}

#[tauri::command]
pub(super) async fn get_memory_view(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<MemoryView, String> {
    let ap = state.active(window.label());
    Ok(MemoryView {
        enabled: load_memory_enabled(&state.store).await,
        today_file: chrono::Local::now().format("%Y-%m-%d.md").to_string(),
        files: list_memory_files(&ap.memory),
    })
}

#[tauri::command]
pub(super) async fn set_memory_enabled(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    enabled: bool,
) -> Result<MemoryView, String> {
    save_memory_enabled(&state.store, enabled).await?;
    clear_idle_agents(&state).await;
    let ap = state.active(window.label());
    Ok(MemoryView {
        enabled,
        today_file: chrono::Local::now().format("%Y-%m-%d.md").to_string(),
        files: list_memory_files(&ap.memory),
    })
}

#[tauri::command]
pub(super) fn read_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
) -> Result<String, String> {
    let ap = state.active(window.label());
    let path = memory_file_path(&ap.memory, &name)?;
    if !path.is_file() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) fn write_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    content: String,
) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let path = memory_file_path(&ap.memory, &name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("{e}"))?;
    Ok(list_memory_files(&ap.memory))
}

#[tauri::command]
pub(super) fn delete_memory_file(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let path = memory_file_path(&ap.memory, &name)?;
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|e| format!("{e}"))?;
    }
    Ok(list_memory_files(&ap.memory))
}

#[tauri::command]
pub(super) fn clear_memory(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<MemoryFile>, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let Ok(rd) = std::fs::read_dir(ap.memory.dir()) else {
        return Ok(vec![]);
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(list_memory_files(&ap.memory))
}
