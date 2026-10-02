use super::*;

const CLI_IMPORT_PAGE_SIZE: usize = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionImportProvider {
    Codex,
    Claude,
}

impl SessionImportProvider {
    fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn list_command(self) -> &'static str {
        match self {
            Self::Codex => "list_codex_sessions",
            Self::Claude => "list_claude_sessions",
        }
    }

    fn import_command(self) -> &'static str {
        match self {
            Self::Codex => "import_codex_sessions",
            Self::Claude => "import_claude_sessions",
        }
    }

    fn preview_command(self) -> &'static str {
        match self {
            Self::Codex => "preview_codex_session",
            Self::Claude => "preview_claude_session",
        }
    }

    fn title_key(self) -> &'static str {
        match self {
            Self::Codex => "codex.title",
            Self::Claude => "claude.title",
        }
    }

    fn hint_key(self) -> &'static str {
        match self {
            Self::Codex => "codex.hint",
            Self::Claude => "claude.hint",
        }
    }

    fn empty_key(self) -> &'static str {
        match self {
            Self::Codex => "codex.empty",
            Self::Claude => "claude.empty",
        }
    }

    fn summary_key(self, failed: bool) -> &'static str {
        match (self, failed) {
            (Self::Codex, false) => "codex.summary",
            (Self::Codex, true) => "codex.summary_failed",
            (Self::Claude, false) => "claude.summary",
            (Self::Claude, true) => "claude.summary_failed",
        }
    }
}

/// Lists cached Codex CLI or Claude Code conversations and imports them into
/// the active project. Explicit refresh is the only operation that rescans a
/// source; importing updates the rows already shown by the modal.
#[component]
pub(crate) fn SessionImportModal(
    locale: RwSignal<Locale>,
    open: RwSignal<Option<SessionImportProvider>>,
    on_imported: Callback<()>,
) -> impl IntoView {
    let items = create_rw_signal(Vec::<ExternalSessionInfo>::new());
    let source_contexts = create_rw_signal(Vec::<ExecutionContext>::new());
    let selected_context_id = create_rw_signal("local".to_string());
    let query = create_rw_signal(String::new());
    let scan_error = create_rw_signal(None::<String>);
    let scan_epoch = create_rw_signal(0_u64);
    let page = create_rw_signal(0_usize);
    let loading = create_rw_signal(false);
    let importing = create_rw_signal(false);
    let import_progress = create_rw_signal(None::<(usize, usize)>);
    let import_notice = create_rw_signal(None::<(String, bool)>);
    let expanded_path = create_rw_signal(None::<String>);
    let preview_loading = create_rw_signal(None::<String>);
    let previews = create_rw_signal(HashMap::<
        String,
        Result<Vec<ExternalSessionPreviewLine>, String>,
    >::new());

    let refresh = move |provider: SessionImportProvider, context_id: String, force: bool| {
        scan_epoch.update(|epoch| *epoch += 1);
        let epoch = scan_epoch.get_untracked();
        page.set(0);
        loading.set(true);
        scan_error.set(None);
        items.set(vec![]);
        expanded_path.set(None);
        preview_loading.set(None);
        previews.set(HashMap::new());
        import_progress.set(None);
        import_notice.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "contextId": context_id.clone(),
                "refresh": force,
            }))
            .unwrap();
            let result = invoke_checked(provider.list_command(), args).await;
            if scan_epoch.get_untracked() != epoch
                || selected_context_id.get_untracked() != context_id
                || open.get_untracked() != Some(provider)
            {
                return;
            }
            match result {
                Ok(value) => items.set(
                    serde_wasm_bindgen::from_value::<Vec<ExternalSessionInfo>>(value)
                        .unwrap_or_default(),
                ),
                Err(error) => scan_error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                ))),
            }
            loading.set(false);
        });
    };
    create_effect(move |_| {
        if let Some(provider) = open.get() {
            selected_context_id.set("local".into());
            query.set(String::new());
            source_contexts.set(vec![]);
            refresh(provider, "local".into(), false);
            spawn_local(async move {
                let value = invoke("list_execution_contexts", JsValue::UNDEFINED).await;
                if let Ok(contexts) = serde_wasm_bindgen::from_value::<Vec<ExecutionContext>>(value)
                {
                    source_contexts.set(contexts);
                }
            });
        }
    });

    let load_preview = move |provider: SessionImportProvider, path: String| {
        if expanded_path.get_untracked().as_deref() == Some(path.as_str()) {
            expanded_path.set(None);
            return;
        }
        expanded_path.set(Some(path.clone()));
        if previews.with_untracked(|cached| cached.contains_key(&path)) {
            return;
        }
        let context_id = selected_context_id.get_untracked();
        preview_loading.set(Some(path.clone()));
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "path": path.clone(),
                "contextId": context_id.clone(),
            }))
            .unwrap();
            let result = match invoke_checked(provider.preview_command(), args).await {
                Ok(value) => {
                    serde_wasm_bindgen::from_value::<Vec<ExternalSessionPreviewLine>>(value)
                        .map_err(|error| error.to_string())
                }
                Err(error) => Err(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                )),
            };
            if selected_context_id.get_untracked() == context_id
                && open.get_untracked() == Some(provider)
            {
                previews.update(|cached| {
                    cached.insert(path.clone(), result);
                });
                if preview_loading.get_untracked().as_deref() == Some(path.as_str()) {
                    preview_loading.set(None);
                }
            }
        });
    };

    let import_paths = move |provider: SessionImportProvider, paths: Vec<String>| {
        if paths.is_empty() || importing.get_untracked() {
            return;
        }
        let context_id = selected_context_id.get_untracked();
        let total = paths.len();
        importing.set(true);
        import_progress.set(Some((0, total)));
        import_notice.set(None);
        spawn_local(async move {
            let mut summary = ExternalImportSummary::default();
            for (index, path) in paths.into_iter().enumerate() {
                let args = to_value(&serde_json::json!({
                    "paths": [path],
                    "contextId": context_id.clone(),
                }))
                .unwrap();
                match invoke_checked(provider.import_command(), args).await {
                    Ok(value) => {
                        let result = serde_wasm_bindgen::from_value::<ExternalImportSummary>(value)
                            .unwrap_or_default();
                        summary.imported += result.imported;
                        summary.updated += result.updated;
                        summary.skipped += result.skipped;
                        summary.failed += result.failed;
                        summary.synced_paths.extend(result.synced_paths);
                    }
                    Err(_) => summary.failed += 1,
                }
                import_progress.set(Some((index + 1, total)));
            }
            let loc = locale.get_untracked();
            let done = summary.imported + summary.updated;
            let failed = summary.failed > 0;
            let mut message = if failed {
                tf(
                    loc,
                    provider.summary_key(true),
                    &[("n", &done.to_string()), ("f", &summary.failed.to_string())],
                )
            } else {
                tf(
                    loc,
                    provider.summary_key(false),
                    &[("n", &done.to_string())],
                )
            };
            // Rollouts already imported and not moved forward count as neither
            // synced nor failed, so without this the numbers don't add up to
            // the selection the user made.
            if summary.skipped > 0 {
                message.push_str(" · ");
                message.push_str(&tf(
                    loc,
                    "import.skipped",
                    &[("n", &summary.skipped.to_string())],
                ));
            }
            import_notice.set(Some((message.clone(), failed)));
            if failed {
                show_warning_toast(&message);
            } else {
                show_toast(&message);
            }
            if selected_context_id.get_untracked() == context_id
                && open.get_untracked() == Some(provider)
            {
                let synced = summary.synced_paths.into_iter().collect::<HashSet<_>>();
                items.update(|rows| {
                    for item in rows {
                        if synced.contains(&item.path) {
                            item.state = "imported".into();
                        }
                    }
                });
            }
            on_imported.call(());
            importing.set(false);
        });
    };

    move || {
        let provider = open.get()?;
        let pending_paths = move || {
            matching_sessions(items.get(), &query.get())
                .into_iter()
                .filter(|item| item.state != "imported")
                .map(|item| item.path)
                .collect::<Vec<_>>()
        };
        Some(view! {
            <div class="overlay" role="presentation" on:click=move |_| open.set(None)>
                <div class="modal codex-import-modal" data-provider=provider.id()
                    role="dialog" aria-modal="true" on:click=|ev| ev.stop_propagation()>
                    <div class="ps-head">
                        <h2>{move || t(locale.get(), provider.title_key())}</h2>
                        <button type="button" class="ps-close"
                            title=move || t(locale.get(), "codex.close")
                            aria-label=move || t(locale.get(), "codex.close")
                            on:click=move |_| open.set(None)>{compose_icon("close")}</button>
                    </div>
                    <div class="codex-import-toolbar">
                        <div class="codex-import-source-wrap">
                            <label class="codex-import-source">
                                <span>{move || t(locale.get(), "codex.source")}</span>
                                <select
                                    aria-label=move || t(locale.get(), "codex.source")
                                    disabled=move || importing.get()
                                    prop:value=move || selected_context_id.get()
                                    on:change=move |ev| {
                                        let context_id = event_target_value(&ev);
                                        selected_context_id.set(context_id.clone());
                                        refresh(provider, context_id, false);
                                    }>
                                    <option value="local">{move || t(locale.get(), "codex.source_local")}</option>
                                    {move || source_contexts.get().into_iter()
                                        .filter(|context| context.kind != "local")
                                        .map(|context| {
                                            let prefix = if context.kind == "wsl" { "WSL" } else { "SSH" };
                                            let label = if context.label.trim().is_empty() {
                                                context.id.clone()
                                            } else {
                                                context.label
                                            };
                                            view! {
                                                <option value=context.id>{format!("{prefix} · {label}")}</option>
                                            }
                                        })
                                        .collect_view()}
                                </select>
                            </label>
                            <span class="codex-import-hint">{move || t(locale.get(), provider.hint_key())}</span>
                        </div>
                        <div class="codex-import-actions">
                            <button type="button" class="icon-btn codex-import-refresh"
                                title=move || t(locale.get(), "codex.refresh")
                                aria-label=move || t(locale.get(), "codex.refresh")
                                disabled=move || importing.get() || loading.get()
                                on:click=move |_| {
                                    refresh(
                                        provider,
                                        selected_context_id.get_untracked(),
                                        true,
                                    );
                                }>
                                {compose_icon("sync")}
                            </button>
                            <button type="button" class="btn-ghost codex-import-all"
                                disabled=move || importing.get() || loading.get() || pending_paths().is_empty()
                                on:click=move |_| import_paths(provider, pending_paths())>
                                {compose_icon("download")}
                                <span>{move || t(locale.get(), "codex.import_all")}</span>
                            </button>
                        </div>
                    </div>
                    <label class="codex-import-search">
                        {compose_icon("search")}
                        <input
                            type="search"
                            aria-label=move || t(locale.get(), "codex.search")
                            placeholder=move || t(locale.get(), "codex.search_placeholder")
                            prop:value=move || query.get()
                            on:input=move |ev| {
                                query.set(event_target_value(&ev));
                                page.set(0);
                            }
                        />
                    </label>
                    {move || import_progress.get().map(|(done, total)| {
                        let notice = import_notice.get();
                        let failed = notice.as_ref().is_some_and(|(_, failed)| *failed);
                        let value = (!importing.get() || done > 0).then_some(done);
                        let label = notice
                            .map(|(message, _)| message)
                            .unwrap_or_else(|| tf(
                                locale.get(),
                                "codex.progress",
                                &[
                                    ("done", &done.to_string()),
                                    ("total", &total.to_string()),
                                ],
                            ));
                        view! {
                            <div class="codex-import-progress" class:failed=failed
                                role="status" aria-live="polite">
                                <span>{label}</span>
                                <progress max=total value=value></progress>
                            </div>
                        }
                    })}
                    <div class="codex-import-list">
                        {move || {
                            let loc = locale.get();
                            if loading.get() {
                                return view! { <div class="side-hint">{t(loc, "codex.loading")}</div> }.into_view();
                            }
                            if let Some(error) = scan_error.get() {
                                return view! { <div class="codex-import-error" role="alert">{error}</div> }.into_view();
                            }
                            let list = matching_sessions(items.get(), &query.get());
                            if list.is_empty() {
                                let empty_key = if query.get().trim().is_empty() {
                                    provider.empty_key()
                                } else {
                                    "codex.no_matches"
                                };
                                return view! { <div class="side-hint">{t(loc, empty_key)}</div> }.into_view();
                            }
                            let page_count = list.len().div_ceil(CLI_IMPORT_PAGE_SIZE);
                            let current_page = page.get().min(page_count.saturating_sub(1));
                            let start = current_page * CLI_IMPORT_PAGE_SIZE;
                            let rows = list
                                .into_iter()
                                .skip(start)
                                .take(CLI_IMPORT_PAGE_SIZE)
                                .map(|item| {
                                    let import_path = item.path.clone();
                                    let toggle_path = item.path.clone();
                                    let expanded_class_path = item.path.clone();
                                    let expanded_aria_path = item.path.clone();
                                    let preview_path = item.path.clone();
                                    let fallback_preview = item.title.clone();
                                    let action_key = match item.state.as_str() {
                                        "updatable" => "codex.update",
                                        "imported" => "codex.imported",
                                        _ => "codex.import",
                                    };
                                    let done = item.state == "imported";
                                    view! {
                                        <div class="codex-import-row" class:imported=done>
                                            <button type="button" class="codex-import-main"
                                                class:expanded=move || expanded_path.get().as_deref() == Some(expanded_class_path.as_str())
                                                title=t(loc, "codex.preview")
                                                aria-expanded=move || if expanded_path.get().as_deref() == Some(expanded_aria_path.as_str()) { "true" } else { "false" }
                                                on:click=move |_| load_preview(provider, toggle_path.clone())>
                                                <span class="codex-import-title" title=item.title.clone()>{item.title.clone()}</span>
                                                <span class="codex-import-meta">
                                                    <span title=item.cwd.clone()>{short_path_label(&item.cwd)}</span>
                                                    <span>{tf(loc, "codex.messages", &[("n", &item.message_count.to_string())])}</span>
                                                    <span>{format_relative_time(item.last_active_at, loc)}</span>
                                                </span>
                                                {move || {
                                                    if expanded_path.get().as_deref() != Some(preview_path.as_str()) {
                                                        return None;
                                                    }
                                                    let loading_preview = preview_loading.get().as_deref()
                                                        == Some(preview_path.as_str());
                                                    let cached = previews.with(|rows| rows.get(&preview_path).cloned());
                                                    let failed = cached.as_ref().is_some_and(Result::is_err);
                                                    let text = if loading_preview {
                                                        t(locale.get(), "codex.preview_loading")
                                                    } else {
                                                        match cached {
                                                            Some(Ok(lines)) if !lines.is_empty() => lines
                                                                .into_iter()
                                                                .map(|line| {
                                                                    let role = if line.role == "assistant" {
                                                                        t(locale.get(), "codex.preview_assistant")
                                                                    } else {
                                                                        t(locale.get(), "codex.preview_user")
                                                                    };
                                                                    format!("{role}\n{}", line.text)
                                                                })
                                                                .collect::<Vec<_>>()
                                                                .join("\n\n"),
                                                            Some(Err(error)) => error,
                                                            _ => fallback_preview.clone(),
                                                        }
                                                    };
                                                    Some(view! {
                                                        <span class="codex-import-preview" class:failed=failed>{text}</span>
                                                    })
                                                }}
                                            </button>
                                            <button type="button" class="btn-ghost codex-import-btn"
                                                disabled=move || done || importing.get()
                                                on:click=move |_| import_paths(provider, vec![import_path.clone()])>
                                                {t(loc, action_key)}
                                            </button>
                                        </div>
                                    }
                                })
                                .collect_view();
                            view! {
                                <>
                                    {rows}
                                    {(page_count > 1).then(|| view! {
                                        <div class="codex-import-pagination">
                                            <button type="button" class="btn-ghost"
                                                disabled={move || current_page == 0}
                                                on:click=move |_| page.update(|value| *value = value.saturating_sub(1))>
                                                {t(loc, "codex.previous")}
                                            </button>
                                            <span>{tf(
                                                loc,
                                                "codex.page",
                                                &[
                                                    ("page", &(current_page + 1).to_string()),
                                                    ("pages", &page_count.to_string()),
                                                ],
                                            )}</span>
                                            <button type="button" class="btn-ghost"
                                                disabled={move || current_page + 1 >= page_count}
                                                on:click=move |_| page.update(|value| {
                                                    *value = (*value + 1).min(page_count - 1);
                                                })>
                                                {t(loc, "codex.next")}
                                            </button>
                                        </div>
                                    })}
                                </>
                            }.into_view()
                        }}
                    </div>
                </div>
            </div>
        })
    }
}

fn matching_sessions(items: Vec<ExternalSessionInfo>, query: &str) -> Vec<ExternalSessionInfo> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return items;
    }
    items
        .into_iter()
        .filter(|item| {
            [
                item.title.as_str(),
                item.cwd.as_str(),
                item.session_id.as_str(),
                item.path.as_str(),
            ]
            .into_iter()
            .any(|value| value.to_lowercase().contains(&query))
        })
        .collect()
}

/// Last two path components — enough to recognize a project directory without
/// overflowing the row.
fn short_path_label(path: &str) -> String {
    let parts = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    match parts.len() {
        0 => path.to_string(),
        1 => parts[0].to_string(),
        n => format!("{}/{}", parts[n - 2], parts[n - 1]),
    }
}
