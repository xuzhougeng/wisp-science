//! App Commands split out of lib.rs; shared state/helpers stay in the crate root.

use super::*;

/// Desktop notification for task status (#327). No-op while any app window is
/// focused (the in-app UI already shows the state) or when disabled in settings.
/// Clicking the notification navigates to the session it was about (#434) —
/// see `pending_notify_targets`.
#[tauri::command]
pub(super) async fn notify_user(
    window: tauri::Window,
    state: State<'_, AppState>,
    title: String,
    body: String,
    session_id: String,
) -> Result<(), String> {
    if app_has_focus() || !load_notifications_enabled(&state.store).await {
        return Ok(());
    }
    // Arm click-to-open before showing: on this window's next focus it jumps to
    // the session. Skipped if the session's project can't be resolved (the
    // notification still shows, just without navigation).
    if let Ok(Some(project_id)) = state.store.frame_project_id(&session_id).await {
        pending_notify_targets().lock().unwrap().insert(
            window.label().to_string(),
            serde_json::json!({ "projectId": project_id, "sessionId": session_id }),
        );
    }
    use tauri_plugin_notification::NotificationExt;
    window
        .app_handle()
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(super) async fn pick_directory(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p);
    });
    let picked = rx.await.map_err(|e| format!("{e}"))?;
    Ok(picked.map(|fp| fp.to_string()))
}

/// Save UI-provided text via the native save dialog. Returns the saved path,
/// or `None` if the user cancelled.
#[tauri::command]
pub(super) async fn export_text_file(
    app: AppHandle,
    default_name: String,
    content: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(dest) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None);
    };
    let dest = std::path::PathBuf::from(dest.to_string());
    tokio::fs::write(&dest, content)
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    Ok(Some(dest.to_string_lossy().into_owned()))
}

/// Pick a JSON file via the native open dialog and return its content, or
/// `None` if the user cancelled.
#[tauri::command]
pub(super) async fn import_json_file(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("JSON", &["json"])
        .pick_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(path) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None);
    };
    let content = tokio::fs::read_to_string(std::path::PathBuf::from(path.to_string()))
        .await
        .map_err(|e| format!("read failed: {e}"))?;
    Ok(Some(content))
}

/// Copy a workspace file to a user-chosen location via the native save dialog.
/// Returns the saved path, or `None` if the user cancelled.
pub(super) fn parse_ssh_artifact_uri(uri: &str) -> Option<(String, String)> {
    let rest = uri.strip_prefix("ssh://")?;
    let (context, path) = rest.split_once('/')?;
    if context.is_empty() || path.is_empty() {
        return None;
    }
    let remote_path = if path.starts_with("~/") {
        path.to_string()
    } else {
        format!("/{path}")
    };
    Some((format!("ssh:{context}"), remote_path))
}

#[tauri::command]
pub(super) async fn download_file(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    path: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let ap = state.active(window.label());
    let remote = parse_ssh_artifact_uri(&path);
    let local = if remote.is_none() {
        let real = wisp_tools::safety::validate_file_path(&ap.root, &path)?;
        if !real.is_file() {
            return Err(format!("file not found: {path}"));
        }
        Some(real)
    } else {
        None
    };
    let default_name = std::path::Path::new(
        remote
            .as_ref()
            .map(|(_, path)| path.as_str())
            .unwrap_or_else(|| local.as_ref().unwrap().to_str().unwrap_or("download")),
    )
    .file_name()
    .and_then(|n| n.to_str())
    .unwrap_or("download")
    .to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(dest) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None); // user cancelled
    };
    let dest_path = std::path::PathBuf::from(dest.to_string());
    if let Some((context_id, remote_path)) = remote {
        let frame_id = state.active_frame(window.label());
        let context = state
            .store
            .get_execution_context(&context_id)
            .await
            .map_err(|e| format!("{e}"))?
            .ok_or_else(|| format!("SSH execution context not found: {context_id}"))?;
        state
            .run_manager
            .download_ssh_file(
                &state.store,
                &ap.id,
                frame_id.as_deref(),
                &context,
                &remote_path,
                &dest_path,
            )
            .await?;
    } else {
        tokio::fs::copy(local.unwrap(), &dest_path)
            .await
            .map_err(|e| format!("copy failed: {e}"))?;
    }
    Ok(Some(dest_path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub(super) async fn get_capabilities(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Capabilities, String> {
    let ap = state.active(window.label());
    let project = build_project_info(&state, window.label()).await;
    let tags = load_skill_tags(&state.store).await;
    let enabled = effective_enabled_skill_names(&state.store, &ap).await;
    let skills = skill_infos(&ap.skills, &tags, enabled.as_ref());
    Ok(Capabilities {
        skills,
        mcp_servers: list_mcp_servers(&ap.root),
        memory_files: list_memory_files(&ap.memory),
        project,
    })
}

#[tauri::command]
pub(super) async fn get_onboarding_state(
    state: State<'_, AppState>,
) -> Result<OnboardingState, String> {
    let (_, _, _, api_key) = load_settings(&state.store).await;
    let done = state
        .store
        .get_setting("onboarding_done")
        .await
        .ok()
        .flatten()
        .is_some();
    Ok(OnboardingState {
        show: !done,
        has_api_key: !api_key.is_empty(),
    })
}

pub(super) fn initial_bootstrap(workspace: &std::path::Path, skills: usize) -> BootstrapStatus {
    let mut status = BootstrapStatus {
        skills_loaded: skills,
        python_ok: false,
        python_initializing: true,
        mcp_catalog: list_mcp_servers(workspace).len(),
        uv_ok: wisp_runtime::PythonEnv::find_uv().is_some(),
        node_ok: wisp_runtime::PythonEnv::find_node().is_some(),
        npm_ok: wisp_runtime::PythonEnv::find_npm().is_some(),
        sci_ok: wisp_runtime::PythonEnv::find_sci().is_some(),
        pixi_ok: wisp_runtime::PythonEnv::find_pixi().is_some(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        workspace: workspace.to_string_lossy().into_owned(),
        errors: vec![],
    };
    if status.skills_loaded == 0 {
        status
            .errors
            .push("No bundled skills found in install resources.".into());
    }
    if !status.uv_ok {
        status
            .errors
            .push("uv not found on PATH; install uv or set UV_PATH.".into());
    }
    if !status.node_ok {
        status
            .errors
            .push("Node.js not found on PATH; bear-* literature skills need Node >= 20.".into());
    } else if !status.npm_ok {
        status.errors.push(
            "npm not found on PATH; install Node.js (includes npm) for scimaster-cli.".into(),
        );
    } else if !status.sci_ok {
        status.errors.push(
            "scimaster-cli (`sci`) not found; run `npm install -g scimaster-cli` then `sci init`."
                .into(),
        );
    }
    if !status.pixi_ok {
        status.errors.push(
            "pixi not found on PATH; optional for local bioinformatics multi-env workflows.".into(),
        );
    }
    if wisp_paths::bio_tools_dir().is_none() {
        status
            .errors
            .push("Bundled bio-tools MCP catalog not found.".into());
    }
    status
}

pub(super) fn finish_python_bootstrap(status: &mut BootstrapStatus, result: Result<(), String>) {
    status.python_initializing = false;
    match result {
        Ok(()) => status.python_ok = true,
        Err(error) => status.errors.push(format!("Python environment: {error}")),
    }
}

pub(super) fn start_python_bootstrap(app: &tauri::AppHandle) {
    let handle = app.clone();
    let app_data = app.state::<AppState>().app_data.clone();
    tauri::async_runtime::spawn(async move {
        // Environment creation invokes uv and may download/install large wheels.
        // Keep all of it off Tauri's event-loop thread so the first window stays
        // responsive while the one-time bootstrap runs.
        let result = tokio::task::spawn_blocking(move || {
            wisp_runtime::PythonEnv::ensure(&app_data)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .await
        .unwrap_or_else(|error| Err(format!("bootstrap task failed: {error}")));

        let status = {
            let state = handle.state::<AppState>();
            let mut status = state.bootstrap.lock().unwrap();
            finish_python_bootstrap(&mut status, result);
            status.clone()
        };
        let _ = handle.emit("bootstrap-status", status);
    });
}

#[tauri::command]
pub(super) fn get_bootstrap_status(state: State<'_, AppState>) -> BootstrapStatus {
    state.bootstrap.lock().unwrap().clone()
}

#[tauri::command]
pub(super) fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(super) fn reveal_in_file_manager(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    path: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let ap = state.active(window.label());
    let real = wisp_tools::safety::validate_file_path(&ap.root, &path)?;
    if !real.exists() {
        return Err(format!("file not found: {path}"));
    }
    app.opener()
        .reveal_item_in_dir(&real)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(super) async fn check_for_updates() -> Result<UpdateCheck, String> {
    const LATEST_RELEASE_API: &str =
        "https://api.github.com/repos/xuzhougeng/wisp-science/releases/latest";

    let release = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|error| format!("Failed to create update client: {error}"))?
        .get(LATEST_RELEASE_API)
        .header(reqwest::header::USER_AGENT, "wisp-science-update-check")
        .send()
        .await
        .map_err(|error| format!("Failed to check GitHub Releases: {error}"))?
        .error_for_status()
        .map_err(|error| format!("GitHub Releases returned an error: {error}"))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| format!("Invalid response from GitHub Releases: {error}"))?;

    update_check_from_release(env!("CARGO_PKG_VERSION"), release)
}

#[tauri::command]
pub(super) async fn dismiss_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    state
        .store
        .set_setting("onboarding_done", "1")
        .await
        .map_err(|e| format!("{e}"))
}
