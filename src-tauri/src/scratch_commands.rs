//! Ephemeral scratch chat: temp sandbox workspace, no project binding, destroyed on close.

use super::*;
use crate::project_commands::{cancel_project_sessions, load_active_project, set_active_project};
use std::path::{Path, PathBuf};
use wisp_store::SCRATCH_PROJECT_PREFIX;

pub(super) fn is_scratch_project_id(id: &str) -> bool {
    wisp_store::is_scratch_project_id(id)
}

#[derive(Clone)]
pub(super) struct ScratchWindow {
    prev_project_id: Option<String>,
    scratch_project_id: String,
    session_id: String,
    sandbox_path: PathBuf,
}

/// Previous workspace to restore when scratch chat closes. File → New Window
/// (`home-*`) has no binding, so scratch can start there and return to home.
fn scratch_restore_project_id(
    label: &str,
    bound_project_id: Result<String, String>,
) -> Result<Option<String>, String> {
    match bound_project_id {
        Ok(id) if is_scratch_project_id(&id) => Err("Scratch chat is already active.".into()),
        Ok(id) => Ok(Some(id)),
        Err(_) if is_blank_window_label(label) => Ok(None),
        Err(err) => Err(err),
    }
}

fn scratch_sandbox_root(app_data: &Path) -> PathBuf {
    app_data.join("scratch")
}

/// Scratch projects and sandbox directories a previous run left behind.
#[derive(Default)]
pub(super) struct OrphanScratchProjects {
    projects: Vec<(String, String)>,
    sandboxes: Vec<PathBuf>,
}

/// Name the orphans while nothing else can create a scratch chat yet. Deleting
/// a sandbox walks a whole directory tree, so the deletion itself runs later
/// (see `purge_orphan_scratch_projects`) and must not see sandboxes created
/// after startup.
pub(super) async fn collect_orphan_scratch_projects(
    store: &Store,
    app_data: &Path,
) -> OrphanScratchProjects {
    let projects = store.list_scratch_projects().await.unwrap_or_default();
    let sandboxes = std::fs::read_dir(scratch_sandbox_root(app_data))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default();
    OrphanScratchProjects {
        projects,
        sandboxes,
    }
}

pub(super) async fn purge_orphan_scratch_projects(store: &Store, orphans: OrphanScratchProjects) {
    for (id, ws) in orphans.projects {
        let _ = store.delete_project(&id).await;
        if !ws.is_empty() {
            let _ = std::fs::remove_dir_all(&ws);
        }
    }
    for sandbox in orphans.sandboxes {
        let _ = std::fs::remove_dir_all(sandbox);
    }
}

async fn destroy_scratch_window(state: &AppState, label: &str, scratch: ScratchWindow) {
    cancel_project_sessions(state, &scratch.scratch_project_id).await;
    state
        .runtime_manager
        .stop_project(&scratch.scratch_project_id)
        .await;
    let _ = state
        .store
        .delete_project(&scratch.scratch_project_id)
        .await;
    let _ = std::fs::remove_dir_all(&scratch.sandbox_path);
    if state.active_frame(label).as_deref() == Some(scratch.session_id.as_str()) {
        state.set_active_frame(label, None);
    }
    if let Some(prev_project_id) = scratch.prev_project_id.as_deref() {
        let _ = set_active_project(state, label, prev_project_id).await;
    } else {
        state.active.write().unwrap().remove(label);
        state.set_active_frame(label, None);
    }
}

async fn close_scratch_for_window(state: &AppState, label: &str) {
    let scratch = state.scratch.write().unwrap().remove(label);
    if let Some(scratch) = scratch {
        destroy_scratch_window(state, label, scratch).await;
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScratchChatInfo {
    session_id: String,
    project_id: String,
}

#[tauri::command]
pub(super) async fn start_scratch_chat(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<ScratchChatInfo, String> {
    let label = window.label().to_string();
    close_scratch_for_window(state.inner(), &label).await;

    let prev_project_id =
        scratch_restore_project_id(&label, state.require_active(&label).map(|ap| ap.id))?;

    let uuid = Uuid::new_v4().to_string();
    let sandbox_path = scratch_sandbox_root(&state.app_data).join(&uuid);
    std::fs::create_dir_all(&sandbox_path)
        .map_err(|e| format!("Failed to create scratch sandbox: {e}"))?;
    let marker = sandbox_path.join(".wisp-write-test");
    std::fs::write(&marker, b"").map_err(|e| format!("Scratch sandbox is not writable: {e}"))?;
    let _ = std::fs::remove_file(&marker);

    let project_id = format!("{SCRATCH_PROJECT_PREFIX}{uuid}");
    let ws = sandbox_path.to_string_lossy().into_owned();
    state
        .store
        .create_project(&project_id, "Scratch", &ws)
        .await
        .map_err(|e| format!("{e}"))?;

    let session_id = create_session_frame(&state.store, &project_id).await?;
    let (ap, _, _) = load_active_project(state.inner(), &project_id).await?;
    state.set_active(&label, ap);
    state.set_active_frame(&label, Some(session_id.clone()));
    state.scratch.write().unwrap().insert(
        label,
        ScratchWindow {
            prev_project_id,
            scratch_project_id: project_id.clone(),
            session_id: session_id.clone(),
            sandbox_path,
        },
    );

    Ok(ScratchChatInfo {
        session_id,
        project_id,
    })
}

#[tauri::command]
pub(super) async fn close_scratch_chat(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    close_scratch_for_window(state.inner(), window.label()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_store::Store;

    #[tokio::test]
    async fn purge_orphan_scratch_projects_cleans_db_and_dirs() {
        let app_data = std::env::temp_dir().join(format!("wisp_scratch_purge_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&app_data).unwrap();
        let db = app_data.join("wisp.sqlite");
        let store = Store::open(&db).await.unwrap();
        let orphan_dir = scratch_sandbox_root(&app_data).join("orphan");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        let project_id = format!("{SCRATCH_PROJECT_PREFIX}orphan");
        store
            .create_project(&project_id, "Scratch", &orphan_dir.to_string_lossy())
            .await
            .unwrap();

        let orphans = collect_orphan_scratch_projects(&store, &app_data).await;
        purge_orphan_scratch_projects(&store, orphans).await;

        assert!(store.get_project(&project_id).await.unwrap().is_none());
        assert!(!orphan_dir.exists());
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[tokio::test]
    async fn purge_spares_scratch_chats_created_after_startup() {
        let app_data = std::env::temp_dir().join(format!("wisp_scratch_race_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&app_data).unwrap();
        let store = Store::open(&app_data.join("wisp.sqlite")).await.unwrap();
        let orphan_dir = scratch_sandbox_root(&app_data).join("orphan");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        let orphan_id = format!("{SCRATCH_PROJECT_PREFIX}orphan");
        store
            .create_project(&orphan_id, "Scratch", &orphan_dir.to_string_lossy())
            .await
            .unwrap();

        let orphans = collect_orphan_scratch_projects(&store, &app_data).await;

        // A scratch chat opened while the deferred purge is still pending.
        let fresh_dir = scratch_sandbox_root(&app_data).join("fresh");
        std::fs::create_dir_all(&fresh_dir).unwrap();
        let fresh_id = format!("{SCRATCH_PROJECT_PREFIX}fresh");
        store
            .create_project(&fresh_id, "Scratch", &fresh_dir.to_string_lossy())
            .await
            .unwrap();

        purge_orphan_scratch_projects(&store, orphans).await;

        assert!(store.get_project(&orphan_id).await.unwrap().is_none());
        assert!(!orphan_dir.exists());
        assert!(store.get_project(&fresh_id).await.unwrap().is_some());
        assert!(fresh_dir.exists());
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn scratch_from_a_blank_window_has_no_project_to_restore() {
        assert_eq!(
            scratch_restore_project_id("home-1", Err(BLANK_WINDOW_NO_PROJECT.into())).unwrap(),
            None
        );
        assert_eq!(
            scratch_restore_project_id("main", Err(BLANK_WINDOW_NO_PROJECT.into())).unwrap_err(),
            BLANK_WINDOW_NO_PROJECT
        );
        assert_eq!(
            scratch_restore_project_id("proj-abc", Ok("workspace-1".into())).unwrap(),
            Some("workspace-1".into())
        );
        assert_eq!(
            scratch_restore_project_id("main", Ok(format!("{SCRATCH_PROJECT_PREFIX}open")))
                .unwrap_err(),
            "Scratch chat is already active."
        );
    }
}
