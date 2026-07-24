//! Project Commands split out of lib.rs; shared state/helpers stay in the crate root.

use super::*;

#[tauri::command]
pub(super) async fn get_research_graph(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<wisp_store::ResearchGraph, String> {
    let ap = state.active(window.label());
    state
        .store
        .research_graph(&ap.id)
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
    for (id, name, ws, _c, upd, cnt, desc) in rows {
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
    let skills = Arc::new(SkillIndex::load(&skill_paths(&root)));
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
    {
        state.bootstrap.lock().unwrap().workspace = root.to_string_lossy().into_owned();
    }
    let _ = state.store.set_setting("active_project_id", id).await;
    Ok((name, ws))
}

#[tauri::command]
pub(super) async fn open_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<ProjectSummary, String> {
    let _project_activity = state.begin_project_activity(&id)?;
    let (name, ws) = set_active_project(state.inner(), window.label(), &id).await?;
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

/// URL for a dedicated project window. Ids are UUIDs or "default" — no
/// percent-encoding needed (matches `url_project_param` in the frontend).
pub(super) fn project_window_url(id: &str, session: Option<&str>) -> String {
    match session {
        Some(sid) => format!("index.html?project={id}&session={sid}"),
        None => format!("index.html?project={id}"),
    }
}

/// Open a project in its own window (or focus the existing one), wiring up
/// cleanup on close. Shared by the `open_project_window` command and the
/// startup restore (#52). With `session`, the window opens straight into that
/// session — an existing window is told via the `open-session` event (#423).
pub(super) async fn spawn_project_window(
    app: &AppHandle,
    state: &AppState,
    id: &str,
    session: Option<&str>,
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
    set_active_project(state, &label, id).await?;
    let url = tauri::WebviewUrl::App(project_window_url(id, session).into());
    let builder = tauri::WebviewWindowBuilder::new(app, &label, url)
        .title("wisp-science")
        .inner_size(1100.0, 760.0)
        .resizable(true);
    #[cfg(target_os = "windows")]
    let builder = builder.decorations(false).shadow(true);
    let win = builder.build().map_err(|e| e.to_string())?;
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
    id: String,
    session: Option<String>,
) -> Result<String, String> {
    spawn_project_window(&app, state.inner(), &id, session.as_deref()).await
}

#[tauri::command]
pub(super) async fn delete_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    let _project_activity = state.begin_project_activity(&id)?;
    // The delete ✕ is only reachable from the projects list, so a project may
    // legitimately be deleted while it's still the backend's *active* one
    // (returning to the list is a frontend-only nav — it never told the backend
    // to leave). Delete it, then fall back to the always-present "default"
    // workspace so `active` never dangles at a deleted project.
    let was_active = state.active(window.label()).id == id;
    // Stop the deleted project's own running sessions (gather frame ids before
    // the store cascade removes them); other projects keep running (#52).
    cancel_project_sessions(state.inner(), &id).await;
    state.runtime_manager.stop_project(&id).await;
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

#[derive(Serialize, Clone)]
pub(super) struct ProjectSettings {
    id: String,
    name: String,
    description: String,
    agent_context: String,
}

/// Read the active project's editable settings for the Project Settings modal.
/// Agent Context is `.wisp/WISP.md`, injected into every seeded system prompt.
#[tauri::command]
pub(super) async fn get_project_settings(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<ProjectSettings, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let (name, description, _ws) = state
        .store
        .get_project_meta(&ap.id)
        .await
        .map_err(|e| format!("{e}"))?
        .unwrap_or_default();
    let agent_context =
        std::fs::read_to_string(ap.root.join(".wisp").join("WISP.md")).unwrap_or_default();
    Ok(ProjectSettings {
        id: ap.id.clone(),
        name,
        description,
        agent_context,
    })
}

/// Save the active project's name/description (DB) and Agent Context (.wisp/WISP.md).
/// An empty Agent Context removes WISP.md so the prompt falls back to "no rules".
/// Takes effect on the next seeded session; already-running agents keep their prompt.
#[tauri::command]
pub(super) async fn update_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
    description: String,
    agent_context: String,
) -> Result<ProjectSummary, String> {
    if name.trim().is_empty() {
        return Err("Project name is required".into());
    }
    let ap = state.active(window.label());
    state
        .store
        .update_project(&ap.id, name.trim(), description.trim())
        .await
        .map_err(|e| format!("{e}"))?;
    let wisp_dir = ap.root.join(".wisp");
    let wisp_md = wisp_dir.join("WISP.md");
    let ctx = agent_context.trim();
    if ctx.is_empty() {
        let _ = std::fs::remove_file(&wisp_md);
    } else {
        std::fs::create_dir_all(&wisp_dir)
            .map_err(|e| format!("Failed to write Agent Context: {e}"))?;
        std::fs::write(&wisp_md, ctx).map_err(|e| format!("Failed to write Agent Context: {e}"))?;
    }
    Ok(build_project_summary(&state, &ap.id).await)
}

#[tauri::command]
pub(super) async fn get_project_info(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<ProjectInfo, String> {
    Ok(build_project_info(&state, window.label()).await)
}
