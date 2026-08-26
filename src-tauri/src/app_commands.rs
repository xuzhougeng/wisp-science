//! App Commands split out of lib.rs; shared state/helpers stay in the crate root.

use super::*;

#[cfg(target_os = "windows")]
fn show_clickable_notification(
    app: AppHandle,
    window_label: String,
    title: String,
    body: String,
    target: Option<serde_json::Value>,
    focus_fallback_armed: bool,
) -> Result<(), String> {
    use tauri_winrt_notification::Toast;

    // Unpackaged development binaries do not have an AppUserModelID. Keep the
    // same PowerShell fallback used by the notification plugin in that case.
    let app_id = if cfg!(debug_assertions) {
        Toast::POWERSHELL_APP_ID.to_string()
    } else {
        app.config().identifier.clone()
    };
    let callback_app = app.clone();
    let callback_label = window_label.clone();
    Toast::new(&app_id)
        .title(&title)
        .text1(&body)
        .on_activated(move |_| {
            // The native callback identifies the notification exactly, so it
            // takes precedence over the latest-session focus fallback.
            if !focus_fallback_armed || claim_notify_activation(&callback_label, target.is_some()) {
                desktop_lifecycle::activate_workspace_window(
                    &callback_app,
                    &callback_label,
                    target.clone(),
                );
            }
            Ok(())
        })
        .show()
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn show_clickable_notification(
    app: AppHandle,
    window_label: String,
    title: String,
    body: String,
    target: Option<serde_json::Value>,
    focus_fallback_armed: bool,
) -> Result<(), String> {
    let application = if tauri::is_dev() {
        "com.apple.Terminal".to_string()
    } else {
        app.config().identifier.clone()
    };
    // This is process-global and intentionally first-wins. A prior notification
    // may already have configured the same application identifier.
    let _ = mac_notification_sys::set_application(&application);

    tauri::async_runtime::spawn_blocking(move || {
        let mut notification = mac_notification_sys::Notification::new();
        notification
            .title(&title)
            .message(&body)
            .wait_for_click(true);
        match notification.send() {
            Ok(mac_notification_sys::NotificationResponse::Click) => {
                if !focus_fallback_armed || claim_notify_activation(&window_label, target.is_some())
                {
                    desktop_lifecycle::activate_workspace_window(&app, &window_label, target);
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(target: "wisp", %error, "desktop notification failed");
            }
        }
    });
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn show_clickable_notification(
    app: AppHandle,
    _window_label: String,
    title: String,
    body: String,
    _target: Option<serde_json::Value>,
    _focus_fallback_armed: bool,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| error.to_string())
}

/// Desktop notification for task status (#327). No-op while any app window is
/// focused (the in-app UI already shows the state) or when disabled in settings.
/// Clicking the notification navigates to the session it was about (#434) —
/// native callbacks also restore a hidden/minimized window (#499).
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
    // The done/attention agent events are broadcast to every window, so every
    // window calls this command for every session. Select one exact owner,
    // rather than only checking the project: two windows may legitimately show
    // different conversations from the same project.
    let project_id = state
        .store
        .frame_project_id(&session_id)
        .await
        .ok()
        .flatten();
    let Some(selection) = state.preferred_notification_window(&session_id, project_id.as_deref())
    else {
        return Ok(());
    };
    if selection.label != window.label() {
        return Ok(());
    }
    // Keep the taskbar/Dock focus fallback only when this window still belongs
    // to the session's project. A foreign-project fallback may show the native
    // notification, but ordinary focus must not replace its current project;
    // an explicit notification click can still navigate there.
    let target = project_id
        .map(|project_id| serde_json::json!({ "projectId": project_id, "sessionId": session_id }));
    let focus_fallback_armed = selection.arm_focus_navigation && target.is_some();
    if focus_fallback_armed {
        let target = target.as_ref().expect("target checked above");
        pending_notify_targets()
            .lock()
            .unwrap()
            .insert(window.label().to_string(), target.clone());
    }
    show_clickable_notification(
        window.app_handle().clone(),
        window.label().to_string(),
        title,
        body,
        target,
        focus_fallback_armed,
    )
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

/// Pick a local interpreter executable (python / Rscript) via the native open
/// dialog. Returns the picked path, or `None` if the user cancelled.
#[tauri::command]
pub(super) async fn pick_executable_file(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_file(move |p| {
        let _ = tx.send(p);
    });
    let picked = rx.await.map_err(|e| format!("{e}"))?;
    Ok(picked.map(|fp| fp.to_string()))
}

/// Upload local files or directories into the remote directory shown in Files.
/// When `source_paths` is omitted, opens the native multi-file picker.
#[tauri::command]
pub(super) async fn upload_to_context(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
    destination_dir: String,
    source_paths: Option<Vec<String>>,
) -> Result<Vec<crate::run_context::UploadToContextItem>, String> {
    use tauri_plugin_dialog::DialogExt;
    let paths = match source_paths {
        Some(paths) if !paths.is_empty() => paths,
        Some(_) => return Ok(Vec::new()),
        None => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            app.dialog().file().pick_files(move |picked| {
                let _ = tx.send(picked);
            });
            match rx.await.map_err(|e| format!("{e}"))? {
                Some(files) if !files.is_empty() => {
                    files.into_iter().map(|path| path.to_string()).collect()
                }
                _ => return Ok(Vec::new()),
            }
        }
    };
    let ap = state.active(window.label());
    let frame_id = state.active_frame(window.label());
    crate::run_context::submit_local_uploads_to_context(
        &state.store,
        &state.run_manager,
        &ap.id,
        frame_id.as_deref(),
        &context_id,
        &destination_dir,
        &paths,
    )
    .await
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
        crate::run_context::remote_files::refuse_if_context_path_discarded(
            &state.store,
            &context_id,
            &remote_path,
        )
        .await?;
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

/// Keep only a safe file-name component for the share-image save dialog.
pub(super) fn share_image_file_name(default_name: &str) -> String {
    std::path::Path::new(default_name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != ".." && !name.contains('\0'))
        .map(|name| name.to_string())
        .unwrap_or_else(|| "wisp-share.png".to_string())
}

// Generous ceiling for one long conversation image (base64 of ~48 MB PNG).
const MAX_SHARE_PNG_BASE64_BYTES: usize = 64 * 1024 * 1024;

/// Save a frontend-rendered `/share` PNG through the native save dialog.
/// Returns the saved path, or `None` when the user cancels.
#[tauri::command]
pub(super) async fn save_share_image(
    app: AppHandle,
    png_base64: String,
    default_name: String,
) -> Result<Option<String>, String> {
    use base64::Engine;
    use tauri_plugin_dialog::DialogExt;
    if png_base64.len() > MAX_SHARE_PNG_BASE64_BYTES {
        return Err("share image exceeds the size limit".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim())
        .map_err(|e| format!("invalid base64: {e}"))?;
    if bytes.is_empty() {
        return Err("share image is empty".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&share_image_file_name(&default_name))
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(dest) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None); // user cancelled
    };
    let dest_path = std::path::PathBuf::from(dest.to_string());
    tokio::fs::write(&dest_path, bytes)
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    Ok(Some(dest_path.to_string_lossy().into_owned()))
}

// Generous ceiling for one self-contained share HTML document.
const MAX_SHARE_HTML_BYTES: usize = 16 * 1024 * 1024;

/// Save a frontend-built `/share` HTML document through the native save
/// dialog. Returns the saved path, or `None` when the user cancels.
#[tauri::command]
pub(super) async fn save_share_html(
    app: AppHandle,
    html: String,
    default_name: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    if html.len() > MAX_SHARE_HTML_BYTES {
        return Err("share HTML exceeds the size limit".into());
    }
    if html.trim().is_empty() {
        return Err("share HTML is empty".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&share_image_file_name(&default_name))
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(dest) = rx.await.map_err(|e| format!("{e}"))? else {
        return Ok(None); // user cancelled
    };
    let dest_path = std::path::PathBuf::from(dest.to_string());
    tokio::fs::write(&dest_path, html)
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    Ok(Some(dest_path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub(super) async fn get_capabilities(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Capabilities, String> {
    let ap = state.active(window.label());
    let tags = load_skill_tags(&state.store).await;
    let (catalog, enabled) = project_skill_catalog(&state.store, &ap).await;
    let skills = skill_infos(&catalog, &tags, enabled.as_ref());
    let skill_counts = capability_skill_counts(&skills);
    let mcp_counts = capability_mcp_counts(&state.store, &ap).await;
    let mut project = build_project_info(&state, window.label()).await;
    project.skill_count = skill_counts.total();
    project.mcp_server_count = mcp_counts.total();
    Ok(Capabilities {
        skills,
        mcp_servers: list_mcp_servers(&ap.root),
        memory_files: list_memory_files(&ap.memory),
        project,
        skill_counts,
        mcp_counts,
    })
}

pub(super) fn capability_skill_counts(skills: &[SkillInfo]) -> CapabilitySourceCounts {
    let mut counts = CapabilitySourceCounts::default();
    for skill in skills.iter().filter(|skill| skill.enabled) {
        if skill.scope == SkillSource::Bundled.as_str() {
            counts.bundled += 1;
        } else {
            counts.project += 1;
        }
    }
    counts
}

async fn capability_mcp_counts(store: &Store, project: &ActiveProject) -> CapabilitySourceCounts {
    let custom = load_mcp_connections(store)
        .await
        .into_iter()
        .filter(|connection| connection.enabled)
        .count();
    let (plugin_launches, plugin_errors) =
        plugins::enabled_plugin_mcp_launches(store, &project.id).await;
    CapabilitySourceCounts {
        bundled: list_mcp_servers(&project.root).len(),
        // Invalid plugin launch configurations are still configured project
        // MCP services and remain visible as unavailable in Settings.
        project: custom + plugin_launches.len() + plugin_errors.len(),
    }
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
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        workspace: workspace.to_string_lossy().into_owned(),
        // Filled in on read: the launch is not over yet at this point.
        startup: String::new(),
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
        let _ = handle.emit("bootstrap-status", with_startup_report(status));
    });
}

/// Stamp the current launch timings onto a status snapshot. They are collected
/// while the app boots, so they are attached when the status is read rather
/// than when it is built.
fn with_startup_report(mut status: BootstrapStatus) -> BootstrapStatus {
    status.startup = crate::startup_report_summary();
    status
}

#[tauri::command]
pub(super) fn get_bootstrap_status(state: State<'_, AppState>) -> BootstrapStatus {
    with_startup_report(state.bootstrap.lock().unwrap().clone())
}

#[tauri::command]
pub(super) fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Open the first available browser on its extension manager page for the
/// offline banner's setup button. Never fails: `opened` reports whether a
/// browser was actually launched, so the UI can fall back to manual steps.
#[tauri::command]
pub(super) fn open_browser_extension_page(
    state: State<'_, AppState>,
) -> browser_bridge::BrowserExtensionSetup {
    state.browser_bridge.open_extension_setup()
}

/// Live extension connection check for the offline banner's display gate.
#[tauri::command]
pub(super) async fn extension_connected(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.browser_bridge.extension_connected().await)
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
pub(super) async fn dismiss_onboarding(state: State<'_, AppState>) -> Result<(), String> {
    state
        .store
        .set_setting("onboarding_done", "1")
        .await
        .map_err(|e| format!("{e}"))
}

#[cfg(test)]
mod share_image_tests {
    use super::share_image_file_name;

    #[test]
    fn keeps_only_a_safe_file_name_component() {
        assert_eq!(
            share_image_file_name("wisp-share-2026-08-14.png"),
            "wisp-share-2026-08-14.png"
        );
        assert_eq!(share_image_file_name("../../etc/passwd"), "passwd");
        assert_eq!(share_image_file_name("/tmp/evil/name.png"), "name.png");
        assert_eq!(share_image_file_name(""), "wisp-share.png");
        assert_eq!(share_image_file_name(".."), "wisp-share.png");
    }
}
