//! Effective storage locations for a (project × execution context): stored
//! preferences when the user confirmed them, deterministic defaults otherwise.

use crate::exploration_commands;
use crate::AppState;
use serde::Serialize;
use tauri::State;
use wisp_store::ContextStoragePrefs;

pub(crate) use wisp_runs::storage_prefs::effective_prefs;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ContextStoragePrefsView {
    pub context_id: String,
    pub remote_data_root: String,
    pub remote_workdir_root: String,
    pub local_results_dir: String,
    /// False while the user has never confirmed locations for this
    /// project × context; the UI prompts once on first enable.
    pub confirmed: bool,
}

#[tauri::command]
pub(crate) async fn get_context_storage_prefs(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
) -> Result<ContextStoragePrefsView, String> {
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let (prefs, confirmed) = effective_prefs(&state.store, &ap.id, &context_id).await?;
    Ok(ContextStoragePrefsView {
        context_id,
        remote_data_root: prefs.remote_data_root,
        remote_workdir_root: prefs.remote_workdir_root,
        local_results_dir: prefs.local_results_dir,
        confirmed,
    })
}

#[tauri::command]
pub(crate) async fn set_context_storage_prefs(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
    remote_data_root: String,
    remote_workdir_root: String,
    local_results_dir: String,
) -> Result<ContextStoragePrefsView, String> {
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let prefs = ContextStoragePrefs {
        project_id: ap.id.clone(),
        context_id: context_id.clone(),
        remote_data_root: remote_data_root.trim().to_string(),
        remote_workdir_root: remote_workdir_root.trim().to_string(),
        local_results_dir: local_results_dir
            .trim()
            .trim_end_matches('/')
            .replace('\\', "/"),
        created_at: 0,
        updated_at: 0,
    };
    state
        .store
        .upsert_context_storage_prefs(&prefs)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ContextStoragePrefsView {
        context_id,
        remote_data_root: prefs.remote_data_root,
        remote_workdir_root: prefs.remote_workdir_root,
        local_results_dir: prefs.local_results_dir,
        confirmed: true,
    })
}
