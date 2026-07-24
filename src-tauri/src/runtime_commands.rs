//! Runtime Commands split out of lib.rs; shared state/helpers stay in the crate root.

use super::*;

#[tauri::command]
pub(super) async fn list_execution_contexts(
    state: State<'_, AppState>,
) -> Result<Vec<wisp_store::ExecutionContext>, String> {
    state
        .store
        .list_execution_contexts()
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) fn list_runtimes(state: State<'_, AppState>) -> Vec<wisp_runtime::RuntimeInfo> {
    state.runtime_manager.list()
}

#[tauri::command]
pub(super) async fn inspect_runtime(
    state: State<'_, AppState>,
    project_id: String,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
) -> Result<wisp_runtime::RuntimeObjectList, String> {
    state
        .runtime_manager
        .inspect(&wisp_runtime::RuntimeKey {
            project_id,
            context_id,
            language,
        })
        .await
        .map_err(|error| error.to_string())
}

/// Run code the user selected in the file preview against their bound runtime.
/// Deferred in the runtime design until the UI gained a code editor; it has one
/// now. The user is looking at the code they pressed Run on, so this path is
/// deliberately outside the agent tool-approval flow.
///
/// Returns console text. Code that raised is still `Ok`: `format_response` tags
/// it `[error]` exactly as the agent tools render it. `Err` means the runtime
/// itself never produced a result.
#[tauri::command]
pub(super) async fn execute_runtime(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
    code: String,
) -> Result<String, String> {
    if code.len() > wisp_runtime::MAX_CODE_BYTES {
        return Err(format!(
            "Selection exceeds the {} byte runtime limit.",
            wisp_runtime::MAX_CODE_BYTES
        ));
    }
    let project = state.active(window.label());
    let key = wisp_runtime::RuntimeKey {
        project_id: project.id,
        context_id,
        language,
    };
    let mut execution = state
        .runtime_manager
        .execute(&key, &project.root, code)
        .await
        .map_err(|error| error.to_string())?;
    loop {
        match execution.recv().await {
            // ponytail: buffered, not streamed — the final frame repeats every
            // chunk. Stream to the console when a cell runs long enough to care.
            Some(wisp_runtime::RuntimeEvent::Stdout(_)) => {}
            Some(wisp_runtime::RuntimeEvent::Finished(Ok(response))) => {
                return Ok(wisp_runtime::format_response(&response))
            }
            Some(wisp_runtime::RuntimeEvent::Finished(Err(error))) => return Err(error),
            None => return Err("Runtime ended before returning a result.".into()),
        }
    }
}

#[tauri::command]
pub(super) async fn start_runtime(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
) -> Result<wisp_runtime::RuntimeInfo, String> {
    let project = state.active(window.label());
    state
        .runtime_manager
        .start(
            wisp_runtime::RuntimeKey {
                project_id: project.id,
                context_id,
                language,
            },
            project.root,
        )
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn stop_runtime(
    state: State<'_, AppState>,
    project_id: String,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
) -> Result<Option<wisp_runtime::RuntimeInfo>, String> {
    Ok(state
        .runtime_manager
        .stop(&wisp_runtime::RuntimeKey {
            project_id,
            context_id,
            language,
        })
        .await)
}

#[tauri::command]
pub(super) async fn restart_runtime(
    state: State<'_, AppState>,
    project_id: String,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
) -> Result<wisp_runtime::RuntimeInfo, String> {
    let (_, workspace) = state
        .store
        .get_project(&project_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Project not found: {project_id}"))?;
    let root = ensure_writable(PathBuf::from(workspace), &state.app_data);
    state
        .runtime_manager
        .restart(
            wisp_runtime::RuntimeKey {
                project_id,
                context_id,
                language,
            },
            root,
        )
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn list_runs(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<wisp_store::RunRecord>, String> {
    let ap = state.active(window.label());
    state
        .store
        .list_runs_by_project(&ap.id)
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) async fn get_run(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<wisp_store::RunRecord, String> {
    let ap = state.active(window.label());
    let run = state
        .store
        .get_run(&run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Run not found".to_string())?;
    if run.project_id != ap.id {
        return Err("Run does not belong to the active project".into());
    }
    Ok(run)
}

#[tauri::command]
pub(super) async fn cancel_run(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<wisp_store::RunRecord, String> {
    let ap = state.active(window.label());
    let run = state
        .store
        .get_run(&run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Run not found".to_string())?;
    if run.project_id != ap.id {
        return Err("Run does not belong to the active project".into());
    }
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state.run_manager.cancel(&state.store, &run_id).await?;
    state
        .store
        .get_run(&run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Run disappeared after cancellation".to_string())
}
