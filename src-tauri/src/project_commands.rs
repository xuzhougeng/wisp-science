//! Project Commands split out of lib.rs; shared state/helpers stay in the crate root.

use super::*;
use std::collections::HashMap;
use tauri::Manager;

pub(crate) fn same_workspace_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => {
            #[cfg(target_os = "windows")]
            {
                left.to_string_lossy()
                    .eq_ignore_ascii_case(&right.to_string_lossy())
            }
            #[cfg(not(target_os = "windows"))]
            {
                left == right
            }
        }
        _ => false,
    }
}

#[tauri::command]
pub(super) async fn get_research_graph(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<wisp_store::ResearchGraph, String> {
    let (_, scope) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    state
        .store
        .research_graph_in_scope(&scope)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(super) async fn list_projects(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectSummary>, String> {
    let running = state.running_turns.lock().await.clone();
    let awaiting = state.awaiting_confirm.lock().unwrap().clone();
    let rows = state
        .store
        .list_projects()
        .await
        .map_err(|e| format!("{e}"))?;
    let mut out = vec![];
    for (id, name, ws, _c, upd, cnt, desc, art) in rows {
        let (running_count, needs_you_count) =
            project_status_counts(&state.store, &id, &running, &awaiting).await;
        let sync_state = state.store.get_project_sync_state(&id).await.ok().flatten();
        let sync_configured = sync_state
            .as_ref()
            .is_some_and(|state| state.base_revision.is_some());
        out.push(ProjectSummary {
            id,
            name,
            description: desc,
            workspace_dir: ws,
            session_count: cnt,
            artifact_count: art,
            updated_at: upd,
            running_count,
            needs_you_count,
            sync_configured,
            last_synced_at: sync_state.and_then(|state| state.last_synced_at),
        });
    }
    Ok(out)
}

#[tauri::command]
pub(super) async fn create_project(
    state: State<'_, AppState>,
    name: String,
    workspace_dir: String,
    description: String,
    agent_context: String,
    standard_layout: bool,
) -> Result<ProjectSummary, String> {
    if name.trim().is_empty() {
        return Err("Project name is required".into());
    }
    let dir = workspace_dir.trim();
    if dir.is_empty() {
        return Err("A working directory is required".into());
    }
    let path = PathBuf::from(dir);
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Failed to create working directory: {e}"))?;
    if state
        .store
        .list_projects()
        .await
        .map_err(|e| format!("{e}"))?
        .iter()
        .any(|project| same_workspace_path(&path, Path::new(&project.2)))
    {
        return Err("This folder is already registered as a project.".into());
    }
    // Writability probe: create + remove a temp marker.
    let marker = path.join(".wisp-write-test");
    std::fs::write(&marker, b"").map_err(|e| format!("Working directory is not writable: {e}"))?;
    let _ = std::fs::remove_file(&marker);

    let id = Uuid::new_v4().to_string();
    // #405: opt-in. Unchecked means the user keeps their own structure, so we
    // create nothing — the convention lives in .wisp/WISP.md instead (below).
    if standard_layout {
        workspace_manifest::init_workspace_layout(&path, &id, name.trim())?;
    }
    state
        .store
        .create_project(&id, name.trim(), dir)
        .await
        .map_err(|e| format!("{e}"))?;
    // Description (DB) + Agent Context (.wisp/WISP.md) — same storage as update_project.
    let desc = description.trim();
    if !desc.is_empty() {
        state
            .store
            .update_project(&id, name.trim(), desc)
            .await
            .map_err(|e| format!("{e}"))?;
    }
    let ctx = agent_context.trim();
    if !ctx.is_empty() {
        let wisp_dir = path.join(".wisp");
        std::fs::create_dir_all(&wisp_dir)
            .map_err(|e| format!("Failed to write Agent Context: {e}"))?;
        std::fs::write(wisp_dir.join("WISP.md"), ctx)
            .map_err(|e| format!("Failed to write Agent Context: {e}"))?;
    }
    Ok(build_project_summary(&state, &id).await)
}

/// Cancel and drop every in-memory runtime belonging to `project_id`'s sessions
/// (e.g. the project is being deleted). Other projects' sessions keep running —
/// switching/closing a project must not stop unrelated work (#52). Call this
/// *before* the project's frames are removed from the store.
pub(super) async fn cancel_project_sessions(state: &AppState, project_id: &str) {
    let frame_ids: Vec<String> = state
        .store
        .list_sessions(project_id)
        .await
        .map(|rows| rows.into_iter().map(|(id, ..)| id).collect())
        .unwrap_or_default();
    let runtimes = {
        let sessions = state.sessions.lock().await;
        frame_ids
            .iter()
            .filter_map(|fid| sessions.get(fid).cloned().map(|rt| (fid.clone(), rt)))
            .collect::<Vec<_>>()
    };
    for (_, rt) in &runtimes {
        rt.deleted.store(true, Ordering::SeqCst);
        rt.cancel.store(true, Ordering::SeqCst);
    }
    for fid in &frame_ids {
        acp::cancel_frame(state, fid).await;
    }
    for (_, rt) in &runtimes {
        let _workflow = rt.workflow.lock().await;
        let _agent = rt.agent.lock().await;
    }
    for fid in &frame_ids {
        acp::close_frame(state, fid).await;
    }
    {
        let mut sessions = state.sessions.lock().await;
        for fid in &frame_ids {
            sessions.remove(fid);
        }
    }
    let mut running = state.running_turns.lock().await;
    for fid in &frame_ids {
        running.remove(fid);
    }
}

/// Point the backend's active project at `id`, rebuilding its skills/memory.
/// Returns the resolved `(name, workspace_dir)`. `id` must exist in the store.
///
/// Switching projects no longer tears down the previous project's sessions —
/// each session's agent already captured its own root/skills/memory at creation,
/// so cross-project turns run in parallel and stay monitorable on the dashboard
/// (#52). Deleting a project stops only *its* sessions (see `delete_project`).
/// Build a project's ActiveProject bundle (root, skills, memory) by id, plus
/// its (name, workspace) for callers that need them. Pure load — does not touch
/// the per-window active slot.
pub(super) async fn load_active_project(
    state: &AppState,
    id: &str,
) -> Result<(ActiveProject, String, String), String> {
    let (name, ws) = state
        .store
        .get_project(id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Project not found".to_string())?;
    let root = ensure_writable(PathBuf::from(&ws), &state.app_data);
    let skills = Arc::new(crate::load_skill_index(&root));
    let memory = Arc::new(MemoryManager::new(&root));
    Ok((
        ActiveProject {
            id: id.to_string(),
            root,
            skills,
            memory,
        },
        name,
        ws,
    ))
}

pub(super) async fn set_active_project(
    state: &AppState,
    label: &str,
    id: &str,
) -> Result<(String, String), String> {
    let (ap, name, ws) = load_active_project(state, id).await?;
    let root = ap.root.clone();
    state.set_active(label, ap);
    state.set_active_frame(label, None);
    remember_window_project(&state.store, label, id).await;
    // Extra windows must not steal the main window's restore mapping or the
    // startup workspace display. remember_window_project already skips home-*.
    if label == "main" {
        state.bootstrap.lock().unwrap().workspace = root.to_string_lossy().into_owned();
        let _ = state.store.set_setting("active_project_id", id).await;
    }
    Ok((name, ws))
}

const WINDOW_ACTIVE_PROJECTS_KEY: &str = "window_active_projects";

async fn load_window_active_projects(store: &Store) -> HashMap<String, String> {
    store
        .get_setting(WINDOW_ACTIVE_PROJECTS_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Persist this window's last project. File → New Window views (`home-*`) are
/// not restored on launch, so they must not overwrite another window's mapping.
/// Only the `main` window still writes the legacy `active_project_id` fallback.
async fn remember_window_project(store: &Store, label: &str, id: &str) {
    if crate::app_state::is_blank_window_label(label) {
        return;
    }
    let mut windows = load_window_active_projects(store).await;
    windows.insert(label.to_string(), id.to_string());
    let _ = store
        .set_setting(
            WINDOW_ACTIVE_PROJECTS_KEY,
            &serde_json::to_string(&windows).unwrap_or_default(),
        )
        .await;
}

/// Project the `main` window should restore. Prefers `window_active_projects`
/// so an extra window opening a different workspace cannot steal main's last
/// project; falls back to the legacy global `active_project_id`.
pub(crate) async fn startup_main_project_id(store: &Store) -> String {
    let windows = load_window_active_projects(store).await;
    if let Some(id) = windows.get("main") {
        if store.get_project(id).await.ok().flatten().is_some() {
            return id.clone();
        }
    }
    match store.get_setting("active_project_id").await.ok().flatten() {
        Some(id) if store.get_project(&id).await.ok().flatten().is_some() => id,
        _ => "default".to_string(),
    }
}

/// Brand string used when no project is open (home, or a window that has not
/// loaded a workspace yet). Taskbar, Alt-Tab, and the macOS title bar all
/// read this, so it must stay in sync with the custom Windows titlebar.
pub(super) const APP_WINDOW_TITLE: &str = "wisp science";

/// Native window title: the app name, plus the project when one is open.
pub(super) fn app_window_title(project_name: Option<&str>) -> String {
    match project_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("{APP_WINDOW_TITLE} \u{2014} {name}"),
        None => APP_WINDOW_TITLE.to_string(),
    }
}

fn apply_app_window_title(window: &tauri::WebviewWindow, project_name: Option<&str>) {
    let _ = window.set_title(&app_window_title(project_name));
}

#[tauri::command]
pub(super) async fn open_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<ProjectSummary, String> {
    let _project_activity = state.begin_project_activity(&id)?;
    let (name, ws) = set_active_project(state.inner(), window.label(), &id).await?;
    apply_app_window_title(&window, Some(&name));
    let _ = state.store.create_project(&id, &name, &ws).await; // touch updated_at → sorts to top
    Ok(build_project_summary(&state, &id).await)
}

/// Project ids that currently have their own window, persisted so the set can be
/// restored on the next launch (#52, Phase 3). Stored as a JSON array setting.
pub(super) async fn persisted_windows(store: &Store) -> Vec<String> {
    store
        .get_setting("open_project_windows")
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

pub(super) async fn update_persisted_windows(store: &Store, id: &str, present: bool) {
    let mut v = persisted_windows(store).await;
    let had = v.iter().any(|x| x == id);
    if present && !had {
        v.push(id.to_string());
    } else if !present && had {
        v.retain(|x| x != id);
    } else {
        return;
    }
    let _ = store
        .set_setting(
            "open_project_windows",
            &serde_json::to_string(&v).unwrap_or_default(),
        )
        .await;
}

pub(super) fn project_window_label(id: &str) -> String {
    format!("proj-{id}") // project ids are UUIDs or "default" — label-safe
}

/// File → New Window: a fresh GUI that lands on the Projects home screen and
/// is not bound to any workspace until the user opens one in that window.
/// Labels must stay on the `home-*` glob in `capabilities/default.json`, or
/// the custom Windows title-bar close/minimize/maximize calls are denied.
pub(super) fn next_blank_window_label() -> String {
    format!("home-{}", Uuid::new_v4())
}

pub(super) fn blank_window_url() -> &'static str {
    "index.html"
}

/// Offset a new File → New Window from its anchor so the extra GUI is obvious
/// instead of covering the caller exactly.
pub(super) fn cascaded_window_position(anchor_pos: (i32, i32), offset: i32) -> (i32, i32) {
    (anchor_pos.0 + offset, anchor_pos.1 + offset)
}

/// URL for a dedicated project window. Ids are UUIDs or "default" — no
/// percent-encoding needed (matches `url_project_param` in the frontend).
pub(super) fn project_window_url(id: &str, session: Option<&str>) -> String {
    match session {
        Some(sid) => format!("index.html?project={id}&session={sid}"),
        None => format!("index.html?project={id}"),
    }
}

/// Top-left position (physical px) that centers a window of `window_size`
/// over an anchor window at `anchor_pos`/`anchor_size`. Pure so placement
/// stays testable; the caller converts to logical coordinates.
pub(super) fn centered_window_position(
    anchor_pos: (i32, i32),
    anchor_size: (u32, u32),
    window_size: (u32, u32),
) -> (i32, i32) {
    (
        anchor_pos.0 + (anchor_size.0 as i32 - window_size.0 as i32) / 2,
        anchor_pos.1 + (anchor_size.1 as i32 - window_size.1 as i32) / 2,
    )
}

/// Open a project in its own window (or focus the existing one), wiring up
/// cleanup on close. Shared by the `open_project_window` command and the
/// startup restore (#52). With `session`, the window opens straight into that
/// session — an existing window is told via the `open-session` event (#423).
/// `anchor_label` is the window the new one centers over; `None` (startup
/// restore) falls back to the main window.
pub(super) async fn spawn_project_window(
    app: &AppHandle,
    state: &AppState,
    id: &str,
    session: Option<&str>,
    anchor_label: Option<&str>,
) -> Result<String, String> {
    let label = project_window_label(id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
        if let Some(sid) = session {
            let _ = app.emit_to(
                label.as_str(),
                "open-session",
                serde_json::json!({ "projectId": id, "sessionId": sid }),
            );
        }
        return Ok(label);
    }
    // Pre-set this window's active project so its first commands resolve correctly
    // even before the window's frontend calls open_project.
    let (name, _) = set_active_project(state, &label, id).await?;
    let url = tauri::WebviewUrl::App(project_window_url(id, session).into());
    let mut builder = tauri::WebviewWindowBuilder::new(app, &label, url)
        .title(app_window_title(Some(&name)))
        .inner_size(1100.0, 760.0)
        .resizable(true)
        .on_navigation(crate::guard_webview_navigation);
    // Center over the requesting window (or the main window on startup
    // restore); otherwise the OS cascades each new window to an arbitrary
    // spot. Sizes/positions are physical, so convert through the anchor's
    // scale factor for the builder's logical `position`.
    let anchor = anchor_label
        .and_then(|label| app.get_webview_window(label))
        .or_else(|| app.get_webview_window("main"));
    if let Some(anchor) = anchor {
        if let (Ok(pos), Ok(size)) = (anchor.outer_position(), anchor.outer_size()) {
            let scale = anchor.scale_factor().unwrap_or(1.0);
            let physical = ((1100.0 * scale) as u32, (760.0 * scale) as u32);
            let (x, y) =
                centered_window_position((pos.x, pos.y), (size.width, size.height), physical);
            builder = builder.position(x as f64 / scale, y as f64 / scale);
        }
    }
    #[cfg(target_os = "windows")]
    let builder = builder.decorations(false).shadow(true);
    let win = builder.build().map_err(|e| e.to_string())?;
    crate::windows_snap::install_for_window(&win);
    #[cfg(target_os = "macos")]
    wire_macos_menu_events(&win);
    let evt_app = app.clone();
    let evt_label = label.clone();
    let evt_id = id.to_string();
    win.on_window_event(move |ev| {
        if matches!(ev, tauri::WindowEvent::Destroyed) {
            // Drop this window's per-window project context and stop persisting
            // it for restore. Its running sessions are tracked globally and keep
            // going until they finish or are stopped.
            let st = evt_app.state::<AppState>();
            st.active.write().unwrap().remove(&evt_label);
            st.active_frame.write().unwrap().remove(&evt_label);
            let store = st.store.clone();
            let id = evt_id.clone();
            tauri::async_runtime::spawn(async move {
                update_persisted_windows(&store, &id, false).await;
            });
        }
    });
    update_persisted_windows(&state.store, id, true).await;
    Ok(label)
}

/// Open a project in its own window (or focus the existing one). Each window
/// carries its own active project, keyed by window label (#52).
#[tauri::command]
pub(super) async fn open_project_window(
    app: AppHandle,
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    session: Option<String>,
) -> Result<String, String> {
    spawn_project_window(
        &app,
        state.inner(),
        &id,
        session.as_deref(),
        Some(window.label()),
    )
    .await
}

/// Open a blank GUI on the Projects home screen. The window has its own label
/// and no `ActiveProject` until the user opens a workspace in it, so it cannot
/// read or mutate another window's files, sessions, or runtimes.
pub(super) async fn spawn_blank_window(
    app: &AppHandle,
    anchor_label: Option<&str>,
) -> Result<String, String> {
    let label = next_blank_window_label();
    let url = tauri::WebviewUrl::App(blank_window_url().into());
    let mut builder = tauri::WebviewWindowBuilder::new(app, &label, url)
        .title(app_window_title(None))
        .inner_size(1100.0, 760.0)
        .resizable(true)
        .on_navigation(crate::guard_webview_navigation);
    let anchor = anchor_label
        .and_then(|label| app.get_webview_window(label))
        .or_else(|| app.get_webview_window("main"));
    if let Some(anchor) = anchor {
        if let Ok(pos) = anchor.outer_position() {
            let scale = anchor.scale_factor().unwrap_or(1.0);
            let offset = (36.0 * scale) as i32;
            let (x, y) = cascaded_window_position((pos.x, pos.y), offset);
            builder = builder.position(x as f64 / scale, y as f64 / scale);
        }
    }
    #[cfg(target_os = "windows")]
    let builder = builder.decorations(false).shadow(true);
    let win = builder.build().map_err(|e| e.to_string())?;
    crate::windows_snap::install_for_window(&win);
    #[cfg(target_os = "macos")]
    wire_macos_menu_events(&win);
    let evt_app = app.clone();
    let evt_label = label.clone();
    win.on_window_event(move |ev| {
        if matches!(ev, tauri::WindowEvent::Destroyed) {
            let st = evt_app.state::<AppState>();
            st.active.write().unwrap().remove(&evt_label);
            st.active_frame.write().unwrap().remove(&evt_label);
        }
    });
    Ok(label)
}

#[tauri::command]
pub(super) async fn open_new_window(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<String, String> {
    spawn_blank_window(&app, Some(window.label())).await
}

#[tauri::command]
pub(super) async fn delete_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    delete_data: Option<bool>,
) -> Result<(), String> {
    exploration_commands::reject_private_exploration_project_mutation(
        &state.store,
        &id,
        "Project deletion",
    )
    .await?;
    let _project_activity = state.begin_project_activity(&id)?;
    let workspace_delete_target = if delete_data.unwrap_or(false) {
        let (_, workspace_dir) = state
            .store
            .get_project(&id)
            .await
            .map_err(|e| format!("{e}"))?
            .ok_or_else(|| "Project not found".to_string())?;
        project_workspace_delete_target(&workspace_dir)?
    } else {
        None
    };
    // The delete ✕ is only reachable from the projects list, so a project may
    // legitimately be deleted while it's still the backend's *active* one
    // (returning to the list is a frontend-only nav — it never told the backend
    // to leave). Delete it, then fall back to the always-present "default"
    // workspace so `active` never dangles at a deleted project. An unbound
    // File → New Window (`home-*`) must not treat `main` as active or panic.
    let was_active = state
        .require_active(window.label())
        .map(|ap| ap.id == id)
        .unwrap_or(false);
    // Stop the deleted project's own running sessions (gather frame ids before
    // the store cascade removes them); other projects keep running (#52).
    cancel_project_sessions(state.inner(), &id).await;
    state.runtime_manager.stop_project(&id).await;
    if let Err(error) = state.run_manager.wind_down_project(&state.store, &id).await {
        tracing::warn!(project_id = %id, "project wind-down failed: {error}");
    }
    if let Some(target) = workspace_delete_target {
        delete_project_workspace_data(target).await?;
    }
    state
        .store
        .delete_project(&id)
        .await
        .map_err(|e| format!("{e}"))?;
    project_sync::forget_project_key(&id).await;
    if was_active {
        let _ = set_active_project(state.inner(), window.label(), "default").await;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ProjectWorkspaceDeleteTarget {
    Directory(PathBuf),
    Symlink(PathBuf),
}

/// Resolve the registered project root before stopping any sessions. Missing
/// workspaces are already effectively data-free, but broad filesystem roots
/// are never valid deletion targets even after an explicit UI confirmation.
pub(super) fn project_workspace_delete_target(
    workspace_dir: &str,
) -> Result<Option<ProjectWorkspaceDeleteTarget>, String> {
    if workspace_dir.trim().is_empty() {
        return Err("Project workspace is empty; refusing to delete data.".into());
    }
    let path = PathBuf::from(workspace_dir);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Failed to inspect project workspace {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(Some(ProjectWorkspaceDeleteTarget::Symlink(path)));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "Project workspace is not a directory: {}",
            path.display()
        ));
    }
    let canonical = dunce::canonicalize(&path).map_err(|error| {
        format!(
            "Failed to inspect project workspace {}: {error}",
            path.display()
        )
    })?;
    if canonical.parent().is_none() {
        return Err("Refusing to delete a filesystem root.".into());
    }
    Ok(Some(ProjectWorkspaceDeleteTarget::Directory(canonical)))
}

pub(super) async fn delete_project_workspace_data(
    target: ProjectWorkspaceDeleteTarget,
) -> Result<(), String> {
    let display_path = match &target {
        ProjectWorkspaceDeleteTarget::Directory(path)
        | ProjectWorkspaceDeleteTarget::Symlink(path) => path.clone(),
    };
    let result = match target {
        ProjectWorkspaceDeleteTarget::Directory(path) => tokio::fs::remove_dir_all(path).await,
        ProjectWorkspaceDeleteTarget::Symlink(path) => {
            #[cfg(target_os = "windows")]
            {
                tokio::fs::remove_dir(path).await
            }
            #[cfg(not(target_os = "windows"))]
            {
                tokio::fs::remove_file(path).await
            }
        }
    };
    result.map_err(|error| {
        format!(
            "Failed to delete project data at {}: {error}",
            display_path.display()
        )
    })
}

#[derive(Serialize, Clone)]
pub(super) struct ProjectSettings {
    id: String,
    name: String,
    description: String,
    agent_context: String,
}

fn project_agent_context_path(root: &Path) -> PathBuf {
    root.join(".wisp").join("WISP.md")
}

fn read_project_agent_context(root: &Path) -> String {
    std::fs::read_to_string(project_agent_context_path(root)).unwrap_or_default()
}

fn write_project_agent_context(root: &Path, agent_context: &str) -> Result<(), String> {
    let wisp_md = project_agent_context_path(root);
    let ctx = agent_context.trim();
    if ctx.is_empty() {
        let _ = std::fs::remove_file(&wisp_md);
        return Ok(());
    }
    if let Some(parent) = wisp_md.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to write Agent Context: {e}"))?;
    }
    std::fs::write(&wisp_md, ctx).map_err(|e| format!("Failed to write Agent Context: {e}"))
}

/// Resolve the project whose settings should be read or written. An explicit
/// `id` (home-card configure) must not switch this window's active project.
async fn settings_project(
    state: &AppState,
    window_label: &str,
    requested_id: Option<&str>,
) -> Result<(String, PathBuf, String, String), String> {
    let id = match requested_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => id.to_string(),
        None => state.require_active(window_label)?.id,
    };
    let (name, description, workspace) = state
        .store
        .get_project_meta(&id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Project not found".to_string())?;
    let root = ensure_writable(PathBuf::from(workspace), &state.app_data);
    Ok((id, root, name, description))
}

/// Read a project's editable settings for the Project Settings modal.
/// `id` targets a specific project (home-card configure). Omit it to use the
/// window's active project. Agent Context is `.wisp/WISP.md`.
#[tauri::command]
pub(super) async fn get_project_settings(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: Option<String>,
) -> Result<ProjectSettings, String> {
    let (project_id, root, name, description) =
        settings_project(state.inner(), window.label(), id.as_deref()).await?;
    let _project_activity = state.begin_project_activity(&project_id)?;
    Ok(ProjectSettings {
        id: project_id,
        name,
        description,
        agent_context: read_project_agent_context(&root),
    })
}

#[derive(Serialize, Clone)]
pub(super) struct ProjectRunRetention {
    run_retention_days: Option<i64>,
    failed_run_retention_days: Option<i64>,
    orphan_file_retention_days: Option<i64>,
}

/// Opt-in retention windows for automatic run-workspace cleanup and orphaned
/// remote-file reclamation on servers.
#[tauri::command]
pub(super) async fn get_project_run_retention(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<ProjectRunRetention, String> {
    let ap = state.require_active(window.label())?;
    let (run_retention_days, failed_run_retention_days, orphan_file_retention_days) = state
        .store
        .project_run_retention(&ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(ProjectRunRetention {
        run_retention_days,
        failed_run_retention_days,
        orphan_file_retention_days,
    })
}

#[tauri::command]
pub(super) async fn set_project_run_retention(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    run_retention_days: Option<i64>,
    failed_run_retention_days: Option<i64>,
    orphan_file_retention_days: Option<i64>,
) -> Result<ProjectRunRetention, String> {
    let ap = state.require_active(window.label())?;
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .store
        .set_project_run_retention(
            &ap.id,
            run_retention_days,
            failed_run_retention_days,
            orphan_file_retention_days,
        )
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(ProjectRunRetention {
        run_retention_days,
        failed_run_retention_days,
        orphan_file_retention_days,
    })
}

/// Save a project's name/description (DB) and Agent Context (.wisp/WISP.md).
/// `id` targets a specific project (home-card configure). Omit it to use the
/// window's active project — this does not switch the active project.
/// An empty Agent Context removes WISP.md so the prompt falls back to "no rules".
/// Takes effect on the next seeded session; already-running agents keep their prompt.
#[tauri::command]
pub(super) async fn update_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: Option<String>,
    name: String,
    description: String,
    agent_context: String,
) -> Result<ProjectSummary, String> {
    if name.trim().is_empty() {
        return Err("Project name is required".into());
    }
    let (project_id, root, _, _) =
        settings_project(state.inner(), window.label(), id.as_deref()).await?;
    let _project_activity = state.begin_project_activity(&project_id)?;
    exploration_commands::reject_private_exploration_project_mutation(
        &state.store,
        &project_id,
        "Project settings changes",
    )
    .await?;
    let name = name.trim();
    state
        .store
        .update_project(&project_id, name, description.trim())
        .await
        .map_err(|e| format!("{e}"))?;
    write_project_agent_context(&root, &agent_context)?;
    // Home-card configure (`id` is Some) may run while this window is still
    // on the projects landing — do not stamp that window with the renamed
    // project. In-project settings omit `id` and should update this window.
    // The dedicated `proj-{id}` window, if open, always shows that project.
    if id.is_none() {
        apply_app_window_title(&window, Some(name));
    }
    if let Some(proj_win) = window
        .app_handle()
        .get_webview_window(&project_window_label(&project_id))
    {
        apply_app_window_title(&proj_win, Some(name));
    }
    Ok(build_project_summary(&state, &project_id).await)
}

#[tauri::command]
pub(super) async fn get_project_info(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<ProjectInfo, String> {
    let _ = state.require_active(window.label())?;
    Ok(build_project_info(&state, window.label()).await)
}

#[cfg(test)]
mod tests {
    use super::{
        app_window_title, blank_window_url, cascaded_window_position, load_window_active_projects,
        next_blank_window_label, read_project_agent_context, remember_window_project,
        same_workspace_path, startup_main_project_id, write_project_agent_context,
        APP_WINDOW_TITLE,
    };

    #[test]
    fn app_window_title_uses_the_project_name() {
        assert_eq!(app_window_title(None), APP_WINDOW_TITLE);
        assert_eq!(app_window_title(Some("")), APP_WINDOW_TITLE);
        assert_eq!(app_window_title(Some("   ")), APP_WINDOW_TITLE);
        assert_eq!(
            app_window_title(Some("fkbp1a-aortic-ring-assay")),
            "wisp science \u{2014} fkbp1a-aortic-ring-assay"
        );
        assert_eq!(
            app_window_title(Some("  fkbp1a  ")),
            "wisp science \u{2014} fkbp1a"
        );
    }

    #[test]
    fn blank_window_url_does_not_bind_a_project() {
        assert_eq!(blank_window_url(), "index.html");
        assert!(!blank_window_url().contains("project="));
        let label = next_blank_window_label();
        assert!(crate::app_state::is_blank_window_label(&label));
        assert_ne!(label, next_blank_window_label());
    }

    #[test]
    fn cascaded_window_position_offsets_from_the_anchor() {
        assert_eq!(cascaded_window_position((100, 50), 36), (136, 86));
        assert_eq!(cascaded_window_position((-1600, -50), 36), (-1564, -14));
    }

    #[test]
    fn workspace_path_match_resolves_equivalent_existing_paths() {
        let root =
            std::env::temp_dir().join(format!("wisp_same_workspace_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        assert!(same_workspace_path(&root, &root.join(".")));
        assert!(!same_workspace_path(&root, &root.join("other")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn agent_context_writes_wisp_md_and_empty_clears_it() {
        let root = std::env::temp_dir().join(format!(
            "wisp_project_settings_ctx_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();

        write_project_agent_context(&root, "  Prefer the project UI setting.  ").unwrap();
        assert_eq!(
            read_project_agent_context(&root),
            "Prefer the project UI setting."
        );
        write_project_agent_context(&root, "   ").unwrap();
        assert!(read_project_agent_context(&root).is_empty());
        assert!(!root.join(".wisp").join("WISP.md").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn extra_windows_do_not_overwrite_main_last_project() {
        let path = std::env::temp_dir().join(format!(
            "wisp_window_projects_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&path).await.unwrap();
        store
            .create_project("main-project", "Main", "/ws/main")
            .await
            .unwrap();
        store
            .create_project("other-project", "Other", "/ws/other")
            .await
            .unwrap();
        let _ = store
            .set_setting("active_project_id", "main-project")
            .await
            .unwrap();

        remember_window_project(&store, "main", "main-project").await;
        remember_window_project(&store, "home-abc", "other-project").await;
        remember_window_project(&store, "proj-other-project", "other-project").await;

        let windows = load_window_active_projects(&store).await;
        assert_eq!(
            windows.get("main").map(String::as_str),
            Some("main-project")
        );
        assert_eq!(
            windows.get("proj-other-project").map(String::as_str),
            Some("other-project")
        );
        assert!(
            !windows.contains_key("home-abc"),
            "blank File → New Window labels must not persist: {windows:?}"
        );
        assert_eq!(startup_main_project_id(&store).await, "main-project");

        let _ = store
            .set_setting("active_project_id", "other-project")
            .await
            .unwrap();
        assert_eq!(
            startup_main_project_id(&store).await,
            "main-project",
            "window_active_projects[main] must beat the legacy global id"
        );

        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn startup_main_project_id_falls_back_to_legacy_then_default() {
        let path = std::env::temp_dir().join(format!(
            "wisp_startup_main_project_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = wisp_store::Store::open(&path).await.unwrap();
        store
            .create_project("legacy-project", "Legacy", "/ws/legacy")
            .await
            .unwrap();
        store
            .create_project("gone-project", "Gone", "/ws/gone")
            .await
            .unwrap();

        assert_eq!(startup_main_project_id(&store).await, "default");

        let _ = store
            .set_setting("active_project_id", "legacy-project")
            .await
            .unwrap();
        assert_eq!(startup_main_project_id(&store).await, "legacy-project");

        remember_window_project(&store, "main", "gone-project").await;
        store.delete_project("gone-project").await.unwrap();
        assert_eq!(
            startup_main_project_id(&store).await,
            "legacy-project",
            "stale window_active_projects[main] must fall back to the legacy id"
        );

        let _ = store
            .set_setting("active_project_id", "missing")
            .await
            .unwrap();
        assert_eq!(startup_main_project_id(&store).await, "default");

        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
