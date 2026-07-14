use crate::app_support::{
    allow_drop, bucket_sessions_by_date, compose_icon, drag_session_id, start_session_drag,
    FolderModal,
};
use crate::dto::*;
use crate::i18n::{t, tf, Locale};
use leptos::*;
use std::collections::HashSet;

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
    pub(super) folders: RwSignal<Vec<FolderInfo>>,
    pub(super) drag_session: RwSignal<Option<String>>,
    pub(super) drop_target: RwSignal<Option<String>>,
    pub(super) active_session: RwSignal<Option<String>>,
    pub(super) running: RwSignal<HashSet<String>>,
    pub(super) rename_session_input: RwSignal<String>,
    pub(super) rename_session_target: RwSignal<Option<(String, String)>>,
    pub(super) collapsed_folders: RwSignal<HashSet<String>>,
    pub(super) folder_modal_input: RwSignal<String>,
    pub(super) folder_modal: RwSignal<Option<FolderModal>>,
    pub(super) demos: RwSignal<Vec<DemoInfo>>,
}

#[component]
pub(super) fn Sidebar(
    state: SidebarState,
    toggle_proj_menu: Callback<web_sys::MouseEvent>,
    open_proj_settings: Callback<web_sys::MouseEvent>,
    switch_project: Callback<String>,
    new_session: Callback<web_sys::MouseEvent>,
    new_folder: Callback<web_sys::MouseEvent>,
    open_files: Callback<web_sys::MouseEvent>,
    open_library: Callback<web_sys::MouseEvent>,
    load_demo: Callback<DemoInfo>,
    load_session: Callback<String>,
    move_session_to: Callback<(String, Option<String>)>,
    open_session_actions: Callback<(web_sys::MouseEvent, String, String)>,
    open_folder_actions: Callback<(web_sys::MouseEvent, String, String)>,
    open_capabilities: Callback<web_sys::MouseEvent>,
    open_settings: Callback<web_sys::MouseEvent>,
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
        folders,
        drag_session,
        drop_target,
        active_session,
        running,
        rename_session_input,
        rename_session_target,
        collapsed_folders,
        folder_modal_input,
        folder_modal,
        demos,
    } = state;

    view! {
        <aside class="sidebar" class:collapsed=move || !show_sidebar.get()
            style=move || format!("--sidebar-width:{}px", sidebar_w.get())>
            <div class="sidebar-head">
                <button class="side-back" title=move || t(locale.get(), "sidebar.back_projects")
                    aria-label=move || t(locale.get(), "sidebar.back_projects")
                    on:click=move |_| { show_proj_menu.set(false); demo_mode.set(false); show_projects.set(true); }>
                    <span class="gi back" aria-hidden="true"></span>
                </button>
                <button class="proj-switch" class:active=move || show_proj_menu.get()
                    title=move || if demo_mode.get() { t(locale.get(), "projects.example").to_string() } else { project_info.get().map(|p| p.name.clone()).unwrap_or_else(|| "wisp-science".into()) }
                    on:click=move |ev| toggle_proj_menu.call(ev)>
                    <span class="proj-name">{move || if demo_mode.get() { t(locale.get(), "projects.example").to_string() } else { project_info.get().map(|p| p.name.clone()).unwrap_or_else(|| "wisp-science".into()) }}</span>
                    <span class="caret">"▾"</span>
                </button>
                <button class="icon-btn" title=move || t(locale.get(), "sidebar.collapse") on:click=move |_| show_sidebar.set(false)>{compose_icon("chevron-left")}</button>
            </div>
            {move || show_proj_menu.get().then(|| view! {
                <div class="proj-menu-backdrop" on:click=move |_| show_proj_menu.set(false)></div>
                <div class="proj-menu">
                    <button type="button" class="proj-menu-item" on:click=move |ev| open_proj_settings.call(ev)>
                        <span class="gi gear"></span>
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
                                        {is_active.then(|| view! { <span class="pm-check">"✓"</span> })}
                                    </button>
                                }
                            }).collect_view()
                        }}
                    </div>
                </div>
            })}
            <nav class="nav">
                <button class="side-btn primary" title=move || t(locale.get(), "sidebar.new_session") on:click=move |ev| new_session.call(ev)><span class="gi plus"></span>{move || t(locale.get(), "sidebar.new_session")}</button>
                <button class="side-btn" title=move || t(locale.get(), "sidebar.new_folder") on:click=move |ev| new_folder.call(ev)><span class="gi folder"></span>{move || t(locale.get(), "sidebar.new_folder")}</button>
                <button class="side-btn" title=move || t(locale.get(), "sidebar.files") on:click=move |ev| open_files.call(ev)><span class="gi doc"></span>{move || t(locale.get(), "sidebar.files")}</button>
                <button class="side-btn" title=move || t(locale.get(), "sidebar.library") on:click=move |ev| open_library.call(ev)>{compose_icon("star")}{move || t(locale.get(), "sidebar.library")}</button>
            </nav>
            <div class="side-list">
                {move || {
                    let loc = locale.get();
                    // Demo ("Example project") mode: the session list shows the bundled
                    // demos; clicking one renders its read-only transcript via load_demo.
                    if demo_mode.get() {
                        return demos.get().into_iter().map(|d| {
                            let d_click = d.clone();
                            view! {
                                <button class="side-item ses" title=d.title.clone() on:click=move |_| load_demo.call(d_click.clone())>
                                    <span class="dot"></span>
                                    <span class="ses-title">{d.title.clone()}</span>
                                </button>
                            }
                        }).collect_view();
                    }
                    let list = sessions.get();
                    let folder_list = folders.get();
                    if list.is_empty() && folder_list.is_empty() {
                        return view! { <div class="side-hint">{t(loc, "sidebar.no_sessions")}</div> }.into_view();
                    }
                    let dragging = drag_session.get();
                    let dragging_for_make = dragging.clone();
                    let make = move |s: &SessionInfo| {
                        let id = s.id.clone();
                        let id_active = id.clone();
                        let id_attr = id.clone();
                        let id_running = id.clone();
                        let id_drag = id.clone();
                        let title = if s.title.trim().is_empty() { t(loc, "sidebar.untitled").into() } else { s.title.clone() };
                        let title_attr = title.clone();
                        let title_tooltip = title.clone();
                        let open = load_session.clone();
                        let is_dragging = dragging_for_make.as_deref() == Some(id_drag.as_str());
                        let id_click = id.clone();
                        let id_key = id.clone();
                        let id_rename = id.clone();
                        let title_rename = title.clone();
                        let id_actions = id.clone();
                        let title_actions = title.clone();
                        let show_actions = open_session_actions.clone();
                        view! {
                            <div class="side-item-wrap">
                                <button type="button" class="side-item ses"
                                    title=title_tooltip
                                    class:active=move || active_session.get().as_deref() == Some(id_active.as_str())
                                    class:running=move || running.get().contains(&id_running)
                                    class:dragging=is_dragging
                                    attr:draggable="true"
                                    data-session-id=id_attr
                                    data-session-title=title_attr
                                    on:click=move |_| {
                                        open.call(id_key.clone());
                                    }
                                    on:dblclick=move |ev: web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        ev.stop_propagation();
                                        rename_session_input.set(title_rename.clone());
                                        rename_session_target.set(Some((id_rename.clone(), title_rename.clone())));
                                    }
                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                        if ev.key() == "Enter" || ev.key() == " " {
                                            ev.prevent_default();
                                            open.call(id_click.clone());
                                        }
                                    }
                                    on:dragstart=move |ev: web_sys::DragEvent| {
                                        start_session_drag(&ev, &id_drag);
                                        drag_session.set(Some(id_drag.clone()));
                                    }
                                    on:dragend=move |_| {
                                        drag_session.set(None);
                                        drop_target.set(None);
                                    }>
                                    <span class="dot"></span>
                                    <span class="ses-title">{title}</span>
                                </button>
                                <button type="button" class="session-actions"
                                    title=move || t(locale.get(), "session.actions")
                                    aria-label=move || t(locale.get(), "session.actions")
                                    on:click=move |ev: web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        ev.stop_propagation();
                                        show_actions.call((ev, id_actions.clone(), title_actions.clone()));
                                    }>"⋯"</button>
                            </div>
                        }.into_view()
                    };
                    let ungrouped: Vec<SessionInfo> = list.iter()
                        .filter(|s| s.folder_id.is_none())
                        .cloned()
                        .collect();
                    let (today, earlier) = bucket_sessions_by_date(&ungrouped);
                    let target = drop_target.get();
                    let move_to = move_session_to.clone();
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
                        let is_target = target.as_deref() == Some(fid_target.as_str());
                        let fid_target_over_enter = fid_target_over.clone();
                        let fid_rename = fid.clone();
                        let fname_rename = fname.clone();
                        let fid_actions = fid.clone();
                        let fname_actions = fname.clone();
                        let show_folder_actions = open_folder_actions.clone();
                        view! {
                            <div class="side-folder-wrap"
                                class:drop-target=is_target
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
                                        move_to.call((id, Some(fid_drop.clone())));
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
                                    <span class="side-folder-caret" class:collapsed=collapsed>"▾"</span>
                                    <span class="gi folder"></span>
                                    <span class="side-folder-name">{fname}</span>
                                    <span class="side-folder-count">{in_folder.len()}</span>
                                    <button type="button" class="folder-actions"
                                        title=move || t(locale.get(), "folder.actions")
                                        aria-label=move || t(locale.get(), "folder.actions")
                                        on:click=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            ev.stop_propagation();
                                            show_folder_actions.call((ev, fid_actions.clone(), fname_actions.clone()));
                                        }>"⋯"</button>
                                </div>
                                {(!collapsed).then(|| view! {
                                    <div class="side-folder-sessions">
                                        {in_folder.iter().map(&make).collect_view()}
                                    </div>
                                })}
                            </div>
                        }
                    }).collect_view();
                    let ungrouped_target = target.as_deref() == Some("ungrouped");
                    view! {
                        {folder_views}
                        {( !ungrouped.is_empty() || dragging.is_some() ).then(|| view! {
                            <div class="side-ungrouped"
                                class:drop-target=ungrouped_target
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
                                        move_to.call((id, None));
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
            <div class="side-foot">
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
                <button class="side-btn" title=move || t(locale.get(), "sidebar.capabilities") on:click=move |ev| open_capabilities.call(ev)><span class="gi grid"></span>{move || t(locale.get(), "sidebar.capabilities")}</button>
                <button class="side-btn" title=move || t(locale.get(), "sidebar.settings") on:click=move |ev| open_settings.call(ev)><span class="gi gear"></span>{move || t(locale.get(), "sidebar.settings")}</button>
            </div>
        </aside>
        {move || show_sidebar.get().then(|| view! {
            <div class="sidebar-resizer" on:mousedown=move |ev| on_sidebar_resize_start.call(ev)></div>
        })}
    }
}
