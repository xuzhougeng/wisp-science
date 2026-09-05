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

/// The conversation whose runtimes this window's commands act on. Runtimes
/// are keyed per conversation (#911), so the panel and the file-preview
/// console follow the frame the window is viewing.
fn active_session_id(state: &AppState, window_label: &str) -> String {
    state.active_frame(window_label).unwrap_or_default()
}

/// Whether one runtime belongs in this window's Runtimes panel. The active
/// project shows the viewed conversation's runtimes plus scope-shared ones;
/// other projects keep all their mainline runtimes visible so a large kernel
/// is never invisible.
fn runtime_visible(
    key: &wisp_runtime::RuntimeKey,
    scope: &wisp_store::StateScope,
    active_session: &str,
) -> bool {
    let session_matches = key.session_id.is_empty() || key.session_id == active_session;
    match scope {
        wisp_store::StateScope::Mainline { project_id } => {
            key.scope_key == wisp_runtime::MAINLINE_RUNTIME_SCOPE
                && (key.project_id != *project_id || session_matches)
        }
        wisp_store::StateScope::Exploration {
            project_id,
            exploration_id,
        } => key.project_id == *project_id && key.scope_key == *exploration_id && session_matches,
    }
}

/// Pick the runtime a UI command targets: prefer the viewed conversation's
/// runtime, fall back to an existing scope-shared one, and default new
/// runtimes to the conversation identity when a conversation is open.
fn resolve_runtime_key(
    manager: &wisp_runtime::RuntimeManager,
    project_id: String,
    scope_key: String,
    active_session: &str,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
) -> wisp_runtime::RuntimeKey {
    let session_key = wisp_runtime::RuntimeKey {
        project_id,
        scope_key,
        session_id: active_session.to_string(),
        context_id,
        language,
    };
    if active_session.is_empty() {
        return session_key;
    }
    let exists = |session: &str| {
        manager.list().iter().any(|runtime| {
            runtime.key.project_id == session_key.project_id
                && runtime.key.scope_key == session_key.scope_key
                && runtime.key.session_id == session
                && runtime.key.context_id == session_key.context_id
                && runtime.key.language == session_key.language
        })
    };
    if exists(active_session) || !exists("") {
        session_key
    } else {
        wisp_runtime::RuntimeKey {
            session_id: String::new(),
            ..session_key
        }
    }
}

#[tauri::command]
pub(super) async fn list_runtimes(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<wisp_runtime::RuntimeInfo>, String> {
    let (_, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let active_session = active_session_id(&state, window.label());
    Ok(state
        .runtime_manager
        .list()
        .into_iter()
        .filter(|runtime| runtime_visible(&runtime.key, &scope, &active_session))
        .collect())
}

#[tauri::command]
pub(super) async fn inspect_runtime(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    project_id: String,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
) -> Result<wisp_runtime::RuntimeObjectList, String> {
    let (_, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    if matches!(&scope, wisp_store::StateScope::Exploration { .. })
        && scope.project_id() != project_id
    {
        return Err(
            "exploration_scope_violation: cross-project runtime inspection is disabled".into(),
        );
    }
    let (scope_key, active_session) = if scope.project_id() == project_id {
        (scope.scope_key(), active_session_id(&state, window.label()))
    } else {
        (wisp_runtime::MAINLINE_RUNTIME_SCOPE, String::new())
    };
    let key = resolve_runtime_key(
        &state.runtime_manager,
        project_id,
        scope_key.into(),
        &active_session,
        context_id,
        language,
    );
    state
        .runtime_manager
        .inspect(&key)
        .await
        .map_err(|error| error.to_string())
}

/// What one user-driven runtime execution hands back to the workbench:
/// rendered console text plus any plots the cell produced (base64 PNGs).
/// Mirrored by `wisp_dto::RuntimeExecutionSummary`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeExecutionSummary {
    pub(crate) text: String,
    pub(crate) plots: Vec<String>,
}

/// Run code the user selected in the file preview — or typed into the
/// workbench console — against their bound runtime. Deferred in the runtime
/// design until the UI gained a code editor; it has one now. The user is
/// looking at the code they pressed Run on, so this path is deliberately
/// outside the agent tool-approval flow.
///
/// Returns console text and plots. Code that raised is still `Ok`:
/// `format_response` tags it `[error]` exactly as the agent tools render it.
/// `Err` means the runtime itself never produced a result.
#[tauri::command]
pub(super) async fn execute_runtime(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
    code: String,
) -> Result<RuntimeExecutionSummary, String> {
    if code.len() > wisp_runtime::MAX_CODE_BYTES {
        return Err(format!(
            "Selection exceeds the {} byte runtime limit.",
            wisp_runtime::MAX_CODE_BYTES
        ));
    }
    let (project, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let _project_activity = state.begin_project_activity(&project.id)?;
    exploration_commands::require_writable_scope(&state.store, &scope).await?;
    if exploration_isolation::is_host_local_context(&context_id) {
        if let Some(boundary) =
            exploration_isolation::boundary_for_scope(&state.store, &scope).await?
        {
            boundary.check_local_source(&code)?;
        }
    }
    let key = resolve_runtime_key(
        &state.runtime_manager,
        project.id.clone(),
        scope.scope_key().to_string(),
        &active_session_id(&state, window.label()),
        context_id,
        language,
    );
    let execution = state
        .runtime_manager
        .execute(&key, &project.root, code)
        .await
        .map_err(|error| error.to_string())?;
    finish_runtime_execution(&state, &scope, execution, |response, _| {
        wisp_runtime::format_response(response)
    })
    .await
}

/// Run a whole saved project script in the runtime the editor bound it to.
/// The editor saves the buffer first, so the bytes read here are the bytes the
/// user is looking at, and the reported hash is a real provenance record rather
/// than a hash of a transient buffer. Reuses the agent tools' reader and
/// `source_name` so a traceback names the script, not an anonymous cell.
#[tauri::command]
pub(super) async fn execute_runtime_script(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
    script_path: String,
) -> Result<RuntimeExecutionSummary, String> {
    let (project, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let _project_activity = state.begin_project_activity(&project.id)?;
    exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let script = wisp_runtime::read_project_script(
        &project.root,
        &script_path,
        language.script_extension(),
    )?;
    if exploration_isolation::is_host_local_context(&context_id) {
        if let Some(boundary) =
            exploration_isolation::boundary_for_scope(&state.store, &scope).await?
        {
            boundary.check_local_source(&script.code)?;
        }
    }
    let key = resolve_runtime_key(
        &state.runtime_manager,
        project.id.clone(),
        scope.scope_key().to_string(),
        &active_session_id(&state, window.label()),
        context_id,
        language,
    );
    let execution = state
        .runtime_manager
        .execute_with_options(
            &key,
            &project.root,
            script.code,
            wisp_runtime::RuntimeExecutionOptions {
                source_name: Some(script.provenance.path.clone()),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    finish_runtime_execution(&state, &scope, execution, |response, runtime| {
        wisp_runtime::format_script_response(response, Some(&script.provenance), runtime)
    })
    .await
}

/// Drain one execution until the worker finishes, then bump the scope so the
/// workbench refreshes. Shared by cell and whole-script runs because the only
/// difference is how the console text is formatted.
async fn finish_runtime_execution(
    state: &AppState,
    scope: &wisp_store::StateScope,
    mut execution: wisp_runtime::RuntimeExecution,
    format: impl FnOnce(&wisp_runtime::KernelResp, &wisp_runtime::RuntimeInfo) -> String,
) -> Result<RuntimeExecutionSummary, String> {
    let runtime = execution.info().clone();
    loop {
        match execution.recv().await {
            // ponytail: buffered, not streamed — the final frame repeats every
            // chunk. Stream to the console when a cell runs long enough to care.
            Some(wisp_runtime::RuntimeEvent::Stdout(_)) => {}
            Some(wisp_runtime::RuntimeEvent::Finished(result)) => {
                state
                    .store
                    .bump_state_generation(scope)
                    .await
                    .map_err(|error| error.to_string())?;
                return result
                    .map(|response| RuntimeExecutionSummary {
                        text: format(&response, &runtime),
                        plots: response.plots,
                    })
                    .map_err(|error| error.to_string());
            }
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
    let (project, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let _project_activity = state.begin_project_activity(&project.id)?;
    exploration_commands::require_writable_scope(&state.store, &scope).await?;
    let key = resolve_runtime_key(
        &state.runtime_manager,
        project.id.clone(),
        scope.scope_key().to_string(),
        &active_session_id(&state, window.label()),
        context_id,
        language,
    );
    state
        .runtime_manager
        .start(key, project.root)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn dismiss_runtime(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    runtime_id: String,
) -> Result<(), String> {
    let (_, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    if let Some(info) = state
        .runtime_manager
        .list()
        .into_iter()
        .find(|info| info.runtime_id == runtime_id)
    {
        if !runtime_visible(
            &info.key,
            &scope,
            &active_session_id(&state, window.label()),
        ) {
            return Err("runtime is outside this window's scope".into());
        }
    }
    state
        .runtime_manager
        .dismiss_dead(&runtime_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn stop_runtime(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    project_id: String,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
) -> Result<Option<wisp_runtime::RuntimeInfo>, String> {
    let (_, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    if matches!(&scope, wisp_store::StateScope::Exploration { .. })
        && scope.project_id() != project_id
    {
        return Err(
            "exploration_scope_violation: cross-project runtime control is disabled".into(),
        );
    }
    if scope.project_id() != project_id {
        // A row from another project may be any of its conversations'
        // runtimes; the panel cannot address one, so free them all.
        let targets = state
            .runtime_manager
            .list()
            .into_iter()
            .map(|runtime| runtime.key)
            .filter(|key| {
                key.project_id == project_id
                    && key.scope_key == wisp_runtime::MAINLINE_RUNTIME_SCOPE
                    && key.context_id == context_id
                    && key.language == language
            })
            .collect::<Vec<_>>();
        let mut stopped = None;
        for key in targets {
            stopped = state.runtime_manager.stop(&key).await.or(stopped);
        }
        return Ok(stopped);
    }
    let key = resolve_runtime_key(
        &state.runtime_manager,
        project_id,
        scope.scope_key().to_string(),
        &active_session_id(&state, window.label()),
        context_id,
        language,
    );
    Ok(state.runtime_manager.stop(&key).await)
}

#[tauri::command]
pub(super) async fn restart_runtime(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    project_id: String,
    context_id: String,
    language: wisp_runtime::RuntimeLanguage,
) -> Result<wisp_runtime::RuntimeInfo, String> {
    let (working, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    if matches!(&scope, wisp_store::StateScope::Exploration { .. })
        && scope.project_id() != project_id
    {
        return Err(
            "exploration_scope_violation: cross-project runtime control is disabled".into(),
        );
    }
    let same_project = working.id == project_id;
    let (root, target_scope) = if same_project {
        (working.root, scope)
    } else {
        let (_, workspace) = state
            .store
            .get_project(&project_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Project not found: {project_id}"))?;
        (
            ensure_writable(PathBuf::from(workspace), &state.app_data),
            wisp_store::StateScope::mainline(project_id.clone()),
        )
    };
    let _project_activity = state.begin_project_activity(&project_id)?;
    exploration_commands::require_writable_scope(&state.store, &target_scope).await?;
    let active_session = if same_project {
        active_session_id(&state, window.label())
    } else {
        String::new()
    };
    let key = resolve_runtime_key(
        &state.runtime_manager,
        project_id,
        target_scope.scope_key().to_string(),
        &active_session,
        context_id,
        language,
    );
    state
        .runtime_manager
        .restart(key, root)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn list_runs(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<wisp_store::RunSummary>, String> {
    let (_, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    state
        .store
        .list_run_summaries_in_scope(&scope)
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) async fn get_run_detail(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<wisp_store::RunRecord, String> {
    let (_, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    if !state
        .store
        .run_visible_in_scope(&run_id, &scope)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Run is not visible in the active state scope".into());
    }
    state
        .store
        .get_run(&run_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Run not found".into())
}

#[tauri::command]
pub(super) async fn cancel_run(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<wisp_store::RunRecord, String> {
    let (ap, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let run = state
        .store
        .get_run(&run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Run not found".to_string())?;
    if run.project_id != ap.id {
        return Err("Run does not belong to the active project".into());
    }
    if state
        .store
        .run_state_scope(&run_id)
        .await
        .map_err(|error| error.to_string())?
        .as_ref()
        != Some(&scope)
    {
        return Err("Run is not visible in the active state scope".into());
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

/// Retry output registration/download for a succeeded Run whose declared
/// outputs were never harvested.
#[tauri::command]
pub(super) async fn harvest_run(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<wisp_store::RunRecord, String> {
    let (ap, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let run = state
        .store
        .get_run(&run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Run not found".to_string())?;
    if run.project_id != ap.id {
        return Err("Run does not belong to the active project".into());
    }
    if state
        .store
        .run_state_scope(&run_id)
        .await
        .map_err(|error| error.to_string())?
        .as_ref()
        != Some(&scope)
    {
        return Err("Run is not visible in the active state scope".into());
    }
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state.run_manager.harvest_run(&state.store, &run_id).await?;
    state
        .store
        .get_run(&run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Run disappeared after harvest".to_string())
}

async fn scoped_run(
    state: &State<'_, AppState>,
    window: &tauri::WebviewWindow,
    run_id: &str,
) -> Result<(), String> {
    let (ap, scope) =
        exploration_commands::working_project_for_active_frame(state, window.label()).await?;
    let run = state
        .store
        .get_run(run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Run not found".to_string())?;
    if run.project_id != ap.id {
        return Err("Run does not belong to the active project".into());
    }
    if state
        .store
        .run_state_scope(run_id)
        .await
        .map_err(|error| error.to_string())?
        .as_ref()
        != Some(&scope)
    {
        return Err("Run is not visible in the active state scope".into());
    }
    Ok(())
}

/// One page of one directory level of a finished Run's server workspace.
/// Ephemeral browse data for the run-review modal; never persisted.
#[tauri::command]
pub(super) async fn list_run_workspace_files(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
    path: Option<String>,
    name_filter: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<crate::run_context::WorkspaceListing, String> {
    scoped_run(&state, &window, &run_id).await?;
    state
        .run_manager
        .list_run_workspace_files(
            &state.store,
            &run_id,
            path.as_deref().unwrap_or_default(),
            name_filter.as_deref().unwrap_or_default(),
            offset.unwrap_or(0),
            limit.unwrap_or(200),
        )
        .await
}

/// Download the user's selection from a finished Run's workspace and register
/// it as project artifacts (directories arrive as one archive each).
#[tauri::command]
pub(super) async fn download_run_files(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
    files: Option<Vec<String>>,
    dirs: Option<Vec<String>>,
) -> Result<Vec<crate::harvest::HarvestedArtifact>, String> {
    scoped_run(&state, &window, &run_id).await?;
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .run_manager
        .download_run_files(
            &state.store,
            &run_id,
            &files.unwrap_or_default(),
            &dirs.unwrap_or_default(),
        )
        .await
}

/// Whether the results-review modal is worth auto-opening for this Run: an
/// unresolved product decision exists (unharvested declared outputs, or no
/// declared outputs but files present in the workspace) and the user has not
/// dismissed the prompt. The UI asks this once per candidate at turn end.
#[tauri::command]
pub(super) async fn should_prompt_run_review(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<bool, String> {
    scoped_run(&state, &window, &run_id).await?;
    state
        .run_manager
        .should_prompt_run_review(&state.store, &run_id)
        .await
}

/// Persist that the user closed this Run's results-review prompt so it never
/// auto-opens again. Manual review from the run card stays available.
#[tauri::command]
pub(super) async fn dismiss_run_review(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
) -> Result<(), String> {
    scoped_run(&state, &window, &run_id).await?;
    state
        .store
        .mark_run_review_dismissed(&run_id)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Delete the user's selection inside a finished Run's workspace.
#[tauri::command]
pub(super) async fn delete_run_files(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    scoped_run(&state, &window, &run_id).await?;
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .run_manager
        .delete_run_files(&state.store, &run_id, &paths)
        .await
}

/// Ledgered files this project placed on one SSH server, with liveness state.
#[tauri::command]
pub(super) async fn list_remote_files(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
) -> Result<Vec<crate::run_context::remote_files::RemoteFileView>, String> {
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    crate::run_context::remote_files::list_remote_files(&state.store, &ap.id, &context_id).await
}

/// Delete ledgered files from a server. `force` carries the user's explicit
/// confirmation for entries a run still references.
#[tauri::command]
pub(super) async fn remove_remote_files(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
    ids: Vec<String>,
    force: Option<bool>,
) -> Result<Vec<crate::run_context::remote_files::RemoteFileView>, String> {
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let context = state
        .store
        .get_execution_context(&context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    if context.kind != wisp_store::ExecutionContextKind::Ssh {
        return Err("Remote file cleanup requires an SSH context".into());
    }
    let _project_activity = state.begin_project_activity(&ap.id)?;
    crate::run_context::remote_files::remove_remote_files(
        &state.store,
        state.run_manager.runner_ref(),
        &ap.id,
        &context,
        &ids,
        force.unwrap_or(false),
    )
    .await?;
    crate::run_context::remote_files::list_remote_files(&state.store, &ap.id, &context_id).await
}

/// What dropping this server would abandon: still-remote artifact references
/// and ledgered files.
#[tauri::command]
pub(super) async fn context_disposal_report(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
) -> Result<crate::run_context::remote_files::ContextDisposalReport, String> {
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let context = state
        .store
        .get_execution_context(&context_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Execution context not found: {context_id}"))?;
    crate::run_context::remote_files::context_disposal_report(&state.store, &ap.id, &context).await
}

/// Delete a terminal Run's server-side workspace. `force` carries the user's
/// explicit confirmation when unharvested outputs would be lost.
#[tauri::command]
pub(super) async fn cleanup_run_workspace(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_id: String,
    force: Option<bool>,
) -> Result<wisp_store::RunRecord, String> {
    let (ap, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let run = state
        .store
        .get_run(&run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Run not found".to_string())?;
    if run.project_id != ap.id {
        return Err("Run does not belong to the active project".into());
    }
    if state
        .store
        .run_state_scope(&run_id)
        .await
        .map_err(|error| error.to_string())?
        .as_ref()
        != Some(&scope)
    {
        return Err("Run is not visible in the active state scope".into());
    }
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .run_manager
        .cleanup_run_workspace(&state.store, &run_id, force.unwrap_or(false))
        .await
}

#[cfg(test)]
mod tests {
    use super::{resolve_runtime_key, runtime_visible};
    use std::path::PathBuf;
    use wisp_runtime::{RuntimeKey, RuntimeManager};

    fn manager() -> RuntimeManager {
        RuntimeManager::local(
            PathBuf::from("unused-app-data"),
            PathBuf::from("unused-worker.py"),
            None,
            vec![],
        )
    }

    #[test]
    fn mainline_panel_shows_own_conversation_shared_and_other_projects() {
        let scope = wisp_store::StateScope::mainline("active");
        let own = RuntimeKey::local_python("active").with_session("frame-1");
        let sibling = RuntimeKey::local_python("active").with_session("frame-2");
        let shared = RuntimeKey::local_python("active");
        let other_project = RuntimeKey::local_python("other").with_session("frame-9");
        let exploration =
            RuntimeKey::python_in_scope("active", "exploration-1", wisp_runtime::LOCAL_CONTEXT_ID);

        assert!(runtime_visible(&own, &scope, "frame-1"));
        assert!(!runtime_visible(&sibling, &scope, "frame-1"));
        assert!(runtime_visible(&shared, &scope, "frame-1"));
        assert!(runtime_visible(&other_project, &scope, "frame-1"));
        assert!(!runtime_visible(&exploration, &scope, "frame-1"));
    }

    #[test]
    fn exploration_panel_is_limited_to_its_own_scope_and_conversation() {
        let scope = wisp_store::StateScope::exploration("p", "exploration-1");
        let own = RuntimeKey::python_in_scope("p", "exploration-1", wisp_runtime::LOCAL_CONTEXT_ID)
            .with_session("frame-1");
        let foreign_session = own.clone().with_session("frame-2");
        let mainline = RuntimeKey::local_python("p");

        assert!(runtime_visible(&own, &scope, "frame-1"));
        assert!(!runtime_visible(&foreign_session, &scope, "frame-1"));
        assert!(!runtime_visible(&mainline, &scope, "frame-1"));
    }

    #[tokio::test]
    async fn resolver_prefers_the_viewed_conversation_and_falls_back_to_shared() {
        let manager = manager();
        // Nothing running: a viewed conversation owns any new runtime.
        let key = resolve_runtime_key(
            &manager,
            "p".into(),
            wisp_runtime::MAINLINE_RUNTIME_SCOPE.into(),
            "frame-1",
            "local".into(),
            wisp_runtime::RuntimeLanguage::Python,
        );
        assert_eq!(key.session_id, "frame-1");

        // A scope-shared runtime exists (registered even though the fake
        // worker path cannot launch): commands fall back to it.
        let shared = RuntimeKey::local_python("p");
        let _ = manager.start(shared.clone(), PathBuf::from("p")).await;
        let key = resolve_runtime_key(
            &manager,
            "p".into(),
            wisp_runtime::MAINLINE_RUNTIME_SCOPE.into(),
            "frame-1",
            "local".into(),
            wisp_runtime::RuntimeLanguage::Python,
        );
        assert!(key.session_id.is_empty());

        // No viewed conversation: the shared identity is the target.
        let key = resolve_runtime_key(
            &manager,
            "p".into(),
            wisp_runtime::MAINLINE_RUNTIME_SCOPE.into(),
            "",
            "local".into(),
            wisp_runtime::RuntimeLanguage::Python,
        );
        assert!(key.session_id.is_empty());
        manager.shutdown_all().await;
    }
}
