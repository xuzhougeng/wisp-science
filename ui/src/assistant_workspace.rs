//! Project context and collapsible sidebars for the assistant's one conversation.
use crate::app_support::compose_icon;
use crate::dto::{ProjectSummary, ProjectTransferProgress, ResearchAssistantPlanItem};
use crate::i18n::Locale;
use crate::research_journey::{call, day_key, j};
use leptos::*;
use wasm_bindgen::JsCast;

const PROJECTS_KEY: &str = "wisp-assistant-projects-visible";
const CALENDAR_KEY: &str = "wisp-assistant-calendar-visible";

fn narrow_window() -> bool {
    window()
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(1280.0)
        < 960.0
}
fn preference(key: &str) -> bool {
    window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .is_none_or(|value| value != "0")
}
fn save_preference(key: &str, visible: bool) {
    if let Some(storage) = window().local_storage().ok().flatten() {
        let _ = storage.set_item(key, if visible { "1" } else { "0" });
    }
}

#[derive(Clone, Copy)]
pub(crate) struct AssistantWorkspaceState {
    pub left: RwSignal<bool>,
    pub right: RwSignal<bool>,
    pub narrow: RwSignal<bool>,
    pub last_left: RwSignal<bool>,
    pub selected: RwSignal<Option<String>>,
    pub projects: Signal<Vec<ProjectSummary>>,
    pub loading: Signal<bool>,
    pub error: Signal<Option<String>>,
    pub refresh: RwSignal<u32>,
}

impl AssistantWorkspaceState {
    pub fn toggle(self, left: bool) {
        let signal = if left { self.left } else { self.right };
        let visible = !signal.get_untracked();
        signal.set(visible);
        if visible {
            self.last_left.set(left);
        }
        if !self.narrow.get_untracked() {
            save_preference(if left { PROJECTS_KEY } else { CALENDAR_KEY }, visible);
        }
        if !visible {
            focus_toggle(left);
        }
    }

    pub fn close_drawer(self) -> bool {
        if !self.narrow.get_untracked() {
            return false;
        }
        let left = self.left.get_untracked();
        let right = self.right.get_untracked();
        if !left && !right {
            return false;
        }
        if left && (!right || self.last_left.get_untracked()) {
            self.left.set(false);
            focus_toggle(true);
        } else {
            self.right.set(false);
            focus_toggle(false);
        }
        true
    }

    pub fn selected_project(self) -> Option<ProjectSummary> {
        let id = self.selected.get()?;
        self.projects
            .get()
            .into_iter()
            .find(|project| project.id == id)
    }
}

fn focus_toggle(left: bool) {
    if let Some(button) = document()
        .get_element_by_id(if left {
            "assistant-projects-toggle"
        } else {
            "assistant-calendar-toggle"
        })
        .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = button.focus();
    }
}

pub(crate) fn use_assistant_workspace(
    active: RwSignal<bool>,
    busy: Signal<bool>,
    hidden_projects: Signal<Vec<String>>,
) -> AssistantWorkspaceState {
    let narrow = create_rw_signal(narrow_window());
    let left = create_rw_signal(!narrow.get_untracked() && preference(PROJECTS_KEY));
    let right = create_rw_signal(!narrow.get_untracked() && preference(CALENDAR_KEY));
    let selected = create_rw_signal(None::<String>);
    let refresh = create_rw_signal(0u32);
    let last_busy = create_rw_signal(false);
    create_effect(move |_| {
        let working = active.get() && busy.get();
        if active.get() && last_busy.get_untracked() && !working {
            refresh.update(|value| *value = value.wrapping_add(1));
        }
        last_busy.set(working);
    });
    let key = move || (active.get(), refresh.get(), hidden_projects.get());
    let data = create_local_resource(key, move |key| async move {
        let result = if key.0 {
            call::<Vec<ProjectSummary>>("get_research_assistant_projects", serde_json::json!({}))
                .await
        } else {
            Ok(Vec::new())
        };
        (key, result)
    });
    let loading = Signal::derive(move || {
        active.get()
            && (data.loading().get() || data.get().is_none_or(|(loaded, _)| loaded != key()))
    });
    let error = Signal::derive(move || {
        if loading.get() {
            None
        } else {
            data.get().and_then(|(_, result)| result.err())
        }
    });
    // Do not let the calendar read stale or privacy-hidden IDs while the
    // authoritative project/permission read is pending or failed.
    let projects = Signal::derive(move || {
        if !active.get() || loading.get() {
            return Vec::new();
        }
        let hidden = hidden_projects.get();
        data.get()
            .and_then(|(_, result)| result.ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|project| !hidden.contains(&project.id))
            .collect::<Vec<_>>()
    });
    create_effect(move |_| {
        if active.get() && !loading.get() && error.get().is_none() {
            if selected
                .get()
                .is_some_and(|id| !projects.get().iter().any(|project| project.id == id))
            {
                selected.set(None);
            }
        }
    });
    let resize = window_event_listener(ev::resize, move |_| {
        let next = narrow_window();
        if next == narrow.get_untracked() {
            return;
        }
        narrow.set(next);
        left.set(!next && preference(PROJECTS_KEY));
        right.set(!next && preference(CALENDAR_KEY));
    });
    on_cleanup(move || resize.remove());
    AssistantWorkspaceState {
        left,
        right,
        narrow,
        last_left: create_rw_signal(false),
        selected,
        projects,
        loading,
        error,
        refresh,
    }
}

#[component]
pub(crate) fn AssistantHeader(
    locale: RwSignal<Locale>,
    state: AssistantWorkspaceState,
    on_close: Callback<()>,
    on_toggle: Callback<bool>,
) -> impl IntoView {
    view! {
        <header class="assistant-workspace-header" data-testid="assistant-header">
            <button type="button" id="assistant-projects-toggle" class="icon-btn assistant-panel-toggle"
                aria-label=move || j(locale.get(), "Toggle projects", "展开或收起项目列表")
                title=move || j(locale.get(), "Projects", "项目列表")
                aria-controls="assistant-projects" aria-expanded=move || state.left.get().to_string()
                on:click=move |_| on_toggle.call(true)>{compose_icon("panel-left")}</button>
            <h1 class="assistant-title">{move || j(locale.get(), "Research assistant", "科研助理")}</h1>
            <span class="assistant-topbar-note">{move || j(locale.get(), "A little more clarity, every day", "让研究，日渐清晰")}</span>
            <button type="button" id="assistant-calendar-toggle" class="icon-btn assistant-panel-toggle assistant-calendar-toggle"
                aria-label=move || j(locale.get(), "Toggle research calendar", "展开或收起研究日历")
                title=move || j(locale.get(), "Research calendar", "研究日历")
                aria-controls="assistant-calendar" aria-expanded=move || state.right.get().to_string()
                on:click=move |_| on_toggle.call(false)>{compose_icon("panel")}</button>
            <button type="button" class="icon-btn assistant-close" aria-label=move || j(locale.get(), "Close research assistant", "关闭科研助理")
                title=move || j(locale.get(), "Back to home", "返回首页") on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
        </header>
    }
}

#[component]
pub(crate) fn AssistantProjects(
    locale: RwSignal<Locale>,
    state: AssistantWorkspaceState,
) -> impl IntoView {
    view! {
        <aside id="assistant-projects" class="assistant-side assistant-projects" data-testid="assistant-projects"
            hidden=move || !state.left.get()
            class:drawer-top=move || state.last_left.get()
            aria-label=move || j(locale.get(), "Projects", "项目列表")>
            <header class="assistant-side-heading"><h2>{move || j(locale.get(), "Projects", "项目")}</h2>
            </header>
            <button type="button" class="assistant-project" aria-pressed=move || state.selected.get().is_none().to_string()
                on:click=move |_| { state.selected.set(None); if state.narrow.get_untracked() { state.left.set(false); focus_toggle(true); } }>
                {compose_icon("layers")}<span>{move || j(locale.get(), "All projects", "全部项目")}<small>{move || j(locale.get(), "Discuss across projects", "跨项目讨论")}</small></span>
            </button>
            <div class="assistant-project-list">
                {move || state.loading.get().then(|| view!{<p class="assistant-side-notice" role="status">{j(locale.get(), "Loading projects…", "正在读取项目…")}</p>})}
                {move || state.error.get().map(|error| view!{<p class="assistant-side-notice" role="alert">{error}<button type="button" on:click=move |_| state.refresh.update(|value| *value += 1)>{j(locale.get(), "Retry", "重试")}</button></p>})}
                <For each=move || state.projects.get() key=|project| (project.id.clone(), project.name.clone(), project.session_count) children=move |project| {
                    let id = project.id.clone(); let chosen = project.id.clone();
                    view!{<button type="button" class="assistant-project" data-project-id=project.id
                        title=project.name.clone() aria-pressed=move || (state.selected.get().as_ref()==Some(&chosen)).to_string()
                        on:click=move |_| {state.selected.set(Some(id.clone())); if state.narrow.get_untracked(){state.left.set(false); focus_toggle(true);}}>
                        {compose_icon("folder")}<span><span class="assistant-project-name">{project.name}</span><small>{move || if locale.get()==Locale::Zh {format!("{} 个会话",project.session_count)} else {format!("{} conversations",project.session_count)}}</small></span>
                    </button>}
                }/>
                {move || (!state.loading.get() && state.error.get().is_none() && state.projects.get().is_empty()).then(|| view!{<p class="assistant-side-notice">{j(locale.get(), "No visible projects", "暂无可见项目")}</p>})}
            </div>
            <footer class="assistant-side-footer">{move || j(locale.get(), "Project context applies to your next message.", "所选项目作为后续提问的上下文。")}</footer>
        </aside>
    }
}

#[component]
pub(crate) fn AssistantCalendar(
    locale: RwSignal<Locale>,
    state: AssistantWorkspaceState,
    project_transfer: ReadSignal<Option<ProjectTransferProgress>>,
    on_draft: Callback<String>,
) -> impl IntoView {
    let ready = Signal::derive(move || !state.loading.get() && state.error.get().is_none());
    view! {
        <aside id="assistant-calendar" class="assistant-side assistant-calendar" data-testid="assistant-calendar"
            hidden=move || !state.right.get() class:drawer-top=move || !state.last_left.get()
            aria-label=move || j(locale.get(), "Research calendar", "研究日历")>
            <header class="assistant-side-heading"><h2>{move || j(locale.get(), "Research calendar", "研究日历")}</h2>
            </header>
            {move || state.loading.get().then(|| view! {<p class="assistant-side-notice" role="status">{j(locale.get(), "Loading visible projects…", "正在确认可见项目…")}</p>})}
            {move || state.error.get().map(|_| view! {<p class="assistant-side-notice" role="alert">{j(locale.get(), "Project visibility could not be verified. Reload to view the calendar.", "暂时无法确认可见项目，请重新读取后查看日历。")}
                <button type="button" on:click=move |_| state.refresh.update(|n| *n += 1)>{j(locale.get(), "Retry", "重试")}</button></p>})}
            <div class="assistant-calendar-body" hidden=move || !ready.get()>
                <crate::research_calendar::ResearchCalendar locale=locale projects=state.projects compact=true
                    external_refresh=Signal::derive(move || state.refresh.get()) ready=ready on_plan=on_draft
                    on_close=Callback::new(move |_| state.toggle(false)) project_transfer=project_transfer
                    on_open_journey=Callback::new(move |(id, day): (String, i64)| {
                        if let Some(project) = state.projects.get_untracked().into_iter().find(|project| project.id == id) {
                            state.selected.set(Some(id));
                            let date = day_key(day);
                            on_draft.call(if locale.get_untracked() == Locale::Zh {
                                format!("请回顾「{}」在 {date} 的研究进展，并指出值得跟进的事项。", project.name)
                            } else { format!("Review the research progress in {} on {date} and suggest follow-ups.", project.name) });
                        }
                    })/>
            </div>
        </aside>
    }
}

#[component]
pub(crate) fn AssistantPlans(
    locale: RwSignal<Locale>,
    projects: Signal<Vec<ProjectSummary>>,
    selected: ReadSignal<i64>,
    ready: Signal<bool>,
    refresh: Signal<(u32, Option<u32>)>,
    on_plan: Option<Callback<String>>,
    on_retry: Callback<()>,
) -> impl IntoView {
    let plans = create_local_resource(
        move || {
            (
                day_key(selected.get()),
                ready.get(),
                projects.get().into_iter().map(|p| p.id).collect::<Vec<_>>(),
                refresh.get(),
            )
        },
        move |(day, ready, _, _)| async move {
            if !ready {
                return Ok(Vec::new());
            }
            call::<Vec<ResearchAssistantPlanItem>>(
                "get_research_assistant_plan",
                serde_json::json!({"day": day}),
            )
            .await
        },
    );
    view! {
        <section class="assistant-plans" data-testid="assistant-plans">
            <header><h4>{move || j(locale.get(), "Saved plans", "已保存计划")}</h4>
                <button type="button" class="assistant-plan-draft" disabled=move || !ready.get()
                    on:click=move |_| { if let Some(callback) = on_plan {
                        let date = day_key(selected.get_untracked());
                        callback.call(if locale.get_untracked() == Locale::Zh { format!("请帮我安排 {date} 的研究计划。") } else { format!("Help me plan my research for {date}.") });
                    }}>{compose_icon("plus")}{move || j(locale.get(), "Plan this day", "安排这一天")}</button>
            </header>
            {move || {
                let loc = locale.get();
                if !ready.get() || plans.loading().get() { return view! { <p class="assistant-side-notice" role="status">{j(loc, "Loading plans…", "正在读取计划…")}</p> }.into_view(); }
                match plans.get() {
                    Some(Err(error)) => view! {<p class="assistant-side-notice" role="alert">{error}<button type="button" on:click=move |_| on_retry.call(())>{j(loc,"Retry","重试")}</button></p>}.into_view(),
                    Some(Ok(items)) => {
                        let items: Vec<_> = items.into_iter().filter(|item| item.project_id.as_ref().is_none_or(|id| projects.with(|ps| ps.iter().any(|p| &p.id == id)))).collect();
                        if items.is_empty() { return view! {<p class="assistant-plan-empty">{j(loc, "No saved plans. Discuss a plan with your assistant.", "暂无已保存计划，可以和助理一起安排。")}</p>}.into_view(); }
                        view! {<ul>{items.into_iter().map(|item| {
                            let state = match item.status.as_str() { "done" => j(loc,"Done","完成"), "dropped" => j(loc,"Dropped","放弃"), _ => j(loc,"To do","待办") };
                            let carried = item.day < day_key(selected.get());
                            view! {<li data-plan-id=item.id data-status=item.status><div class="assistant-plan-title"><span>{item.title}</span><small>{state}</small></div>
                                <p>{item.project_name.unwrap_or_else(|| j(loc,"Across projects","跨项目").into())}{carried.then(|| format!(" · {} {}", j(loc,"Originally","原定"),item.day))}</p>
                            </li>}
                        }).collect_view()}</ul>}.into_view()
                    },
                    None => ().into_view(),
                }
            }}
        </section>
    }
}
