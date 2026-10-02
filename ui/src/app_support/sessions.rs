use super::*;

pub(crate) async fn invoke_string_id(cmd: &str, args: JsValue) -> Result<String, String> {
    match invoke_checked(cmd, args).await {
        Ok(value) => value
            .as_string()
            .filter(|id| !id.is_empty())
            .ok_or_else(String::new),
        Err(error) => Err(js_error_text(error)),
    }
}

pub(crate) async fn invoke_new_session() -> Result<String, String> {
    invoke_string_id("new_session", JsValue::UNDEFINED).await
}

/// Conversation `resume_last_session` should reopen. Named unused drafts stay
/// out of this path — they are sidebar rows, not "the last conversation".
pub(crate) async fn invoke_latest_used_session() -> Option<String> {
    let value = invoke_checked("latest_used_session", JsValue::UNDEFINED)
        .await
        .ok()?;
    serde_wasm_bindgen::from_value::<Option<String>>(value)
        .ok()
        .flatten()
        .filter(|id| !id.is_empty())
}

pub(crate) fn refresh_sessions(
    sessions: RwSignal<Vec<SessionInfo>>,
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    next_cursor: RwSignal<Option<SessionCursor>>,
    active_session: RwSignal<Option<String>>,
    exploration_frames: RwSignal<HashSet<String>>,
) {
    next_cursor.set(None);
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "cursor": null })).unwrap();
        let v = invoke("list_sessions_page", args).await;
        if let Ok(mut page) = serde_wasm_bindgen::from_value::<SessionPage>(v) {
            let set = pending.with_untracked(|m| rebuilt_running_set(&page.running_ids, m));
            running.set(set);
            let active = active_session.get_untracked();
            let active_is_listed = active.as_ref().is_none_or(|id| {
                page.items
                    .iter()
                    .any(|session| session.id.as_str() == id.as_str())
            });
            let active_is_exploration = active
                .as_ref()
                .is_some_and(|id| exploration_frames.with_untracked(|frames| frames.contains(id)));
            if !active_is_listed && !active_is_exploration && active != page.shelved_active_id {
                let id = active.expect("an unlisted active session has an id");
                let draft = sessions
                    .with_untracked(|current| {
                        current.iter().find(|session| session.id == id).cloned()
                    })
                    .unwrap_or(SessionInfo {
                        running: false,
                        id,
                        title: String::new(),
                        ts: js_sys::Date::now() as i64,
                        folder_id: None,
                        branched_from: None,
                        pinned: false,
                        branch_state: None,
                        stale_prompt: false,
                    });
                page.items.insert(0, draft);
            }
            sessions.set(page.items);
            next_cursor.set(page.next_cursor);
        }
    });
}

pub(crate) fn load_older_sessions(
    sessions: RwSignal<Vec<SessionInfo>>,
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    next_cursor: RwSignal<Option<SessionCursor>>,
    loading: RwSignal<bool>,
) {
    let Some(cursor) = next_cursor.get_untracked() else {
        return;
    };
    loading.set(true);
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "cursor": cursor })).unwrap();
        let v = invoke("list_sessions_page", args).await;
        if let Ok(page) = serde_wasm_bindgen::from_value::<SessionPage>(v) {
            let set = pending.with_untracked(|m| rebuilt_running_set(&page.running_ids, m));
            running.set(set);
            sessions.update(|current| {
                let existing = current
                    .iter()
                    .map(|item| item.id.clone())
                    .collect::<HashSet<_>>();
                current.extend(
                    page.items
                        .into_iter()
                        .filter(|item| !existing.contains(&item.id)),
                );
            });
            next_cursor.set(page.next_cursor);
        }
        loading.set(false);
    });
}

/// Rebuild the local `running` set from the backend's session-page snapshot
/// so restarts, project switches and other windows' turns are reflected. Keeps
/// locally pending sends the backend may not have registered yet.
pub(crate) fn rebuilt_running_set(
    running_ids: &[String],
    pending: &HashMap<String, usize>,
) -> HashSet<String> {
    let mut set: HashSet<String> = running_ids.iter().cloned().collect();
    set.extend(pending.keys().cloned());
    set
}

/// Keep the stopping banner only while that session is still the active
/// running turn. A Stop click after Done (or a missed terminal event) must
/// not leave "Stopping…" over an idle Send button.
pub(crate) fn next_stopping_session(
    stopping: Option<String>,
    active: Option<&str>,
    running: &HashSet<String>,
) -> Option<String> {
    let sid = stopping?;
    (active == Some(sid.as_str()) && running.contains(&sid)).then_some(sid)
}

#[cfg(test)]
mod rebuilt_running_set_tests {
    use super::*;

    #[test]
    fn keeps_server_running_and_local_pending() {
        let running = vec!["a".to_string()];
        let pending = HashMap::from([("c".to_string(), 1)]);
        let set = rebuilt_running_set(&running, &pending);
        assert!(set.contains("a"), "server-running kept");
        assert!(!set.contains("b"), "stale local state dropped");
        assert!(set.contains("c"), "local pending send kept");
    }

    #[test]
    fn stopping_banner_clears_when_the_session_is_idle() {
        let running = HashSet::from(["s1".to_string()]);
        assert_eq!(
            next_stopping_session(Some("s1".into()), Some("s1"), &running).as_deref(),
            Some("s1")
        );
        assert_eq!(
            next_stopping_session(Some("s1".into()), Some("s1"), &HashSet::new()),
            None,
            "idle session must not keep Stopping over Send"
        );
        assert_eq!(
            next_stopping_session(Some("s1".into()), Some("s2"), &running),
            None,
            "another conversation must not inherit the banner"
        );
        assert_eq!(next_stopping_session(None, Some("s1"), &running), None);
    }
}

pub(crate) fn refresh_folders(folders: RwSignal<Vec<FolderInfo>>) {
    spawn_local(async move {
        let v = invoke("list_folders", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<FolderInfo>>(v) {
            folders.set(list);
        }
    });
}

pub(crate) fn refresh_explorations(explorations: RwSignal<Vec<ExplorationSummary>>) {
    spawn_local(async move {
        let value = invoke("list_project_explorations", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ExplorationSummary>>(value) {
            explorations.set(list);
        }
    });
}

/// Split the sidebar list into top-level sessions plus, per session id, the
/// branches forked from it — the sidebar draws those nested underneath (#531).
///
/// A branch only nests when its source is in the same visible list *and* the
/// same folder; otherwise it stays top-level, so nothing silently disappears
/// when the source is on an older page or was dragged elsewhere. Branches of
/// branches flatten onto the topmost visible source: one level of indent only.
pub(crate) fn nest_branch_sessions(
    list: &[SessionInfo],
) -> (Vec<SessionInfo>, HashMap<String, Vec<SessionInfo>>) {
    let by_id: HashMap<&str, &SessionInfo> = list.iter().map(|s| (s.id.as_str(), s)).collect();
    let visible_source = |s: &SessionInfo| -> Option<String> {
        let mut cur = s;
        let mut hops = 0;
        loop {
            let Some(src) = cur.branched_from.as_deref().and_then(|id| by_id.get(id)) else {
                break;
            };
            if src.folder_id != cur.folder_id {
                return None;
            }
            cur = src;
            hops += 1;
            // ponytail: hop cap is the cycle guard; real branch chains are short.
            if hops > 16 {
                return None;
            }
        }
        (cur.id != s.id).then(|| cur.id.clone())
    };
    let mut top = Vec::new();
    let mut branches: HashMap<String, Vec<SessionInfo>> = HashMap::new();
    for s in list {
        match visible_source(s) {
            Some(source) => branches.entry(source).or_default().push(s.clone()),
            None => top.push(s.clone()),
        }
    }
    (top, branches)
}

#[cfg(test)]
mod branch_nesting_tests {
    use super::*;

    fn ses(id: &str, branched_from: Option<&str>, folder: Option<&str>) -> SessionInfo {
        SessionInfo {
            running: false,
            id: id.into(),
            title: id.into(),
            ts: 0,
            folder_id: folder.map(Into::into),
            branched_from: branched_from.map(Into::into),
            pinned: false,
            branch_state: branched_from.map(|_| "active".into()),
            stale_prompt: false,
        }
    }

    #[test]
    fn branches_nest_under_their_visible_source() {
        let list = vec![
            ses("main", None, None),
            ses("b1", Some("main"), None),
            ses("b2", Some("b1"), None),
            ses("other", None, None),
        ];
        let (top, branches) = nest_branch_sessions(&list);
        assert_eq!(
            top.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["main", "other"]
        );
        // Branch-of-a-branch flattens onto the topmost visible source.
        assert_eq!(
            branches["main"]
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["b1", "b2"]
        );
    }

    #[test]
    fn branch_stays_top_level_when_source_is_absent_or_elsewhere() {
        let list = vec![
            ses("orphan", Some("gone"), None),
            ses("main", None, None),
            ses("moved", Some("main"), Some("f1")),
            ses("cycle-a", Some("cycle-b"), None),
            ses("cycle-b", Some("cycle-a"), None),
        ];
        let (top, branches) = nest_branch_sessions(&list);
        assert!(branches.is_empty());
        assert_eq!(top.len(), list.len());
    }
}

pub(crate) fn bucket_sessions_by_date(
    list: &[SessionInfo],
) -> (Vec<SessionInfo>, Vec<SessionInfo>) {
    let now_ms = js_sys::Date::now();
    let (mut today, mut earlier) = (Vec::new(), Vec::new());
    for s in list {
        let ts_ms = if s.ts > 1_000_000_000_000 {
            s.ts as f64
        } else {
            s.ts as f64 * 1000.0
        };
        if s.ts > 0 && ts_ms >= now_ms - 86_400_000.0 {
            today.push(s.clone());
        } else {
            earlier.push(s.clone());
        }
    }
    (today, earlier)
}
