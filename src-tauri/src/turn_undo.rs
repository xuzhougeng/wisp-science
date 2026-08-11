use super::*;

const MAX_UNDO_TEXT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnUndoPreview {
    restore_files: Vec<String>,
    remove_files: Vec<String>,
    remove_artifacts: Vec<String>,
    unsupported_files: Vec<String>,
    conflicts: Vec<String>,
}

#[derive(Debug)]
struct UndoTarget {
    keep_seq: i64,
    user_seq: i64,
}

#[derive(Debug)]
struct AppliedFileUndo {
    path: String,
    real: PathBuf,
    trash: PathBuf,
    before_exists: bool,
}

fn persist_snapshot(root: &Path, text: &str, checksum: &str) -> Result<String, String> {
    let path = root
        .join(".wisp")
        .join("undo")
        .join("sha256")
        .join(&checksum[..2])
        .join(checksum);
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, text.as_bytes()).map_err(|error| error.to_string())?;
    }
    Ok(path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/"))
}

pub(super) async fn persist_provenance_changes(
    store: &Store,
    root: &Path,
    frame_id: &str,
    user_message_seq: i64,
    changes: &[wisp_core::provenance::ProvenanceFileChange],
) {
    for change in changes {
        let snapshot = match (
            change.before_text.as_deref(),
            change.before_checksum.as_deref(),
        ) {
            (Some(text), Some(checksum)) => match persist_snapshot(root, text, checksum) {
                Ok(path) => Some(path),
                Err(error) => {
                    tracing::warn!(
                        %frame_id,
                        path = %change.path,
                        %error,
                        "failed to persist turn undo snapshot"
                    );
                    None
                }
            },
            _ => None,
        };
        let reversible = change.reversible
            && (!change.before_exists || snapshot.is_some())
            && change.after_checksum.is_some();
        let reason = if reversible {
            None
        } else {
            change
                .reason
                .as_deref()
                .or(Some("undo snapshot could not be persisted"))
        };
        if let Err(error) = store
            .save_turn_file_undo(
                frame_id,
                user_message_seq,
                &change.path,
                change.before_exists,
                snapshot.as_deref(),
                change.before_checksum.as_deref(),
                change.after_checksum.as_deref(),
                reversible,
                reason,
            )
            .await
        {
            tracing::warn!(
                %frame_id,
                path = %change.path,
                %error,
                "failed to persist turn undo lineage"
            );
        }
    }
}

async fn target_for_user_index(
    store: &Store,
    frame_id: &str,
    user_index: usize,
) -> Result<UndoTarget, String> {
    let messages = store
        .load_messages_with_seq(frame_id)
        .await
        .map_err(|error| error.to_string())?;
    let users = messages
        .iter()
        .enumerate()
        .filter(|(_, (_, message))| {
            message.role == wisp_llm::Role::User
                && message.tool_name.as_deref() != Some(wisp_store::AGENT_WORKFLOW_COMPLETION_TOOL)
                && !message.content.as_text().trim().is_empty()
        })
        .collect::<Vec<_>>();
    if users.is_empty() || user_index + 1 != users.len() {
        return Err("Only the latest completed turn can be undone.".into());
    }
    let (message_index, (user_seq, _)) = users[user_index];
    if !messages[message_index + 1..]
        .iter()
        .any(|(_, message)| message.role == wisp_llm::Role::Assistant)
    {
        return Err("The latest turn has no completed reply to undo.".into());
    }
    Ok(UndoTarget {
        keep_seq: message_index
            .checked_sub(1)
            .map(|index| messages[index].0)
            .unwrap_or(0),
        user_seq: *user_seq,
    })
}

fn checksum_file(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_UNDO_TEXT_BYTES {
        return Err("file now exceeds the 10 MiB undo limit".into());
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    Ok(wisp_sync::sha256_hex(&bytes))
}

fn current_matches(
    root: &Path,
    change: &wisp_store::TurnFileUndo,
) -> Result<(PathBuf, bool), String> {
    let real = wisp_tools::safety::validate_file_path(root, &change.path)?;
    let Some(after) = change.after_checksum.as_deref() else {
        return Err("missing post-change checksum".into());
    };
    if !real.exists() {
        return if change.before_exists {
            Err("file was removed after this turn".into())
        } else {
            Ok((real, true))
        };
    }
    let current = checksum_file(&real)?;
    let already_restored = change
        .before_checksum
        .as_deref()
        .is_some_and(|before| before == current);
    if current != after && !already_restored {
        return Err("file changed again after this turn".into());
    }
    Ok((real, already_restored))
}

async fn build_preview(
    store: &Store,
    root: &Path,
    frame_id: &str,
    target: &UndoTarget,
) -> Result<(TurnUndoPreview, Vec<wisp_store::TurnFileUndo>), String> {
    let changes = store
        .list_turn_file_undo(frame_id, target.user_seq)
        .await
        .map_err(|error| error.to_string())?;
    let artifacts = store
        .list_owned_message_artifacts(frame_id, target.keep_seq)
        .await
        .map_err(|error| error.to_string())?;
    let mut preview = TurnUndoPreview {
        restore_files: Vec::new(),
        remove_files: Vec::new(),
        remove_artifacts: artifacts.iter().map(|(name, _)| name.clone()).collect(),
        unsupported_files: Vec::new(),
        conflicts: Vec::new(),
    };
    for change in &changes {
        if !change.reversible {
            preview.unsupported_files.push(change.path.clone());
            continue;
        }
        match current_matches(root, change) {
            Ok(_) if change.before_exists => preview.restore_files.push(change.path.clone()),
            Ok(_) => preview.remove_files.push(change.path.clone()),
            Err(error) => preview.conflicts.push(format!("{}: {error}", change.path)),
        }
    }
    for (name, mime) in artifacts {
        let tracked_file = changes
            .iter()
            .any(|change| change.path == name || change.path.ends_with(&format!("/{name}")));
        if !mime.starts_with("text/")
            && !wisp_core::provenance::is_text_path(Path::new(&name))
            && !tracked_file
            && !preview
                .unsupported_files
                .iter()
                .any(|path| path.ends_with(&name))
        {
            preview.unsupported_files.push(name);
        }
    }
    for list in [
        &mut preview.restore_files,
        &mut preview.remove_files,
        &mut preview.remove_artifacts,
        &mut preview.unsupported_files,
        &mut preview.conflicts,
    ] {
        list.sort();
        list.dedup();
    }
    Ok((preview, changes))
}

fn trash_path(root: &Path, real: &Path) -> Result<PathBuf, String> {
    let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let relative = real
        .strip_prefix(&root)
        .map_err(|_| "undo target is outside the project root".to_string())?;
    Ok(root
        .join(".wisp")
        .join("undo")
        .join("trash")
        .join(Uuid::new_v4().to_string())
        .join(relative))
}

fn apply_file_undo(
    root: &Path,
    changes: &[wisp_store::TurnFileUndo],
) -> Result<Vec<AppliedFileUndo>, String> {
    for change in changes.iter().filter(|change| change.reversible) {
        current_matches(root, change)
            .map_err(|error| format!("Cannot undo '{}': {error}", change.path))?;
    }

    let mut applied = Vec::new();
    for change in changes.iter().filter(|change| change.reversible) {
        let result = (|| -> Result<Option<AppliedFileUndo>, String> {
            let (real, already_restored) = current_matches(root, change)?;
            if already_restored || (!change.before_exists && !real.exists()) {
                return Ok(None);
            }
            let trash = trash_path(root, &real)?;
            if let Some(parent) = trash.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            if change.before_exists {
                let snapshot = change
                    .before_snapshot_path
                    .as_deref()
                    .ok_or_else(|| format!("Missing undo snapshot for '{}'.", change.path))?;
                let snapshot = wisp_tools::safety::validate_file_path(root, snapshot)?;
                let bytes = std::fs::read(&snapshot).map_err(|error| error.to_string())?;
                let checksum = wisp_sync::sha256_hex(&bytes);
                if change.before_checksum.as_deref() != Some(checksum.as_str()) {
                    return Err(format!("Undo snapshot for '{}' is corrupt.", change.path));
                }
                std::fs::copy(&real, &trash).map_err(|error| error.to_string())?;
                if let Err(error) = std::fs::write(&real, bytes) {
                    let recovery = std::fs::copy(&trash, &real);
                    return Err(match recovery {
                        Ok(_) => error.to_string(),
                        Err(recovery) => {
                            format!("{error}; restoring the changed file also failed: {recovery}")
                        }
                    });
                }
            } else {
                std::fs::rename(&real, &trash).map_err(|error| error.to_string())?;
            }
            Ok(Some(AppliedFileUndo {
                path: change.path.clone(),
                real,
                trash,
                before_exists: change.before_exists,
            }))
        })();
        match result {
            Ok(Some(change)) => applied.push(change),
            Ok(None) => {}
            Err(error) => {
                if let Err(rollback) = rollback_file_undo(&applied) {
                    return Err(format!(
                        "{error}; rolling back the partial undo failed: {rollback}"
                    ));
                }
                return Err(error);
            }
        }
    }
    Ok(applied)
}

fn rollback_file_undo(applied: &[AppliedFileUndo]) -> Result<(), String> {
    for change in applied.iter().rev() {
        if change.before_exists {
            std::fs::copy(&change.trash, &change.real).map_err(|error| error.to_string())?;
        } else {
            if change.real.exists() {
                return Err(format!(
                    "cannot restore '{}' because the path is occupied",
                    change.path
                ));
            }
            std::fs::rename(&change.trash, &change.real).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

async fn frame_and_project(
    state: &AppState,
    window_label: &str,
    session_id: Option<String>,
) -> Result<(String, ActiveProject), String> {
    let frame_id = session_id
        .filter(|id| !id.is_empty())
        .or_else(|| state.active_frame(window_label))
        .ok_or_else(|| "No active session to undo.".to_string())?;
    let project_id = state
        .store
        .frame_project_id(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Session project was not found.".to_string())?;
    let project = project_commands::load_active_project(state, &project_id)
        .await?
        .0;
    Ok((frame_id, project))
}

async fn validate_undo_session(state: &AppState, frame_id: &str) -> Result<(), String> {
    if state.running_turns.lock().await.contains(frame_id) {
        return Err("Wait for the current turn to finish before undoing it.".into());
    }
    if state.reviewing.lock().unwrap().contains(frame_id) {
        return Err("Wait for review to finish before undoing this turn.".into());
    }
    if state
        .store
        .get_acp_session(frame_id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("ACP sessions cannot be undone in protocol v1.".into());
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn preview_turn_undo(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
    user_index: usize,
) -> Result<TurnUndoPreview, String> {
    let (frame_id, project) = frame_and_project(state.inner(), window.label(), session_id).await?;
    validate_undo_session(state.inner(), &frame_id).await?;
    let target = target_for_user_index(&state.store, &frame_id, user_index).await?;
    build_preview(&state.store, &project.root, &frame_id, &target)
        .await
        .map(|(preview, _)| preview)
}

#[tauri::command]
pub(super) async fn undo_turn(
    state: State<'_, AppState>,
    app: AppHandle,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
    user_index: usize,
) -> Result<TurnUndoPreview, String> {
    let (frame_id, project) = frame_and_project(state.inner(), window.label(), session_id).await?;
    let _project_activity = state.begin_project_activity(&project.id)?;
    let scope = state
        .store
        .frame_state_scope(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Session state scope was not found.".to_string())?;
    exploration_commands::require_writable_scope(&state.store, &scope).await?;
    validate_undo_session(state.inner(), &frame_id).await?;
    let runtime = state.sessions.lock().await.get(&frame_id).cloned();
    let _workflow = match runtime.as_ref() {
        Some(runtime) => Some(
            runtime
                .workflow
                .clone()
                .try_lock_owned()
                .map_err(|_| "The session is busy; try undo again when it finishes.")?,
        ),
        None => None,
    };
    if runtime
        .as_ref()
        .is_some_and(|runtime| !runtime.control.follow_ups().is_empty())
    {
        return Err("Remove queued messages before undoing the latest turn.".into());
    }

    let target = target_for_user_index(&state.store, &frame_id, user_index).await?;
    let (preview, changes) = build_preview(&state.store, &project.root, &frame_id, &target).await?;
    if !preview.conflicts.is_empty() {
        return Err(format!(
            "Some files changed after this turn:\n{}",
            preview.conflicts.join("\n")
        ));
    }
    let applied = apply_file_undo(&project.root, &changes)?;
    if let Err(error) = state
        .store
        .truncate_messages_for_undo(&frame_id, target.keep_seq)
        .await
    {
        return match rollback_file_undo(&applied) {
            Ok(()) => Err(error.to_string()),
            Err(rollback) => Err(format!(
                "{error}; restoring files after the database failure also failed: {rollback}"
            )),
        };
    }
    if let Some(runtime) = runtime {
        *runtime.agent.lock().await = None;
        runtime.set_last_seq(target.keep_seq);
    }
    for change in applied {
        crate::emit_agent_event(
            &app,
            AgentEvent::FileChanged {
                frame_id: frame_id.clone(),
                path: change.path,
            },
        );
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_the_latest_completed_user_turn_is_targetable() {
        let database =
            std::env::temp_dir().join(format!("wisp_turn_undo_target_{}.sqlite", Uuid::new_v4()));
        let store = Store::open(&database).await.unwrap();
        store.create_project("p", "Project", "").await.unwrap();
        store
            .create_frame("f", "p", "OPERON", "model")
            .await
            .unwrap();
        for (seq, message) in [
            wisp_llm::Message::system("system"),
            wisp_llm::Message::user("first"),
            wisp_llm::Message::assistant("first reply"),
            wisp_llm::Message::user("second"),
            wisp_llm::Message::assistant("second reply"),
        ]
        .iter()
        .enumerate()
        {
            store
                .append_message("f", seq as i64 + 1, message)
                .await
                .unwrap();
        }

        assert!(target_for_user_index(&store, "f", 0).await.is_err());
        let target = target_for_user_index(&store, "f", 1).await.unwrap();
        assert_eq!(target.keep_seq, 3);
        assert_eq!(target.user_seq, 4);
        store
            .append_message("f", 6, &wisp_llm::Message::user("unfinished"))
            .await
            .unwrap();
        assert!(target_for_user_index(&store, "f", 2)
            .await
            .unwrap_err()
            .contains("no completed reply"));
        std::fs::remove_file(database).ok();
    }

    #[test]
    fn applying_text_undo_restores_existing_and_trashes_new_files() {
        let root = std::env::temp_dir().join(format!("wisp_turn_undo_{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".wisp/undo/sha256/aa")).unwrap();
        std::fs::write(root.join("notes.md"), "after\n").unwrap();
        std::fs::write(root.join("new.md"), "new\n").unwrap();
        let before = "before\n";
        let before_checksum = wisp_sync::sha256_hex(before.as_bytes());
        let snapshot = root
            .join(".wisp/undo/sha256")
            .join(&before_checksum[..2])
            .join(&before_checksum);
        std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        std::fs::write(&snapshot, before).unwrap();
        let changes = vec![
            wisp_store::TurnFileUndo {
                frame_id: "f".into(),
                user_message_seq: 2,
                path: "notes.md".into(),
                before_exists: true,
                before_snapshot_path: Some(
                    snapshot
                        .strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                ),
                before_checksum: Some(before_checksum),
                after_checksum: Some(wisp_sync::sha256_hex(b"after\n")),
                reversible: true,
                reason: None,
            },
            wisp_store::TurnFileUndo {
                frame_id: "f".into(),
                user_message_seq: 2,
                path: "new.md".into(),
                before_exists: false,
                before_snapshot_path: None,
                before_checksum: None,
                after_checksum: Some(wisp_sync::sha256_hex(b"new\n")),
                reversible: true,
                reason: None,
            },
        ];

        let applied = apply_file_undo(&root, &changes).unwrap();
        assert_eq!(
            applied
                .iter()
                .map(|change| change.path.as_str())
                .collect::<Vec<_>>(),
            vec!["notes.md", "new.md"]
        );
        assert_eq!(
            std::fs::read_to_string(root.join("notes.md")).unwrap(),
            before
        );
        assert!(!root.join("new.md").exists());
        assert!(root.join(".wisp/undo/trash").exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn applied_text_undo_can_be_rolled_forward_after_a_later_failure() {
        let root = std::env::temp_dir().join(format!("wisp_turn_undo_rollback_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("notes.md"), "after\n").unwrap();
        let before = "before\n";
        let before_checksum = wisp_sync::sha256_hex(before.as_bytes());
        let snapshot = root
            .join(".wisp/undo/sha256")
            .join(&before_checksum[..2])
            .join(&before_checksum);
        std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        std::fs::write(&snapshot, before).unwrap();
        let change = wisp_store::TurnFileUndo {
            frame_id: "f".into(),
            user_message_seq: 2,
            path: "notes.md".into(),
            before_exists: true,
            before_snapshot_path: Some(
                snapshot
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            ),
            before_checksum: Some(before_checksum),
            after_checksum: Some(wisp_sync::sha256_hex(b"after\n")),
            reversible: true,
            reason: None,
        };

        let applied = apply_file_undo(&root, &[change]).unwrap();
        rollback_file_undo(&applied).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("notes.md")).unwrap(),
            "after\n"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn applying_text_undo_rejects_a_later_user_edit() {
        let root = std::env::temp_dir().join(format!("wisp_turn_undo_conflict_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("notes.md"), "user changed it\n").unwrap();
        let change = wisp_store::TurnFileUndo {
            frame_id: "f".into(),
            user_message_seq: 2,
            path: "notes.md".into(),
            before_exists: true,
            before_snapshot_path: Some(".wisp/undo/missing".into()),
            before_checksum: Some(wisp_sync::sha256_hex(b"before\n")),
            after_checksum: Some(wisp_sync::sha256_hex(b"after\n")),
            reversible: true,
            reason: None,
        };

        assert!(apply_file_undo(&root, &[change])
            .unwrap_err()
            .contains("changed again"));
        assert_eq!(
            std::fs::read_to_string(root.join("notes.md")).unwrap(),
            "user changed it\n"
        );
        std::fs::remove_dir_all(root).ok();
    }
}
