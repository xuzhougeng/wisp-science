use super::*;

pub(crate) const LITERATURE_RESEARCH_ACTION_ID: &str = "literature_research";
pub(crate) const LITERATURE_REVIEW_SKILL: &str = "literature-review";

/// Apply a transcript mutation to the right session: the live `items` view when
/// `fid` is the active session, otherwise the background cache keyed by `fid`.
/// This is what lets a second conversation stream while the user views another.
pub(crate) fn route_items(
    active: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    fid: &str,
    f: impl FnOnce(&mut Vec<ChatItem>),
) {
    if active.get().as_deref() == Some(fid) {
        items.update(f);
    } else {
        transcripts.update(|m| f(m.entry(fid.to_string()).or_insert_with(Vec::new)));
    }
}

pub(crate) fn quick_action_label(locale: Locale, action: &QuickAction) -> String {
    if action.id == LITERATURE_RESEARCH_ACTION_ID && action.name == "Research literature" {
        t(locale, "selection.research_literature").into()
    } else {
        action.name.clone()
    }
}

/// The built-in literature action stays in the current conversation so its
/// progress and result share the conversation's durable transcript. Custom
/// Quick Actions continue to instantiate their bound Workflow.
pub(crate) fn quick_action_uses_current_conversation(action: &QuickAction) -> bool {
    action.builtin && action.id == LITERATURE_RESEARCH_ACTION_ID
}

pub(crate) fn append_composer_prompt(current: &str, prompt: &str) -> String {
    let prompt = prompt.trim();
    let current_text = current.trim();
    if current_text.is_empty() {
        prompt.to_string()
    } else if prompt.is_empty() || current_text.ends_with(prompt) {
        current.to_string()
    } else {
        format!("{}\n\n{prompt}", current.trim_end())
    }
}

#[cfg(test)]
mod quick_action_routing_tests {
    use super::*;

    fn action(id: &str, builtin: bool) -> QuickAction {
        QuickAction {
            id: id.into(),
            name: "Action".into(),
            description: String::new(),
            icon: "search".into(),
            context: "selection".into(),
            workflow_template_id: "workflow".into(),
            enabled: true,
            sort_order: 0,
            builtin,
        }
    }

    #[test]
    fn only_builtin_literature_research_stays_in_current_conversation() {
        assert!(quick_action_uses_current_conversation(&action(
            LITERATURE_RESEARCH_ACTION_ID,
            true,
        )));
        assert!(!quick_action_uses_current_conversation(&action(
            LITERATURE_RESEARCH_ACTION_ID,
            false,
        )));
        assert!(!quick_action_uses_current_conversation(&action(
            "custom", true
        )));
    }

    #[test]
    fn composer_prompt_preserves_an_existing_draft() {
        assert_eq!(append_composer_prompt("", "Research this"), "Research this");
        assert_eq!(
            append_composer_prompt("Keep this context", "Research this"),
            "Keep this context\n\nResearch this"
        );
        assert_eq!(
            append_composer_prompt("Keep this context\n\nResearch this", "Research this"),
            "Keep this context\n\nResearch this"
        );
    }
}

pub(crate) fn selection_popup_x(x: i32) -> i32 {
    selection_popup_x_with_max_width(x, 720)
}

pub(crate) fn selection_popup_x_with_max_width(x: i32, max_width: i32) -> i32 {
    let Some(viewport) = web_sys::window()
        .and_then(|window| window.inner_width().ok())
        .and_then(|value| value.as_f64())
        .map(|value| value.round() as i32)
    else {
        return x;
    };
    let usable = (viewport - 24).max(0);
    let half_width = usable.min(max_width.max(0)) / 2;
    x.clamp(half_width, viewport - half_width)
}

/// Keep enough room above the anchor for the popup. Code selections use a
/// vertical menu (~4 stacked actions), so they need a taller clearance than
/// the compact horizontal chat/preview popup.
pub(crate) fn selection_popup_y(y: i32) -> i32 {
    selection_popup_y_with_clearance(y, 120)
}

pub(crate) fn selection_popup_y_with_clearance(y: i32, min_clearance: i32) -> i32 {
    let Some(viewport) = web_sys::window()
        .and_then(|window| window.inner_height().ok())
        .and_then(|value| value.as_f64())
        .map(|value| value.round() as i32)
    else {
        return y;
    };
    let max_y = (viewport - 12).max(0);
    y.clamp(min_clearance.min(max_y), max_y)
}

/// Bounding box of the open runtime workbench, for divider-drag geometry.
/// Only one center file is mounted at a time, so the selector is unambiguous.
pub(crate) fn runtime_workbench_rect() -> Option<web_sys::DomRect> {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| {
            document
                .query_selector(".center-file-runtime-preview.runtime-panel-open")
                .ok()
                .flatten()
        })
        .map(|element| element.get_bounding_client_rect())
}

/// A dedicated project window (#52) carries `?project=<id>` in its URL. Returns
/// that id so the window opens straight into the project and skips the landing.
/// Project ids are UUIDs or "default" — no percent-decoding needed.
pub(crate) fn url_project_param() -> Option<String> {
    url_query_param("project")
}

/// Optional `&session=<id>` companion to `?project=` (#423): the window opens
/// straight into that session. Session ids are UUIDs — no percent-decoding.
pub(crate) fn url_session_param() -> Option<String> {
    url_query_param("session")
}

fn url_query_param(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let q = search.strip_prefix('?').unwrap_or(&search);
    let prefix = format!("{key}=");
    q.split('&')
        .find_map(|p| p.strip_prefix(prefix.as_str()))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

#[component]
pub(crate) fn CommandPalette(
    open: RwSignal<bool>,
    current_project_id: Signal<Option<String>>,
    privacy_mode_active: RwSignal<bool>,
    privacy_hidden_project_ids: RwSignal<HashSet<String>>,
    on_open_project: Callback<(String, bool)>,
    on_open_session: Callback<(String, String, bool)>,
    on_open_artifact: Callback<(String, String, String)>,
    on_command: Callback<&'static str>,
    on_new_session: Callback<()>,
    on_open_scratch: Callback<()>,
    on_project_settings: Callback<()>,
    on_manage_skills: Callback<()>,
    on_attach: Callback<ComposerReferenceChip>,
) -> impl IntoView {
    let locale = use_locale();
    let query = create_rw_signal(String::new());
    let active = create_rw_signal(0usize);
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let artifacts = create_rw_signal(Vec::<ArtifactInfo>::new());
    let sessions = create_rw_signal(Vec::<SessionSearchInfo>::new());
    create_effect(move |_| {
        if !open.get() {
            return;
        }
        let q = query.get();
        let preferred_project_id = current_project_id.get();
        spawn_local(async move {
            let p = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(p) {
                projects.set(rows);
            }
            let a = invoke(
                "search_artifacts",
                to_value(&serde_json::json!({ "query": q, "limit": 12, "allProjects": true }))
                    .unwrap(),
            )
            .await;
            if query.get_untracked() == q {
                if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(a) {
                    artifacts.set(rows);
                }
            }
            let s = invoke(
                "search_sessions",
                to_value(&serde_json::json!({
                    "query": q,
                    "limit": 12,
                    "preferredProjectId": preferred_project_id,
                }))
                .unwrap(),
            )
            .await;
            if query.get_untracked() == q
                && current_project_id.get_untracked() == preferred_project_id
            {
                if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SessionSearchInfo>>(s) {
                    sessions.set(rows);
                }
            }
        });
    });
    create_effect(move |_| {
        open.get();
        query.get();
        active.set(0);
    });
    create_effect(move |_| {
        if open.get() {
            focus_element_soon("command-palette-input");
        }
    });
    let items = create_memo(move |_| {
        let q = query.get().trim().to_lowercase();
        let current = current_project_id.get();
        let hidden = privacy_mode_active
            .get()
            .then(|| privacy_hidden_project_ids.get())
            .unwrap_or_default();
        let mut out = Vec::new();
        let mut ps: Vec<_> = projects
            .get()
            .into_iter()
            .filter(|p| !hidden.contains(&p.id) && contains_search(&q, &[&p.name, &p.description]))
            .collect();
        ps.sort_by_key(|p| (current.as_deref() != Some(p.id.as_str()), p.name.clone()));
        out.extend(ps.into_iter().map(CommandPaletteItem::Project));
        let mut ars = artifacts.get();
        ars.sort_by_key(|a| {
            (
                current.as_deref() != a.project_id.as_deref(),
                std::cmp::Reverse(a.ts),
            )
        });
        out.extend(
            ars.into_iter()
                .filter(|a| !a.project_id.as_ref().is_some_and(|id| hidden.contains(id)))
                .map(CommandPaletteItem::Artifact),
        );
        let mut ss = sessions.get();
        // The store ranks title hits ahead of transcript-body hits. This stable
        // partition keeps that order while retaining a current-project bias
        // when an older backend or mocked bridge returns unranked rows.
        ss.sort_by_key(|s| current.as_deref() != Some(s.project_id.as_str()));
        out.extend(
            ss.into_iter()
                .filter(|s| !hidden.contains(&s.project_id))
                .map(CommandPaletteItem::Session),
        );
        out.push(CommandPaletteItem::Command("scratch"));
        out.push(CommandPaletteItem::Command("new"));
        out.push(CommandPaletteItem::Command("check-updates"));
        out.push(CommandPaletteItem::Command("star-us"));
        if current.is_some() {
            out.push(CommandPaletteItem::Command("settings"));
            out.push(CommandPaletteItem::Command("skills"));
        }
        out
    });
    let open_item = Callback::new(move |(idx, new_window): (usize, bool)| {
        let Some(item) = items.get().get(idx).cloned() else {
            return;
        };
        open.set(false);
        match item {
            CommandPaletteItem::Project(p) => on_open_project.call((p.id, new_window)),
            CommandPaletteItem::Artifact(a) => {
                let kind = file_kind(&a.name)
                    .or_else(|| file_kind(&a.path))
                    .unwrap_or("text")
                    .to_string();
                on_open_artifact.call((format!("artifact:{}", a.id), a.name, kind));
            }
            CommandPaletteItem::Session(s) => {
                on_open_session.call((s.project_id, s.id, new_window))
            }
            CommandPaletteItem::Command("scratch") => on_open_scratch.call(()),
            CommandPaletteItem::Command("new") => on_new_session.call(()),
            CommandPaletteItem::Command("check-updates") => on_command.call("check-updates"),
            CommandPaletteItem::Command("star-us") => on_command.call("star-us"),
            CommandPaletteItem::Command("settings") => on_project_settings.call(()),
            CommandPaletteItem::Command("skills") => on_manage_skills.call(()),
            CommandPaletteItem::Command(_) => {}
        }
    });
    let attach_item = Callback::new(move |idx: usize| {
        let list = items.get();
        let item = list
            .get(idx)
            .cloned()
            .filter(|item| {
                matches!(
                    item,
                    CommandPaletteItem::Artifact(_) | CommandPaletteItem::Session(_)
                )
            })
            .or_else(|| {
                list.into_iter().find(|item| {
                    matches!(
                        item,
                        CommandPaletteItem::Artifact(_) | CommandPaletteItem::Session(_)
                    )
                })
            });
        let Some(item) = item else {
            return;
        };
        match item {
            CommandPaletteItem::Artifact(a) => on_attach.call(ComposerReferenceChip::Artifact {
                id: a.id,
                name: a.name,
            }),
            CommandPaletteItem::Session(s) => on_attach.call(ComposerReferenceChip::Session {
                id: s.id,
                title: s.title,
                project_name: s.project_name,
            }),
            _ => return,
        }
        open.set(false);
        focus_composer();
    });
    view! {
        {move || open.get().then(|| view! {
            <div class="project-search-overlay conversation-search-overlay" on:click=move |_| open.set(false)>
                <div class="project-search-dialog conversation-search-dialog" role="dialog" aria-label="Search"
                    on:click=|ev| ev.stop_propagation()>
                    <div class="project-search-input">
                        {compose_icon("search")}
                        <input id="command-palette-input" type="text" inputmode="search" autofocus=true
                            autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                            placeholder=move || t(locale.get(), "command.search_ph")
                            prop:value=move || query.get()
                            on:input=move |ev| query.set(event_target_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ime_composing(&ev) { return; }
                                let n = items.get().len();
                                match ev.key().as_str() {
                                    "Escape" => { ev.prevent_default(); open.set(false); }
                                    "ArrowDown" => { ev.prevent_default(); if n > 0 { let next = (active.get() + 1) % n; active.set(next); scroll_picker_item(".project-search-dialog:not(.action-palette) .project-search-row", next); } }
                                    "ArrowUp" => { ev.prevent_default(); if n > 0 { let next = (active.get() + n - 1) % n; active.set(next); scroll_picker_item(".project-search-dialog:not(.action-palette) .project-search-row", next); } }
                                    "Enter" if ev.shift_key() => { ev.prevent_default(); attach_item.call(active.get()); }
                                    "Enter" if ev.ctrl_key() || ev.meta_key() => { ev.prevent_default(); open_item.call((active.get(), true)); }
                                    "Enter" => { ev.prevent_default(); open_item.call((active.get(), false)); }
                                    _ => {}
                                }
                            } />
                    </div>
                    <div class="project-search-results">
                        {move || items.get().into_iter().enumerate().map(|(i, item)| {
                            let opens_project_window = matches!(&item, CommandPaletteItem::Project(_) | CommandPaletteItem::Session(_));
                            let (icon, title, sub) = match item {
                                CommandPaletteItem::Project(p) => ("folder", p.name, p.description),
                                CommandPaletteItem::Artifact(a) => ("doc", a.name, a.project_name.unwrap_or_default()),
                                CommandPaletteItem::Session(s) => ("bubble", s.title, s.project_name),
                                CommandPaletteItem::Command("scratch") => ("bubble", t(locale.get(), "command.scratch").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("new") => ("plus", t(locale.get(), "projects.new").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("check-updates") => ("gear", t(locale.get(), "command.check_updates").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("star-us") => ("star", t(locale.get(), "command.star_us").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("settings") => ("gear", t(locale.get(), "proj_settings.title").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command("skills") => ("grid", t(locale.get(), "settings.nav.skills").to_string(), t(locale.get(), "command.category")),
                                CommandPaletteItem::Command(_) => ("doc", String::new(), String::new()),
                            };
                            view! {
                                <button type="button" class="project-search-row" class:active=move || active.get() == i
                                    data-icon=icon
                                    on:mousemove=move |_| active.set(i)
                                    on:click=move |_| open_item.call((i, false))>
                                    {compose_icon(icon)}
                                    <span class="project-search-main">
                                        <span class="project-search-title">{title}</span>
                                        {(!sub.trim().is_empty()).then(|| view! { <span class="project-search-sub">{sub}</span> })}
                                    </span>
                                    {opens_project_window.then(|| view! {
                                        <kbd class="action-shortcut project-window-shortcut">
                                            {if is_mac() { "⌘↵" } else { "Ctrl↵" }}" "{t(locale.get(), "command.hint.open_new_window")}
                                        </kbd>
                                    })}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                    <div class="project-search-foot"><span><kbd>"↑↓"</kbd>{t(locale.get(), "command.hint.navigate")}</span><span><kbd>"↵"</kbd>{t(locale.get(), "command.hint.open")}</span><span><kbd>"⇧↵"</kbd>{t(locale.get(), "command.hint.attach")}</span><span><kbd>"esc"</kbd>{t(locale.get(), "command.hint.close")}</span><span class="palette-version">{concat!("v", env!("CARGO_PKG_VERSION"))}</span></div>
                </div>
            </div>
        })}
    }
}

#[component]
pub(crate) fn ActionPalette(
    open: RwSignal<bool>,
    has_current_project: Signal<bool>,
    on_action: Callback<&'static str>,
) -> impl IntoView {
    let locale = use_locale();
    let query = create_rw_signal(String::new());
    let active = create_rw_signal(0usize);
    let mac = is_mac();
    create_effect(move |_| {
        if !open.get() {
            return;
        }
        query.set(String::new());
        active.set(0);
        let focus = Closure::once(|| {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Some(input) = doc.get_element_by_id("action-palette-input") else {
                return;
            };
            let _ = input.dyn_ref::<web_sys::HtmlElement>().map(|el| el.focus());
        });
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                focus.as_ref().unchecked_ref(),
                0,
            );
        }
        focus.forget();
    });
    let actions = create_memo(move |_| {
        let loc = locale.get();
        let general = t(loc, "command.group.general").to_string();
        let transfer = t(loc, "command.group.transfer").to_string();
        let navigate = t(loc, "command.group.navigate").to_string();
        let appearance = t(loc, "command.group.appearance").to_string();
        let help = t(loc, "command.group.help").to_string();
        let entries = [
            (
                "scratch",
                "bubble",
                "command.scratch",
                general.clone(),
                "shift-n",
                "scratch chat 随手 对话",
                false,
            ),
            (
                "new",
                "plus",
                "command.new_session",
                general.clone(),
                "n",
                "new conversation session 新建 会话",
                true,
            ),
            (
                "new-window",
                "expand",
                "command.new_window",
                general.clone(),
                "",
                "new window gui 新建 窗口",
                false,
            ),
            (
                "search",
                "search",
                "command.search",
                general.clone(),
                "k",
                "find project artifact session 搜索 项目 会话",
                false,
            ),
            (
                "settings",
                "gear",
                "command.settings",
                general.clone(),
                ",",
                "preferences config 设置",
                false,
            ),
            (
                "privacy-mode",
                "eye-off",
                "command.privacy_mode",
                general.clone(),
                "shift-h",
                "privacy recording hide projects recent sessions 隐私 录屏 隐藏 项目 最近会话",
                false,
            ),
            (
                "setup",
                "wrench",
                "command.setup",
                general.clone(),
                "",
                "quick setup onboarding model environment python r uv node 快速 配置 初始化 模型 环境",
                false,
            ),
            (
                "import-codex",
                "download",
                "command.import_codex",
                transfer.clone(),
                "",
                "import codex conversation session 导入 会话",
                true,
            ),
            (
                "import-claude",
                "download",
                "command.import_claude",
                transfer.clone(),
                "",
                "import claude code conversation session 导入 会话",
                true,
            ),
            (
                "import-session",
                "download",
                "command.import_session",
                transfer.clone(),
                "",
                "import restore archive zip conversation session 导入 恢复 归档 会话",
                true,
            ),
            (
                "export-current-project",
                "download",
                "command.export_current_project",
                transfer,
                "",
                "export backup project archive zip 导出 备份 项目 归档",
                true,
            ),
            (
                "check-updates",
                "gear",
                "command.check_updates",
                general.clone(),
                "",
                "update upgrade version 更新 升级 版本",
                false,
            ),
            (
                "project-settings",
                "gear",
                "command.project_settings",
                general.clone(),
                "",
                "project preferences config 项目 设置",
                true,
            ),
            (
                "skills",
                "grid",
                "command.skills",
                general,
                "",
                "manage skill 管理 技能",
                true,
            ),
            (
                "projects",
                "folder",
                "command.projects",
                navigate.clone(),
                "",
                "open switch project 打开 切换 项目",
                false,
            ),
            (
                "library",
                "star",
                "command.library",
                navigate.clone(),
                "",
                "favorites collection 收藏 库",
                false,
            ),
            (
                "toggle-sidebar",
                "panel",
                "command.toggle_sidebar",
                navigate.clone(),
                "b",
                "show hide sidebar 显示 隐藏 侧栏",
                false,
            ),
            (
                "artifacts",
                "grid",
                "command.artifacts",
                navigate.clone(),
                "",
                "open outputs results 产物 结果",
                true,
            ),
            (
                "notebook",
                "doc",
                "command.notebook",
                navigate.clone(),
                "",
                "open notebook 笔记本",
                true,
            ),
            (
                "files",
                "doc",
                "command.files",
                navigate.clone(),
                "",
                "open browser files 文件 浏览",
                true,
            ),
            (
                "provenance",
                "copy",
                "command.provenance",
                navigate.clone(),
                "",
                "open provenance history lineage 溯源 历史",
                true,
            ),
            (
                "contexts",
                "server",
                "command.contexts",
                navigate.clone(),
                "",
                "execution host runtime ssh wsl 执行 上下文 主机",
                true,
            ),
            (
                "side-chat",
                "bubble",
                "command.side_chat",
                navigate.clone(),
                "",
                "ask side chat 侧边 对话",
                true,
            ),
            (
                "close-panel",
                "panel",
                "command.close_panel",
                navigate,
                "",
                "hide right panel 关闭 右侧",
                true,
            ),
            (
                "theme-light",
                "gear",
                "command.theme_light",
                appearance.clone(),
                "",
                "light color appearance 浅色 主题",
                false,
            ),
            (
                "theme-dark",
                "gear",
                "command.theme_dark",
                appearance.clone(),
                "",
                "dark color appearance 深色 主题",
                false,
            ),
            (
                "theme-system",
                "gear",
                "command.theme_system",
                appearance.clone(),
                "",
                "system auto color appearance 系统 自动 主题",
                false,
            ),
            (
                "font-ui-increase",
                "plus",
                "command.font_ui_increase",
                appearance.clone(),
                "",
                "font ui increase larger 字体 界面 增大",
                false,
            ),
            (
                "font-ui-decrease",
                "minus",
                "command.font_ui_decrease",
                appearance.clone(),
                "",
                "font ui decrease smaller 字体 界面 缩小",
                false,
            ),
            (
                "font-code-increase",
                "plus",
                "command.font_code_increase",
                appearance.clone(),
                "",
                "font code increase larger 字体 代码 增大",
                false,
            ),
            (
                "font-code-decrease",
                "minus",
                "command.font_code_decrease",
                appearance,
                "",
                "font code decrease smaller 字体 代码 缩小",
                false,
            ),
            (
                "star-us",
                "star",
                "command.star_us",
                help.clone(),
                "",
                "github star",
                false,
            ),
            (
                "docs",
                "doc",
                "menu.docs",
                help.clone(),
                "",
                "documentation readme help 文档 帮助",
                false,
            ),
            (
                "issues",
                "bubble",
                "menu.issues",
                help,
                "",
                "github issue bug feedback report 问题 反馈 报告",
                false,
            ),
        ];
        let q = query.get().trim().to_lowercase();
        entries
            .into_iter()
            .filter_map(|(id, icon, key, group, shortcut, aliases, project_only)| {
                if project_only && !has_current_project.get() {
                    return None;
                }
                let title = t(loc, key).to_string();
                contains_search(&q, &[id, &title, &group, aliases]).then(|| {
                    let shortcut = match (mac, shortcut) {
                        (_, "") => String::new(),
                        (true, "shift-n") => "⌘⇧N".into(),
                        (false, "shift-n") => "Ctrl+Shift+N".into(),
                        (true, "shift-h") => "⌘⇧H".into(),
                        (false, "shift-h") => "Ctrl+Shift+H".into(),
                        (true, key) => format!("⌘{}", key.to_uppercase()),
                        (false, key) => format!("Ctrl+{}", key.to_uppercase()),
                    };
                    CommandAction {
                        id,
                        icon,
                        title,
                        group,
                        shortcut,
                    }
                })
            })
            .collect::<Vec<_>>()
    });
    let run = Callback::new(move |index: usize| {
        let Some(action) = actions.get().get(index).cloned() else {
            return;
        };
        open.set(false);
        on_action.call(action.id);
    });
    view! {
        {move || open.get().then(|| view! {
            <div class="project-search-overlay action-palette-overlay" on:click=move |_| open.set(false)>
                <div class="project-search-dialog action-palette" role="dialog" aria-label="Command Palette"
                    on:click=|ev| ev.stop_propagation()>
                    <div class="project-search-input">
                        {compose_icon("search")}
                        <input id="action-palette-input" type="text" inputmode="search" autofocus=true
                            autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                            placeholder=move || t(locale.get(), "command.placeholder")
                            prop:value=move || query.get()
                            on:input=move |ev| { query.set(event_target_value(&ev)); active.set(0); }
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ime_composing(&ev) { return; }
                                let n = actions.get().len();
                                match ev.key().as_str() {
                                    "Escape" => { ev.prevent_default(); open.set(false); }
                                    "ArrowDown" => {
                                        ev.prevent_default();
                                        if n > 0 {
                                            let next = (active.get() + 1) % n;
                                            active.set(next);
                                            scroll_picker_item(".action-palette .project-search-row", next);
                                        }
                                    }
                                    "ArrowUp" => {
                                        ev.prevent_default();
                                        if n > 0 {
                                            let next = (active.get() + n - 1) % n;
                                            active.set(next);
                                            scroll_picker_item(".action-palette .project-search-row", next);
                                        }
                                    }
                                    "Enter" => { ev.prevent_default(); run.call(active.get()); }
                                    _ => {}
                                }
                            } />
                    </div>
                    <div class="project-search-results action-palette-results">
                        {move || {
                            let rows = actions.get();
                            if rows.is_empty() {
                                return view! {
                                    <div class="project-search-empty" data-testid="action-palette-empty">
                                        {t(locale.get(), "command.no_results")}
                                    </div>
                                }.into_view();
                            }
                            rows.into_iter().enumerate().map(|(i, action)| {
                                let previous_group = (i > 0).then(|| actions.get().get(i - 1).map(|a| a.group.clone())).flatten();
                                let show_group = previous_group.as_deref() != Some(action.group.as_str());
                                view! {
                                    {show_group.then(|| view! { <div class="action-palette-group">{action.group.clone()}</div> })}
                                    <button type="button" class="project-search-row action-palette-row" class:active=move || active.get() == i
                                        on:mousemove=move |_| active.set(i)
                                        on:click=move |_| run.call(i)>
                                        {compose_icon(&action.icon)}
                                        <span class="project-search-main"><span class="project-search-title">{action.title}</span></span>
                                        {(!action.shortcut.is_empty()).then(|| view! { <kbd class="action-shortcut">{action.shortcut}</kbd> })}
                                    </button>
                                }
                            }).collect_view().into_view()
                        }}
                    </div>
                    <div class="project-search-foot"><span><kbd>"↑↓"</kbd>{t(locale.get(), "command.hint.navigate")}</span><span><kbd>"↵"</kbd>{t(locale.get(), "command.hint.run")}</span><span><kbd>"esc"</kbd>{t(locale.get(), "command.hint.close")}</span></div>
                </div>
            </div>
        })}
    }
}

#[component]
pub(crate) fn PrivacyModeModal(
    open: RwSignal<bool>,
    active: RwSignal<bool>,
    hidden_project_ids: RwSignal<HashSet<String>>,
    on_hide: Callback<HashSet<String>>,
    on_restore: Callback<()>,
) -> impl IntoView {
    let locale = use_locale();
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let selected = create_rw_signal(HashSet::<String>::new());

    create_effect(move |_| {
        if !open.get() {
            return;
        }
        selected.set(hidden_project_ids.get_untracked());
        spawn_local(async move {
            let value = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(value) {
                projects.set(rows);
            }
        });
    });

    view! {
        {move || open.get().then(|| view! {
            <div class="overlay privacy-mode-overlay" on:click=move |_| open.set(false)>
                <div class="modal privacy-mode-modal" role="dialog"
                    aria-label=move || t(locale.get(), "privacy.title")
                    on:click=|event| event.stop_propagation()>
                    <div class="privacy-mode-head">
                        <div class="privacy-mode-icon" aria-hidden="true">{compose_icon("eye-off")}</div>
                        <div>
                            <h2>{move || t(locale.get(), "privacy.title")}</h2>
                            <p>{move || t(locale.get(), "privacy.hint")}</p>
                        </div>
                    </div>
                    <div class="privacy-project-list" data-testid="privacy-project-list">
                        {move || (!projects.with(|rows| rows.is_empty())).then(|| view! {
                            <label class="privacy-project-row privacy-project-row-all">
                                <input type="checkbox" data-testid="privacy-select-all"
                                    prop:checked=move || projects.with(|rows| {
                                        !rows.is_empty()
                                            && rows.iter().all(|p| selected.with(|ids| ids.contains(&p.id)))
                                    })
                                    on:change=move |event| {
                                        let checked = event_target_checked(&event);
                                        if checked {
                                            selected.set(projects.with(|rows| {
                                                rows.iter().map(|p| p.id.clone()).collect()
                                            }));
                                        } else {
                                            selected.set(HashSet::new());
                                        }
                                    } />
                                <span class="privacy-project-main">
                                    <span>{move || t(locale.get(), "privacy.select_all")}</span>
                                </span>
                            </label>
                        })}
                        {move || projects.get().into_iter().map(|project| {
                            let id = project.id.clone();
                            let checked_id = project.id.clone();
                            view! {
                                <label class="privacy-project-row">
                                    <input type="checkbox"
                                        prop:checked=move || selected.with(|ids| ids.contains(&checked_id))
                                        on:change=move |event| {
                                            let checked = event_target_checked(&event);
                                            selected.update(|ids| {
                                                if checked { ids.insert(id.clone()); } else { ids.remove(&id); }
                                            });
                                        } />
                                    <span class="privacy-project-main">
                                        <span>{project.name}</span>
                                        {(!project.workspace_dir.trim().is_empty()).then(|| view! {
                                            <small title=project.workspace_dir.clone()>{project.workspace_dir}</small>
                                        })}
                                    </span>
                                </label>
                            }
                        }).collect_view()}
                    </div>
                    <div class="privacy-mode-foot">
                        <p class="privacy-mode-status">
                            {move || t(locale.get(), if active.get() { "privacy.active" } else { "privacy.inactive" })}
                        </p>
                        <div class="row privacy-mode-actions">
                            <button type="button" on:click=move |_| open.set(false)>
                                {move || t(locale.get(), "projects.cancel")}
                            </button>
                            <button type="button" disabled=move || !active.get()
                                on:click=move |_| on_restore.call(())>
                                {move || t(locale.get(), "privacy.restore")}
                            </button>
                            <button type="button" class="primary"
                                disabled=move || selected.with(|ids| ids.is_empty())
                                on:click=move |_| on_hide.call(selected.get_untracked())>
                                {move || t(locale.get(), "privacy.hide")}
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        })}
    }
}
