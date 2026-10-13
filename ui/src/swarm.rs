use crate::{compose_icon, dto::*, i18n::Locale, invoke_checked, js_error_text};
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::HashMap;
use std::rc::Rc;

type CurrentSwarm = Memo<Option<Rc<SwarmProgress>>>;

fn label(loc: Locale, en: &str, zh: &str) -> String {
    if loc == Locale::Zh {
        zh.into()
    } else {
        en.into()
    }
}

pub fn phase_label(loc: Locale, phase: SwarmPhase) -> String {
    let (en, zh) = match phase {
        SwarmPhase::Starting => ("Starting tasks", "启动任务"),
        SwarmPhase::Running => ("Working in parallel", "任务并行"),
        SwarmPhase::Summarizing => ("Bringing results together", "任务汇总"),
        SwarmPhase::Completed => ("Results ready", "汇总完成"),
        SwarmPhase::Partial => ("Finished with gaps", "已结束，部分任务失败"),
        SwarmPhase::Failed => ("Needs attention", "任务遇到问题"),
        SwarmPhase::Paused => ("Paused", "已暂停"),
        SwarmPhase::Cancelled => ("Stopped", "已停止"),
    };
    label(loc, en, zh)
}

fn task_label(loc: Locale, status: SwarmTaskStatus) -> String {
    let (en, zh) = match status {
        SwarmTaskStatus::Queued => ("Waiting", "等待任务"),
        SwarmTaskStatus::Running => ("Working", "正在执行"),
        SwarmTaskStatus::Completed => ("Completed", "已完成"),
        SwarmTaskStatus::Failed => ("Failed", "执行失败"),
        SwarmTaskStatus::Paused => ("Paused", "已暂停"),
        SwarmTaskStatus::Cancelled => ("Stopped", "已停止"),
    };
    label(loc, en, zh)
}

fn status_name(status: SwarmTaskStatus) -> &'static str {
    match status {
        SwarmTaskStatus::Queued => "queued",
        SwarmTaskStatus::Running => "running",
        SwarmTaskStatus::Completed => "completed",
        SwarmTaskStatus::Failed => "failed",
        SwarmTaskStatus::Paused => "paused",
        SwarmTaskStatus::Cancelled => "cancelled",
    }
}

#[derive(Clone, Copy)]
pub struct SwarmViewState {
    pub progress: RwSignal<HashMap<String, Rc<SwarmProgress>>>,
    pub open: RwSignal<bool>,
    pub selected: RwSignal<Option<String>>,
    pub error: RwSignal<Option<String>>,
}

impl SwarmViewState {
    pub fn new() -> Self {
        Self {
            progress: create_rw_signal(HashMap::new()),
            open: create_rw_signal(false),
            selected: create_rw_signal(None),
            error: create_rw_signal(None),
        }
    }

    pub fn accept(self, frame: &str, update: SwarmProgress) {
        self.progress.update(|all| {
            if let Some(current) = all.get_mut(frame) {
                Rc::make_mut(current).merge(update);
            } else {
                all.insert(frame.into(), Rc::new(update));
            }
        });
    }

    /// Root-owned overlay participates in the app's window Escape stack.
    pub fn escape(self) -> bool {
        if !self.open.get_untracked() {
            return false;
        }
        if self.selected.get_untracked().is_some() {
            self.selected.set(None);
        } else {
            self.open.set(false);
        }
        true
    }
}

pub fn refresh(state: SwarmViewState, session_id: String) {
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "sessionId": session_id })).unwrap();
        if let Ok(value) = invoke_checked("get_swarm_progress", args).await {
            if let Ok(Some(progress)) =
                serde_wasm_bindgen::from_value::<Option<SwarmProgress>>(value)
            {
                state.accept(&session_id, progress);
            }
        }
    });
}

fn phase_index(phase: SwarmPhase) -> usize {
    match phase {
        SwarmPhase::Starting => 0,
        SwarmPhase::Running => 1,
        _ => 2,
    }
}

fn stages(current: CurrentSwarm, locale: RwSignal<Locale>) -> impl IntoView {
    view! {
        <ol class="swarm-stages" aria-label=move || label(locale.get(), "Task stages", "任务阶段")>
            {(0..3).map(|index| view! {
                <li class:current=move || current.get().is_some_and(|p| !p.phase.terminal() && phase_index(p.phase) == index)
                    class:passed=move || current.get().is_some_and(|p| p.phase == SwarmPhase::Completed || (!p.phase.terminal() && phase_index(p.phase) > index))>
                    <span class="swarm-stage-number">{index + 1}</span>
                    <span>{move || match index {
                        0 => label(locale.get(), "Start", "启动任务"),
                        1 => label(locale.get(), "Parallel tasks", "任务并行"),
                        _ => label(locale.get(), "Summarize", "任务汇总"),
                    }}</span>
                </li>
            }).collect_view()}
        </ol>
    }
}

fn counts(progress: &SwarmProgress, loc: Locale) -> String {
    if progress.total == 0 {
        return if progress.phase == SwarmPhase::Starting {
            label(loc, "Preparing tasks…", "正在准备任务…")
        } else {
            label(loc, "No tasks in this scope", "当前范围内没有任务")
        };
    }
    let done = progress
        .tasks
        .iter()
        .filter(|task| task.status == SwarmTaskStatus::Completed)
        .count();
    let running = progress
        .tasks
        .iter()
        .filter(|task| task.status == SwarmTaskStatus::Running)
        .count();
    let failed = progress
        .tasks
        .iter()
        .filter(|task| task.status == SwarmTaskStatus::Failed)
        .count();
    if loc == Locale::Zh {
        format!(
            "{done}/{} 已完成 · {running} 执行中 · {failed} 失败",
            progress.total
        )
    } else {
        format!(
            "{done}/{} completed · {running} working · {failed} failed",
            progress.total
        )
    }
}

#[component]
fn WorkerRow(
    id: String,
    current: CurrentSwarm,
    state: SwarmViewState,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let lookup = id.clone();
    let task = create_memo(move |_| {
        current
            .get()
            .and_then(|p| p.tasks.iter().find(|task| task.id == lookup).cloned())
    });
    let selected_id = id.clone();
    let data_id = id.clone();
    view! {
        <button type="button" class="swarm-worker" data-testid="swarm-worker" data-task-id=data_id
            aria-pressed=move || (state.selected.get().as_deref() == Some(selected_id.as_str())).to_string()
            data-status=move || task.get().map(|t| status_name(t.status)).unwrap_or("queued")
            on:click=move |_| state.selected.set(Some(id.clone()))>
            <span class="swarm-avatar">{compose_icon("bot")}</span>
            <span class="swarm-worker-copy">
                <strong>{move || task.get().map(|t| t.title)}</strong>
                <small>{move || task.get().map(|t| if t.total_units > 1 { format!("{} · {}/{}", task_label(locale.get(), t.status), t.completed_units, t.total_units) } else { task_label(locale.get(), t.status) })}</small>
            </span>
            <span class="swarm-worker-dot" aria-hidden="true"></span>
        </button>
    }
}

#[component]
pub fn SwarmDock(
    state: SwarmViewState,
    session: RwSignal<Option<String>>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let current = create_memo(move |_| {
        session
            .get()
            .and_then(|id| state.progress.with(|all| all.get(&id).cloned()))
    });
    view! {
        <Show when=move || current.get().is_some()>
            <section class="swarm-dock" data-testid="swarm-dock" data-phase=move || current.get().map(|p| format!("{:?}", p.phase).to_lowercase())>
                <button type="button" class="swarm-dock-open" on:click=move |_| { state.selected.set(None); state.open.set(true); }>
                    <span class="swarm-dock-icon" class:active=move || current.get().is_some_and(|p| !p.phase.terminal())>{compose_icon("swarm")}</span>
                    <span class="swarm-dock-copy">
                        <strong>{move || current.get().map(|p| phase_label(locale.get(), p.phase))}</strong>
                        <small>{move || current.get().map(|p| counts(&p, locale.get()))}</small>
                    </span>
                    <span class="swarm-open-label">{move || label(locale.get(), "View assistants", "查看助手")}</span>
                    {compose_icon("chevron-right")}
                </button>
                {stages(current, locale)}
            </section>
        </Show>
    }
}

#[component]
pub fn SwarmOverlay(
    state: SwarmViewState,
    session: RwSignal<Option<String>>,
    locale: RwSignal<Locale>,
) -> impl IntoView {
    let current = create_memo(move |_| {
        session
            .get()
            .and_then(|id| state.progress.with(|all| all.get(&id).cloned()))
    });
    let selected = create_memo(move |_| {
        let id = state.selected.get()?;
        current
            .get()?
            .tasks
            .iter()
            .find(|task| task.id == id)
            .cloned()
    });
    let visible = create_rw_signal(80usize);
    let batch_id = create_memo(move |_| current.with(|value| value.as_ref().map(|p| p.id.clone())));
    create_effect(move |_| {
        batch_id.track();
        state.selected.set(None);
        state.error.set(None);
        visible.set(80);
    });
    create_effect(move |_| {
        session.track();
        state.open.set(false);
        state.selected.set(None);
        visible.set(80);
    });
    view! {
        <Show when=move || state.open.get() && current.get().is_some()>
            <div class="swarm-backdrop" on:click=move |_| { state.open.set(false); state.selected.set(None); }>
                <section class="swarm-window" role="dialog" aria-modal="true" aria-labelledby="swarm-title"
                    data-testid="swarm-window" on:click=move |event| event.stop_propagation()
                    class:swarm-moving=move || current.get().is_some_and(|p| !p.phase.terminal())
                    class:swarm-starting=move || current.get().is_some_and(|p| p.phase == SwarmPhase::Starting)
                    class:swarm-gathering=move || current.get().is_some_and(|p| p.phase == SwarmPhase::Summarizing)>
                    <header class="swarm-header">
                        <span class="swarm-brand">{compose_icon("swarm")}</span>
                        <div><h2 id="swarm-title">{move || label(locale.get(), "Agent swarm", "并行探索")}</h2>
                            <p role="status">{move || current.get().map(|p| phase_label(locale.get(), p.phase))}</p></div>
                        <button type="button" class="swarm-close" aria-label=move || label(locale.get(), "Close exploration", "关闭探索视图") on:click=move |_| state.open.set(false)>{compose_icon("close")}</button>
                    </header>
                    {stages(current, locale)}
                    <div class="swarm-layout">
                        <aside class="swarm-roster">
                            <div class="swarm-roster-head"><strong>{move || label(locale.get(), "Assistants", "任务助手")}</strong><small>{move || current.get().map(|p| p.total)}</small></div>
                            <div class="swarm-worker-list">
                                <For each=move || current.get().map(|p| p.tasks.iter().take(visible.get()).map(|t| t.id.clone()).collect::<Vec<_>>()).unwrap_or_default()
                                    key=|id| id.clone() children=move |id| view! { <WorkerRow id=id current=current state=state locale=locale/> }/>
                                <Show when=move || current.get().is_some_and(|p| p.tasks.len() > visible.get())>
                                    <button type="button" class="swarm-more" on:click=move |_| visible.update(|n| *n += 80)>{move || label(locale.get(), "Show more", "显示更多")}</button>
                                </Show>
                            </div>
                            <div class="swarm-roster-footer">
                                <small>{move || current.get().map(|p| p.model.clone())}</small>
                                <Show when=move || current.get().is_some_and(|p| !p.phase.terminal())>
                                    <button type="button" class="swarm-stop" on:click=move |_| {
                                        if let Some(id) = session.get_untracked() {
                                            spawn_local(async move {
                                                let args = to_value(&serde_json::json!({"sessionId": id})).unwrap();
                                                if let Err(error) = invoke_checked("stop_agent", args).await { state.error.set(Some(js_error_text(error))); }
                                            });
                                        }
                                    }>{compose_icon("stop")}<span>{move || label(locale.get(), "Stop this turn", "停止本轮")}</span></button>
                                </Show>
                            </div>
                        </aside>
                        <main class="swarm-detail">
                            {move || state.error.get().map(|error| view! { <p role="alert">{error}</p> })}
                            <Show when=move || selected.get().is_some() fallback=move || view! {
                                <div class="swarm-overview" data-testid="swarm-overview">
                                    <div class="swarm-constellation" aria-hidden="true">
                                        <span class="swarm-hub">{compose_icon("swarm")}</span>
                                        {(0..6).map(|i| view! { <span class=format!("swarm-orbit swarm-orbit-{i}")><span>{compose_icon("bot")}</span></span> }).collect_view()}
                                    </div>
                                    <h3>{move || current.get().map(|p| p.title.clone())}</h3>
                                    <p>{move || current.get().map(|p| counts(&p, locale.get()))}</p>
                                    <p class="swarm-help">{move || label(locale.get(), "Select an assistant to follow its progress and results.", "点击助手，查看当前进展与结果。")}</p>
                                    <Show when=move || current.get().is_some_and(|p| !p.summary.is_empty())>
                                        <section class="swarm-result"><h4>{move || label(locale.get(), "Summary", "任务汇总")}</h4><pre data-testid="swarm-summary">{move || current.get().map(|p| p.summary.clone())}</pre></section>
                                    </Show>
                                </div>
                            }>
                                <div class="swarm-task-detail" data-testid="swarm-task-detail">
                                    <button type="button" class="swarm-back" on:click=move |_| state.selected.set(None)>{compose_icon("arrow-left")}{move || label(locale.get(), "Overview", "返回总览")}</button>
                                    <h3>{move || selected.get().map(|t| t.title)}</h3>
                                    <p class="swarm-task-state" role="status">{move || selected.get().map(|t| task_label(locale.get(), t.status))}</p>
                                    <p class="swarm-task-instruction">{move || selected.get().map(|t| t.instruction)}</p>
                                    <h4>{move || label(locale.get(), "Progress", "执行进展")}</h4>
                                    <ol class="swarm-task-log">{move || selected.get().map(|t| t.log.into_iter().map(|line| view! { <li>{line}</li> }).collect_view())}</ol>
                                    <Show when=move || selected.get().is_some_and(|t| t.log.is_empty())><p class="swarm-help">{move || label(locale.get(), "Waiting for the next progress update.", "等待下一条进度更新。")}</p></Show>
                                    <Show when=move || selected.get().is_some_and(|t| !t.result.is_empty())>
                                        <section class="swarm-result"><h4>{move || label(locale.get(), "Result", "任务结果")}</h4><pre data-testid="swarm-task-result">{move || selected.get().map(|t| t.result)}</pre></section>
                                    </Show>
                                </div>
                            </Show>
                        </main>
                    </div>
                </section>
            </div>
        </Show>
    }
}
