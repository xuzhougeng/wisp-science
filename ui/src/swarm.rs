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

fn phase_icon(phase: SwarmPhase) -> &'static str {
    match phase {
        SwarmPhase::Starting => "clock",
        SwarmPhase::Running => "activity-orbit",
        SwarmPhase::Summarizing => "layers",
        SwarmPhase::Completed => "check",
        SwarmPhase::Partial | SwarmPhase::Failed => "circle-alert",
        SwarmPhase::Paused => "pause",
        SwarmPhase::Cancelled => "stop",
    }
}

#[derive(Clone, Copy)]
pub struct SwarmViewState {
    pub progress: RwSignal<HashMap<String, Rc<SwarmProgress>>>,
    /// Hide only this window's dock for a finished batch; keep its results.
    pub dismissed: RwSignal<HashMap<String, String>>,
    pub open: RwSignal<bool>,
    pub selected: RwSignal<Option<String>>,
    pub error: RwSignal<Option<String>>,
    /// A retry request is on its way to the backend; the batch's own progress
    /// events take over as soon as it starts.
    pub retrying: RwSignal<bool>,
}

impl SwarmViewState {
    pub fn new() -> Self {
        Self {
            progress: create_rw_signal(HashMap::new()),
            dismissed: create_rw_signal(HashMap::new()),
            open: create_rw_signal(false),
            selected: create_rw_signal(None),
            error: create_rw_signal(None),
            retrying: create_rw_signal(false),
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

    pub fn dismiss_finished(self, frame: &str) {
        let batch = self.progress.with_untracked(|all| {
            all.get(frame)
                .filter(|p| p.phase.terminal())
                .map(|p| p.id.clone())
        });
        if let Some(batch) = batch {
            self.dismissed.update(|all| {
                all.insert(frame.into(), batch);
            });
        }
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

fn stop_turn(state: SwarmViewState, session: RwSignal<Option<String>>) {
    if let Some(id) = session.get_untracked() {
        state.error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({"sessionId": id})).unwrap();
            if let Err(error) = invoke_checked("stop_agent", args).await {
                state.error.set(Some(js_error_text(error)));
            }
        });
    }
}

/// Re-run failed Reader tasks (all of them, or the given ids) in the latest
/// finished batch of this conversation. Progress arrives as swarm events; the
/// recovered evidence is handed to the next message.
fn retry_tasks(
    state: SwarmViewState,
    session: RwSignal<Option<String>>,
    task_ids: Option<Vec<String>>,
) {
    let Some(id) = session.get_untracked() else {
        return;
    };
    if state.retrying.get_untracked() {
        return;
    }
    state.error.set(None);
    state.retrying.set(true);
    spawn_local(async move {
        let args = to_value(&serde_json::json!({"sessionId": id, "taskIds": task_ids})).unwrap();
        if let Err(error) = invoke_checked("retry_swarm_tasks", args).await {
            state.error.set(Some(js_error_text(error)));
        }
        state.retrying.set(false);
    });
}

fn retryable_count(current: CurrentSwarm) -> usize {
    current
        .get()
        .map(|p| p.retryable_task_ids().len())
        .unwrap_or(0)
}

/// Offered only for finished Reader batches with failed tasks. `compact`
/// renders an icon-only button for the dock row.
#[component]
fn RetryButton(
    current: CurrentSwarm,
    state: SwarmViewState,
    session: RwSignal<Option<String>>,
    locale: RwSignal<Locale>,
    #[prop(optional)] compact: bool,
) -> impl IntoView {
    let text = move || {
        let count = retryable_count(current);
        if locale.get() == Locale::Zh {
            format!("重试失败任务 ({count})")
        } else {
            format!("Retry failed tasks ({count})")
        }
    };
    view! {
        <Show when=move || (retryable_count(current) > 0)>
            <button type="button" class="swarm-retry" class:swarm-retry-compact=compact data-testid="swarm-retry"
                aria-label=text title=move || label(locale.get(), "Re-read the failed parts; the recovered evidence joins your next message", "重新读取失败的部分；恢复的证据会随下一条消息提供给助手")
                disabled=move || state.retrying.get()
                on:click=move |_| retry_tasks(state, session, None)>
                {compose_icon("refresh")}
                <Show when=move || !compact><span>{text}</span></Show>
            </button>
        </Show>
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
                    aria-current=move || current.get().and_then(|p| (!p.phase.terminal() && phase_index(p.phase) == index).then_some("step"))
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

fn task_count(progress: &SwarmProgress, status: SwarmTaskStatus) -> usize {
    progress
        .tasks
        .iter()
        .filter(|task| task.status == status)
        .count()
}

fn counts(progress: &SwarmProgress, loc: Locale) -> String {
    if progress.total == 0 {
        return if progress.phase == SwarmPhase::Starting {
            label(loc, "Preparing tasks…", "正在准备任务…")
        } else {
            label(loc, "No tasks in this scope", "当前范围内没有任务")
        };
    }
    let done = task_count(progress, SwarmTaskStatus::Completed);
    let running = task_count(progress, SwarmTaskStatus::Running);
    let failed = task_count(progress, SwarmTaskStatus::Failed);
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

/// Shares of observed task states, not an estimate of time remaining.
fn task_meter(current: CurrentSwarm) -> impl IntoView {
    view! {
        <div class="swarm-meter" aria-hidden="true">
            {[SwarmTaskStatus::Completed, SwarmTaskStatus::Failed, SwarmTaskStatus::Running, SwarmTaskStatus::Paused, SwarmTaskStatus::Cancelled]
                .into_iter().map(|status| view! {
                    <span data-status=status_name(status) style:width=move || {
                        let share = current.get().map(|p| {
                            100.0 * task_count(&p, status) as f64 / p.total.max(p.tasks.len()).max(1) as f64
                        }).unwrap_or(0.0);
                        format!("{share}%")
                    }></span>
                }).collect_view()}
        </div>
    }
}

fn overview_metrics(current: CurrentSwarm, locale: RwSignal<Locale>) -> impl IntoView {
    view! {
        <dl class="swarm-metrics" aria-label=move || label(locale.get(), "Task counts", "任务统计")>
            {[
                (None, "total", "All tasks", "全部任务", "layers"),
                (Some(SwarmTaskStatus::Running), "running", "Working", "执行中", "activity-orbit"),
                (Some(SwarmTaskStatus::Completed), "completed", "Completed", "已完成", "check"),
                (Some(SwarmTaskStatus::Failed), "failed", "Failed", "失败", "circle-alert"),
            ].into_iter().map(|(status, name, en, zh, icon)| view! {
                <div class="swarm-metric" data-status=name>
                    <dt>{compose_icon(icon)}<span>{move || label(locale.get(), en, zh)}</span></dt>
                    <dd>{move || current.get().map(|p| status.map(|s| task_count(&p, s)).unwrap_or(p.total))}</dd>
                </div>
            }).collect_view()}
        </dl>
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
            title=move || task.get().map(|t| t.title)
            aria-pressed=move || (state.selected.get().as_deref() == Some(selected_id.as_str())).to_string()
            data-status=move || task.get().map(|t| status_name(t.status)).unwrap_or("queued")
            on:click=move |_| state.selected.set(Some(id.clone()))>
            <span class="swarm-avatar">{compose_icon("bot")}</span>
            <span class="swarm-worker-copy">
                <strong>{move || task.get().map(|t| t.title)}</strong>
                <small>{move || task.get().map(|t| if t.total_units > 1 { format!("{} · {}/{}", task_label(locale.get(), t.status), t.completed_units, t.total_units) } else { task_label(locale.get(), t.status) })}</small>
                <Show when=move || task.get().is_some_and(|t| t.total_units > 1)>
                    <span class="swarm-worker-bar" aria-hidden="true">
                        <span class="swarm-worker-bar-done" style:width=move || task.get().map(|t| format!("{}%", 100.0 * t.completed_units.saturating_sub(t.failed_units) as f64 / t.total_units.max(1) as f64)).unwrap_or_default()></span>
                        <span class="swarm-worker-bar-failed" style:width=move || task.get().map(|t| format!("{}%", 100.0 * t.failed_units as f64 / t.total_units.max(1) as f64)).unwrap_or_default()></span>
                    </span>
                </Show>
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
    let visible = create_memo(move |_| {
        let Some(progress) = current.get() else {
            return false;
        };
        !progress.phase.terminal()
            || !session.get().is_some_and(|id| {
                state
                    .dismissed
                    .with(|all| all.get(&id) == Some(&progress.id))
            })
    });
    view! {
        <Show when=move || visible.get()>
            <section class="swarm-dock" data-testid="swarm-dock" data-phase=move || current.get().map(|p| format!("{:?}", p.phase).to_lowercase())>
                <div class="swarm-dock-row">
                <button type="button" class="swarm-dock-open" aria-label=move || label(locale.get(), "View assistants", "查看助手") on:click=move |_| { state.selected.set(None); state.open.set(true); }>
                    <span class="swarm-dock-icon" class:active=move || current.get().is_some_and(|p| !p.phase.terminal())>{move || compose_icon(current.get().map(|p| phase_icon(p.phase)).unwrap_or("swarm"))}</span>
                    <span class="swarm-dock-copy">
                        <span class="swarm-dock-heading">
                            <strong>{move || label(locale.get(), "Agent swarm", "并行探索")}</strong>
                            <span class="swarm-phase-badge">{move || current.get().map(|p| phase_label(locale.get(), p.phase))}</span>
                        </span>
                        <small>{move || current.get().map(|p| counts(&p, locale.get()))}</small>
                    </span>
                    <span class="swarm-open-action"><span class="swarm-open-label">{move || label(locale.get(), "View assistants", "查看助手")}</span>{compose_icon("chevron-right")}</span>
                </button>
                <RetryButton current=current state=state session=session locale=locale compact=true/>
                <Show when=move || current.get().is_some_and(|p| p.phase.terminal())>
                    <button type="button" class="swarm-dismiss" data-testid="swarm-dismiss"
                        aria-label=move || label(locale.get(), "Hide finished progress", "关闭已结束的进度卡片")
                        title=move || label(locale.get(), "Hide progress; results remain in Agents", "关闭进度卡片，结果仍可在 Agents 中查看")
                        on:click=move |_| { if let Some(id) = session.get_untracked() { state.dismiss_finished(&id); } }>{compose_icon("close")}</button>
                </Show>
                </div>
                {task_meter(current)}
            </section>
        </Show>
    }
}

/// The Agents pane shares the same live/persisted batch as the dock. Reader
/// workers do not need fabricated child conversations to expose their results.
#[component]
pub fn SwarmPanel(
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
            <section class="swarm-panel" data-testid="swarm-panel"
                data-phase=move || current.get().map(|p| format!("{:?}", p.phase).to_lowercase())>
                <h2>{move || label(locale.get(), "Parallel tasks", "并行任务")}</h2>
                <div class="swarm-panel-card">
                    <button type="button" class="swarm-panel-open"
                        aria-label=move || label(locale.get(), "View parallel tasks and results", "查看并行任务与结果")
                        on:click=move |_| { state.selected.set(None); state.open.set(true); }>
                        <span class="swarm-panel-icon">{compose_icon("swarm")}</span>
                        <span class="swarm-panel-copy">
                            <strong>{move || current.get().map(|p| match p.source.as_str() {
                                "reader" => label(locale.get(), "Session retrieval", "会话检索"),
                                "map_items" => label(locale.get(), "Batch tasks", "批量任务"),
                                _ => label(locale.get(), "Parallel exploration", "并行探索"),
                            })}</strong>
                            <span class="swarm-phase-badge">{move || current.get().map(|p| phase_label(locale.get(), p.phase))}</span>
                        </span>
                        {compose_icon("chevron-right")}
                    </button>
                    <p class="swarm-panel-title" title=move || current.get().map(|p| p.title.clone())>{move || current.get().map(|p| p.title.clone())}</p>
                    <p class="swarm-panel-counts">{move || current.get().map(|p| counts(&p, locale.get()))}</p>
                    {task_meter(current)}
                    <Show when=move || current.get().is_some_and(|p| !p.phase.terminal())>
                        <button type="button" class="swarm-stop" on:click=move |_| stop_turn(state, session)>{compose_icon("stop")}<span>{move || label(locale.get(), "Stop this turn", "停止本轮")}</span></button>
                    </Show>
                    <RetryButton current=current state=state session=session locale=locale/>
                    {move || state.error.get().map(|error| view! { <p class="swarm-panel-error" role="alert">{error}</p> })}
                </div>
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
                    data-phase=move || current.get().map(|p| format!("{:?}", p.phase).to_lowercase())
                    class:swarm-moving=move || current.get().is_some_and(|p| !p.phase.terminal())
                    class:swarm-starting=move || current.get().is_some_and(|p| p.phase == SwarmPhase::Starting)
                    class:swarm-gathering=move || current.get().is_some_and(|p| p.phase == SwarmPhase::Summarizing)>
                    <header class="swarm-header">
                        <span class="swarm-brand">{compose_icon("swarm")}</span>
                        <div class="swarm-header-copy"><h2 id="swarm-title">{move || label(locale.get(), "Agent swarm", "并行探索")}</h2>
                            <p class="swarm-phase-badge" role="status">{move || compose_icon(current.get().map(|p| phase_icon(p.phase)).unwrap_or("swarm"))}{move || current.get().map(|p| phase_label(locale.get(), p.phase))}</p></div>
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
                                <div class="swarm-model"><span>{move || label(locale.get(), "Model", "模型")}</span><small>{move || current.get().map(|p| p.model.clone())}</small></div>
                                <Show when=move || current.get().is_some_and(|p| !p.phase.terminal())>
                                    <button type="button" class="swarm-stop" on:click=move |_| stop_turn(state, session)>{compose_icon("stop")}<span>{move || label(locale.get(), "Stop this turn", "停止本轮")}</span></button>
                                </Show>
                                <RetryButton current=current state=state session=session locale=locale/>
                            </div>
                        </aside>
                        <main class="swarm-detail">
                            {move || state.error.get().map(|error| view! { <p role="alert">{error}</p> })}
                            <Show when=move || selected.get().is_some() fallback=move || view! {
                                <div class="swarm-overview" data-testid="swarm-overview">
                                    <div class="swarm-overview-hero">
                                        <div class="swarm-overview-copy">
                                            <h3>{move || label(locale.get(), "Task overview", "任务总览")}</h3>
                                            <p class="swarm-help">{move || label(locale.get(), "Select an assistant to follow its progress and results.", "选择任务助手，查看进展与结果。")}</p>
                                        </div>
                                        <div class="swarm-constellation" aria-hidden="true">
                                            <span class="swarm-hub">{compose_icon("swarm")}</span>
                                            {(0..6).map(|i| view! {
                                                <span class=format!("swarm-orbit swarm-orbit-{i}")
                                                    data-status=move || current.get().and_then(|p| p.tasks.get(i).map(|t| status_name(t.status))).unwrap_or("idle")>
                                                    <span>{compose_icon("bot")}</span>
                                                </span>
                                            }).collect_view()}
                                        </div>
                                    </div>
                                    <section class="swarm-request">
                                        <h4>{move || label(locale.get(), "Task", "任务目标")}</h4>
                                        <p data-testid="swarm-request">{move || current.get().map(|p| p.title.clone())}</p>
                                    </section>
                                    {overview_metrics(current, locale)}
                                    {task_meter(current)}
                                    <Show when=move || (retryable_count(current) > 0)>
                                        <p class="swarm-help swarm-retry-note" data-testid="swarm-retry-note">{move || label(locale.get(),
                                            "Failed assistants can be retried from the roster or from their own page. Only the failed parts are re-read, and the recovered evidence joins your next message.",
                                            "失败的任务助手可以在左侧列表或任务页中重试。重试只重新读取失败的部分，恢复的证据会随你的下一条消息提供给助手。")}</p>
                                    </Show>
                                    <Show when=move || current.get().is_some_and(|p| !p.summary.is_empty())>
                                        <section class="swarm-result"><h4>{move || label(locale.get(), "Summary", "任务汇总")}</h4><pre data-testid="swarm-summary">{move || current.get().map(|p| p.summary.clone())}</pre></section>
                                    </Show>
                                </div>
                            }>
                                <div class="swarm-task-detail" data-testid="swarm-task-detail">
                                    <div class="swarm-task-toolbar">
                                        <button type="button" class="swarm-back" on:click=move |_| state.selected.set(None)>{compose_icon("arrow-left")}{move || label(locale.get(), "Overview", "返回总览")}</button>
                                        <p class="swarm-task-state" role="status" data-status=move || selected.get().map(|t| status_name(t.status))>{move || selected.get().map(|t| task_label(locale.get(), t.status))}</p>
                                        <Show when=move || {
                                            let Some(task) = selected.get() else { return false };
                                            current.get().is_some_and(|p| p.retryable_task_ids().contains(&task.id))
                                        }>
                                            <button type="button" class="swarm-retry" data-testid="swarm-retry-task"
                                                disabled=move || state.retrying.get()
                                                title=move || label(locale.get(), "Re-read only this assistant's failed parts", "只重新读取这个任务助手失败的部分")
                                                on:click=move |_| { if let Some(task) = selected.get_untracked() { retry_tasks(state, session, Some(vec![task.id])); } }>
                                                {compose_icon("refresh")}<span>{move || label(locale.get(), "Retry this task", "重试此任务")}</span>
                                            </button>
                                        </Show>
                                    </div>
                                    <h3>{move || selected.get().map(|t| t.title)}</h3>
                                    <section class="swarm-request">
                                        <h4>{move || label(locale.get(), "Instructions", "任务说明")}</h4>
                                        <p class="swarm-task-instruction">{move || selected.get().map(|t| t.instruction)}</p>
                                    </section>
                                    <h4>{move || label(locale.get(), "Progress", "执行进展")}</h4>
                                    <ol class="swarm-task-log">{move || selected.get().map(|t| t.log.into_iter().map(|line| view! { <li><span>{line}</span></li> }).collect_view())}</ol>
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
