//! Context-usage panel domain: the signals that own the panel's open/dock/
//! drag/resize state, the pointer behaviour that mutates them, and the panel
//! component itself. `App` only constructs `ContextUsageState`, wires the
//! callbacks, and places `<ContextUsagePanel/>` in the two slots.

use super::*;
use crate::chat_render::{
    context_percent, context_usage_detail_text, context_usage_rows, context_usage_tone,
    fmt_context_limit, fmt_context_tokens, ContextUsageTone,
};

const CONTEXT_USAGE_DRAG_THRESHOLD: f64 = 8.0;

#[derive(Clone, Copy)]
pub(crate) struct ModelViewCtrl {
    pub(crate) on: RwSignal<bool>,
    pub(crate) toggle: Callback<()>,
}

#[component]
pub(crate) fn TranscriptViewToggle(#[prop(optional)] in_panel: bool) -> impl IntoView {
    let locale = use_locale();
    let Some(ctrl) = use_context::<ModelViewCtrl>() else {
        return ().into_view();
    };
    let model_view = ctrl.on;
    let toggle_full = ctrl.toggle.clone();
    let toggle_model = ctrl.toggle;
    let (wrap, full, model) = if in_panel {
        (
            "transcript-view-toggle context-usage-view-toggle",
            "context-usage-view-full",
            "context-usage-view-model",
        )
    } else {
        (
            "transcript-view-toggle",
            "transcript-view-full",
            "transcript-view-model",
        )
    };
    let wrap_test = if in_panel {
        "context-usage-view-toggle"
    } else {
        "transcript-view-toggle"
    };
    view! {
        <div class=wrap data-testid=wrap_test class:model-view=move || model_view.get()>
            <button type="button"
                class:active=move || !model_view.get()
                aria-pressed=move || (!model_view.get()).to_string()
                aria-label=move || t(locale.get(), "chat.view_full")
                data-testid=full
                title=move || t(locale.get(), "chat.view_full")
                on:click=move |_| if model_view.get() { toggle_full.call(()); }>
                {compose_icon("history")}
                <span class="transcript-view-label">{move || t(locale.get(), "chat.view_full")}</span>
            </button>
            <button type="button"
                class:active=move || model_view.get()
                aria-pressed=move || model_view.get().to_string()
                aria-label=move || t(locale.get(), "chat.view_model")
                data-testid=model
                title=move || t(locale.get(), "chat.view_model")
                on:click=move |_| if !model_view.get() { toggle_model.call(()); }>
                {compose_icon("eye")}
                <span class="transcript-view-label">{move || t(locale.get(), "chat.view_model")}</span>
            </button>
        </div>
    }
    .into_view()
}

pub(crate) fn model_view_active() -> bool {
    use_context::<ModelViewCtrl>().is_some_and(|ctrl| ctrl.on.get())
}

pub(crate) fn model_view_active_untracked() -> bool {
    use_context::<ModelViewCtrl>().is_some_and(|ctrl| ctrl.on.get_untracked())
}

pub(crate) fn context_epoch_line(
    locale: Locale,
    head_epoch: u64,
    has_checkpoint: bool,
    kept_turns: usize,
) -> Option<String> {
    if head_epoch == 0 {
        return None;
    }
    Some(tf(
        locale,
        if has_checkpoint {
            "context_usage.epoch_line"
        } else {
            "context_usage.epoch_line_no_checkpoint"
        },
        &[
            ("epoch", &head_epoch.to_string()),
            ("turns", &kept_turns.to_string()),
        ],
    ))
}

/// Session health thresholds (#1541). The usage percentage drops back after
/// every compaction, so it cannot tell whether compaction is still buying
/// room. Health looks at the compaction history on the head epoch's chain:
///
/// - Fewer than [`HEALTH_MIN_COMPACTIONS`] compactions never judge the freed
///   ratio, and the count alone never triggers anything: sessions that
///   monitor long runs compact often and legitimately.
/// - `SplitSoon` when the latest compaction freed less than
///   [`HEALTH_SPLIT_SOON_FREED`] of its window, or freed less than
///   [`HEALTH_SPLIT_SOON_DROP`] of what the previous one freed while staying
///   under [`HEALTH_HEALTHY_FREED`] (a fold that still frees half the window
///   is healthy however it compares).
/// - `SplitNow` when the last two compactions both freed less than
///   [`HEALTH_SPLIT_NOW_FREED`], when the window is in the danger band after
///   at least one compaction, or when a `SplitSoon` session's conversation
///   already takes [`HEALTH_SPLIT_NOW_CONVERSATION_SHARE`] of the window (the
///   residue left by folding is itself filling the window).
///
/// The freed ratio of one compaction is `(before - after) / before`.
pub(crate) const HEALTH_MIN_COMPACTIONS: usize = 2;
pub(crate) const HEALTH_SPLIT_SOON_FREED: f64 = 0.25;
pub(crate) const HEALTH_SPLIT_SOON_DROP: f64 = 0.5;
pub(crate) const HEALTH_HEALTHY_FREED: f64 = 0.5;
pub(crate) const HEALTH_SPLIT_NOW_FREED: f64 = 0.15;
pub(crate) const HEALTH_SPLIT_NOW_CONVERSATION_SHARE: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionHealth {
    Ok,
    SplitSoon,
    SplitNow,
}

impl SessionHealth {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::SplitSoon => "split-soon",
            Self::SplitNow => "split-now",
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Ok => "context_usage.health.ok",
            Self::SplitSoon => "context_usage.health.split_soon",
            Self::SplitNow => "context_usage.health.split_now",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SessionHealthReport {
    pub(crate) level: SessionHealth,
    /// Compactions on the head epoch's parent chain (undone epochs excluded).
    pub(crate) compactions: usize,
    /// Freed ratio of the latest compaction, when its token counts are known.
    pub(crate) latest_freed: Option<f64>,
}

fn freed_ratio(epoch: &ContextEpochDto) -> Option<f64> {
    (epoch.before_tokens > 0).then(|| {
        epoch.before_tokens.saturating_sub(epoch.after_tokens) as f64 / epoch.before_tokens as f64
    })
}

/// Epochs from the head back to the root, newest first. Undone epochs and
/// sibling branches are not on the chain, so they do not count.
fn head_epoch_chain(head_epoch: u64, epochs: &[ContextEpochDto]) -> Vec<&ContextEpochDto> {
    let mut chain = Vec::new();
    let mut cursor = head_epoch;
    while cursor != 0 && chain.len() < epochs.len() {
        let Some(row) = epochs.iter().find(|row| row.epoch == cursor) else {
            break;
        };
        chain.push(row);
        cursor = row.parent_epoch;
    }
    chain
}

pub(crate) fn session_health(
    head_epoch: u64,
    epochs: &[ContextEpochDto],
    snapshot: &ContextUsageSnapshot,
) -> SessionHealthReport {
    let chain = head_epoch_chain(head_epoch, epochs);
    let compactions = chain.len();
    let latest_freed = chain.first().and_then(|row| freed_ratio(row));
    let previous_freed = chain.get(1).and_then(|row| freed_ratio(row));
    let report = |level| SessionHealthReport {
        level,
        compactions,
        latest_freed,
    };
    if compactions == 0 {
        return report(SessionHealth::Ok);
    }
    if context_usage_tone(snapshot.used, snapshot.max) == ContextUsageTone::Danger {
        return report(SessionHealth::SplitNow);
    }
    if compactions < HEALTH_MIN_COMPACTIONS {
        return report(SessionHealth::Ok);
    }
    let Some(latest) = latest_freed else {
        return report(SessionHealth::Ok);
    };
    if previous_freed.is_some_and(|previous| {
        latest < HEALTH_SPLIT_NOW_FREED && previous < HEALTH_SPLIT_NOW_FREED
    }) {
        return report(SessionHealth::SplitNow);
    }
    let diminishing = latest < HEALTH_SPLIT_SOON_FREED
        || previous_freed.is_some_and(|previous| {
            latest < HEALTH_HEALTHY_FREED && latest < previous * HEALTH_SPLIT_SOON_DROP
        });
    if !diminishing {
        return report(SessionHealth::Ok);
    }
    let conversation_share = snapshot
        .breakdown
        .filter(|_| snapshot.max > 0)
        .map(|usage| usage.conversation as f64 / snapshot.max as f64);
    if conversation_share.is_some_and(|share| share >= HEALTH_SPLIT_NOW_CONVERSATION_SHARE) {
        report(SessionHealth::SplitNow)
    } else {
        report(SessionHealth::SplitSoon)
    }
}

pub(crate) fn session_health_line(locale: Locale, report: &SessionHealthReport) -> Option<String> {
    if report.compactions == 0 {
        return None;
    }
    let level = t(locale, report.level.label_key());
    let count = report.compactions.to_string();
    Some(match report.latest_freed {
        Some(freed) => tf(
            locale,
            "context_usage.health.line",
            &[
                ("level", &level),
                ("count", &count),
                ("freed", &format!("{:.0}", freed * 100.0)),
            ],
        ),
        None => tf(
            locale,
            "context_usage.health.line_no_ratio",
            &[("level", &level), ("count", &count)],
        ),
    })
}

fn context_usage_event_target(ev: &web_sys::MouseEvent) -> Option<web_sys::Element> {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
}

fn context_usage_panel_el() -> Option<web_sys::Element> {
    web_sys::window()?
        .document()?
        .query_selector("[data-testid='context-usage-panel']")
        .ok()
        .flatten()
}

pub(crate) fn event_inside_selector(ev: &web_sys::MouseEvent, selector: &str) -> bool {
    context_usage_event_target(ev)
        .and_then(|element| element.closest(selector).ok().flatten())
        .is_some()
}

/// All reactive state of the context-usage panel. `RwSignal` is `Copy`, so the
/// whole struct is `Copy` and can be captured by every closure that needs it.
#[derive(Clone, Copy)]
pub(crate) struct ContextUsageState {
    pub(crate) open: RwSignal<bool>,
    pub(crate) mode: RwSignal<ContextUsageMode>,
    pub(crate) geom: RwSignal<Option<ContextUsageGeom>>,
    pub(crate) dragging: RwSignal<bool>,
    pub(crate) tracking: RwSignal<bool>,
    pub(crate) resizing: RwSignal<bool>,
    pub(crate) drag_origin: RwSignal<(f64, f64)>,
    pub(crate) grab: RwSignal<(f64, f64)>,
    pub(crate) resize_origin: RwSignal<(f64, f64)>,
    pub(crate) resize_start: RwSignal<ContextUsageGeom>,
    pub(crate) passed_threshold: RwSignal<bool>,
    pub(crate) suppress_click: RwSignal<bool>,
    pub(crate) details: RwSignal<Option<ContextUsageDetails>>,
    pub(crate) detail_open: RwSignal<Option<String>>,
}

impl ContextUsageState {
    pub(crate) fn new() -> Self {
        Self {
            open: create_rw_signal(false),
            mode: create_rw_signal(ContextUsageMode::Docked),
            geom: create_rw_signal(load_context_usage_geom()),
            dragging: create_rw_signal(false),
            tracking: create_rw_signal(false),
            resizing: create_rw_signal(false),
            drag_origin: create_rw_signal((0.0_f64, 0.0_f64)),
            grab: create_rw_signal((0.0_f64, 0.0_f64)),
            resize_origin: create_rw_signal((0.0_f64, 0.0_f64)),
            resize_start: create_rw_signal(ContextUsageGeom {
                x: 0.0,
                y: 0.0,
                w: CONTEXT_USAGE_DEFAULT_W,
                h: CONTEXT_USAGE_DEFAULT_H,
            }),
            passed_threshold: create_rw_signal(false),
            suppress_click: create_rw_signal(false),
            details: create_rw_signal(None),
            detail_open: create_rw_signal(None),
        }
    }

    pub(crate) fn header_down(self, ev: web_sys::MouseEvent) {
        if ev.button() != 0 || event_inside_selector(&ev, "button") {
            return;
        }
        let x = ev.client_x() as f64;
        let y = ev.client_y() as f64;
        self.tracking.set(true);
        self.passed_threshold.set(false);
        self.drag_origin.set((x, y));
        if let Some(panel) = context_usage_panel_el() {
            let rect = panel.get_bounding_client_rect();
            self.grab.set((x - rect.x(), y - rect.y()));
            if self.geom.get_untracked().is_none() {
                self.geom.set(Some(ContextUsageGeom {
                    x: rect.x(),
                    y: rect.y(),
                    w: rect.width(),
                    h: rect.height(),
                }));
            }
        }
    }

    pub(crate) fn header_dblclick(self, ev: web_sys::MouseEvent) {
        if event_inside_selector(&ev, "button") {
            return;
        }
        if self.mode.get_untracked() == ContextUsageMode::Floating {
            self.mode.set(ContextUsageMode::Docked);
        }
    }

    pub(crate) fn dock(self) {
        self.mode.set(ContextUsageMode::Docked);
    }

    pub(crate) fn drag_move(self, ev: web_sys::MouseEvent) {
        if !self.tracking.get() && !self.dragging.get() {
            return;
        }
        let x = ev.client_x() as f64;
        let y = ev.client_y() as f64;
        let (origin_x, origin_y) = self.drag_origin.get();
        let dx = x - origin_x;
        let dy = y - origin_y;
        if !self.passed_threshold.get() {
            if (dx * dx + dy * dy).sqrt() < CONTEXT_USAGE_DRAG_THRESHOLD {
                return;
            }
            self.passed_threshold.set(true);
            self.dragging.set(true);
            if self.mode.get_untracked() == ContextUsageMode::Docked {
                let (grab_x, grab_y) = self.grab.get_untracked();
                let (width, height) = self
                    .geom
                    .get_untracked()
                    .map(|geom| (geom.w, geom.h))
                    .or_else(|| {
                        context_usage_panel_el().map(|panel| {
                            let rect = panel.get_bounding_client_rect();
                            (rect.width(), rect.height())
                        })
                    })
                    .unwrap_or((CONTEXT_USAGE_DEFAULT_W, CONTEXT_USAGE_DEFAULT_H));
                let (viewport_w, viewport_h) = viewport_size();
                self.geom.set(Some(clamp_context_usage_geom(
                    x - grab_x,
                    y - grab_y,
                    width,
                    height,
                    viewport_w,
                    viewport_h,
                )));
                self.mode.set(ContextUsageMode::Floating);
            }
        }
        if self.mode.get_untracked() != ContextUsageMode::Floating {
            return;
        }
        let (grab_x, grab_y) = self.grab.get_untracked();
        let (width, height) = self
            .geom
            .get_untracked()
            .map(|geom| (geom.w, geom.h))
            .unwrap_or((CONTEXT_USAGE_DEFAULT_W, CONTEXT_USAGE_DEFAULT_H));
        let (viewport_w, viewport_h) = viewport_size();
        self.geom.set(Some(clamp_context_usage_geom(
            x - grab_x,
            y - grab_y,
            width,
            height,
            viewport_w,
            viewport_h,
        )));
    }

    pub(crate) fn drag_end(self) {
        if !self.tracking.get() && !self.dragging.get() {
            return;
        }
        let moved = self.passed_threshold.get();
        self.tracking.set(false);
        self.dragging.set(false);
        if moved {
            if let Some(geom) = self.geom.get() {
                save_context_usage_geom(geom);
            }
            self.suppress_click.set(true);
        }
        self.passed_threshold.set(false);
    }

    pub(crate) fn resize_begin(self, ev: web_sys::MouseEvent) {
        if ev.button() != 0 {
            return;
        }
        ev.prevent_default();
        ev.stop_propagation();
        let geom = context_usage_panel_el()
            .map(|panel| {
                let rect = panel.get_bounding_client_rect();
                ContextUsageGeom {
                    x: rect.x(),
                    y: rect.y(),
                    w: rect.width(),
                    h: rect.height(),
                }
            })
            .or_else(|| self.geom.get_untracked());
        let Some(geom) = geom else {
            return;
        };
        self.geom.set(Some(geom));
        self.resizing.set(true);
        self.resize_origin
            .set((ev.client_x() as f64, ev.client_y() as f64));
        self.resize_start.set(geom);
    }

    pub(crate) fn resize_move(self, ev: web_sys::MouseEvent) {
        if !self.resizing.get() {
            return;
        }
        let start = self.resize_start.get();
        let (origin_x, origin_y) = self.resize_origin.get();
        let (viewport_w, viewport_h) = viewport_size();
        self.geom.set(Some(clamp_context_usage_geom(
            start.x,
            start.y,
            start.w + ev.client_x() as f64 - origin_x,
            start.h + ev.client_y() as f64 - origin_y,
            viewport_w,
            viewport_h,
        )));
    }

    pub(crate) fn resize_end(self) {
        if !self.resizing.get() {
            return;
        }
        self.resizing.set(false);
        if let Some(geom) = self.geom.get() {
            save_context_usage_geom(geom);
        }
        self.suppress_click.set(true);
    }
}

#[component]
pub(crate) fn ContextUsagePanel(
    snapshot: ContextUsageSnapshot,
    floating: bool,
    locale: ReadSignal<Locale>,
    context_usage_open: RwSignal<bool>,
    context_usage_details: RwSignal<Option<ContextUsageDetails>>,
    context_usage_detail_open: RwSignal<Option<String>>,
    context_usage_geom: RwSignal<Option<ContextUsageGeom>>,
    on_header_down: Callback<web_sys::MouseEvent>,
    on_header_dblclick: Callback<web_sys::MouseEvent>,
    on_dock: Callback<()>,
    on_resize_start: Callback<web_sys::MouseEvent>,
    on_compact: Callback<()>,
    on_new_session: Callback<()>,
    on_handoff: Callback<()>,
    compact_disabled: Signal<bool>,
    epoch_line: Option<String>,
    health: SessionHealthReport,
) -> impl IntoView {
    let loc = locale.get();
    let pct = context_percent(snapshot.used, snapshot.max);
    let danger = context_usage_tone(snapshot.used, snapshot.max) == ContextUsageTone::Danger;
    let health_line = session_health_line(loc, &health);
    let show_nudge = danger || health.level != SessionHealth::Ok;
    let nudge_copy = if danger {
        "context_usage.nudge"
    } else {
        "context_usage.health.nudge"
    };
    let used = fmt_context_tokens(snapshot.used);
    let total = if snapshot.max == 0 {
        tf(loc, "context_usage.total_used", &[("used", &used)])
    } else {
        let max = fmt_context_limit(snapshot.max);
        tf(
            loc,
            if snapshot.estimated {
                "context_usage.total_estimated"
            } else {
                "context_usage.total_exact"
            },
            &[("used", &used), ("max", &max)],
        )
    };
    let rows = context_usage_rows(&snapshot, loc);
    let segments = rows.clone();
    let denominator = snapshot.max.max(snapshot.used).max(1);
    let mode = if floating { "floating" } else { "docked" };
    view! {
        <section id="context-usage-panel" class="context-usage-panel"
            class:is-docked=!floating
            class:is-floating=floating
            data-testid="context-usage-panel"
            data-mode=mode
            role="dialog"
            aria-labelledby="context-usage-title"
            style=move || {
                if !floating {
                    String::new()
                } else if let Some(geom) = context_usage_geom.get() {
                    format!(
                        "left:{}px;top:{}px;width:{}px;height:{}px;--context-usage-h:{}px",
                        geom.x, geom.y, geom.w, geom.h, geom.h
                    )
                } else {
                    String::new()
                }
            }>
            <div class="context-usage-head" data-testid="context-usage-head"
                on:mousedown=move |ev| on_header_down.call(ev)
                on:dblclick=move |ev| on_header_dblclick.call(ev)>
                <h2 id="context-usage-title">{t(loc, "context_usage.title")}</h2>
                <div class="context-usage-head-actions">
                    <button type="button" class="context-usage-compact"
                        data-testid="context-usage-compact-header"
                        title=t(loc, "context_usage.nudge_compact")
                        aria-label=t(loc, "context_usage.nudge_compact")
                        disabled=move || compact_disabled.get()
                        on:mousedown=move |ev| ev.stop_propagation()
                        on:click=move |ev| {
                            ev.stop_propagation();
                            on_compact.call(());
                        }>
                        {compose_icon("context-compact")}
                        <span>{t(loc, "context_usage.nudge_compact")}</span>
                    </button>
                    {floating.then(|| view! {
                        <button type="button" class="context-usage-dock"
                            data-testid="context-usage-dock"
                            title=t(loc, "context_usage.dock")
                            aria-label=t(loc, "context_usage.dock")
                            on:click=move |_| on_dock.call(())>
                            {compose_icon("dock")}
                        </button>
                    })}
                    <button type="button" class="context-usage-close"
                        title=t(loc, "context_usage.close")
                        aria-label=t(loc, "context_usage.close")
                        on:click=move |_| context_usage_open.set(false)>
                        {compose_icon("close")}
                    </button>
                </div>
            </div>
            <div class="context-usage-summary">
                <span>{tf(loc, "context_usage.full", &[("pct", &pct.to_string())])}</span>
                <span>{total}</span>
            </div>
            {epoch_line.as_ref().map(|line| {
                let line = line.clone();
                view! {
                    <div class="context-usage-epoch" data-testid="context-usage-epoch">{line}</div>
                }
            })}
            {health_line.map(|line| view! {
                <div class="context-usage-health" data-testid="context-usage-health"
                    data-level=health.level.as_str()>{line}</div>
            })}
            <TranscriptViewToggle in_panel=true />
            {show_nudge.then(|| view! {
                <div class="context-usage-nudge" data-testid="context-usage-nudge" role="status"
                    data-health=health.level.as_str()>
                    <span class="context-usage-nudge-copy">{t(loc, nudge_copy)}</span>
                    <div class="context-usage-nudge-actions">
                        <button type="button" class="context-usage-nudge-action"
                            data-testid="context-usage-compact"
                            disabled=move || compact_disabled.get()
                            on:click=move |ev| {
                                ev.stop_propagation();
                                on_compact.call(());
                            }>
                            {t(loc, "context_usage.nudge_compact")}
                        </button>
                        <button type="button" class="context-usage-nudge-action"
                            data-testid="context-usage-new-session"
                            on:click=move |ev| {
                                ev.stop_propagation();
                                on_new_session.call(());
                            }>
                            {t(loc, "context_usage.nudge_new_session")}
                        </button>
                        <button type="button" class="context-usage-nudge-action"
                            data-testid="context-usage-handoff"
                            disabled=move || compact_disabled.get()
                            on:click=move |ev| {
                                ev.stop_propagation();
                                on_handoff.call(());
                            }>
                            {t(loc, "context_usage.nudge_handoff")}
                        </button>
                    </div>
                </div>
            })}
            <div class="context-usage-bar" role="img"
                aria-label=tf(loc, "context_usage.full", &[("pct", &pct.to_string())])>
                {segments.into_iter().filter(|row| row.tokens > 0).map(|row| {
                    let width = row.tokens as f64 * 100.0 / denominator as f64;
                    view! {
                        <span class=format!("context-usage-segment {}", row.color)
                            style=format!("width:{width:.4}%")></span>
                    }
                }).collect_view()}
            </div>
            <div class="context-usage-list">
                {rows.into_iter().map(|row| {
                    let expandable = row.color != "conversation";
                    let color = row.color.to_string();
                    let detail_color = color.clone();
                    let open_color = color.clone();
                    view! {
                        <div class="context-usage-item">
                            <button type="button" class="context-usage-row"
                                class:expandable=expandable
                                disabled=!expandable
                                aria-expanded=move || (expandable && context_usage_detail_open.get().as_deref() == Some(open_color.as_str())).to_string()
                                on:click=move |_| {
                                    if expandable {
                                        context_usage_detail_open.update(|active| {
                                            *active = (active.as_deref() != Some(color.as_str())).then(|| color.clone());
                                        });
                                    }
                                }>
                                <span class=format!("context-usage-swatch {}", row.color)
                                    aria-hidden="true"></span>
                                <span class="context-usage-label">{row.label}</span>
                                <span class="context-usage-value">{fmt_context_tokens(row.tokens)}</span>
                                {expandable.then(|| view! { <span class="context-usage-chevron">{"⌄"}</span> })}
                            </button>
                            {move || (context_usage_detail_open.get().as_deref() == Some(detail_color.as_str())).then(|| {
                                let content = context_usage_details.get()
                                    .map(|details| context_usage_detail_text(&details, &detail_color))
                                    .unwrap_or_else(|| t(locale.get(), "context_usage.loading").into());
                                view! { <pre class="context-usage-detail">{content}</pre> }
                            })}
                        </div>
                    }
                }).collect_view()}
            </div>
            {floating.then(|| view! {
                <button type="button" class="context-usage-resize"
                    data-testid="context-usage-resize"
                    title=t(loc, "context_usage.resize")
                    aria-label=t(loc, "context_usage.resize")
                    on:mousedown=move |ev| on_resize_start.call(ev)>
                </button>
            })}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        context_epoch_line, session_health, session_health_line, SessionHealth, SessionHealthReport,
    };
    use crate::dto::{ContextEpochDto, ContextUsage, ContextUsageSnapshot};
    use crate::i18n::Locale;

    /// A linear chain of compactions, oldest first, as `(before, after)`.
    fn chain(folds: &[(u64, u64)]) -> Vec<ContextEpochDto> {
        folds
            .iter()
            .enumerate()
            .map(|(index, &(before_tokens, after_tokens))| ContextEpochDto {
                epoch: index as u64 + 1,
                parent_epoch: index as u64,
                strategy: "auto".into(),
                kind: "semantic".into(),
                before_tokens,
                after_tokens,
                initial_head_seq: 0,
                first_kept_seq: None,
                checkpoint_seq: None,
                ui_event_seq: None,
                has_new_turns: false,
            })
            .collect()
    }

    fn usage(used: usize, conversation: Option<usize>) -> ContextUsageSnapshot {
        ContextUsageSnapshot {
            used,
            max: 1_000,
            breakdown: conversation.map(|conversation| ContextUsage {
                conversation,
                ..ContextUsage::default()
            }),
            estimated: false,
        }
    }

    fn level(folds: &[(u64, u64)], snapshot: &ContextUsageSnapshot) -> SessionHealth {
        session_health(folds.len() as u64, &chain(folds), snapshot).level
    }

    #[test]
    fn health_is_ok_and_silent_without_compaction() {
        let report = session_health(0, &[], &usage(950, None));
        assert_eq!(report.level, SessionHealth::Ok);
        assert_eq!(report.compactions, 0);
        assert_eq!(session_health_line(Locale::En, &report), None);
    }

    #[test]
    fn one_compaction_is_ok_unless_the_window_is_full_again() {
        assert_eq!(level(&[(1_000, 950)], &usage(200, None)), SessionHealth::Ok);
        assert_eq!(
            level(&[(1_000, 200)], &usage(950, None)),
            SessionHealth::SplitNow
        );
    }

    #[test]
    fn diminishing_freed_ratios_recommend_a_split() {
        // 80% then 20%: the latest is under the split-soon floor.
        assert_eq!(
            level(&[(1_000, 200), (1_000, 800)], &usage(200, None)),
            SessionHealth::SplitSoon
        );
        // 80% then 35%: above the floor but less than half the previous one.
        assert_eq!(
            level(&[(1_000, 200), (1_000, 650)], &usage(200, None)),
            SessionHealth::SplitSoon
        );
        // Both of the last two under 15%.
        assert_eq!(
            level(
                &[(1_000, 200), (1_000, 900), (1_000, 880)],
                &usage(200, None)
            ),
            SessionHealth::SplitNow
        );
        // Diminishing, and the conversation residue already fills half the window.
        assert_eq!(
            level(&[(1_000, 200), (1_000, 800)], &usage(600, Some(500))),
            SessionHealth::SplitNow
        );
    }

    #[test]
    fn run_monitoring_with_many_healthy_compactions_stays_ok() {
        let folds = vec![(1_000, 280); 12];
        assert_eq!(level(&folds, &usage(300, Some(250))), SessionHealth::Ok);
        // A fold that still frees half the window is healthy even after a better one.
        assert_eq!(
            level(&[(1_000, 100), (1_000, 450)], &usage(300, None)),
            SessionHealth::Ok
        );
    }

    #[test]
    fn undone_epochs_off_the_head_chain_do_not_count() {
        let mut epochs = chain(&[(1_000, 200), (1_000, 900)]);
        // Epoch 2 was undone and epoch 3 forked from epoch 1 instead.
        let fork = ContextEpochDto {
            epoch: 3,
            parent_epoch: 1,
            before_tokens: 1_000,
            after_tokens: 250,
            ..epochs[0].clone()
        };
        epochs.push(fork);
        let report = session_health(3, &epochs, &usage(200, None));
        assert_eq!(report.compactions, 2);
        assert_eq!(report.level, SessionHealth::Ok);
    }

    #[test]
    fn health_line_names_level_count_and_latest_freed_ratio() {
        let report = SessionHealthReport {
            level: SessionHealth::SplitSoon,
            compactions: 3,
            latest_freed: Some(0.204),
        };
        assert_eq!(
            session_health_line(Locale::En, &report).as_deref(),
            Some("Health: split soon · compacted 3× · last freed 20%")
        );
        assert_eq!(
            session_health_line(Locale::Zh, &report).as_deref(),
            Some("健康度：建议尽快拆分 · 已压缩 3 次 · 最近释放 20%")
        );
        let unknown = SessionHealthReport {
            latest_freed: None,
            ..report
        };
        assert_eq!(
            session_health_line(Locale::En, &unknown).as_deref(),
            Some("Health: split soon · compacted 3×")
        );
    }

    #[test]
    fn epoch_line_hidden_without_compaction() {
        assert_eq!(context_epoch_line(Locale::En, 0, true, 3), None);
    }

    #[test]
    fn epoch_line_names_checkpoint_and_kept_turns() {
        assert_eq!(
            context_epoch_line(Locale::En, 1, true, 1).as_deref(),
            Some("Epoch 1 · system + checkpoint + 1 kept turns")
        );
        assert_eq!(
            context_epoch_line(Locale::Zh, 2, false, 4).as_deref(),
            Some("纪元 2 · system + 4 轮 tail")
        );
    }
}
