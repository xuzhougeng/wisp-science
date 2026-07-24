//! Session Commands split out of lib.rs; shared state/helpers stay in the crate root.

use super::*;

#[tauri::command]
pub(super) async fn new_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<String, String> {
    // Create a fresh frame and hand its id to the UI up front, so the UI can
    // route streamed events to the right transcript *before* the first delta
    // arrives. Does NOT cancel any running turn — parallel conversations keep
    // running. Empty frames are filtered out of the sidebar until they get a
    // user message.
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let id = create_session_frame(&state.store, &ap.id).await?;
    state.set_active_frame(window.label(), Some(id.clone()));
    Ok(id)
}

#[tauri::command]
pub(super) async fn branch_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
    title: Option<String>,
    user_index: Option<usize>,
) -> Result<String, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let id = create_session_frame(&state.store, &ap.id).await?;
    if let Some(source) = session_id.as_deref().filter(|s| !s.is_empty()) {
        let model_id = models::session_profile_id(&state.store, source).await;
        state
            .store
            .set_frame_model(&id, &ap.id, &model_id)
            .await
            .map_err(|error| error.to_string())?;
        let msgs = state
            .store
            .load_messages(source)
            .await
            .map_err(|e| format!("{e}"))?;
        let keep = user_index
            .map(|idx| user_message_start(&msgs, idx))
            .unwrap_or(msgs.len());
        for (idx, msg) in msgs.iter().take(keep).enumerate() {
            state
                .store
                .append_message(&id, idx as i64 + 1, msg)
                .await
                .map_err(|e| format!("{e}"))?;
        }
    }
    if let Some(t) = branch_title(title.as_deref()) {
        let _ = state.store.rename_session(&id, &ap.id, &t).await;
    }
    state.set_active_frame(window.label(), Some(id.clone()));
    Ok(id)
}

#[tauri::command]
pub(super) async fn list_sessions(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<SessionInfo>, String> {
    let ap = state.active(window.label());
    let rows = state
        .store
        .list_sessions(&ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    let pinned_ids = state
        .store
        .list_pinned_sessions(&ap.id)
        .await
        .map(|rows| rows.into_iter().map(|row| row.0).collect::<HashSet<_>>())
        .unwrap_or_default();
    let running = state.running_turns.lock().await.clone();
    Ok(rows
        .into_iter()
        .map(|(id, title, ts, folder_id)| SessionInfo {
            running: running.contains(&id),
            pinned: pinned_ids.contains(&id),
            id,
            title,
            ts,
            folder_id,
        })
        .collect())
}

#[tauri::command]
pub(super) async fn list_sessions_page(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    cursor: Option<SessionCursor>,
) -> Result<SessionPage, String> {
    let ap = state.active(window.label());
    let mut rows = state
        .store
        .list_sessions_page(
            &ap.id,
            cursor
                .as_ref()
                .map(|cursor| (cursor.ts, cursor.id.as_str())),
            SESSION_HISTORY_PAGE_SIZE + 1,
        )
        .await
        .map_err(|e| format!("{e}"))?;
    let has_more = rows.len() > SESSION_HISTORY_PAGE_SIZE;
    rows.truncate(SESSION_HISTORY_PAGE_SIZE);
    let next_cursor = has_more.then(|| {
        let row = rows.last().expect("a full session page has a final row");
        SessionCursor {
            ts: row.2,
            id: row.0.clone(),
        }
    });
    // Pinned sessions float to the top and must show even when they fall outside
    // the newest keyset page, so fetch them once (first page only) and prepend any
    // that aren't already in this page. The keyset cursor is left untouched.
    let pinned_rows = match cursor {
        None => state
            .store
            .list_pinned_sessions(&ap.id)
            .await
            .map_err(|e| format!("{e}"))?,
        Some(_) => Vec::new(),
    };
    let pinned_ids: HashSet<String> = pinned_rows.iter().map(|row| row.0.clone()).collect();
    let page_ids: HashSet<String> = rows.iter().map(|row| row.0.clone()).collect();
    let running = state.running_turns.lock().await.clone();
    let mut items: Vec<SessionInfo> = pinned_rows
        .into_iter()
        .filter(|(id, ..)| !page_ids.contains(id))
        .map(|(id, title, ts, folder_id)| SessionInfo {
            running: running.contains(&id),
            pinned: true,
            id,
            title,
            ts,
            folder_id,
        })
        .collect();
    items.extend(
        rows.into_iter()
            .map(|(id, title, ts, folder_id)| SessionInfo {
                running: running.contains(&id),
                pinned: pinned_ids.contains(&id),
                id,
                title,
                ts,
                folder_id,
            }),
    );
    Ok(SessionPage {
        items,
        next_cursor,
        running_ids: running.into_iter().collect(),
    })
}

#[tauri::command]
pub(super) async fn list_folders(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> Result<Vec<FolderInfo>, String> {
    let ap = state.active(window.label());
    let rows = state
        .store
        .list_folders(&ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|(id, name, _)| FolderInfo { id, name })
        .collect())
}

#[tauri::command]
pub(super) async fn create_folder(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    name: String,
) -> Result<FolderInfo, String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let id = Uuid::new_v4().to_string();
    state
        .store
        .create_folder(&id, &ap.id, &name)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(FolderInfo {
        id,
        name: name.trim().to_string(),
    })
}

#[tauri::command]
pub(super) async fn rename_folder(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    name: String,
) -> Result<(), String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .store
        .rename_folder(&id, &ap.id, &name)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
pub(super) async fn delete_folder(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .store
        .delete_folder(&id, &ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
pub(super) async fn move_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    folder_id: Option<String>,
) -> Result<(), String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .store
        .move_session_to_folder(&id, &ap.id, folder_id.as_deref())
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
pub(super) async fn transfer_session_to_project(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    target_project_id: String,
    mode: String,
) -> Result<String, String> {
    let source = state.active(window.label());
    if target_project_id == source.id {
        return Err("Source and target projects must be different.".into());
    }
    if state
        .store
        .get_project(&target_project_id)
        .await
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Err("Target project not found.".into());
    }
    let owner = state
        .store
        .frame_project_id(&id)
        .await
        .map_err(|error| error.to_string())?;
    if owner.as_deref() != Some(source.id.as_str()) {
        return Err("Session does not belong to the active project.".into());
    }
    let remove_source = match mode.as_str() {
        "copy" => false,
        "move" => true,
        _ => return Err("Transfer mode must be 'copy' or 'move'.".into()),
    };

    let session_is_busy = || {
        state.awaiting_confirm.lock().unwrap().contains(&id)
            || state.reviewing.lock().unwrap().contains(&id)
    };
    if state.running_turns.lock().await.contains(&id) || session_is_busy() {
        return Err(
            "Wait for the session to finish its turn, approval, or review before transferring it."
                .into(),
        );
    }

    let _source_activity = state.begin_project_activity(&source.id)?;
    let _target_activity = state.begin_project_activity(&target_project_id)?;
    let runtime = state.sessions.lock().await.get(&id).cloned();
    let _workflow_guard = match runtime.as_ref() {
        Some(runtime) => Some(runtime.workflow.lock().await),
        None => None,
    };
    let _agent_guard = match runtime.as_ref() {
        Some(runtime) => Some(runtime.agent.lock().await),
        None => None,
    };
    if state.running_turns.lock().await.contains(&id) || session_is_busy() {
        return Err(
            "Wait for the session to finish its turn, approval, or review before transferring it."
                .into(),
        );
    }

    let new_id = Uuid::new_v4().to_string();
    if remove_source {
        state
            .store
            .move_session_to_project(&id, &source.id, &target_project_id, &new_id)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(runtime) = runtime.as_ref() {
            runtime.deleted.store(true, Ordering::SeqCst);
            runtime.cancel.store(true, Ordering::Relaxed);
        }
        acp::close_frame(&state, &id).await;
        state.sessions.lock().await.remove(&id);
        if state.active_frame(window.label()).as_deref() == Some(id.as_str()) {
            state.set_active_frame(window.label(), None);
        }
    } else {
        state
            .store
            .copy_session_to_project(&id, &source.id, &target_project_id, &new_id)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(new_id)
}

#[tauri::command]
pub(super) async fn delete_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    let owner = state
        .store
        .frame_project_id(&id)
        .await
        .map_err(|error| error.to_string())?;
    if owner.as_deref() != Some(ap.id.as_str()) {
        return Err("Session does not belong to the active project.".into());
    }
    let runtime = state.sessions.lock().await.get(&id).cloned();
    if let Some(rt) = runtime.as_ref() {
        rt.deleted.store(true, Ordering::SeqCst);
        rt.cancel.store(true, Ordering::Relaxed);
    }
    acp::cancel_frame(&state, &id).await;
    // Match send/Plan lock order. The tombstone prevents work already queued
    // behind these guards from restarting after the DB cascade.
    let _workflow_guard = match runtime.as_ref() {
        Some(rt) => Some(rt.workflow.lock().await),
        None => None,
    };
    let _agent_guard = match runtime.as_ref() {
        Some(rt) => Some(rt.agent.lock().await),
        None => None,
    };
    acp::close_frame(&state, &id).await;
    state.sessions.lock().await.remove(&id);
    if state.active_frame(window.label()).as_deref() == Some(id.as_str()) {
        state.set_active_frame(window.label(), None);
    }
    state
        .store
        .delete_session(&id, &ap.id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
pub(super) async fn rename_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    title: String,
) -> Result<(), String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .store
        .rename_session(&id, &ap.id, &title)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
pub(super) async fn set_session_pinned(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    pinned: bool,
) -> Result<(), String> {
    let ap = state.active(window.label());
    let _project_activity = state.begin_project_activity(&ap.id)?;
    state
        .store
        .set_session_pinned(&id, &ap.id, pinned)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

/// How many sessions appear on the Projects landing "Recent sessions" column.
pub(super) const RECENT_SESSIONS_LIMIT: i64 = 5;

#[tauri::command]
pub(super) async fn list_recent_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let running = state.running_turns.lock().await.clone();
    let awaiting = state.awaiting_confirm.lock().unwrap().clone();
    let rows = state
        .store
        .list_recent_sessions_detail(RECENT_SESSIONS_LIMIT)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let status = session_runtime_status(
                &r.id,
                r.last_role.as_deref(),
                r.unseen,
                &running,
                &awaiting,
            );
            serde_json::json!({
                "id": r.id,
                "project_id": r.project_id,
                "title": r.title,
                "ts": r.created_at,
                "status": status,
            })
        })
        .collect())
}

/// Switch the active session to `id`, load its transcript, and return the
/// rendered rows so the UI can repopulate the conversation view.
/// Rewind the named session to just before the given user turn (for message
/// edit). Only touches that session's agent context and DB rows.
#[tauri::command]
pub(super) async fn rewind_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    session_id: Option<String>,
    user_index: usize,
) -> Result<(), String> {
    let frame_id = match session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => state
            .active_frame(window.label())
            .ok_or_else(|| "No active session to rewind.".to_string())?,
    };
    let project_id = state
        .store
        .frame_project_id(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Session project was not found.".to_string())?;
    let _project_activity = state.begin_project_activity(&project_id)?;
    if state
        .store
        .get_acp_session(&frame_id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("ACP sessions cannot be rewound in protocol v1.".into());
    }
    let rt = state.sessions.lock().await.get(&frame_id).cloned();
    let keep = if let Some(rt) = rt {
        let mut guard = rt.agent.lock().await;
        if let Some(agent) = guard.as_mut() {
            let k = user_message_start(&agent.ctx.messages, user_index);
            agent.ctx.messages.truncate(k);
            k
        } else {
            user_index_to_keep_after_db(&state.store, &frame_id, user_index).await?
        }
    } else {
        user_index_to_keep_after_db(&state.store, &frame_id, user_index).await?
    };
    state
        .store
        .truncate_messages(&frame_id, keep as i64)
        .await
        .map_err(|e| format!("{e}"))?;
    if let Some(rt) = state.sessions.lock().await.get(&frame_id) {
        rt.set_last_seq(keep as i64);
    }
    Ok(())
}

/// Compute the `keep` index purely from persisted messages when no in-memory
/// agent exists for the session yet.
pub(super) async fn user_index_to_keep_after_db(
    store: &Store,
    frame_id: &str,
    user_index: usize,
) -> Result<usize, String> {
    let msgs = store
        .load_messages(frame_id)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(user_message_start(&msgs, user_index))
}

pub(super) fn transcript_page_items(
    page: &wisp_store::SessionTranscriptPage,
) -> Result<Vec<UiItem>, String> {
    let msgs = page
        .messages
        .iter()
        .map(|(_, message)| message.clone())
        .collect::<Vec<_>>();
    let events: Vec<AgentEvent> = page
        .ui_events
        .iter()
        .map(|json| serde_json::from_str(json))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("invalid persisted UI event: {e}"))?;
    let (mut items, boundaries) = if events.is_empty() {
        (messages_to_items(&msgs), HashMap::new())
    } else {
        let first_seq = events.iter().find_map(|event| match event {
            AgentEvent::MessageBoundary { seq, .. } => Some(*seq),
            _ => None,
        });
        let prefix_len = first_seq.map_or(msgs.len(), |first_seq| {
            page.messages
                .iter()
                .take_while(|(seq, _)| *seq < first_seq)
                .count()
        });
        let mut prefix = messages_to_items(&msgs[..prefix_len]);
        let prefix_items = prefix.len();
        let (event_items, event_boundaries) = events_to_items(&events);
        prefix.extend(event_items);
        (
            prefix,
            event_boundaries
                .into_iter()
                .map(|(seq, offset)| (seq, prefix_items + offset))
                .collect(),
        )
    };
    let mut resources_by_seq = HashMap::<i64, Vec<resource_refs::UiMessageResource>>::new();
    for resource in &page.resources {
        resources_by_seq
            .entry(resource.message_seq)
            .or_default()
            .push(resource.into());
    }
    for (message_seq, resources) in resources_by_seq {
        let end = boundaries.get(&message_seq).copied().unwrap_or_else(|| {
            let message_count = page
                .messages
                .iter()
                .take_while(|(seq, _)| *seq <= message_seq)
                .count();
            messages_to_items(&msgs[..message_count]).len()
        });
        let end = end.min(items.len());
        if let Some(item) = items[..end]
            .iter_mut()
            .rev()
            .find(|item| item.role == "assistant")
        {
            item.resources = resources;
        }
    }
    let mut inserted = 0usize;
    for (message_seq, report_json) in &page.reviews {
        let report: review::ReviewReport = serde_json::from_str(&report_json)
            .map_err(|e| format!("invalid persisted review: {e}"))?;
        let at = boundaries.get(message_seq).copied().unwrap_or_else(|| {
            let message_count = page
                .messages
                .iter()
                .take_while(|(seq, _)| seq <= message_seq)
                .count();
            messages_to_items(&msgs[..message_count]).len()
        }) + inserted;
        items.insert(
            at,
            UiItem {
                role: "review".into(),
                text: serde_json::to_string(&report).map_err(|e| format!("{e}"))?,
                tool_name: None,
                ok: None,
                duration_ms: None,
                input: None,
                model_name: None,
                call_id: None,
                kind: None,
                status: None,
                locations: None,
                resources: Vec::new(),
            },
        );
        inserted += 1;
    }
    Ok(items)
}

#[tauri::command]
pub(super) async fn load_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
    before_seq: Option<i64>,
) -> Result<SessionTranscriptPage, String> {
    let page = state
        .store
        .load_session_transcript_page(&id, before_seq, SESSION_TRANSCRIPT_PAGE_TURNS)
        .await
        .map_err(|e| format!("{e}"))?;
    let presentations = if before_seq.is_none() {
        state
            .store
            .load_latest_session_ui_event(&id, "ToolPresentation")
            .await
            .map_err(|e| format!("{e}"))?
            .map(|json| serde_json::from_str::<AgentEvent>(&json))
            .transpose()
            .map_err(|e| format!("invalid persisted tool presentation: {e}"))?
            .and_then(|event| match event {
                AgentEvent::ToolPresentation {
                    presentation_id,
                    presentation_kind,
                    payload,
                    ..
                } => Some(SessionPresentation {
                    presentation_id,
                    presentation_kind,
                    payload,
                }),
                _ => None,
            })
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let outline = if before_seq.is_none() {
        state
            .store
            .load_session_user_messages(&id)
            .await
            .map_err(|e| format!("{e}"))?
            .into_iter()
            .enumerate()
            .map(|(user_index, (seq, text))| SessionOutlineItem {
                user_index,
                seq,
                text,
            })
            .collect()
    } else {
        Vec::new()
    };
    if before_seq.is_none() {
        state.set_active_frame(window.label(), Some(id.clone()));
        let _ = state.store.mark_frame_seen(&id).await;
        if let Some(rt) = state.sessions.lock().await.get(&id).cloned() {
            rt.set_last_seq(page.latest_seq);
        }
    }
    let items = transcript_page_items(&page)?;
    Ok(SessionTranscriptPage {
        items,
        next_before_seq: page.next_before_seq,
        user_offset: page.user_offset,
        outline,
        presentations,
    })
}

/// Mark which session this window is viewing without loading it. The UI calls
/// this instead of `load_session` when switching to a *running* session (it
/// renders the cached streaming transcript), so uploads still attach to the
/// viewed session (#194) — `load_session` would clobber the runtime's
/// `last_seq` with the DB snapshot mid-stream.
#[tauri::command]
pub(super) async fn set_viewed_session(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    state.set_active_frame(window.label(), Some(id.clone()));
    let _ = state.store.mark_frame_seen(&id).await;
    Ok(())
}

#[tauri::command]
pub(super) async fn search_sessions(
    state: State<'_, AppState>,
    query: Option<String>,
    limit: Option<i64>,
    project_id: Option<String>,
) -> Result<Vec<SessionSearchInfo>, String> {
    let running = state.running_turns.lock().await.clone();
    let awaiting = state.awaiting_confirm.lock().unwrap().clone();
    let rows = state
        .store
        .search_sessions(
            project_id.as_deref(),
            query.as_deref().unwrap_or(""),
            limit.unwrap_or(12),
            None,
        )
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(rows
        .into_iter()
        .map(|s| SessionSearchInfo {
            status: session_runtime_status(
                &s.id,
                s.last_role.as_deref(),
                s.unseen,
                &running,
                &awaiting,
            )
            .into(),
            id: s.id,
            project_id: s.project_id,
            project_name: s.project_name,
            title: s.title,
            ts: s.created_at,
            activity_at: s.activity_at,
        })
        .collect())
}
