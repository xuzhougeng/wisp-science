use crate::app_support::{
    allow_drop, bucket_sessions_by_date, compose_icon, drag_session_id, load_view_pref,
    nest_branch_sessions, save_view_pref, start_session_drag, AvailableUpdate, FolderModal,
};
use crate::bindings::is_mac;
use crate::dto::*;
use crate::i18n::{t, tf, Locale};
use crate::text::dom_value;
use crate::window_capture_escape;
use leptos::*;
use std::collections::{HashMap, HashSet};

fn toggle_collapsed_key(set: &mut HashSet<String>, key: String) {
    if !set.insert(key.clone()) {
        set.remove(&key);
    }
}

#[cfg(test)]
mod nest_toggle_tests {
    use super::toggle_collapsed_key;
    use std::collections::HashSet;

    #[test]
    fn toggle_collapsed_key_inserts_then_removes() {
        let mut set = HashSet::new();
        toggle_collapsed_key(&mut set, "e:main".into());
        assert!(set.contains("e:main"));
        toggle_collapsed_key(&mut set, "e:main".into());
        assert!(set.is_empty());
        toggle_collapsed_key(&mut set, "b:main".into());
        toggle_collapsed_key(&mut set, "e:main".into());
        assert_eq!(set.len(), 2);
    }
}

fn nest_group_toggle(
    locale: Locale,
    label_key: &'static str,
    expand_key: &'static str,
    collapse_key: &'static str,
    count: usize,
    collapsed: bool,
    test_id: &'static str,
    on_click: impl Fn(web_sys::MouseEvent) + 'static,
) -> impl IntoView {
    let action = t(locale, if collapsed { expand_key } else { collapse_key });
    let count_label = count.to_string();
    let aria = format!("{action} ({count})");
    view! {
        <button type="button" class="side-nest-toggle"
            class:collapsed=collapsed
            data-testid=test_id
            aria-expanded=if collapsed { "false" } else { "true" }
            title=action
            aria-label=aria
            on:click=on_click>
            <span class="side-nest-caret" class:collapsed=collapsed aria-hidden="true">
                {compose_icon("chevron-down")}
            </span>
            <span class="side-nest-label">{t(locale, label_key)}</span>
            <span class="side-nest-count">{count_label}</span>
        </button>
    }
}

/// Secondary nav entry. `tier` names the window-height breakpoint in sidebar.css
/// below which the entry leaves the inline nav for the "More" flyout.
type NavEntry = (
    &'static str,
    &'static str,
    &'static str,
    Signal<bool>,
    Callback<web_sys::MouseEvent>,
);

fn nav_entry_button(locale: RwSignal<Locale>, entry: NavEntry) -> impl IntoView {
    let (icon, label_key, tier, active, on_click) = entry;
    view! {
        <button class=format!("side-btn {tier}") class:active=move || active.get()
            aria-current=move || active.get().then_some("page")
            title=move || t(locale.get(), label_key)
            on:click=move |ev| on_click.call(ev)>
            {compose_icon(icon)}
            <span class="side-btn-label">{move || t(locale.get(), label_key)}</span>
        </button>
    }
}

#[derive(Clone, Copy)]
pub(super) struct SidebarState {
    pub(super) locale: RwSignal<Locale>,
    pub(super) show_sidebar: RwSignal<bool>,
    pub(super) sidebar_w: RwSignal<f64>,
    pub(super) show_proj_menu: RwSignal<bool>,
    pub(super) show_projects: RwSignal<bool>,
    pub(super) demo_mode: RwSignal<bool>,
    pub(super) project_info: RwSignal<Option<ProjectInfo>>,
    pub(super) proj_list: RwSignal<Vec<ProjectSummary>>,
    pub(super) sessions: RwSignal<Vec<SessionInfo>>,
    pub(super) explorations: RwSignal<Vec<ExplorationSummary>>,
    pub(super) folders: RwSignal<Vec<FolderInfo>>,
    pub(super) drag_session: RwSignal<Option<String>>,
    pub(super) drop_target: RwSignal<Option<String>>,
    pub(super) active_session: RwSignal<Option<String>>,
    pub(super) running: RwSignal<HashSet<String>>,
    /// Sessions blocked on a tool-approval / permission card (#327).
    pub(super) attention: RwSignal<HashSet<String>>,
    pub(super) rename_session_input: RwSignal<String>,
    pub(super) rename_session_target: RwSignal<Option<(String, String)>>,
    pub(super) collapsed_folders: RwSignal<HashSet<String>>,
    pub(super) folder_modal_input: RwSignal<String>,
    pub(super) folder_modal: RwSignal<Option<FolderModal>>,
    pub(super) demos: RwSignal<Vec<DemoInfo>>,
    pub(super) session_history_cursor: RwSignal<Option<SessionCursor>>,
    pub(super) session_history_loading: RwSignal<bool>,
    /// Newer release surfaced by the auto-check → prompt card in the footer.
    pub(super) update_banner: RwSignal<Option<AvailableUpdate>>,
}

#[component]
pub(super) fn Sidebar(
    state: SidebarState,
    toggle_proj_menu: Callback<web_sys::MouseEvent>,
    open_proj_settings: Callback<web_sys::MouseEvent>,
    switch_project: Callback<String>,
    new_session: Callback<web_sys::MouseEvent>,
    open_search: Callback<web_sys::MouseEvent>,
    open_shelved: Callback<web_sys::MouseEvent>,
    new_folder: Callback<web_sys::MouseEvent>,
    open_files: Callback<web_sys::MouseEvent>,
    open_research_graph: Callback<web_sys::MouseEvent>,
    research_journey_open: ReadSignal<bool>,
    open_publication_workspace: Callback<web_sys::MouseEvent>,
    publication_open: ReadSignal<bool>,
    open_library: Callback<web_sys::MouseEvent>,
    load_demo: Callback<DemoInfo>,
    open_demo_actions: Callback<(web_sys::MouseEvent, String, String)>,
    load_session: Callback<String>,
    open_exploration: Callback<Exploration>,
    open_exploration_actions: Callback<(web_sys::MouseEvent, String, String)>,
    load_older_sessions: Callback<()>,
    move_sessions_to: Callback<(Vec<String>, Option<String>)>,
    delete_sessions: Callback<Vec<String>>,
    open_session_actions: Callback<(
        web_sys::MouseEvent,
        String,
        String,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
    )>,
    open_folder_actions: Callback<(web_sys::MouseEvent, String, String)>,
    open_capabilities: Callback<web_sys::MouseEvent>,
    open_settings: Callback<web_sys::MouseEvent>,
    open_issue_report: Callback<web_sys::MouseEvent>,
    open_update: Callback<web_sys::MouseEvent>,
    on_sidebar_resize_start: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let SidebarState {
        locale,
        show_sidebar,
        sidebar_w,
        show_proj_menu,
        show_projects,
        demo_mode,
        project_info,
        proj_list,
        sessions,
        explorations,
        folders,
        drag_session,
        drop_target,
        active_session,
        running,
        attention,
        rename_session_input,
        rename_session_target,
        collapsed_folders,
        folder_modal_input,
        folder_modal,
        demos,
        session_history_cursor,
        session_history_loading,
        update_banner,
    } = state;
    let load_older_sessions_button = load_older_sessions.clone();
    let bulk_move_sessions = move_sessions_to.clone();
    let bulk_delete_sessions = delete_sessions.clone();

    // Sidebar-local view state (persisted to localStorage; nothing else reads it,
    // so it stays out of the shared SidebarState). Sort is client-side over the
    // sessions already loaded; group is a pure render toggle over existing data.
    let sort_by = create_rw_signal(load_view_pref(
        "wisp-session-sort",
        "newest",
        &["newest", "name"],
    ));
    let group_by = create_rw_signal(load_view_pref(
        "wisp-session-group",
        "folder",
        &["folder", "date", "none"],
    ));
    let sort_menu_open = create_rw_signal(false);
    // Collapsed nested branch / exploration groups, keyed as "b:{parent}" / "e:{parent}".
    let collapsed_nests = create_rw_signal(HashSet::<String>::new());
    let selecting_sessions = create_rw_signal(false);
    let selected_sessions = create_rw_signal::<HashSet<String>>(HashSet::new());
    let bulk_move_target = create_rw_signal(String::new());
    let new_session_shortcut = if is_mac() { "⌘N" } else { "Ctrl+N" };
    let search_shortcut = if is_mac() { "⌘K" } else { "Ctrl+K" };
    create_effect(move |_| {
        let available: HashSet<String> = sessions.get().into_iter().map(|s| s.id).collect();
        selected_sessions.update(|selected| selected.retain(|id| available.contains(id)));
    });
    window_capture_escape(move || {
        if !sort_menu_open.get_untracked() {
            return false;
        }
        sort_menu_open.set(false);
        true
    });
    // Short windows fold the tail of the nav into a "More" flyout instead of
    // scrolling it. Holds the flyout's viewport (left, top) while open.
    let nav_more_at = create_rw_signal::<Option<(f64, f64)>>(None);
    window_capture_escape(move || {
        if nav_more_at.get_untracked().is_none() {
            return false;
        }
        nav_more_at.set(None);
        true
    });
    let never = Signal::derive(|| false);
    let nav_entries: [NavEntry; 5] = [
        (
            "folder-plus",
            "sidebar.new_folder",
            "nav-tier-2",
            never,
            new_folder,
        ),
        ("doc", "sidebar.files", "nav-tier-2", never, open_files),
        (
            "research-trail",
            "sidebar.graph",
            "nav-tier-1",
            research_journey_open.into(),
            open_research_graph,
        ),
        (
            "book",
            "sidebar.publication",
            "nav-tier-1",
            publication_open.into(),
            open_publication_workspace,
        ),
        ("star", "sidebar.library", "nav-tier-1", never, open_library),
    ];

    view! {
        <aside class="sidebar" class:collapsed=move || !show_sidebar.get()
            style=move || format!("--sidebar-width:{}px", sidebar_w.get())>
            <div class="sidebar-head">
                <button class="side-back" title=move || t(locale.get(), "sidebar.back_projects")
                    aria-label=move || t(locale.get(), "sidebar.back_projects")
                    on:click=move |_| { show_proj_menu.set(false); demo_mode.set(false); show_projects.set(true); }>
                    {compose_icon("arrow-left")}
                </button>
                <button class="proj-switch" class:active=move || show_proj_menu.get()
                    disabled=move || demo_mode.get()
                    title=move || if demo_mode.get() { t(locale.get(), "projects.example").to_string() } else { project_info.get().map(|p| p.name.clone()).unwrap_or_else(|| t(locale.get(), "projects.opening").into()) }
                    on:click=move |ev| toggle_proj_menu.call(ev)>
                    <span class="proj-name">{move || if demo_mode.get() { t(locale.get(), "projects.example").to_string() } else { project_info.get().map(|p| p.name.clone()).unwrap_or_else(|| t(locale.get(), "projects.opening").into()) }}</span>
                </button>
                <button class="icon-btn" title=move || t(locale.get(), "sidebar.collapse") on:click=move |_| show_sidebar.set(false)>{compose_icon("chevron-left")}</button>
            </div>
            {move || (show_proj_menu.get() && !demo_mode.get()).then(|| view! {
                <div class="proj-menu-backdrop" on:click=move |_| show_proj_menu.set(false)></div>
                <div class="proj-menu">
                    <button type="button" class="proj-menu-item" on:click=move |ev| open_proj_settings.call(ev)>
                        {compose_icon("gear")}
                        {move || t(locale.get(), "proj_menu.settings")}
                    </button>
                    <div class="proj-menu-sep"></div>
                    <div class="proj-menu-list">
                        {move || {
                            let active_id = project_info.get().map(|p| p.id.clone()).unwrap_or_default();
                            let dm = demo_mode.get();
                            proj_list.get().into_iter().map(|p| {
                                let is_active = !dm && p.id == active_id;
                                let pid = p.id.clone();
                                let desc = p.description.clone();
                                view! {
                                    <button type="button" class="proj-menu-row" class:active=is_active
                                        on:click=move |_| switch_project.call(pid.clone())>
                                        <span class="pm-text">
                                            <span class="pm-name">{p.name.clone()}</span>
                                            {(!desc.trim().is_empty()).then(|| view! { <span class="pm-desc">{desc.clone()}</span> })}
                                        </span>
                                        {is_active.then(|| view! { <span class="pm-check">{compose_icon("check")}</span> })}
                                    </button>
                                }
                            }).collect_view()
                        }}
                    </div>
                </div>
            })}
            {move || (!demo_mode.get()).then(|| {
                // Fresh per render: this closure rebuilds the nav whenever demo_mode is set.
                let nav_more_ref = create_node_ref::<html::Button>();
                view! {
                    <nav class="nav">
                        <button class="side-btn primary" title=move || t(locale.get(), "sidebar.new_session")
                            aria-label=move || t(locale.get(), "sidebar.new_session")
                            on:click=move |ev| new_session.call(ev)>
                            {compose_icon("plus")}
                            <span class="side-btn-label">{move || t(locale.get(), "sidebar.new_session")}</span>
                            <kbd class="side-shortcut" aria-hidden="true">{new_session_shortcut}</kbd>
                        </button>
                        <button class="side-btn" title=move || t(locale.get(), "sidebar.search_sessions")
                            aria-label=move || t(locale.get(), "sidebar.search_sessions")
                            on:click=move |ev| open_search.call(ev)>
                            {compose_icon("search")}
                            <span class="side-btn-label">{move || t(locale.get(), "sidebar.search")}</span>
                            <kbd class="side-shortcut" aria-hidden="true">{search_shortcut}</kbd>
                        </button>
                        {nav_entries.into_iter().map(|entry| nav_entry_button(locale, entry)).collect_view()}
                        <button type="button" class="side-btn nav-more"
                            class:open=move || nav_more_at.get().is_some()
                            aria-haspopup="menu"
                            aria-expanded=move || nav_more_at.get().is_some().to_string()
                            title=move || t(locale.get(), "sidebar.more")
                            aria-label=move || t(locale.get(), "sidebar.more")
                            node_ref=nav_more_ref
                            // While open, the backdrop covers this button and closes the menu.
                            on:click=move |_| if let Some(el) = nav_more_ref.get_untracked() {
                                let rect = el.get_bounding_client_rect();
                                nav_more_at.set(Some((rect.right() + 6.0, rect.top())));
                            }>
                            {compose_icon("more")}
                            <span class="side-btn-label">{move || t(locale.get(), "sidebar.more")}</span>
                            <span class="nav-more-caret" aria-hidden="true">{compose_icon("chevron-right")}</span>
                        </button>
                    </nav>
                }
            })}
            {move || (!demo_mode.get()).then(|| {
                let loc = locale.get();
                let group_opts = [("none", "sidebar.group_none"), ("folder", "sidebar.group_folder"), ("date", "sidebar.group_date")];
                let sort_opts = [("newest", "sidebar.sort_newest"), ("name", "sidebar.sort_name")];
                view! {
                    <div class="side-sessions-head">
                        <span class="side-sessions-title">{t(loc, "sidebar.sessions")}</span>
                        <div class="side-sessions-head-actions">
                            <button type="button" class="side-select-btn"
                                disabled=move || sessions.get().is_empty()
                                aria-pressed=move || selecting_sessions.get().to_string()
                                on:click=move |_| {
                                    let next = !selecting_sessions.get_untracked();
                                    selecting_sessions.set(next);
                                    selected_sessions.set(HashSet::new());
                                    bulk_move_target.set(String::new());
                                    sort_menu_open.set(false);
                                }>
                                {move || t(locale.get(), if selecting_sessions.get() { "settings.cancel" } else { "sidebar.select_sessions" })}
                            </button>
                            <button type="button" class="icon-btn side-shelved-btn"
                                title=move || t(locale.get(), "session.shelved")
                                aria-label=move || t(locale.get(), "session.shelved")
                                on:click=move |ev| { sort_menu_open.set(false); open_shelved.call(ev); }>
                                {compose_icon("eye-off")}
                            </button>
                            <button type="button" class="icon-btn side-sort-btn"
                                class:active=move || sort_menu_open.get()
                                title=move || t(locale.get(), "sidebar.sort_group")
                                aria-label=move || t(locale.get(), "sidebar.sort_group")
                                on:click=move |ev: web_sys::MouseEvent| {
                                    ev.stop_propagation();
                                    sort_menu_open.update(|v| *v = !*v);
                                }>
                                {compose_icon("adjustments")}
                            </button>
                        </div>
                        {move || sort_menu_open.get().then(|| view! {
                            <div class="side-sort-backdrop" on:click=move |_| sort_menu_open.set(false)></div>
                            <div class="side-sort-menu" role="menu" on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()>
                                <div class="side-sort-label">{t(loc, "sidebar.group_by")}</div>
                                <div class="side-sort-opts">
                                    {group_opts.into_iter().map(|(val, key)| view! {
                                        <button type="button" class="side-sort-opt"
                                            class:active=move || group_by.get().as_str() == val
                                            on:click=move |_| { group_by.set(val.into()); save_view_pref("wisp-session-group", val); sort_menu_open.set(false); }>
                                            <span>{t(loc, key)}</span>
                                            <span class="side-sort-check" class:on=move || group_by.get().as_str() == val>{compose_icon("check")}</span>
                                        </button>
                                    }).collect_view()}
                                </div>
                                <div class="side-sort-sep"></div>
                                <div class="side-sort-label">{t(loc, "sidebar.sort_by")}</div>
                                <div class="side-sort-opts">
                                    {sort_opts.into_iter().map(|(val, key)| view! {
                                        <button type="button" class="side-sort-opt"
                                            class:active=move || sort_by.get().as_str() == val
                                            on:click=move |_| { sort_by.set(val.into()); save_view_pref("wisp-session-sort", val); sort_menu_open.set(false); }>
                                            <span>{t(loc, key)}</span>
                                            <span class="side-sort-check" class:on=move || sort_by.get().as_str() == val>{compose_icon("check")}</span>
                                        </button>
                                    }).collect_view()}
                                </div>
                            </div>
                        })}
                    </div>
                    {move || selecting_sessions.get().then(|| {
                        let count = selected_sessions.get().len();
                        let all_selected = count > 0 && count == sessions.get().len();
                        let move_selected = bulk_move_sessions.clone();
                        let delete_selected = bulk_delete_sessions.clone();
                        view! {
                            <div class="side-bulk-actions" role="toolbar"
                                aria-label=t(loc, "sidebar.bulk_actions")>
                                <div class="side-bulk-summary">
                                    <button type="button" class="side-bulk-link"
                                        on:click=move |_| {
                                            if all_selected {
                                                selected_sessions.set(HashSet::new());
                                            } else {
                                                selected_sessions.set(sessions.get_untracked().into_iter().map(|s| s.id).collect());
                                            }
                                        }>
                                        {t(loc, if all_selected { "sidebar.clear_selection" } else { "sidebar.select_all" })}
                                    </button>
                                    <span>{tf(loc, "sidebar.selected_n", &[("n", &count.to_string())])}</span>
                                </div>
                                <div class="side-bulk-controls">
                                    <select data-testid="bulk-move-sessions"
                                        aria-label=t(loc, "sidebar.move_selected")
                                        disabled=count == 0
                                        prop:value=move || bulk_move_target.get()
                                        on:change=move |ev| {
                                            let target = dom_value(&ev);
                                            if target.is_empty() {
                                                return;
                                            }
                                            let ids: Vec<String> = selected_sessions.get_untracked().into_iter().collect();
                                            let folder_id = (target != "__ungrouped__").then_some(target);
                                            bulk_move_target.set(String::new());
                                            if !ids.is_empty() {
                                                move_selected.call((ids, folder_id));
                                                selected_sessions.set(HashSet::new());
                                                selecting_sessions.set(false);
                                            }
                                        }>
                                        <option value="" disabled=true>{t(loc, "sidebar.move_selected")}</option>
                                        <option value="__ungrouped__">{t(loc, "ctx.move_to_ungrouped")}</option>
                                        {move || folders.get().into_iter().map(|folder| {
                                            let name = if folder.name.trim().is_empty() {
                                                t(locale.get(), "folder.untitled")
                                            } else {
                                                folder.name
                                            };
                                            view! { <option value=folder.id>{name}</option> }
                                        }).collect_view()}
                                    </select>
                                    <button type="button" class="side-bulk-delete"
                                        data-testid="bulk-delete-sessions"
                                        disabled=count == 0
                                        on:click=move |_| {
                                            let ids: Vec<String> = selected_sessions.get_untracked().into_iter().collect();
                                            if !ids.is_empty() {
                                                delete_selected.call(ids);
                                            }
                                        }>
                                        {t(loc, "ctx.delete_session")}
                                    </button>
                                </div>
                            </div>
                        }
                    })}
                }
            })}
            <div class="side-list">
                {move || {
                    let loc = locale.get();
                    // Demo ("Example project") mode: the session list shows the bundled
                    // demos; clicking one renders its read-only transcript via load_demo.
                    if demo_mode.get() {
                        return demos.get().into_iter().map(|d| {
                            let d_click = d.clone();
                            let d_menu = d.clone();
                            let d_actions = d.clone();
                            let open_demo = open_demo_actions.clone();
                            let open_demo_btn = open_demo_actions.clone();
                            view! {
                                <div class="side-item-wrap">
                                    <button class="side-item ses" data-demo-id=d.id.clone() title=d.title.clone()
                                        on:click=move |_| load_demo.call(d_click.clone())
                                        on:contextmenu=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            ev.stop_propagation();
                                            open_demo.call((ev, d_menu.id.clone(), d_menu.title.clone()));
                                        }>
                                        <span class="dot"></span>
                                        <span class="ses-title">{d.title.clone()}</span>
                                    </button>
                                    <button type="button" class="session-actions"
                                        title=move || t(locale.get(), "demo.actions")
                                        aria-label=move || t(locale.get(), "demo.actions")
                                        on:click=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            ev.stop_propagation();
                                            open_demo_btn.call((ev, d_actions.id.clone(), d_actions.title.clone()));
                                        }>{compose_icon("more")}</button>
                                </div>
                            }
                        }).collect_view();
                    }
                    let mut list = sessions.get();
                    let exploration_rows = explorations.get();
                    let folder_list = folders.get();
                    if list.is_empty() && folder_list.is_empty() {
                        return view! { <div class="side-hint">{t(loc, "sidebar.no_sessions")}</div> }.into_view();
                    }
                    // Sort applies to the sessions already loaded into the sidebar;
                    // "load older" pages are appended and re-sorted on the next render.
                    // ponytail: no server-side sort — the loaded set is rarely large
                    // enough for client-side ordering to feel wrong; revisit if it is.
                    if sort_by.get() == "name" {
                        list.sort_by(|a, b| a.title.trim().to_lowercase().cmp(&b.title.trim().to_lowercase()));
                    }
                    // "newest" keeps the backend's latest-activity-first order.
                    // Pinned sessions are pulled out of whatever grouping is active
                    // and shown in one block at the very top. The backend guarantees
                    // they arrive on the first page even when older than the window.
                    let pinned: Vec<SessionInfo> =
                        list.iter().filter(|s| s.pinned).cloned().collect();
                    if !pinned.is_empty() {
                        list.retain(|s| !s.pinned);
                    }
                    // Branch sessions (#531) are lifted out of the flat list and drawn
                    // nested under the session they were forked from, in whatever group
                    // that session lands in.
                    let branch_family_ids = list
                        .iter()
                        .filter_map(|session| {
                            session.branched_from.as_ref().map(|source| {
                                [source.clone(), session.id.clone()]
                            })
                        })
                        .flatten()
                        .collect::<HashSet<_>>();
                    let exploration_source_ids = exploration_rows
                        .iter()
                        .map(|row| row.source_frame_id.clone())
                        .collect::<HashSet<_>>();
                    let (list, branch_kids) = nest_branch_sessions(&list);
                    let group = group_by.get();
                    // Whether any folder exists — used to keep the "ungrouped" drop
                    // zone available (so a session can be dragged out of a folder) without
                    // reading drag_session here, which would rebuild the whole list mid-drag.
                    let has_folders = !folder_list.is_empty();
                    let item = move |s: &SessionInfo| {
                        let is_branch = s.branch_state.is_some();
                        let is_subagent = s.dispatched_from.is_some();
                        let has_branch_family = branch_family_ids.contains(&s.id);
                        let has_exploration_round = exploration_source_ids.contains(&s.id);
                        let id = s.id.clone();
                        let id_active = id.clone();
                        let id_attr = id.clone();
                        let id_running = id.clone();
                        let id_attention = id.clone();
                        let id_drag = id.clone();
                        let id_dragcls = id.clone();
                        let id_selected = id.clone();
                        let id_pressed = id.clone();
                        let id_select_click = id.clone();
                        let title = if s.title.trim().is_empty() {
                            t(loc, "sidebar.untitled").into()
                        } else if is_branch {
                            s.title.strip_prefix("Branch: ").unwrap_or(&s.title).to_string()
                        } else {
                            s.title.clone()
                        };
                        let title_attr = title.clone();
                        let title_tooltip = title.clone();
                        // The compact rail hides titles; the initial keeps rows tellable apart.
                        let initial: String = title.chars().next().into_iter().flat_map(char::to_uppercase).collect();
                        let open = load_session.clone();
                        let id_click = id.clone();
                        let id_key = id.clone();
                        let id_rename = id.clone();
                        let title_rename = title.clone();
                        let id_actions = id.clone();
                        let title_actions = title.clone();
                        let show_actions = open_session_actions.clone();
                        let pinned = s.pinned;
                        let stale_prompt = s.stale_prompt;
                        let branch_merged = matches!(
                            s.branch_state.as_deref(),
                            Some("merged" | "orphaned")
                        );
                        view! {
                            <div class="side-item-wrap">
                                <button type="button" class="side-item ses"
                                    class:pinned=pinned
                                    class:branch-session=is_branch
                                    title=title_tooltip
                                    class:active=move || active_session.get().as_deref() == Some(id_active.as_str())
                                    class:running=move || running.get().contains(&id_running)
                                    class:attention=move || attention.get().contains(&id_attention)
                                    class:dragging=move || drag_session.get().as_deref() == Some(id_dragcls.as_str())
                                    class:selecting=move || selecting_sessions.get()
                                    class:selected=move || selected_sessions.get().contains(&id_selected)
                                    attr:aria-pressed=move || selecting_sessions.get().then(|| {
                                        selected_sessions.get().contains(&id_pressed).to_string()
                                    })
                                    attr:draggable=move || if selecting_sessions.get() { "false" } else { "true" }
                                    data-session-id=id_attr
                                    data-session-title=title_attr
                                    data-session-pinned=if pinned { "true" } else { "false" }
                                    data-session-stale=if stale_prompt { "true" } else { "false" }
                                    data-session-branch=if is_branch { "true" } else { "false" }
                                    data-session-subagent=if is_subagent { "true" } else { "false" }
                                    data-session-family=if has_branch_family { "true" } else { "false" }
                                    data-exploration-round=if has_exploration_round { "true" } else { "false" }
                                    data-branch-merged=if branch_merged { "true" } else { "false" }
                                    on:click=move |_| {
                                        if selecting_sessions.get_untracked() {
                                            selected_sessions.update(|selected| {
                                                if !selected.insert(id_select_click.clone()) {
                                                    selected.remove(&id_select_click);
                                                }
                                            });
                                        } else {
                                            open.call(id_key.clone());
                                        }
                                    }
                                    on:dblclick=move |ev: web_sys::MouseEvent| {
                                        if selecting_sessions.get_untracked() {
                                            return;
                                        }
                                        ev.prevent_default();
                                        ev.stop_propagation();
                                        rename_session_input.set(title_rename.clone());
                                        rename_session_target.set(Some((id_rename.clone(), title_rename.clone())));
                                    }
                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                        if !selecting_sessions.get_untracked() && (ev.key() == "Enter" || ev.key() == " ") {
                                            ev.prevent_default();
                                            open.call(id_click.clone());
                                        }
                                    }
                                    on:dragstart=move |ev: web_sys::DragEvent| {
                                        if selecting_sessions.get_untracked() {
                                            ev.prevent_default();
                                            return;
                                        }
                                        start_session_drag(&ev, &id_drag);
                                        drag_session.set(Some(id_drag.clone()));
                                    }
                                    on:dragend=move |_| {
                                        drag_session.set(None);
                                        drop_target.set(None);
                                    }>
                                    <span class="session-select-mark" aria-hidden="true">{compose_icon("check")}</span>
                                    <span class="ses-status" aria-hidden="true">
                                        <span class="ses-live" title=move || t(locale.get(), "sess_status.running")>
                                            {compose_icon("loader")}
                                        </span>
                                        <span class="ses-attention" title=move || t(locale.get(), "sess_status.needs_you")>
                                            {compose_icon("circle-alert")}
                                        </span>
                                    </span>
                                    <span class="ses-initial" aria-hidden="true">{initial}</span>
                                    {is_branch.then(|| view! {
                                        <span class="session-branch-icon" aria-hidden="true">{compose_icon("branch")}</span>
                                    })}
                                    {is_subagent.then(|| view! {
                                        <span class="session-branch-icon" aria-hidden="true">{compose_icon("eye")}</span>
                                    })}
                                    <span class="ses-title">{title}</span>
                                </button>
                                <button type="button" class="session-actions"
                                    class:selection-hidden=move || selecting_sessions.get()
                                    disabled=move || selecting_sessions.get()
                                    title=move || t(locale.get(), "session.actions")
                                    aria-label=move || t(locale.get(), "session.actions")
                                    on:click=move |ev: web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        ev.stop_propagation();
                                        show_actions.call((ev, id_actions.clone(), title_actions.clone(), pinned, is_branch, branch_merged, has_branch_family, has_exploration_round, stale_prompt));
                                    }>{compose_icon("more")}</button>
                            </div>
                        }.into_view()
                    };
                    let exploration_groups = exploration_rows.into_iter().fold(
                        HashMap::<String, Vec<ExplorationSummary>>::new(),
                        |mut groups, row| {
                            groups.entry(row.source_frame_id.clone()).or_default().push(row);
                            groups
                        },
                    );
                    let exploration_item = move |summary: &ExplorationSummary| {
                        let exploration = summary.exploration.clone();
                        let exploration_for_open = exploration.clone();
                        let exploration_id_for_actions = exploration.id.clone();
                        let exploration_status_for_actions = exploration.status.clone();
                        let frame_id = exploration.frame_id.clone();
                        let title = exploration.name.clone();
                        let status_key = match exploration.status.as_str() {
                            "active" => "exploration.status_active",
                            "promoting" => "exploration.status_promoting",
                            "creating" => "exploration.status_creating",
                            _ => "exploration.status_failed",
                        };
                        let isolation_key = if summary.isolation_is_full() {
                            "exploration.isolation_full"
                        } else {
                            "exploration.isolation_partial"
                        };
                        let open = open_exploration.clone();
                        view! {
                            <button type="button" class="side-exploration"
                                class:active=move || active_session.get().as_deref() == Some(frame_id.as_str())
                                data-exploration-id=exploration.id.clone()
                                data-exploration-status=exploration.status.clone()
                                title=title.clone()
                                on:contextmenu=move |ev: web_sys::MouseEvent| {
                                    ev.prevent_default();
                                    ev.stop_propagation();
                                    open_exploration_actions.call((
                                        ev,
                                        exploration_id_for_actions.clone(),
                                        exploration_status_for_actions.clone(),
                                    ));
                                }
                                on:click=move |_| open.call(exploration_for_open.clone())>
                                <span class="exploration-kind-icon" aria-hidden="true">{compose_icon("flask")}</span>
                                <span class="side-exploration-copy">
                                    <span class="side-exploration-title">{title}</span>
                                    <span class="side-exploration-meta">
                                        <span>{t(loc, status_key)}</span>
                                        <span>" · "</span>
                                        <span>{t(loc, isolation_key)}</span>
                                    </span>
                                </span>
                            </button>
                        }.into_view()
                    };
                    let make = move |s: &SessionInfo| {
                        // Subagents (#1061) nest like branches but under their own heading.
                        let (subagent_kids, kids): (Vec<SessionInfo>, Vec<SessionInfo>) = branch_kids
                            .get(&s.id)
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .partition(|kid| kid.dispatched_from.is_some());
                        let exploration_kids = exploration_groups.get(&s.id).cloned().unwrap_or_default();
                        if kids.is_empty() && subagent_kids.is_empty() && exploration_kids.is_empty() {
                            return item(s);
                        }
                        let nests = collapsed_nests.get();
                        let exploration_key = format!("e:{}", s.id);
                        let branch_key = format!("b:{}", s.id);
                        let subagent_key = format!("s:{}", s.id);
                        let subagent_collapsed = nests.contains(&subagent_key);
                        let subagent_count = subagent_kids.len();
                        let exploration_collapsed = nests.contains(&exploration_key);
                        let branch_collapsed = nests.contains(&branch_key);
                        let exploration_count = exploration_kids.len();
                        let branch_count = kids.len();
                        let exploration_toggle_key = exploration_key.clone();
                        let branch_toggle_key = branch_key.clone();
                        let parent = item(s);
                        view! {
                            <div class="side-branch-group">
                                {parent}
                                {(!exploration_kids.is_empty()).then(|| {
                                    let exploration_toggle_key = exploration_toggle_key.clone();
                                    view! {
                                    <div class="side-exploration-group" data-testid="sidebar-explorations">
                                        {nest_group_toggle(
                                            loc,
                                            "exploration.group",
                                            "exploration.expand",
                                            "exploration.collapse",
                                            exploration_count,
                                            exploration_collapsed,
                                            "sidebar-exploration-toggle",
                                            move |ev: web_sys::MouseEvent| {
                                                ev.prevent_default();
                                                ev.stop_propagation();
                                                collapsed_nests.update(|set| {
                                                    toggle_collapsed_key(set, exploration_toggle_key.clone());
                                                });
                                            },
                                        )}
                                        {(!exploration_collapsed).then(|| view! {
                                            <div class="side-nest-items">
                                                {exploration_kids.iter().map(&exploration_item).collect_view()}
                                            </div>
                                        })}
                                    </div>
                                    }
                                })}
                                {(!kids.is_empty()).then(|| {
                                    let branch_toggle_key = branch_toggle_key.clone();
                                    view! {
                                    <div class="side-branch-kids" data-testid="sidebar-branches">
                                        {nest_group_toggle(
                                            loc,
                                            "branch.group",
                                            "branch.expand",
                                            "branch.collapse",
                                            branch_count,
                                            branch_collapsed,
                                            "sidebar-branch-toggle",
                                            move |ev: web_sys::MouseEvent| {
                                                ev.prevent_default();
                                                ev.stop_propagation();
                                                collapsed_nests.update(|set| {
                                                    toggle_collapsed_key(set, branch_toggle_key.clone());
                                                });
                                            },
                                        )}
                                        {(!branch_collapsed).then(|| view! {
                                            <div class="side-nest-items">
                                                {kids.iter().map(&item).collect_view()}
                                            </div>
                                        })}
                                    </div>
                                    }
                                })}
                                {(!subagent_kids.is_empty()).then(|| {
                                    let subagent_key = subagent_key.clone();
                                    view! {
                                    <div class="side-branch-kids" data-testid="sidebar-subagents">
                                        {nest_group_toggle(
                                            loc,
                                            "subagent.group",
                                            "subagent.expand",
                                            "subagent.collapse",
                                            subagent_count,
                                            subagent_collapsed,
                                            "sidebar-subagent-toggle",
                                            move |ev: web_sys::MouseEvent| {
                                                ev.prevent_default();
                                                ev.stop_propagation();
                                                collapsed_nests.update(|set| {
                                                    toggle_collapsed_key(set, subagent_key.clone());
                                                });
                                            },
                                        )}
                                        {(!subagent_collapsed).then(|| view! {
                                            <div class="side-nest-items">
                                                {subagent_kids.iter().map(&item).collect_view()}
                                            </div>
                                        })}
                                    </div>
                                    }
                                })}
                            </div>
                        }.into_view()
                    };
                    let pinned_view = (!pinned.is_empty()).then(|| view! {
                        <div class="side-group-title">{t(loc, "sidebar.pinned")}</div>
                        {pinned.iter().map(&make).collect_view()}
                    });
                    // "None": one flat, sorted list, no folder blocks or date headers.
                    if group == "none" {
                        return view! { {pinned_view}{list.iter().map(&make).collect_view()} }.into_view();
                    }
                    // "Date": folders hidden, every session bucketed by date. Folder
                    // membership stays in the DB — switch to "Folders" to drag/reassign.
                    if group == "date" {
                        let (today, earlier) = bucket_sessions_by_date(&list);
                        return view! {
                            {pinned_view}
                            {(!today.is_empty()).then(|| view! {
                                <div class="side-group-title">{t(loc, "sidebar.today")}</div>
                                {today.iter().map(&make).collect_view()}
                            })}
                            {(!earlier.is_empty()).then(|| view! {
                                <div class="side-group-title">{t(loc, "sidebar.earlier")}</div>
                                {earlier.iter().map(&make).collect_view()}
                            })}
                        }.into_view();
                    }
                    // "Folders" (default): folder blocks + ungrouped sessions by date.
                    let ungrouped: Vec<SessionInfo> = list.iter()
                        .filter(|s| s.folder_id.is_none())
                        .cloned()
                        .collect();
                    let (today, earlier) = bucket_sessions_by_date(&ungrouped);
                    let move_to = move_sessions_to.clone();
                    let folder_views = folder_list.into_iter().map(|f| {
                        let fid = f.id.clone();
                        let fid_toggle = fid.clone();
                        let fid_drop = fid.clone();
                        let fid_target = format!("folder:{fid_drop}");
                        let fid_target_over = fid_target.clone();
                        let fname = if f.name.trim().is_empty() {
                            t(loc, "folder.untitled").into()
                        } else {
                            f.name.clone()
                        };
                        let fname_attr = fname.clone();
                        let collapsed = collapsed_folders.get().contains(&fid_toggle);
                        let in_folder: Vec<SessionInfo> = list.iter()
                            .filter(|s| s.folder_id.as_deref() == Some(fid.as_str()))
                            .cloned()
                            .collect();
                        let fid_target_cls = fid_target.clone();
                        let fid_target_over_enter = fid_target_over.clone();
                        let fid_rename = fid.clone();
                        let fname_rename = fname.clone();
                        let fid_actions = fid.clone();
                        let fname_actions = fname.clone();
                        let show_folder_actions = open_folder_actions.clone();
                        view! {
                            <div class="side-folder-wrap"
                                class:drop-target=move || drop_target.get().as_deref() == Some(fid_target_cls.as_str())
                                data-folder-id=fid.clone()
                                on:dragenter=move |ev: web_sys::DragEvent| {
                                    allow_drop(&ev);
                                    if drop_target.get().as_deref() != Some(fid_target_over_enter.as_str()) {
                                        drop_target.set(Some(fid_target_over_enter.clone()));
                                    }
                                }
                                on:dragover=move |ev: web_sys::DragEvent| {
                                    allow_drop(&ev);
                                    if drop_target.get().as_deref() != Some(fid_target_over.as_str()) {
                                        drop_target.set(Some(fid_target_over.clone()));
                                    }
                                }
                                on:drop=move |ev: web_sys::DragEvent| {
                                    ev.prevent_default();
                                    ev.stop_propagation();
                                    let sid = drag_session_id(&ev, drag_session.get());
                                    drag_session.set(None);
                                    drop_target.set(None);
                                    if let Some(id) = sid {
                                        move_to.call((vec![id], Some(fid_drop.clone())));
                                    }
                                }>
                                <div class="side-folder"
                                    title=fname_attr.clone()
                                    data-folder-id=fid.clone()
                                    data-folder-name=fname_attr
                                    on:click=move |_| {
                                        collapsed_folders.update(|set| {
                                            if set.contains(&fid_toggle) { set.remove(&fid_toggle); }
                                            else { set.insert(fid_toggle.clone()); }
                                        });
                                    }
                                    on:dblclick=move |ev: web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        ev.stop_propagation();
                                        folder_modal_input.set(fname_rename.clone());
                                        folder_modal.set(Some(FolderModal::Rename(fid_rename.clone())));
                                    }>
                                    <span class="side-folder-caret" class:collapsed=collapsed>{compose_icon("chevron-down")}</span>
                                    {compose_icon("folder")}
                                    <span class="side-folder-name">{fname}</span>
                                    <span class="side-folder-count">{in_folder.len()}</span>
                                    <button type="button" class="folder-actions"
                                        title=move || t(locale.get(), "folder.actions")
                                        aria-label=move || t(locale.get(), "folder.actions")
                                        on:click=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            ev.stop_propagation();
                                            show_folder_actions.call((ev, fid_actions.clone(), fname_actions.clone()));
                                        }>{compose_icon("more")}</button>
                                </div>
                                {(!collapsed).then(|| view! {
                                    <div class="side-folder-sessions">
                                        {in_folder.iter().map(&make).collect_view()}
                                    </div>
                                })}
                            </div>
                        }
                    }).collect_view();
                    view! {
                        {pinned_view}
                        {folder_views}
                        {( !ungrouped.is_empty() || has_folders ).then(|| view! {
                            <div class="side-ungrouped"
                                class:drop-target=move || drop_target.get().as_deref() == Some("ungrouped")
                                on:dragenter=move |ev: web_sys::DragEvent| {
                                    allow_drop(&ev);
                                    if drop_target.get().as_deref() != Some("ungrouped") {
                                        drop_target.set(Some("ungrouped".into()));
                                    }
                                }
                                on:dragover=move |ev: web_sys::DragEvent| {
                                    allow_drop(&ev);
                                    if drop_target.get().as_deref() != Some("ungrouped") {
                                        drop_target.set(Some("ungrouped".into()));
                                    }
                                }
                                on:drop=move |ev: web_sys::DragEvent| {
                                    ev.prevent_default();
                                    ev.stop_propagation();
                                    let sid = drag_session_id(&ev, drag_session.get());
                                    drag_session.set(None);
                                    drop_target.set(None);
                                    if let Some(id) = sid {
                                        move_to.call((vec![id], None));
                                    }
                                }>
                                {(!today.is_empty()).then(|| view! {
                                    <div class="side-group-title">{t(loc, "sidebar.today")}</div>
                                    {today.iter().map(&make).collect_view()}
                                })}
                                {(!earlier.is_empty()).then(|| view! {
                                    <div class="side-group-title">{t(loc, "sidebar.earlier")}</div>
                                    {earlier.iter().map(&make).collect_view()}
                                })}
                            </div>
                        })}
                    }.into_view()
                }}
            </div>
            {move || {
                let load_older = load_older_sessions_button.clone();
                (!demo_mode.get() && session_history_cursor.get().is_some()).then(|| view! {
                    <button type="button" class="side-load-more"
                        disabled=move || session_history_loading.get()
                        on:click=move |_| load_older.call(())>
                        {move || t(locale.get(), if session_history_loading.get() { "loading" } else { "sidebar.load_older" })}
                    </button>
                })
            }}
            {move || (!demo_mode.get()).then(|| view! {
                <div class="side-foot">
                    {move || update_banner.get().map(|u| view! {
                        <button type="button" class="update-card" data-testid="update-card"
                            title=move || t(locale.get(), "update_card.title")
                            on:click=move |ev| open_update.call(ev)>
                            <span class="update-card-icon" aria-hidden="true">{compose_icon("download")}</span>
                            <span class="update-card-text">
                                <span class="update-card-title">{move || t(locale.get(), "update_card.title")}</span>
                                <span class="update-card-ver">{format!("v{}", u.version)}</span>
                            </span>
                            <span class="update-card-arrow" aria-hidden="true">{compose_icon("arrow-right")}</span>
                        </button>
                    })}
                    {move || project_info.get().map(|p| {
                        let loc = locale.get();
                        view! {
                        <div class="proj-meta">
                            <span>{tf(loc, "sidebar.skills_meta", &[
                                ("skills", &p.skill_count.to_string()),
                                ("mcp", &p.mcp_server_count.to_string()),
                                ("mem", &p.memory_file_count.to_string()),
                            ])}</span>
                        </div>
                    }})}
                    <button class="side-btn" title=move || t(locale.get(), "sidebar.capabilities") on:click=move |ev| open_capabilities.call(ev)>{compose_icon("grid")}<span class="side-btn-label">{move || t(locale.get(), "sidebar.capabilities")}</span></button>
                    <button class="side-btn" data-testid="report-problem-entry" title=move || t(locale.get(), "issue_report.sidebar") on:click=move |ev| open_issue_report.call(ev)>{compose_icon("chat")}<span class="side-btn-label">{move || t(locale.get(), "issue_report.sidebar")}</span></button>
                    <button class="side-btn" title=move || t(locale.get(), "sidebar.settings") on:click=move |ev| open_settings.call(ev)>{compose_icon("gear")}<span class="side-btn-label">{move || t(locale.get(), "sidebar.settings")}</span></button>
                </div>
            })}
        </aside>
        // Outside the aside so the compact rail's `.sidebar .side-btn-label`
        // hiding does not reach the flyout's labels.
        {move || nav_more_at.get().map(|(left, top)| view! {
            <div class="nav-more-backdrop" on:click=move |_| nav_more_at.set(None)></div>
            <div class="nav-more-menu" role="menu" data-testid="sidebar-more-menu"
                style=format!("left:{left}px;top:{top}px")
                on:click=move |_| nav_more_at.set(None)>
                {nav_entries.into_iter().map(|entry| nav_entry_button(locale, entry)).collect_view()}
            </div>
        })}
        {move || show_sidebar.get().then(|| view! {
            <div class="sidebar-resizer" on:mousedown=move |ev| on_sidebar_resize_start.call(ev)></div>
        })}
    }
}
