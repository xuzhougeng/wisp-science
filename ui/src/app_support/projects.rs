use super::*;

#[component]
pub(crate) fn ProjectsScreen(
    locale: RwSignal<Locale>,
    running: RwSignal<HashSet<String>>,
    approval_pending: ReadSignal<HashSet<String>>,
    sync_actions_available: ReadSignal<bool>,
    open_error: RwSignal<Option<String>>,
    on_open: Callback<String>,
    on_open_session: Callback<(String, String)>,
    on_open_artifact: Callback<(String, String, String)>,
    on_open_settings: Callback<()>,
    on_open_library: Callback<()>,
    on_open_demo: Callback<()>,
    on_open_scratch: Callback<()>,
    on_search: Callback<()>,
    on_export_project: Callback<(String, String)>,
    project_transfer: RwSignal<Option<ProjectTransferProgress>>,
    privacy_mode_active: RwSignal<bool>,
    privacy_hidden_project_ids: RwSignal<HashSet<String>>,
    // One-shot requests from the Windows titlebar File menu, which lives
    // outside this screen; the dialogs themselves are owned here.
    menu_new_project: RwSignal<bool>,
    menu_import_project: RwSignal<bool>,
) -> impl IntoView {
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let recent = create_rw_signal(Vec::<RecentSession>::new());
    let artifact_hits = create_rw_signal(Vec::<ArtifactInfo>::new());
    let search_open = create_rw_signal(false);
    let search_query = create_rw_signal(String::new());
    let search_active = create_rw_signal(0usize);
    let project_is_hidden = move |id: &str| {
        privacy_mode_active.get() && privacy_hidden_project_ids.with(|ids| ids.contains(id))
    };
    let demo_count = create_rw_signal(0usize);
    let creating = create_rw_signal(false);
    let new_name = create_rw_signal(String::new());
    let new_dir = create_rw_signal(String::new());
    let new_desc = create_rw_signal(String::new());
    let new_ctx = create_rw_signal(String::new());
    let import_options_open = create_rw_signal(false);
    let opening_in_place = create_rw_signal(false);
    let recovery_preview = create_rw_signal(None::<WorkspaceSessionRecoveryPreview>);
    let recovery_name = create_rw_signal(String::new());
    let recovery_busy = create_rw_signal(false);
    let recovery_error = create_rw_signal(None::<String>);
    // Off by default: pointing at an existing repo must not litter it. Checking
    // the box reveals the convention text, which doubles as a worked example of
    // what Agent Context is for.
    let new_layout = create_rw_signal(false);
    let syncing_projects = create_rw_signal(HashSet::<String>::new());
    let sync_notice = create_rw_signal(None::<(bool, String)>);
    let sync_conflict_project = create_rw_signal(None::<String>);
    // Pending project deletion, awaiting in-app confirmation. Native
    // `window.confirm()` is a no-op in this webview (wry's WKUIDelegate doesn't
    // implement the JS confirm panel), so it always returned false and the ✕
    // did nothing — use an in-app modal instead.
    let pending_delete = create_rw_signal(None::<String>);
    let confirm_delete_data = create_rw_signal(false);
    let settings_project_id = create_rw_signal(None::<String>);
    let settings_form = create_rw_signal(ProjectSettings::default());
    let settings_baseline = create_rw_signal(ProjectSettings::default());
    let settings_busy = create_rw_signal(false);
    let settings_confirm_context = create_rw_signal(false);
    let delete_data_countdown = create_rw_signal(0_u8);
    let delete_data_unlock_at = Rc::new(Cell::new(0_f64));

    // The destructive confirmation unlocks only after five full seconds. Keep
    // one owner-scoped timer and make it inert whenever that second layer is
    // closed, so reopening always starts from five again.
    let countdown_deadline = Rc::clone(&delete_data_unlock_at);
    let countdown_tick = Closure::wrap(Box::new(move || {
        if confirm_delete_data.get_untracked() {
            let milliseconds = (countdown_deadline.get() - js_sys::Date::now()).max(0.0);
            let remaining = (milliseconds / 1_000.0).ceil() as u8;
            if delete_data_countdown.get_untracked() != remaining {
                delete_data_countdown.set(remaining);
            }
        }
    }) as Box<dyn FnMut()>);
    let countdown_window = web_sys::window();
    let countdown_interval = countdown_window.as_ref().and_then(|window| {
        window
            .set_interval_with_callback_and_timeout_and_arguments_0(
                countdown_tick.as_ref().unchecked_ref(),
                100,
            )
            .ok()
    });
    on_cleanup(move || {
        if let (Some(window), Some(interval)) = (countdown_window, countdown_interval) {
            window.clear_interval_with_handle(interval);
        }
        drop(countdown_tick);
    });

    let reload = move || {
        spawn_local(async move {
            let v = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) {
                projects.set(list);
            }
            let r = invoke("list_recent_sessions", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<RecentSession>>(r) {
                recent.set(list);
            }
            let dm = invoke("list_demos", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DemoInfo>>(dm) {
                demo_count.set(list.len());
            }
        });
    };
    reload();

    // Refresh dashboard when a background turn starts/finishes or waits on approval.
    create_effect(move |_| {
        running.get();
        approval_pending.get();
        reload();
    });

    // Refresh the landing list when an import finishes while this screen is
    // still mounted. If the user navigated elsewhere during the background
    // import, the normal initial load refreshes it when they return.
    create_effect(move |_| {
        if project_transfer
            .get()
            .is_some_and(|transfer| transfer.direction == "import" && transfer.is_complete())
        {
            reload();
        }
    });

    create_effect(move |_| {
        if search_open.get() {
            search_query.get();
            refresh_artifact_search(search_query, artifact_hits);
            focus_element_soon("project-search-input");
        }
    });

    create_effect(move |_| {
        if creating.get() {
            focus_and_select_soon("new-project-name");
        }
    });

    create_effect(move |_| {
        if settings_project_id.get().is_some() && !settings_confirm_context.get() {
            focus_and_select_soon("project-home-settings-name");
        }
    });
    create_effect(move |_| {
        if recovery_preview.get().is_some() {
            focus_and_select_soon("workspace-recovery-name");
        }
    });

    create_effect(move |_| {
        search_open.get();
        search_query.get();
        search_active.set(0);
    });

    let search_count = move || {
        let q = search_query.get().trim().to_lowercase();
        let projects_n = projects
            .get()
            .into_iter()
            .filter(|p| {
                !project_is_hidden(&p.id) && contains_search(&q, &[&p.name, &p.description])
            })
            .take(HOME_SEARCH_PROJECT_LIMIT)
            .count();
        let artifacts_n = artifact_hits
            .get()
            .into_iter()
            .filter(|artifact| {
                !artifact
                    .project_id
                    .as_deref()
                    .is_some_and(project_is_hidden)
            })
            .take(HOME_SEARCH_ARTIFACT_LIMIT)
            .count();
        let sessions_n = recent
            .get()
            .into_iter()
            .filter(|s| !project_is_hidden(&s.project_id) && contains_search(&q, &[&s.title]))
            .take(HOME_SEARCH_SESSION_LIMIT)
            .count();
        projects_n + artifacts_n + sessions_n + 1
    };

    let run_search_action = Callback::new(move |idx: usize| {
        let q = search_query.get().trim().to_lowercase();
        let mut pos = 0usize;
        for p in projects
            .get()
            .into_iter()
            .filter(|p| {
                !project_is_hidden(&p.id) && contains_search(&q, &[&p.name, &p.description])
            })
            .take(HOME_SEARCH_PROJECT_LIMIT)
        {
            if pos == idx {
                search_open.set(false);
                on_open.call(p.id);
                return;
            }
            pos += 1;
        }
        for a in artifact_hits
            .get()
            .into_iter()
            .filter(|artifact| {
                !artifact
                    .project_id
                    .as_deref()
                    .is_some_and(project_is_hidden)
            })
            .take(HOME_SEARCH_ARTIFACT_LIMIT)
        {
            if pos == idx {
                search_open.set(false);
                let path = stored_artifact_path(a.location.as_deref().unwrap_or(&a.path));
                let kind = file_kind(&a.name)
                    .or_else(|| file_kind(&path))
                    .unwrap_or_else(|| {
                        if a.kind.starts_with("image/") {
                            "image"
                        } else if a.kind.contains("pdf") {
                            "pdf"
                        } else if a.kind.contains("csv") {
                            "csv"
                        } else {
                            "text"
                        }
                    })
                    .to_string();
                on_open_artifact.call((path, a.name, kind));
                return;
            }
            pos += 1;
        }
        for s in recent
            .get()
            .into_iter()
            .filter(|s| !project_is_hidden(&s.project_id) && contains_search(&q, &[&s.title]))
            .take(HOME_SEARCH_SESSION_LIMIT)
        {
            if pos == idx {
                search_open.set(false);
                on_open_session.call((s.project_id, s.id));
                return;
            }
            pos += 1;
        }
        if pos == idx {
            search_open.set(false);
            opening_in_place.set(false);
            creating.set(true);
        }
    });

    let choose_dir = move |_| {
        spawn_local(async move {
            let v = invoke("pick_directory", JsValue::UNDEFINED).await;
            if let Ok(Some(p)) = serde_wasm_bindgen::from_value::<Option<String>>(v) {
                new_dir.set(p);
            }
        })
    };

    // Seed the convention text on open so the checkbox's default is visible and
    // editable rather than applied behind the user's back.
    create_effect(move |_| {
        if creating.get() {
            let block = t(locale.get(), "projects.layout_context");
            new_ctx.update(|c| *c = apply_layout_block(c, &block, new_layout.get()));
        }
    });

    // Titlebar File menu requests (one-shot flags; reset before opening so a
    // repeated request retriggers the effect).
    create_effect(move |_| {
        if menu_new_project.get() {
            menu_new_project.set(false);
            creating.set(true);
        }
    });
    create_effect(move |_| {
        if menu_import_project.get() {
            menu_import_project.set(false);
            import_options_open.set(true);
        }
    });

    let submit = move |_| {
        let (n, d, desc, ctx) = (new_name.get(), new_dir.get(), new_desc.get(), new_ctx.get());
        let layout = new_layout.get();
        if n.trim().is_empty() || d.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "name": n, "workspaceDir": d, "description": desc, "agentContext": ctx,
                "standardLayout": layout,
            }))
            .unwrap();
            match invoke_checked("create_project", arg).await {
                Ok(value) => {
                    if let Ok(project) = serde_wasm_bindgen::from_value::<ProjectSummary>(value) {
                        new_name.set(String::new());
                        new_dir.set(String::new());
                        new_desc.set(String::new());
                        new_ctx.set(String::new());
                        new_layout.set(false);
                        creating.set(false);
                        opening_in_place.set(false);
                        on_open.call(project.id);
                    }
                }
                Err(error) => {
                    creating.set(false);
                    opening_in_place.set(false);
                    open_error.set(Some(localize_backend(
                        locale.get_untracked(),
                        &js_error_text(error),
                    )));
                }
            }
        });
    };

    let delete = Callback::new(move |(id, delete_data): (String, bool)| {
        spawn_local(async move {
            let arg =
                to_value(&serde_json::json!({ "id": id, "deleteData": delete_data })).unwrap();
            match invoke_checked("delete_project", arg).await {
                Ok(_) => {
                    let v = invoke("list_projects", JsValue::UNDEFINED).await;
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) {
                        projects.set(list);
                    }
                }
                Err(error) => {
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    open_error.set(Some(message));
                }
            }
        });
    });
    let delete_confirmed = delete; // used by the confirm modal below

    let save_home_settings = Callback::new(move |_: ()| {
        if settings_busy.get() {
            return;
        }
        let Some(id) = settings_project_id.get() else {
            return;
        };
        let form = settings_form.get();
        if form.name.trim().is_empty() {
            return;
        }
        let baseline = settings_baseline.get();
        if form.agent_context.trim() != baseline.agent_context.trim()
            && !settings_confirm_context.get()
        {
            settings_confirm_context.set(true);
            return;
        }
        settings_busy.set(true);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "id": id,
                "name": form.name,
                "description": form.description,
                "agentContext": form.agent_context,
            }))
            .unwrap();
            match invoke_checked("update_project", arg).await {
                Ok(_) => {
                    settings_busy.set(false);
                    settings_confirm_context.set(false);
                    settings_project_id.set(None);
                    reload();
                }
                Err(error) => {
                    settings_busy.set(false);
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    open_error.set(Some(message));
                }
            }
        });
    });

    let import_archive = Callback::new(move |_: ()| {
        import_options_open.set(false);
        if project_transfer
            .get_untracked()
            .is_some_and(|transfer| transfer.is_active())
        {
            return;
        }
        project_transfer.set(Some(ProjectTransferProgress::selecting("import", None)));
        open_error.set(None);
        spawn_local(async move {
            match invoke_checked("import_project", JsValue::UNDEFINED).await {
                Ok(value) => {
                    if let Ok(Some(project)) =
                        serde_wasm_bindgen::from_value::<Option<ProjectSummary>>(value)
                    {
                        project_transfer.set(Some(ProjectTransferProgress::complete(
                            "import",
                            Some(project.id),
                            Some(project.name.clone()),
                        )));
                    } else {
                        project_transfer.set(None);
                    }
                }
                Err(error) => {
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    project_transfer.set(Some(ProjectTransferProgress::failed(
                        "import", None, message,
                    )));
                }
            }
        });
    });

    let import_in_place = Callback::new(move |_: ()| {
        import_options_open.set(false);
        open_error.set(None);
        spawn_local(async move {
            let value = invoke("pick_directory", JsValue::UNDEFINED).await;
            let Ok(Some(path)) = serde_wasm_bindgen::from_value::<Option<String>>(value) else {
                return;
            };
            let name = path
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_string();
            new_name.set(name);
            new_dir.set(path);
            new_desc.set(String::new());
            new_ctx.set(String::new());
            new_layout.set(false);
            opening_in_place.set(true);
            creating.set(true);
        });
    });

    let recover_from_workspace = Callback::new(move |_: ()| {
        import_options_open.set(false);
        open_error.set(None);
        recovery_error.set(None);
        spawn_local(async move {
            let value = invoke("pick_directory", JsValue::UNDEFINED).await;
            let Ok(Some(path)) = serde_wasm_bindgen::from_value::<Option<String>>(value) else {
                return;
            };
            let args = to_value(&serde_json::json!({ "workspaceDir": path })).unwrap();
            match invoke_checked("preview_workspace_session_recovery", args).await {
                Ok(value) => {
                    if let Ok(preview) =
                        serde_wasm_bindgen::from_value::<WorkspaceSessionRecoveryPreview>(value)
                    {
                        recovery_name.set(preview.suggested_name.clone());
                        recovery_preview.set(Some(preview));
                    }
                }
                Err(error) => {
                    open_error.set(Some(localize_backend(
                        locale.get_untracked(),
                        &js_error_text(error),
                    )));
                }
            }
        });
    });

    let confirm_workspace_recovery = Callback::new(move |_: ()| {
        if recovery_busy.get_untracked() {
            return;
        }
        let Some(preview) = recovery_preview.get_untracked() else {
            return;
        };
        let name = recovery_name.get_untracked();
        if name.trim().is_empty() || preview.recoverable_session_count == 0 {
            return;
        }
        recovery_busy.set(true);
        recovery_error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "workspaceDir": preview.workspace_dir,
                "name": name,
            }))
            .unwrap();
            match invoke_checked("recover_workspace_sessions", args).await {
                Ok(value) => {
                    if let Ok(result) =
                        serde_wasm_bindgen::from_value::<WorkspaceSessionRecoveryResult>(value)
                    {
                        recovery_preview.set(None);
                        recovery_name.set(String::new());
                        recovery_busy.set(false);
                        recovery_error.set(None);
                        on_open.call(result.project_id);
                    } else {
                        recovery_busy.set(false);
                    }
                }
                Err(error) => {
                    recovery_busy.set(false);
                    recovery_error.set(Some(localize_backend(
                        locale.get_untracked(),
                        &js_error_text(error),
                    )));
                }
            }
        });
    });

    let resolve_sync_conflict = Callback::new(move |strategy: String| {
        let Some(id) = sync_conflict_project.get_untracked() else {
            return;
        };
        if syncing_projects.with_untracked(|ids| ids.contains(&id)) {
            return;
        }
        syncing_projects.update(|ids| {
            ids.insert(id.clone());
        });
        open_error.set(None);
        sync_notice.set(Some((
            true,
            t(locale.get_untracked(), "projects.sync.running").into(),
        )));
        spawn_local(async move {
            let args =
                to_value(&serde_json::json!({ "id": id.clone(), "strategy": strategy })).unwrap();
            match invoke_checked("resolve_project_sync", args).await {
                Ok(value) => {
                    if let Ok(result) = serde_wasm_bindgen::from_value::<ProjectSyncResult>(value) {
                        let loc = locale.get_untracked();
                        let text = if result.direction == "pull" {
                            tf(
                                loc,
                                "projects.sync.pulled",
                                &[("n", &result.downloaded_files.to_string())],
                            )
                        } else {
                            tf(
                                loc,
                                "projects.sync.pushed",
                                &[("n", &result.uploaded_files.to_string())],
                            )
                        };
                        sync_notice.set(Some((true, text)));
                    }
                    sync_conflict_project.set(None);
                    reload();
                }
                Err(error) => {
                    sync_notice.set(None);
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    open_error.set(Some(message));
                }
            }
            syncing_projects.update(|ids| {
                ids.remove(&id);
            });
        });
    });

    // Local Escape stack — ProjectsScreen owns its own modals, so the App
    // window listener cannot see `creating` / `pending_delete`. Opening a
    // project disposes this component, so the listener has to go with it:
    // window listeners outlive their owner and would keep reading the signals
    // below after they are gone.
    let escape_listener = window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        if ev.key() != "Escape" || ev.default_prevented() || ime_composing(ev) {
            return;
        }
        if confirm_delete_data.get() {
            ev.prevent_default();
            confirm_delete_data.set(false);
            delete_data_countdown.set(0);
            return;
        }
        if settings_confirm_context.get() {
            ev.prevent_default();
            settings_confirm_context.set(false);
            return;
        }
        if settings_project_id.get().is_some() {
            ev.prevent_default();
            settings_confirm_context.set(false);
            settings_project_id.set(None);
            return;
        }
        if recovery_preview.get().is_some() {
            ev.prevent_default();
            if !recovery_busy.get() {
                recovery_preview.set(None);
                recovery_name.set(String::new());
                recovery_error.set(None);
            }
            return;
        }
        if import_options_open.get() {
            ev.prevent_default();
            import_options_open.set(false);
            return;
        }
        if pending_delete.get().is_some() {
            ev.prevent_default();
            pending_delete.set(None);
            return;
        }
        if sync_conflict_project.get().is_some() {
            ev.prevent_default();
            sync_conflict_project.set(None);
            return;
        }
        if search_open.get() {
            ev.prevent_default();
            search_open.set(false);
            return;
        }
        if creating.get() {
            ev.prevent_default();
            creating.set(false);
            opening_in_place.set(false);
        }
    });
    on_cleanup(move || escape_listener.remove());

    view! {
        <div class="projects-screen" on:contextmenu=move |ev| {
            if crate::context_menu::uses_native_text_menu(&ev) {
                return;
            }
            ev.prevent_default();
        }>
            <div class="projects-head">
                <div class="projects-brand">
                    <span class="projects-brand-mark" aria-hidden="true"></span>
                    <div class="projects-title">"Wisp Science"</div>
                </div>
                <div class="projects-actions">
                    <button type="button" class="projects-icon-btn"
                        title=move || t(locale.get(), "sidebar.library")
                        aria-label=move || t(locale.get(), "sidebar.library")
                        on:click=move |_| on_open_library.call(())>
                        {compose_icon("star")}
                    </button>
                    <button type="button" class="projects-icon-btn"
                        title=move || t(locale.get(), "projects.search")
                        aria-label=move || t(locale.get(), "projects.search")
                        on:click=move |_| on_search.call(())>
                        {compose_icon("search")}
                    </button>
                    <button type="button" class="projects-icon-btn"
                        title=move || t(locale.get(), "sidebar.settings")
                        aria-label=move || t(locale.get(), "sidebar.settings")
                        on:click=move |_| on_open_settings.call(())>
                        {compose_icon("gear")}
                    </button>
                    <button type="button" class="btn-ghost projects-scratch"
                        on:click=move |_| on_open_scratch.call(())>
                        {move || t(locale.get(), "scratch.open")}
                    </button>
                    <button type="button" class="btn-ghost projects-import"
                        disabled=move || project_transfer.get().is_some_and(|transfer| transfer.is_active())
                        on:click=move |_| import_options_open.set(true)>
                        {compose_icon("upload")}<span>{move || t(locale.get(), "projects.import")}</span>
                    </button>
                    <button class="btn-primary" on:click=move |_| {
                        opening_in_place.set(false);
                        creating.set(true);
                    }>
                        <span class="new-plus">"+"</span>{move || t(locale.get(), "projects.new")}
                    </button>
                </div>
            </div>
            {move || open_error.get().map(|message| view! {
                <div class="project-open-error" role="alert">{message}</div>
            })}
            {move || search_open.get().then(|| view! {
                <div class="project-search-overlay" on:click=move |_| search_open.set(false)>
                    <div class="project-search-dialog" role="dialog" aria-label=move || t(locale.get(), "projects.search")
                        on:click=|ev| ev.stop_propagation()>
                        <div class="project-search-input">
                            {compose_icon("search")}
                            <input id="project-search-input" type="text" inputmode="search" autofocus=true
                                autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                                placeholder=move || t(locale.get(), "projects.search_ph")
                                prop:value=move || search_query.get()
                                on:input=move |ev| search_query.set(event_target_value(&ev))
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ime_composing(&ev) { return; }
                                    let key = ev.key();
                                    let last = search_count().saturating_sub(1);
                                    match key.as_str() {
                                        "Escape" => {
                                            ev.prevent_default();
                                            search_open.set(false);
                                        }
                                        "ArrowDown" => {
                                            ev.prevent_default();
                                            search_active.update(|i| *i = (*i + 1).min(last));
                                        }
                                        "ArrowUp" => {
                                            ev.prevent_default();
                                            search_active.update(|i| *i = i.saturating_sub(1));
                                        }
                                        "Enter" => {
                                            ev.prevent_default();
                                            run_search_action.call(search_active.get().min(last));
                                        }
                                        _ => {}
                                    }
                                } />
                        </div>
                        <div class="project-search-results">
                            {move || {
                                let loc = locale.get();
                                let q = search_query.get().trim().to_lowercase();
                                let mut idx = 0usize;

                                let project_start = idx;
                                let project_rows = projects
                                    .get()
                                    .into_iter()
                                    .filter(|p| !project_is_hidden(&p.id) && contains_search(&q, &[&p.name, &p.description]))
                                    .take(HOME_SEARCH_PROJECT_LIMIT)
                                    .map(|p| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let sessions = tf(loc, "projects.sessions_n", &[("n", &p.session_count.to_string())]);
                                        let when = format_relative_time(p.updated_at, loc);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                {compose_icon("folder")}
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{p.name.clone()}</span>
                                                    <span class="project-search-sub">
                                                        {sessions}{(!when.is_empty()).then(|| format!(" · {when}")).unwrap_or_default()}
                                                    </span>
                                                </span>
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_project_rows = idx > project_start;
                                let artifact_start = idx;
                                let artifact_rows = artifact_hits
                                    .get()
                                    .into_iter()
                                    .filter(|artifact| !artifact.project_id.as_deref().is_some_and(project_is_hidden))
                                    .take(HOME_SEARCH_ARTIFACT_LIMIT)
                                    .map(|a| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let badge = artifact_badge(&a.kind, &a.name);
                                        let when = format_relative_time(a.ts, loc);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                {compose_icon("doc")}
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{a.name.clone()}</span>
                                                    <span class="project-search-sub">
                                                        {a.location.as_deref().unwrap_or(&a.path).to_string()}{(!when.is_empty()).then(|| format!(" · {when}")).unwrap_or_default()}
                                                    </span>
                                                </span>
                                                <span class="project-search-badge">{badge}</span>
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_artifact_rows = idx > artifact_start;
                                let session_start = idx;
                                let session_rows = recent
                                    .get()
                                    .into_iter()
                                    .filter(|s| !project_is_hidden(&s.project_id) && contains_search(&q, &[&s.title]))
                                    .take(HOME_SEARCH_SESSION_LIMIT)
                                    .map(|s| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let status = SessionStatusKind::from_str(&s.status);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                {compose_icon("bubble")}
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{s.title.clone()}</span>
                                                    <span class="project-search-sub">{format_relative_time(s.ts, loc)}</span>
                                                </span>
                                                <SessionStatusBadge status=status locale=locale />
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_session_rows = idx > session_start;
                                view! {
                                    {has_project_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.title")}</div>
                                            {project_rows}
                                        </div>
                                    })}
                                    {has_artifact_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.search_artifacts")}</div>
                                            {artifact_rows}
                                        </div>
                                    })}
                                    {has_session_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.recent")}</div>
                                            {session_rows}
                                        </div>
                                    })}
                                }.into_view()
                            }}
                        </div>
                        <button type="button" class="project-search-new"
                            class:active=move || search_active.get() + 1 == search_count()
                            on:click=move |_| {
                                search_open.set(false);
                                opening_in_place.set(false);
                                creating.set(true);
                            }>
                            {compose_icon("plus")}
                            <span>{move || t(locale.get(), "projects.new")}</span>
                        </button>
                        <div class="project-search-foot">
                            <span><kbd>"↑↓"</kbd>{move || t(locale.get(), "projects.search_nav")}</span>
                            <span><kbd>"↵"</kbd>{move || t(locale.get(), "projects.search_open")}</span>
                            <span><kbd>"esc"</kbd>{move || t(locale.get(), "projects.search_close")}</span>
                        </div>
                    </div>
                </div>
            })}
            {move || import_options_open.get().then(|| view! {
                <div class="overlay" data-testid="project-import-options">
                    <div class="modal confirm-modal project-import-options-modal"
                        role="dialog" aria-modal="true"
                        aria-label=move || t(locale.get(), "projects.import")>
                        <h2>{move || t(locale.get(), "projects.import")}</h2>
                        <p class="project-import-options-hint">
                            {move || t(locale.get(), "projects.import_options_hint")}
                        </p>
                        <div class="project-import-options">
                            <button type="button" class="project-import-option"
                                on:click=move |_| import_in_place.call(())>
                                <strong>{move || t(locale.get(), "projects.import_in_place")}</strong>
                                <span>{move || t(locale.get(), "projects.import_in_place_hint")}</span>
                            </button>
                            <button type="button" class="project-import-option"
                                on:click=move |_| import_archive.call(())>
                                <strong>{move || t(locale.get(), "projects.import_zip")}</strong>
                                <span>{move || t(locale.get(), "projects.import_zip_hint")}</span>
                            </button>
                            <button type="button" class="project-import-option"
                                on:click=move |_| recover_from_workspace.call(())>
                                <strong>{move || t(locale.get(), "projects.recover_workspace")}</strong>
                                <span>{move || t(locale.get(), "projects.recover_workspace_hint")}</span>
                            </button>
                        </div>
                        <div class="row">
                            <button type="button" on:click=move |_| import_options_open.set(false)>
                                {move || t(locale.get(), "settings.cancel")}
                            </button>
                        </div>
                    </div>
                </div>
            })}
            {move || recovery_preview.get().map(|preview| {
                let range = match (preview.earliest_message_at, preview.latest_message_at) {
                    (Some(start), Some(end)) => tf(
                        locale.get(),
                        "projects.recover_range",
                        &[
                            ("start", &format_message_datetime(start, locale.get())),
                            ("end", &format_message_datetime(end, locale.get())),
                        ],
                    ),
                    _ => t(locale.get(), "projects.recover_range_unknown").into(),
                };
                view! {
                    <div class="overlay" data-testid="workspace-session-recovery">
                        <div class="modal proj-settings-modal workspace-recovery-modal"
                            role="dialog" aria-modal="true"
                            aria-label=move || t(locale.get(), "projects.recover_title")>
                            <div class="ps-head">
                                <h2>{move || t(locale.get(), "projects.recover_title")}</h2>
                                <button type="button" class="ps-close"
                                    title=move || t(locale.get(), "projects.cancel")
                                    disabled=move || recovery_busy.get()
                                    on:click=move |_| {
                                        recovery_preview.set(None);
                                        recovery_name.set(String::new());
                                        recovery_error.set(None);
                                    }>{compose_icon("close")}</button>
                            </div>
                            <p class="project-in-place-hint">
                                {move || t(locale.get(), "projects.recover_notice")}
                            </p>
                            <label>
                                {move || t(locale.get(), "proj_settings.name")}
                                <input id="workspace-recovery-name"
                                    prop:value=move || recovery_name.get()
                                    on:input=move |event| recovery_name.set(event_target_value(&event)) />
                            </label>
                            <div class="workspace-recovery-path" title=preview.workspace_dir.clone()>
                                {preview.workspace_dir.clone()}
                            </div>
                            <div class="workspace-recovery-stats">
                                <div><strong>{preview.recoverable_session_count}</strong><span>{move || t(locale.get(), "projects.recover_sessions")}</span></div>
                                <div><strong>{preview.message_count}</strong><span>{move || t(locale.get(), "projects.recover_messages")}</span></div>
                                <div><strong>{preview.valid_archive_count}</strong><span>{move || t(locale.get(), "projects.recover_archives")}</span></div>
                            </div>
                            <p class="workspace-recovery-range">{range}</p>
                            {(preview.invalid_archive_count > 0 || preview.duplicate_archive_count > 0).then(|| view! {
                                <p class="workspace-recovery-warning">
                                    {tf(
                                        locale.get(),
                                        "projects.recover_skipped",
                                        &[
                                            ("invalid", &preview.invalid_archive_count.to_string()),
                                            ("duplicate", &preview.duplicate_archive_count.to_string()),
                                        ],
                                    )}
                                </p>
                            })}
                            {(preview.recoverable_session_count == 0).then(|| view! {
                                <div class="workspace-recovery-error" role="alert">
                                    {move || t(locale.get(), "projects.recover_empty")}
                                </div>
                            })}
                            {move || recovery_error.get().map(|message| view! {
                                <div class="workspace-recovery-error" role="alert">{message}</div>
                            })}
                            <div class="row">
                                <button type="button"
                                    disabled=move || recovery_busy.get()
                                    on:click=move |_| {
                                        recovery_preview.set(None);
                                        recovery_name.set(String::new());
                                        recovery_error.set(None);
                                    }>{move || t(locale.get(), "projects.cancel")}</button>
                                <button type="button" class="primary"
                                    disabled=move || recovery_busy.get()
                                        || recovery_name.get().trim().is_empty()
                                        || preview.recoverable_session_count == 0
                                    on:click=move |_| confirm_workspace_recovery.call(())>
                                    {move || t(
                                        locale.get(),
                                        if recovery_busy.get() {
                                            "projects.recovering"
                                        } else {
                                            "projects.recover_action"
                                        },
                                    )}
                                </button>
                            </div>
                        </div>
                    </div>
                }
            })}
            {move || creating.get().then(|| view! {
                <div class="overlay">
                    <div class="modal proj-settings-modal" role="dialog" aria-modal="true">
                        <div class="ps-head">
                            <h2>{move || t(
                                locale.get(),
                                if opening_in_place.get() {
                                    "projects.open_folder_title"
                                } else {
                                    "projects.new"
                                },
                            )}</h2>
                            <button type="button" class="ps-close"
                                title=move || t(locale.get(), "projects.cancel")
                                on:click=move |_| {
                                    creating.set(false);
                                    opening_in_place.set(false);
                                }>{compose_icon("close")}</button>
                        </div>
                        {move || opening_in_place.get().then(|| view! {
                            <p class="project-in-place-hint">
                                {move || t(locale.get(), "projects.open_folder_hint")}
                            </p>
                        })}
                        <label>
                            {move || t(locale.get(), "proj_settings.name")}
                            <input id="new-project-name" autofocus=true
                                placeholder=move || t(locale.get(), "projects.name_ph")
                                prop:value=move || new_name.get()
                                on:input=move |e| new_name.set(event_target_value(&e)) />
                        </label>
                        <label>
                            {move || t(locale.get(), "projects.directory")}
                            <div class="pn-dir">
                                <button type="button" class="btn-ghost" on:click=choose_dir>
                                    {move || t(locale.get(), "projects.choose_dir")}</button>
                                <span class="path">{move || new_dir.get()}</span>
                            </div>
                        </label>
                        <label>
                            {move || t(locale.get(), "proj_settings.description")}
                            <span class="ps-hint">{move || t(locale.get(), "proj_settings.description_hint")}</span>
                            <textarea class="ps-textarea" rows="2"
                                prop:value=move || new_desc.get()
                                on:input=move |ev| new_desc.set(event_target_value(&ev))></textarea>
                        </label>
                        {move || (!opening_in_place.get()).then(|| view! {
                            <label class="pn-layout">
                                <span class="toggle">
                                    <input type="checkbox" prop:checked=move || new_layout.get()
                                        on:change=move |ev| new_layout.set(event_target_checked(&ev)) />
                                    <span class="toggle-track" aria-hidden="true"></span>
                                </span>
                                <span>
                                    {move || t(locale.get(), "projects.standard_layout")}
                                    <span class="ps-hint">{move || t(locale.get(), "projects.standard_layout_hint")}</span>
                                </span>
                            </label>
                        })}
                        <label>
                            {move || t(locale.get(), "proj_settings.agent_context")}
                            <span class="ps-hint">{move || t(locale.get(), "proj_settings.agent_context_hint")}</span>
                            <textarea class="ps-textarea ps-ctx" rows="8"
                                prop:value=move || new_ctx.get()
                                on:input=move |ev| new_ctx.set(event_target_value(&ev))></textarea>
                        </label>
                        <div class="row">
                            <button type="button" on:click=move |_| {
                                creating.set(false);
                                opening_in_place.set(false);
                            }>
                                {move || t(locale.get(), "projects.cancel")}</button>
                            <button type="button" class="primary"
                                disabled=move || new_name.get().trim().is_empty() || new_dir.get().trim().is_empty()
                                on:click=submit>{move || t(
                                    locale.get(),
                                    if opening_in_place.get() {
                                        "projects.open_folder_action"
                                    } else {
                                        "projects.create"
                                    },
                                )}</button>
                        </div>
                    </div>
                </div>
            })}
            {move || settings_project_id.get().map(|_| {
                view! {
                    <div class="overlay" data-testid="project-home-settings">
                        <div class="modal proj-settings-modal" role="dialog" aria-modal="true"
                            aria-label=move || t(locale.get(), "proj_settings.title")>
                            <div class="ps-head">
                                <h2>{move || t(locale.get(), "proj_settings.title")}</h2>
                                <button type="button" class="ps-close"
                                    title=move || t(locale.get(), "settings.cancel")
                                    on:click=move |_| {
                                        settings_confirm_context.set(false);
                                        settings_project_id.set(None);
                                    }>{compose_icon("close")}</button>
                            </div>
                            <label>
                                <span class="ps-label">{move || t(locale.get(), "proj_settings.name")}</span>
                                <input id="project-home-settings-name" data-testid="project-home-settings-name"
                                    prop:value=move || settings_form.get().name
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        settings_form.update(|s| s.name = v);
                                    } />
                            </label>
                            <label>
                                <span class="ps-label">{move || t(locale.get(), "proj_settings.description")}</span>
                                <span class="ps-hint">{move || t(locale.get(), "proj_settings.description_hint")}</span>
                                <textarea class="ps-textarea" rows="2"
                                    prop:value=move || settings_form.get().description
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        settings_form.update(|s| s.description = v);
                                    }></textarea>
                            </label>
                            <label>
                                <span class="ps-label">{move || t(locale.get(), "proj_settings.agent_context")}</span>
                                <span class="ps-hint">{move || t(locale.get(), "proj_settings.agent_context_hint")}</span>
                                <textarea class="ps-textarea ps-ctx" rows="8"
                                    prop:value=move || settings_form.get().agent_context
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        settings_form.update(|s| s.agent_context = v);
                                    }></textarea>
                            </label>
                            <div class="row">
                                <button type="button" disabled=move || settings_busy.get()
                                    on:click=move |_| {
                                        settings_confirm_context.set(false);
                                        settings_project_id.set(None);
                                    }>{move || t(locale.get(), "settings.cancel")}</button>
                                <button type="button" class="primary" data-testid="save-project-home-settings"
                                    disabled=move || settings_busy.get() || settings_form.get().name.trim().is_empty()
                                    on:click=move |_| save_home_settings.call(())>
                                    {move || t(locale.get(), "settings.save")}</button>
                            </div>
                        </div>
                    </div>
                }
            })}
            {move || settings_confirm_context.get().then(|| view! {
                <div class="overlay" data-testid="project-home-settings-confirm">
                    <div class="modal confirm-modal" role="dialog" aria-modal="true"
                        aria-label=move || t(locale.get(), "proj_settings.agent_context_confirm_action")>
                        <h2>{move || t(locale.get(), "proj_settings.agent_context_confirm_action")}</h2>
                        <div class="hint">{move || t(locale.get(), "proj_settings.agent_context_confirm")}</div>
                        <div class="row">
                            <button type="button" on:click=move |_| settings_confirm_context.set(false)>
                                {move || t(locale.get(), "settings.cancel")}</button>
                            <button type="button" class="primary"
                                on:click=move |_| save_home_settings.call(())>
                                {move || t(locale.get(), "proj_settings.agent_context_confirm_action")}
                            </button>
                        </div>
                    </div>
                </div>
            })}
            <div class="projects-cols">
                <div class="projects-col">
                    <h2>{move || t(locale.get(), "projects.title")}</h2>
                    <button type="button" class="proj-card proj-example" on:click=move |_| on_open_demo.call(())>
                        <div>
                            <div class="pc-name">
                                {move || t(locale.get(), "projects.example")}
                                <span class="pc-tag">{move || t(locale.get(), "projects.example_tag")}</span>
                            </div>
                            <div class="pc-meta">{move || tf(locale.get(), "projects.sessions_n", &[("n", &demo_count.get().to_string())])}</div>
                        </div>
                    </button>
                    {move || {
                        let loc = locale.get();
                        let list = projects
                            .get()
                            .into_iter()
                            .filter(|project| !project_is_hidden(&project.id))
                            .collect::<Vec<_>>();
                        let show_sync_actions = sync_actions_available.get();
                        if list.is_empty() && !creating.get() {
                            return view! {}.into_view();
                        }
                        list.into_iter().map(|p| {
                            let id_open = p.id.clone();
                            let id_open_locked = p.id.clone();
                            let id_card_locked = p.id.clone();
                            let id_badge_locked = p.id.clone();
                            let id_del = p.id.clone();
                            let id_del_locked = p.id.clone();
                            let id_win = p.id.clone();
                            let id_win_locked = p.id.clone();
                            let id_export = p.id.clone();
                            let id_settings = p.id.clone();
                            let id_settings_locked = p.id.clone();
                            let workspace_export = p.workspace_dir.clone();
                            let workspace_path = p.workspace_dir.clone();
                            let id_sync = p.id.clone();
                            let id_sync_disabled = p.id.clone();
                            let id_sync_locked = p.id.clone();
                            let id_code = p.id.clone();
                            let meta = tf(loc, "projects.sessions_n", &[("n", &p.session_count.to_string())]);
                            let artifacts_meta = tf(loc, "projects.artifacts_n", &[("n", &p.artifact_count.to_string())]);
                            let active = p.running_count + p.needs_you_count;
                            let dot_class = if p.running_count > 0 { "running" } else { "ready" };
                            let when = format_relative_time(p.updated_at, loc);
                            let sync_when = p.last_synced_at
                                .map(|timestamp| format_relative_time(timestamp, loc))
                                .filter(|value| !value.is_empty());
                            let sync_label = if p.sync_configured {
                                Some(sync_when.as_deref().map_or_else(
                                    || t(loc, "projects.sync.enabled").into(),
                                    |when| tf(loc, "projects.sync.last", &[("when", when)]),
                                ))
                            } else {
                                None
                            };
                            view! {
                                <div class="proj-card"
                                    class:project-exporting=move || project_transfer.get().is_some_and(|transfer| transfer.is_exporting_project(&id_card_locked))>
                                    <button type="button" class="proj-card-main"
                                        disabled=move || project_transfer.get().is_some_and(|transfer| transfer.is_exporting_project(&id_open_locked))
                                        on:click=move |_| on_open.call(id_open.clone())>
                                    <div class="pc-main">
                                        <div class="pc-name-row">
                                            <div class="pc-name">{p.name.clone()}</div>
                                            {move || project_transfer.get()
                                                .is_some_and(|transfer| transfer.is_exporting_project(&id_badge_locked))
                                                .then(|| view! {
                                                    <span class="pc-transfer-lock">
                                                        {t(locale.get(), "projects.transfer.export_locked_badge")}
                                                    </span>
                                                })}
                                            {(active > 0).then(|| view! {
                                                <span class=format!("pc-dot {dot_class}")>
                                                    <span class="pc-dot-mark"></span>
                                                    <span class="pc-dot-n">{active}</span>
                                                </span>
                                            })}
                                            {(!when.is_empty()).then(|| view! { <span class="pc-when">{when.clone()}</span> })}
                                        </div>
                                        {(!workspace_path.trim().is_empty()).then(|| view! {
                                            <div class="pc-path" title=workspace_path.clone()>{workspace_path.clone()}</div>
                                        })}
                                        <div class="pc-meta-row">
                                            <span class="pc-meta">{meta}</span>
                                            <span class="pc-meta">{artifacts_meta}</span>
                                            {sync_label.clone().map(|label| view! { <span class="pc-sync-state">{label}</span> })}
                                        </div>
                                    </div>
                                    </button>
                                    <div class="pc-actions">
                                    <button type="button" class="pc-settings" data-testid="project-card-settings"
                                        title=t(loc, "projects.settings")
                                        aria-label=t(loc, "projects.settings")
                                        disabled=move || project_transfer.get().is_some_and(|transfer| transfer.is_exporting_project(&id_settings_locked))
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            let id = id_settings.clone();
                                            settings_confirm_context.set(false);
                                            settings_busy.set(false);
                                            open_error.set(None);
                                            spawn_local(async move {
                                                let arg = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                match invoke_checked("get_project_settings", arg).await {
                                                    Ok(value) => {
                                                        if let Ok(settings) = serde_wasm_bindgen::from_value::<ProjectSettings>(value) {
                                                            settings_baseline.set(settings.clone());
                                                            settings_form.set(settings);
                                                            settings_project_id.set(Some(id));
                                                        }
                                                    }
                                                    Err(error) => {
                                                        let message = localize_backend(
                                                            locale.get_untracked(),
                                                            &js_error_text(error),
                                                        );
                                                        open_error.set(Some(message));
                                                    }
                                                }
                                            });
                                        }>{compose_icon("gear")}</button>
                                    {show_sync_actions.then(|| view! {
                                        <button class="pc-sync" title=t(loc, "projects.sync.now")
                                            aria-label=t(loc, "projects.sync.now")
                                            disabled=move || syncing_projects.with(|ids| ids.contains(&id_sync_disabled))
                                                || project_transfer.get().is_some_and(|transfer| transfer.is_exporting_project(&id_sync_locked))
                                            on:click=move |e| {
                                                e.stop_propagation();
                                                let id = id_sync.clone();
                                                if syncing_projects.with(|ids| ids.contains(&id)) { return; }
                                                syncing_projects.update(|ids| { ids.insert(id.clone()); });
                                                sync_notice.set(Some((true, t(locale.get_untracked(), "projects.sync.running").into())));
                                                open_error.set(None);
                                                spawn_local(async move {
                                                    let args = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                    match invoke_checked("sync_project", args).await {
                                                        Ok(value) => {
                                                            if let Ok(result) = serde_wasm_bindgen::from_value::<ProjectSyncResult>(value) {
                                                                let loc = locale.get_untracked();
                                                                let text = match result.direction.as_str() {
                                                                    "push" => tf(loc, "projects.sync.pushed", &[("n", &result.uploaded_files.to_string())]),
                                                                    "pull" => tf(loc, "projects.sync.pulled", &[("n", &result.downloaded_files.to_string())]),
                                                                    _ => t(loc, "projects.sync.current").into(),
                                                                };
                                                                let text = if result.skipped_paths.is_empty() {
                                                                    text
                                                                } else {
                                                                    format!("{text} {}", tf(loc, "projects.sync.skipped", &[("n", &result.skipped_paths.len().to_string())]))
                                                                };
                                                                sync_notice.set(Some((true, text)));
                                                            }
                                                            reload();
                                                        }
                                                        Err(error) => {
                                                            sync_notice.set(None);
                                                            let raw = js_error_text(error);
                                                            if raw.contains("Sync conflict") {
                                                                sync_conflict_project.set(Some(id.clone()));
                                                            } else {
                                                                let message = localize_backend(locale.get_untracked(), &raw);
                                                                open_error.set(Some(message));
                                                            }
                                                        }
                                                    }
                                                    syncing_projects.update(|ids| { ids.remove(&id); });
                                                });
                                            }>{compose_icon("sync")}</button>
                                        <button class="pc-sync-code" title=t(loc, "projects.sync.copy_code")
                                            aria-label=t(loc, "projects.sync.copy_code")
                                            on:click=move |e| {
                                                e.stop_propagation();
                                                let id = id_code.clone();
                                                open_error.set(None);
                                                spawn_local(async move {
                                                    let args = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                    match invoke_checked("project_sync_code", args).await {
                                                        Ok(value) => {
                                                            if let Ok(code) = serde_wasm_bindgen::from_value::<String>(value) {
                                                                copy_text(code);
                                                                sync_notice.set(Some((true, t(locale.get_untracked(), "projects.sync.code_copied").into())));
                                                            }
                                                        }
                                                        Err(error) => {
                                                            let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                                                            open_error.set(Some(message));
                                                        }
                                                    }
                                                });
                                            }>{compose_icon("link")}</button>
                                    })}
                                    <button class="pc-export" title=t(loc, "projects.export")
                                        aria-label=t(loc, "projects.export")
                                        disabled=move || project_transfer.get().is_some_and(|transfer| transfer.is_active())
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            if project_transfer.get_untracked().is_some_and(|transfer| transfer.is_active()) { return; }
                                            open_error.set(None);
                                            on_export_project.call((id_export.clone(), workspace_export.clone()));
                                        }>{compose_icon("download")}</button>
                                    <button class="pc-window" title=t(loc, "projects.new_window")
                                        disabled=move || project_transfer.get().is_some_and(|transfer| transfer.is_exporting_project(&id_win_locked))
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            let id = id_win.clone();
                                            spawn_local(async move {
                                                let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                let _ = invoke("open_project_window", arg).await;
                                            });
                                        }>{compose_icon("expand")}</button>
                                    <button class="pc-del" title=t(loc, "projects.delete")
                                        disabled=move || project_transfer.get().is_some_and(|transfer| transfer.is_exporting_project(&id_del_locked))
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            confirm_delete_data.set(false);
                                            delete_data_countdown.set(0);
                                            pending_delete.set(Some(id_del.clone()));
                                        }>{compose_icon("close")}</button>
                                    </div>
                                </div>
                            }
                        }).collect_view()
                    }}
                </div>
                <div class="projects-col">
                    <h2>{move || t(locale.get(), "projects.recent")}</h2>
                    {move || recent.get().into_iter().filter(|session| {
                        !project_is_hidden(&session.project_id)
                    }).map(|s| {
                        let (pid, sid) = (s.project_id.clone(), s.id.clone());
                        let transfer_pid = s.project_id.clone();
                        let status = SessionStatusKind::from_str(&s.status);
                        view! {
                            <button type="button" class="proj-card proj-recent" data-testid="recent-session-card"
                                disabled=move || project_transfer.get().is_some_and(|transfer| transfer.is_exporting_project(&transfer_pid))
                                on:click=move |_| on_open_session.call((pid.clone(), sid.clone()))>
                                <div class="pc-main">
                                    <div class="pc-name-row">
                                        <div class="pc-name">{s.title.clone()}</div>
                                        <SessionStatusBadge status=status locale=locale />
                                    </div>
                                </div>
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>
            <div class="projects-footer">
                <span>{move || t(locale.get(), "projects.star_hint")}</span>
                <button type="button" class="projects-star-link"
                    on:click=move |_| open_external_url("https://github.com/xuzhougeng/wisp-science".into())>
                    {move || t(locale.get(), "projects.star_link")}
                </button>
            </div>
            {move || sync_notice.get().map(|(ok, text)| view! {
                <div class="projects-sync-notice" class:ok=move || ok>{text}</div>
            })}
            {move || sync_conflict_project.get().map(|_| {
                let use_remote = resolve_sync_conflict;
                let use_local = resolve_sync_conflict;
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal project-sync-conflict-modal" role="dialog"
                            aria-label=move || t(locale.get(), "projects.sync.conflict_title")>
                            <h2>{move || t(locale.get(), "projects.sync.conflict_title")}</h2>
                            <p class="hint">{move || t(locale.get(), "projects.sync.conflict_hint")}</p>
                            <p class="hint">{move || t(locale.get(), "projects.sync.conflict_backup")}</p>
                            <div class="row project-sync-conflict-actions">
                                <button type="button" on:click=move |_| sync_conflict_project.set(None)>
                                    {move || t(locale.get(), "projects.cancel")}</button>
                                <button type="button" on:click=move |_| use_remote.call("remote".into())>
                                    {move || t(locale.get(), "projects.sync.use_remote")}</button>
                                <button type="button" class="primary" on:click=move |_| use_local.call("local".into())>
                                    {move || t(locale.get(), "projects.sync.use_local")}</button>
                            </div>
                        </div>
                    </div>
                }
            })}
            {move || {
                let delete_data_unlock_at = Rc::clone(&delete_data_unlock_at);
                pending_delete.get().map(|id| {
                if confirm_delete_data.get() {
                    let confirm_del = delete_confirmed;
                    let delete_id = id.clone();
                    let workspace_dir = projects
                        .get()
                        .into_iter()
                        .find(|project| project.id == id)
                        .map(|project| project.workspace_dir)
                        .unwrap_or_default();
                    view! {
                        <div class="overlay project-delete-data-overlay">
                            <div class="modal confirm-modal project-delete-data-modal"
                                role="alertdialog" aria-modal="true"
                                aria-label=move || t(locale.get(), "projects.delete_data_title")>
                                <h2>{move || t(locale.get(), "projects.delete_data_title")}</h2>
                                <div class="hint project-delete-warning">
                                    {move || t(locale.get(), "projects.delete_data_warning")}
                                </div>
                                <code class="project-delete-path">{move || tf(
                                    locale.get(),
                                    "projects.delete_data_path",
                                    &[("path", &workspace_dir)],
                                )}</code>
                                <div class="row">
                                    <button type="button" on:click=move |_| {
                                        confirm_delete_data.set(false);
                                        delete_data_countdown.set(0);
                                    }>{move || t(locale.get(), "settings.back")}</button>
                                    <button type="button" class="primary danger"
                                        aria-live="polite"
                                        disabled=move || delete_data_countdown.get() != 0
                                        on:click=move |_| {
                                            if delete_data_countdown.get_untracked() == 0 {
                                                confirm_delete_data.set(false);
                                                delete_data_countdown.set(0);
                                                pending_delete.set(None);
                                                confirm_del.call((delete_id.clone(), true));
                                            }
                                        }>{move || {
                                            let remaining = delete_data_countdown.get();
                                            if remaining > 0 {
                                                tf(
                                                    locale.get(),
                                                    "projects.delete_data_countdown",
                                                    &[("n", &remaining.to_string())],
                                                )
                                            } else {
                                                t(locale.get(), "projects.delete_data_confirm").into()
                                            }
                                        }}</button>
                                </div>
                            </div>
                        </div>
                    }.into_view()
                } else {
                    let confirm_del = delete_confirmed;
                    let remove_id = id.clone();
                    view! {
                        <div class="overlay">
                            <div class="modal confirm-modal project-delete-choice-modal"
                                role="dialog" aria-modal="true"
                                aria-label=move || t(locale.get(), "confirm.title")>
                                <h2>{move || t(locale.get(), "confirm.title")}</h2>
                                <div class="hint">{move || t(locale.get(), "projects.delete_confirm")}</div>
                                <div class="row">
                                    <button type="button" on:click=move |_| pending_delete.set(None)>
                                        {move || t(locale.get(), "settings.cancel")}</button>
                                    <button type="button" class="primary" on:click=move |_| {
                                        pending_delete.set(None);
                                        confirm_del.call((remove_id.clone(), false));
                                    }>{move || t(locale.get(), "projects.remove_only")}</button>
                                    <button type="button" class="delete-with-data" on:click=move |_| {
                                        delete_data_unlock_at.set(js_sys::Date::now() + 5_000.0);
                                        delete_data_countdown.set(5);
                                        confirm_delete_data.set(true);
                                    }>{move || t(locale.get(), "projects.remove_with_data")}</button>
                                </div>
                            </div>
                        </div>
                    }.into_view()
                }
            })}}
        </div>
    }
}
