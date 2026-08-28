use crate::app_support::{compose_icon, js_error_text, show_toast, show_warning_toast};
use crate::bindings::invoke_checked;
use crate::chat_render::fmt_tokens;
use crate::dto::{
    TrajectoryCellDto, TrajectorySnapshotDto, TrajectoryStatsDto, TrajectoryUsageDto,
};
use crate::i18n::{localize_backend, t, tf, use_locale, Locale};
use crate::text::{event_target_value, format_duration_ms};
use leptos::*;
use serde_wasm_bindgen::to_value;
use wasm_bindgen::JsCast;

/// Session-level Gantt scale, matching the DeepSeek session-log axis.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum TimelineAxis {
    #[default]
    Duration,
    Turns,
    Calls,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum InspectorTab {
    #[default]
    Summary,
    Preview,
    Raw,
    Source,
}

#[derive(Clone, Debug)]
struct GanttSeg {
    key: String,
    lane: &'static str,
    left_pct: f64,
    width_pct: f64,
}

/// Per-turn split of wall time: idle/input gap, model time, tool time.
/// Derived from cell timestamps and durations; `None` when the cells carry
/// no usable timing at all.
fn turn_timing(cells: &[TrajectoryCellDto]) -> Option<(u64, u64, u64)> {
    let mut model_ms = 0i64;
    let mut tool_ms = 0i64;
    let mut start: Option<i64> = None;
    let mut end: Option<i64> = None;
    for cell in cells {
        match cell.kind.as_str() {
            "assistant" => model_ms += cell.duration_ms.unwrap_or(0),
            "tool" => tool_ms += cell.duration_ms.unwrap_or(0),
            _ => {}
        }
        if let Some(ts) = cell.ts {
            let cell_end = ts + cell.duration_ms.unwrap_or(0);
            start = Some(start.map_or(ts, |s| s.min(ts)));
            end = Some(end.map_or(cell_end, |e| e.max(cell_end)));
        }
    }
    let span = end? - start?;
    if span <= 0 {
        return None;
    }
    let input_ms = (span - model_ms - tool_ms).max(0) as u64;
    Some((input_ms, model_ms.max(0) as u64, tool_ms.max(0) as u64))
}

fn badge_label(kind: &str) -> String {
    match kind {
        "user" => "USER".to_string(),
        "assistant" => "ASSISTANT".to_string(),
        "tool" => "TOOL".to_string(),
        "usage" => "USAGE".to_string(),
        other => other.to_uppercase(),
    }
}

fn cell_lane(kind: &str) -> &'static str {
    match kind {
        "user" => "input",
        "tool" => "tools",
        _ => "model",
    }
}

fn cell_kind_icon(kind: &str) -> &'static str {
    match kind {
        "user" => "user",
        "assistant" => "sparkles",
        "tool" => "wrench",
        "usage" => "gauge",
        _ => "sparkles",
    }
}

fn cell_matches(cell: &TrajectoryCellDto, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let hit = |text: &str| text.to_lowercase().contains(query);
    hit(&cell.summary)
        || cell.detail_input.as_deref().is_some_and(hit)
        || cell.detail_output.as_deref().is_some_and(hit)
}

fn usage_line(locale: Locale, usage: &TrajectoryUsageDto) -> String {
    let mut line = tf(
        locale,
        "trajectory.usage.line",
        &[
            ("round", &usage.round.to_string()),
            ("in", &fmt_tokens(usage.input_tokens.max(0) as u64)),
            ("out", &fmt_tokens(usage.output_tokens.max(0) as u64)),
        ],
    );
    if usage.input_tokens > 0 && usage.cached_input_tokens > 0 {
        let pct = (usage.cached_input_tokens as f64 * 100.0 / usage.input_tokens as f64).round();
        line.push_str(&tf(
            locale,
            "trajectory.usage.cached",
            &[("pct", &format!("{pct:.0}"))],
        ));
    }
    line
}

fn stats_line(locale: Locale, stats: &TrajectoryStatsDto) -> String {
    let dash = "–".to_string();
    let tok_per_sec = stats
        .tokens_per_sec
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| dash.clone());
    let cache_pct = stats
        .cache_hit_pct
        .map(|v| format!("{v:.0}"))
        .unwrap_or(dash);
    format!(
        "{} · {} | LLM {} · {} {} | {} tok/s | {} | {} · {}",
        tf(
            locale,
            "trajectory.stats.turns",
            &[("n", &stats.turns.to_string())]
        ),
        tf(
            locale,
            "trajectory.stats.steps",
            &[("n", &stats.steps.to_string())]
        ),
        format_duration_ms(stats.llm_ms.max(0) as u64),
        t(locale, "trajectory.legend.tools"),
        format_duration_ms(stats.tool_ms.max(0) as u64),
        tok_per_sec,
        tf(locale, "trajectory.stats.cache_hit", &[("pct", &cache_pct)]),
        tf(
            locale,
            "trajectory.stats.input",
            &[("tokens", &fmt_tokens(stats.input_tokens.max(0) as u64))]
        ),
        tf(
            locale,
            "trajectory.stats.output",
            &[("tokens", &fmt_tokens(stats.output_tokens.max(0) as u64))]
        ),
    )
}

fn cell_status_key(cell: &TrajectoryCellDto, running: bool) -> &'static str {
    if cell.is_error || cell.ok == Some(false) {
        "trajectory.status.error"
    } else if cell.kind == "tool" && cell.ok.is_none() {
        if running {
            "trajectory.status.running"
        } else {
            "trajectory.status.pending"
        }
    } else if running && cell.kind == "assistant" && cell.ok.is_none() && cell.duration_ms.is_none()
    {
        "trajectory.status.running"
    } else {
        "trajectory.status.completed"
    }
}

fn source_key(kind: &str) -> &'static str {
    match kind {
        "user" => "trajectory.source.user",
        "assistant" => "trajectory.source.assistant",
        "tool" => "trajectory.source.tool",
        "usage" => "trajectory.source.usage",
        _ => "trajectory.source.unknown",
    }
}

fn head_kind_key(kind: &str) -> &'static str {
    match kind {
        "user" => "trajectory.head.user",
        "assistant" => "trajectory.head.assistant",
        "tool" => "trajectory.head.tool",
        "usage" => "trajectory.head.usage",
        _ => "trajectory.head.event",
    }
}

fn cell_raw_json(cell: &TrajectoryCellDto) -> String {
    serde_json::to_string_pretty(cell).unwrap_or_else(|_| cell.summary.clone())
}

fn cell_preview_text(locale: Locale, cell: &TrajectoryCellDto) -> String {
    if cell.kind == "usage" {
        return cell
            .usage
            .as_ref()
            .map(|usage| usage_line(locale, usage))
            .unwrap_or_else(|| cell.summary.clone());
    }
    cell.detail_output
        .clone()
        .filter(|text| !text.trim().is_empty())
        .or_else(|| cell.detail_input.clone())
        .unwrap_or_else(|| cell.summary.clone())
}

fn cell_source_text(locale: Locale, cell: &TrajectoryCellDto) -> String {
    match cell.kind.as_str() {
        "tool" => cell
            .detail_input
            .clone()
            .unwrap_or_else(|| cell.summary.clone()),
        "usage" => cell
            .usage
            .as_ref()
            .and_then(|usage| serde_json::to_string_pretty(usage).ok())
            .unwrap_or_else(|| {
                cell.usage
                    .as_ref()
                    .map(|usage| usage_line(locale, usage))
                    .unwrap_or_else(|| cell.summary.clone())
            }),
        _ => cell
            .detail_output
            .clone()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| cell.summary.clone()),
    }
}

fn visible_rows<'a>(
    snap: &'a Option<TrajectorySnapshotDto>,
    live: &'a [TrajectoryCellDto],
    query: &str,
) -> Vec<(String, i64, &'a TrajectoryCellDto)> {
    let mut rows = Vec::new();
    if let Some(snapshot) = snap {
        for turn in &snapshot.turns {
            for (ci, cell) in turn.cells.iter().enumerate() {
                if cell_matches(cell, query) {
                    rows.push((format!("{}:{ci}", turn.index), turn.index, cell));
                }
            }
        }
    }
    let next_turn = snap.as_ref().map(|s| s.turns.len() as i64 + 1).unwrap_or(1);
    for (ci, cell) in live.iter().enumerate() {
        if cell_matches(cell, query) {
            rows.push((format!("live:{ci}"), next_turn, cell));
        }
    }
    rows
}

fn gantt_events<'a>(
    rows: &[(String, i64, &'a TrajectoryCellDto)],
) -> Vec<(String, i64, &'a TrajectoryCellDto)> {
    rows.iter()
        .filter(|(_, _, cell)| cell.kind != "usage")
        .map(|(key, turn, cell)| (key.clone(), *turn, *cell))
        .collect()
}

fn traj_dom_id(key: &str) -> String {
    format!("traj-cell-{}", key.replace(':', "-"))
}

fn scroll_traj_row_to_top(key: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(row) = document.get_element_by_id(&traj_dom_id(key)) else {
        return;
    };
    let Ok(Some(list_el)) = document.query_selector("[data-testid='traj-list']") else {
        row.scroll_into_view_with_bool(true);
        return;
    };
    let Ok(list) = list_el.dyn_into::<web_sys::HtmlElement>() else {
        row.scroll_into_view_with_bool(true);
        return;
    };
    let delta = row.get_bounding_client_rect().top() - list.get_bounding_client_rect().top();
    list.set_scroll_top((list.scroll_top() as f64 + delta).round().max(0.0) as i32);
}

fn sequential_pack(cells: &[(String, i64, &TrajectoryCellDto)], weights: &[f64]) -> Vec<GanttSeg> {
    let total = weights.iter().copied().sum::<f64>().max(1.0);
    let mut acc = 0.0;
    cells
        .iter()
        .enumerate()
        .map(|(i, (key, _, cell))| {
            let w = weights.get(i).copied().unwrap_or(1.0).max(0.0);
            let left = acc / total * 100.0;
            acc += w;
            GanttSeg {
                key: key.clone(),
                lane: cell_lane(&cell.kind),
                left_pct: left,
                width_pct: w / total * 100.0,
            }
        })
        .collect()
}

fn duration_weights(cells: &[(String, i64, &TrajectoryCellDto)]) -> Vec<f64> {
    let durs: Vec<f64> = cells
        .iter()
        .map(|(_, _, cell)| cell.duration_ms.unwrap_or(0).max(0) as f64)
        .collect();
    let positive: Vec<f64> = durs.iter().copied().filter(|d| *d > 0.0).collect();
    if positive.is_empty() {
        return cells.iter().map(|_| 1.0).collect();
    }
    let avg = positive.iter().sum::<f64>() / positive.len() as f64;
    let floor = (avg * 0.5).max(1.0);
    durs.iter()
        .map(|d| if *d > 0.0 { *d } else { floor })
        .collect()
}

fn gantt_segments(axis: TimelineAxis, rows: &[(String, i64, &TrajectoryCellDto)]) -> Vec<GanttSeg> {
    let events = gantt_events(rows);
    if events.is_empty() {
        return Vec::new();
    }
    match axis {
        TimelineAxis::Duration => sequential_pack(&events, &duration_weights(&events)),
        TimelineAxis::Turns => {
            let mut groups: Vec<(i64, Vec<usize>)> = Vec::new();
            for (i, (_, turn, _)) in events.iter().enumerate() {
                match groups.last_mut() {
                    Some((t, idxs)) if *t == *turn => idxs.push(i),
                    _ => groups.push((*turn, vec![i])),
                }
            }
            let n = groups.len().max(1) as f64;
            let mut out = Vec::new();
            for (gi, (_turn, idxs)) in groups.iter().enumerate() {
                let col = 100.0 / n;
                let n_cells = idxs.len().max(1) as f64;
                let cell_w = col / n_cells;
                for (ci, &row_i) in idxs.iter().enumerate() {
                    let (key, _, cell) = &events[row_i];
                    out.push(GanttSeg {
                        key: key.clone(),
                        lane: cell_lane(&cell.kind),
                        left_pct: gi as f64 * col + ci as f64 * cell_w,
                        width_pct: cell_w,
                    });
                }
            }
            out
        }
        TimelineAxis::Calls => {
            let n = events.len() as f64;
            events
                .iter()
                .enumerate()
                .map(|(i, (key, _, cell))| GanttSeg {
                    key: key.clone(),
                    lane: cell_lane(&cell.kind),
                    left_pct: i as f64 / n * 100.0,
                    width_pct: 100.0 / n,
                })
                .collect()
        }
    }
}

#[component]
fn TrajectoryCellRow(
    cell: TrajectoryCellDto,
    cell_key: String,
    selected: RwSignal<Option<String>>,
    inspector_open: RwSignal<bool>,
) -> impl IntoView {
    let locale = use_locale();
    let is_usage = cell.kind == "usage";
    let duration = cell.duration_ms.filter(|ms| *ms > 0);
    let row_class = format!("traj-row {}", cell.kind);
    let select_key = cell_key.clone();
    let active_key = cell_key.clone();
    let icon = cell_kind_icon(&cell.kind);
    view! {
        <div class=row_class
            class:error=cell.is_error
            class:pending=cell.ok.is_none() && cell.kind == "tool"
            class:selected=move || selected.get().as_deref() == Some(active_key.as_str())
            data-testid=format!("traj-row-{}", cell.kind)
            data-traj-key=cell_key.clone()
            id=traj_dom_id(&cell_key)
            on:click=move |_| {
                selected.set(Some(select_key.clone()));
                inspector_open.set(true);
            }>
            <span class="traj-row-icon" data-testid="traj-row-icon" aria-hidden="true">
                {compose_icon(icon)}
            </span>
            <span class=format!("traj-badge {}", cell.kind)>{badge_label(&cell.kind)}</span>
            <span class="traj-summary">{move || {
                if is_usage {
                    cell.usage
                        .as_ref()
                        .map(|usage| usage_line(locale.get(), usage))
                        .unwrap_or_default()
                } else {
                    cell.summary.clone()
                }
            }}</span>
            {duration.map(|ms| view! {
                <span class="traj-duration">{format_duration_ms(ms.max(0) as u64)}</span>
            })}
        </div>
    }
}

#[component]
fn TrajectoryInspector(
    cell: TrajectoryCellDto,
    turn: i64,
    running: bool,
    tab: RwSignal<InspectorTab>,
    inspector_open: RwSignal<bool>,
) -> impl IntoView {
    let locale = use_locale();
    let kind = cell.kind.clone();
    let badge = badge_label(&kind);
    let kind_for_title = kind.clone();
    view! {
        <div class="traj-inspector" data-testid="traj-inspector">
            <div class="traj-inspector-head">
                <div class="traj-inspector-ident">
                    <span class=format!("traj-badge {kind}")>{badge}</span>
                    <span class="traj-inspector-title">{move || {
                        let loc = locale.get();
                        format!(
                            "{} · {}",
                            tf(loc, "trajectory.turn", &[("n", &turn.to_string())]),
                            t(loc, head_kind_key(&kind_for_title))
                        )
                    }}</span>
                </div>
                <button type="button" class="ps-close" data-testid="traj-inspector-close"
                    title=move || t(locale.get(), "trajectory.close_inspector")
                    aria-label=move || t(locale.get(), "trajectory.close_inspector")
                    on:click=move |ev| {
                        ev.stop_propagation();
                        inspector_open.set(false);
                    }>
                    {compose_icon("close")}
                </button>
            </div>
            <div class="traj-inspector-tabs" role="tablist">
                <button type="button" class="traj-inspector-tab" data-testid="traj-tab-summary"
                    class:active=move || tab.get() == InspectorTab::Summary
                    on:click=move |_| tab.set(InspectorTab::Summary)>
                    {move || t(locale.get(), "trajectory.tab.summary")}
                </button>
                <button type="button" class="traj-inspector-tab" data-testid="traj-tab-preview"
                    class:active=move || tab.get() == InspectorTab::Preview
                    on:click=move |_| tab.set(InspectorTab::Preview)>
                    {move || t(locale.get(), "trajectory.tab.preview")}
                </button>
                <button type="button" class="traj-inspector-tab" data-testid="traj-tab-raw"
                    class:active=move || tab.get() == InspectorTab::Raw
                    on:click=move |_| tab.set(InspectorTab::Raw)>
                    {move || t(locale.get(), "trajectory.tab.raw")}
                </button>
                <button type="button" class="traj-inspector-tab" data-testid="traj-tab-source"
                    class:active=move || tab.get() == InspectorTab::Source
                    on:click=move |_| tab.set(InspectorTab::Source)>
                    {move || t(locale.get(), "trajectory.tab.source")}
                </button>
            </div>
            <div class="traj-inspector-body">
                {move || inspector_body(locale.get(), tab.get(), &cell, running)}
            </div>
        </div>
    }
}

fn inspector_body(loc: Locale, tab: InspectorTab, cell: &TrajectoryCellDto, running: bool) -> View {
    let kind = cell.kind.as_str();
    match tab {
        InspectorTab::Summary => {
            let duration_label = cell
                .duration_ms
                .map(|ms| format_duration_ms(ms.max(0) as u64))
                .unwrap_or_else(|| "—".into());
            let preview = cell_preview_text(loc, cell);
            let usage = cell.usage.clone();
            view! {
                <dl class="traj-meta">
                    <dt>{t(loc, "trajectory.meta.source")}</dt>
                    <dd data-testid="traj-meta-source">{t(loc, source_key(kind))}</dd>
                    <dt>{t(loc, "trajectory.meta.status")}</dt>
                    <dd data-testid="traj-meta-status">{t(loc, cell_status_key(cell, running))}</dd>
                    <dt>{t(loc, "trajectory.meta.duration")}</dt>
                    <dd data-testid="traj-meta-duration">{duration_label}</dd>
                </dl>
                {usage.map(|usage| {
                    let model = usage.model.clone().unwrap_or_else(|| "—".into());
                    view! {
                        <dl class="traj-meta traj-meta-usage">
                            <dt>{t(loc, "trajectory.meta.model")}</dt>
                            <dd>{model}</dd>
                            <dt>{t(loc, "trajectory.meta.input")}</dt>
                            <dd>{fmt_tokens(usage.input_tokens.max(0) as u64)}</dd>
                            <dt>{t(loc, "trajectory.meta.output")}</dt>
                            <dd>{fmt_tokens(usage.output_tokens.max(0) as u64)}</dd>
                            <dt>{t(loc, "trajectory.meta.cached")}</dt>
                            <dd>{fmt_tokens(usage.cached_input_tokens.max(0) as u64)}</dd>
                        </dl>
                    }
                })}
                {(!preview.trim().is_empty()).then(|| view! {
                    <div class="traj-preview-block">
                        <span class="traj-detail-label">{t(loc, "trajectory.tab.preview")}</span>
                        <pre data-testid="traj-summary-preview">{preview.clone()}</pre>
                    </div>
                })}
            }
            .into_view()
        }
        InspectorTab::Preview if kind == "tool" => view! {
            <div class="traj-preview-block">
                {cell.detail_input.as_ref().map(|input| view! {
                    <span class="traj-detail-label">{t(loc, "trajectory.detail_input")}</span>
                    <pre data-testid="traj-detail-input">{input.clone()}</pre>
                })}
                {cell.detail_output.as_ref().map(|output| view! {
                    <span class="traj-detail-label">{t(loc, "trajectory.detail_output")}</span>
                    <pre data-testid="traj-detail-output">{output.clone()}</pre>
                })}
            </div>
        }
        .into_view(),
        InspectorTab::Preview => view! {
            <div class="traj-preview-block">
                <pre data-testid="traj-preview">{cell_preview_text(loc, cell)}</pre>
            </div>
        }
        .into_view(),
        InspectorTab::Raw => view! {
            <div class="traj-preview-block">
                <pre data-testid="traj-raw">{cell_raw_json(cell)}</pre>
            </div>
        }
        .into_view(),
        InspectorTab::Source => view! {
            <div class="traj-preview-block">
                {(kind == "tool").then(|| view! {
                    <span class="traj-detail-label">{t(loc, "trajectory.detail_input")}</span>
                })}
                <pre data-testid="traj-source">{cell_source_text(loc, cell)}</pre>
            </div>
        }
        .into_view(),
    }
}

#[component]
pub(crate) fn TrajectoryView(
    snapshot: RwSignal<Option<TrajectorySnapshotDto>>,
    live: RwSignal<Vec<TrajectoryCellDto>>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    let locale = use_locale();
    let query = create_rw_signal(String::new());
    let selected = create_rw_signal(None::<String>);
    let inspector_open = create_rw_signal(true);
    let axis = create_rw_signal(TimelineAxis::Duration);
    let tab = create_rw_signal(InspectorTab::Summary);
    let jump_seq = create_rw_signal(0u32);
    let jump_target = create_rw_signal(None::<String>);
    let search = move || query.get().trim().to_lowercase();

    create_effect(move |_| {
        let _tick = jump_seq.get();
        let Some(key) = jump_target.get() else {
            return;
        };
        request_animation_frame(move || {
            request_animation_frame(move || {
                scroll_traj_row_to_top(&key);
            });
        });
    });

    create_effect(move |_| {
        let snap = snapshot.get();
        let live_cells = live.get();
        let keys: Vec<String> = visible_rows(&snap, &live_cells, &search())
            .into_iter()
            .map(|(key, _, _)| key)
            .collect();
        let current = selected.get_untracked();
        if current
            .as_ref()
            .is_some_and(|key| keys.iter().any(|candidate| candidate == key))
        {
            return;
        }
        selected.set(keys.first().cloned());
    });

    // The list body, the Gantt, and the inspector each track their own inputs.
    // Selection must never invalidate the list body: re-rendering it would
    // recreate the scroll container and send the reader back to turn 1.
    let is_empty = create_memo(move |_| {
        snapshot.with(|snap| snap.as_ref().map_or(0, |s| s.turns.len())) == 0
            && live.with(|cells| cells.is_empty())
    });
    let selected_cell = create_memo(move |_| {
        let key = selected.get()?;
        let snap = snapshot.get();
        let live_cells = live.get();
        visible_rows(&snap, &live_cells, &search())
            .into_iter()
            .find(|(candidate, _, _)| *candidate == key)
            .map(|(_, turn, cell)| (turn, cell.clone()))
    });

    let gantt = move || {
        let loc = locale.get();
        let snap = snapshot.get();
        let live_cells = live.get();
        let rows = visible_rows(&snap, &live_cells, &search());
        let segs = gantt_segments(axis.get(), &rows);
        (!segs.is_empty()).then(|| {
            let lanes = ["input", "model", "tools"];
            view! {
                <div class="traj-gantt" data-testid="traj-gantt">
                    {lanes.into_iter().map(|lane| {
                        let lane_segs: Vec<GanttSeg> = segs.iter().filter(|seg| seg.lane == lane).cloned().collect();
                        let label_key = match lane {
                            "input" => "trajectory.legend.input",
                            "model" => "trajectory.legend.model",
                            _ => "trajectory.legend.tools",
                        };
                        view! {
                            <div class="traj-gantt-lane">
                                <span class="traj-gantt-label">{t(loc, label_key)}</span>
                                <div class="traj-gantt-track">
                                    {lane_segs.into_iter().map(|seg| {
                                        let key = seg.key.clone();
                                        let select_key = key.clone();
                                        let active_key = key.clone();
                                        view! {
                                            <button type="button" class=format!("traj-gantt-seg {}", seg.lane)
                                                class:selected=move || selected.get().as_deref() == Some(active_key.as_str())
                                                style=format!("left:{:.2}%;width:{:.2}%", seg.left_pct, seg.width_pct)
                                                data-testid="traj-gantt-seg"
                                                data-traj-key=key.clone()
                                                aria-label=key.clone()
                                                on:click=move |_| {
                                                    selected.set(Some(select_key.clone()));
                                                    inspector_open.set(true);
                                                    jump_target.set(Some(select_key.clone()));
                                                    jump_seq.update(|n| *n = n.saturating_add(1));
                                                }>
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>
                            </div>
                        }
                    }).collect_view()}
                </div>
            }
        })
    };

    let list_body = move || {
        let loc = locale.get();
        let q = search();
        let snap = snapshot.get();
        let live_cells = live.get();
        let running = busy.get();
        let turns = snap.as_ref().map(|s| s.turns.len()).unwrap_or(0);
        let mut any_visible = false;
        let turn_views = snap
            .as_ref()
            .map(|s| {
                s.turns
                    .iter()
                    .filter_map(|turn| {
                        let cells: Vec<(usize, &TrajectoryCellDto)> = turn
                            .cells
                            .iter()
                            .enumerate()
                            .filter(|(_, cell)| cell_matches(cell, &q))
                            .collect();
                        if cells.is_empty() {
                            return None;
                        }
                        any_visible = true;
                        let running_turn = running && turn.index as usize == turns;
                        let timing = turn_timing(&turn.cells);
                        Some(view! {
                            <section class="traj-turn">
                                <div class="traj-turn-head">
                                    <span>{tf(loc, "trajectory.turn", &[("n", &turn.index.to_string())])}</span>
                                    {running_turn.then(|| view! {
                                        <span class="traj-running">{t(loc, "trajectory.running")}</span>
                                    })}
                                </div>
                                {timing.map(|(input_ms, model_ms, tool_ms)| view! {
                                    <div class="traj-bar">
                                        {(input_ms > 0).then(|| view! {
                                            <div class="traj-bar-seg input" style=format!("flex-grow:{input_ms}")></div>
                                        })}
                                        {(model_ms > 0).then(|| view! {
                                            <div class="traj-bar-seg model" style=format!("flex-grow:{model_ms}")></div>
                                        })}
                                        {(tool_ms > 0).then(|| view! {
                                            <div class="traj-bar-seg tools" style=format!("flex-grow:{tool_ms}")></div>
                                        })}
                                    </div>
                                })}
                                <div class="traj-rows">
                                    {cells.into_iter().map(|(ci, cell)| {
                                        let key = format!("{}:{ci}", turn.index);
                                        view! {
                                            <TrajectoryCellRow
                                                cell=cell.clone()
                                                cell_key=key
                                                selected=selected
                                                inspector_open=inspector_open />
                                        }
                                    }).collect_view()}
                                </div>
                            </section>
                        })
                    })
                    .collect_view()
            })
            .unwrap_or_default();
        let live_view = (!live_cells.is_empty()).then(|| {
            let next_turn = turns as i64 + 1;
            let visible: Vec<(usize, &TrajectoryCellDto)> = live_cells
                .iter()
                .enumerate()
                .filter(|(_, cell)| cell_matches(cell, &q))
                .collect();
            view! {
                <section class="traj-turn live" data-testid="traj-live-turn">
                    <div class="traj-turn-head">
                        <span>{tf(loc, "trajectory.turn", &[("n", &next_turn.to_string())])}</span>
                        <span class="traj-running">{t(loc, "trajectory.running")}</span>
                    </div>
                    <div class="traj-rows">
                        {visible.into_iter().map(|(ci, cell)| {
                            let key = format!("live:{ci}");
                            view! {
                                <TrajectoryCellRow
                                    cell=cell.clone()
                                    cell_key=key
                                    selected=selected
                                    inspector_open=inspector_open />
                            }
                        }).collect_view()}
                    </div>
                </section>
            }
        });
        let no_match = (!any_visible && !q.is_empty() && live_cells.is_empty())
            .then(|| view! { <div class="trajectory-empty">{t(loc, "trajectory.no_match")}</div> });
        view! {
            {turn_views}
            {live_view}
            {no_match}
        }
    };

    let inspector = move || {
        if !inspector_open.get() {
            return None;
        }
        let loc = locale.get();
        let view = match selected_cell.get() {
            Some((turn, cell)) => {
                let live_selected = selected
                    .get()
                    .is_some_and(|key| key.starts_with("live:"));
                view! {
                    <TrajectoryInspector
                        cell=cell
                        turn=turn
                        running=busy.get() && live_selected
                        tab=tab
                        inspector_open=inspector_open />
                }
                .into_view()
            }
            None => view! {
                <div class="traj-inspector traj-inspector-empty" data-testid="traj-inspector">
                    <div class="traj-inspector-head">
                        <span class="traj-inspector-title">{t(loc, "trajectory.select_event")}</span>
                        <button type="button" class="ps-close" data-testid="traj-inspector-close"
                            title=t(loc, "trajectory.close_inspector")
                            aria-label=t(loc, "trajectory.close_inspector")
                            on:click=move |_| inspector_open.set(false)>
                            {compose_icon("close")}
                        </button>
                    </div>
                </div>
            }
            .into_view(),
        };
        Some(view)
    };

    let footer = move || {
        let loc = locale.get();
        snapshot.with(|snap| {
            snap.as_ref().map(|s| {
                view! {
                    <div class="trajectory-footer" data-testid="trajectory-footer">
                        {stats_line(loc, &s.stats)}
                    </div>
                }
            })
        })
    };

    view! {
        <div class="trajectory" data-testid="trajectory-view">
            <div class="trajectory-toolbar">
                <div class="traj-axis" role="tablist" aria-label=move || t(locale.get(), "trajectory.axis")>
                    <button type="button" class="traj-axis-btn" data-testid="traj-axis-duration"
                        class:active=move || axis.get() == TimelineAxis::Duration
                        on:click=move |_| axis.set(TimelineAxis::Duration)>
                        {compose_icon("clock")}
                        <span>{move || t(locale.get(), "trajectory.axis.duration")}</span>
                    </button>
                    <button type="button" class="traj-axis-btn" data-testid="traj-axis-turns"
                        class:active=move || axis.get() == TimelineAxis::Turns
                        on:click=move |_| axis.set(TimelineAxis::Turns)>
                        {compose_icon("list")}
                        <span>{move || t(locale.get(), "trajectory.axis.turns")}</span>
                    </button>
                    <button type="button" class="traj-axis-btn" data-testid="traj-axis-calls"
                        class:active=move || axis.get() == TimelineAxis::Calls
                        on:click=move |_| axis.set(TimelineAxis::Calls)>
                        {compose_icon("bolt")}
                        <span>{move || t(locale.get(), "trajectory.axis.calls")}</span>
                    </button>
                </div>
                <div class="trajectory-search">
                    {compose_icon("search")}
                    <input type="search"
                        placeholder=move || t(locale.get(), "trajectory.search")
                        aria-label=move || t(locale.get(), "trajectory.search")
                        prop:value=query
                        on:input=move |ev| query.set(event_target_value(&ev)) />
                </div>
                <div class="trajectory-legend">
                    <span><i class="traj-swatch input"></i>{move || t(locale.get(), "trajectory.legend.input")}</span>
                    <span><i class="traj-swatch model"></i>{move || t(locale.get(), "trajectory.legend.model")}</span>
                    <span><i class="traj-swatch tools"></i>{move || t(locale.get(), "trajectory.legend.tools")}</span>
                </div>
            </div>
            {move || {
                if is_empty.get() {
                    return view! {
                        <div class="trajectory-empty">
                            {move || t(locale.get(), "trajectory.empty")}
                        </div>
                    }.into_view();
                }
                view! {
                    {gantt}
                    <div class="traj-split" class:is-open=move || inspector_open.get()
                        data-testid="traj-split">
                        <div class="traj-list" data-testid="traj-list">{list_body}</div>
                        {inspector}
                    </div>
                    {footer}
                }.into_view()
            }}
        </div>
    }
}

/// Full-window inspector for the current session's event trajectory.
#[component]
pub(crate) fn TrajectoryOverlay(
    open: RwSignal<bool>,
    snapshot: RwSignal<Option<TrajectorySnapshotDto>>,
    live: RwSignal<Vec<TrajectoryCellDto>>,
    busy: RwSignal<bool>,
    session_id: RwSignal<Option<String>>,
) -> impl IntoView {
    let locale = use_locale();
    let export = move |_| {
        let loc = locale.get_untracked();
        let Some(frame_id) = session_id
            .get_untracked()
            .filter(|id| !id.is_empty())
            .or_else(|| {
                snapshot
                    .get_untracked()
                    .map(|snap| snap.frame_id)
                    .filter(|id| !id.is_empty())
            })
        else {
            return;
        };
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "frameId": frame_id,
                "locale": loc.code(),
            }))
            .unwrap();
            match invoke_checked("export_session_trajectory", args).await {
                Ok(value) => {
                    if let Some(path) = value.as_string().filter(|path| !path.is_empty()) {
                        show_toast(&tf(
                            locale.get_untracked(),
                            "trajectory.export_saved",
                            &[("path", &path)],
                        ));
                    }
                }
                Err(error) => show_warning_toast(&localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                )),
            }
        });
    };
    move || {
        open.get().then(move || {
            view! {
                <div class="overlay" data-testid="trajectory-overlay">
                    <div class="modal traj-modal" role="dialog" aria-modal="true"
                        aria-labelledby="trajectory-modal-title">
                        <div class="ps-head">
                            <h2 id="trajectory-modal-title">{move || t(locale.get(), "trajectory.title")}</h2>
                            <div class="traj-head-actions">
                                <button type="button" class="ps-close" data-testid="trajectory-export"
                                    title=move || t(locale.get(), "trajectory.export")
                                    aria-label=move || t(locale.get(), "trajectory.export")
                                    disabled=move || session_id.with(|id| {
                                        id.as_ref().map_or(true, |value| value.is_empty())
                                            && snapshot.with(|snap| {
                                                snap.as_ref().map_or(true, |s| s.frame_id.is_empty())
                                            })
                                    })
                                    on:click=export>
                                    {compose_icon("download")}
                                </button>
                                <button type="button" class="ps-close" data-testid="trajectory-close"
                                    title=move || t(locale.get(), "trajectory.close")
                                    aria-label=move || t(locale.get(), "trajectory.close")
                                    on:click=move |_| open.set(false)>
                                    {compose_icon("close")}
                                </button>
                            </div>
                        </div>
                        <TrajectoryView snapshot=snapshot live=live busy=busy />
                    </div>
                </div>
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(kind: &str, summary: &str, ts: i64, duration_ms: Option<i64>) -> TrajectoryCellDto {
        TrajectoryCellDto {
            kind: kind.into(),
            summary: summary.into(),
            ts: Some(ts),
            duration_ms,
            ..Default::default()
        }
    }

    #[test]
    fn turn_timing_splits_input_model_and_tools() {
        let cells = vec![
            cell("user", "q", 1000, None),
            cell("assistant", "a", 1500, Some(400)),
            cell("tool", "t", 1900, Some(200)),
        ];
        assert_eq!(turn_timing(&cells), Some((500, 400, 200)));
    }

    #[test]
    fn cell_matches_searches_summary_and_detail() {
        let mut tool = cell("tool", "python", 0, None);
        tool.detail_input = Some(r#"{"code":"df.describe()"}"#.into());
        tool.detail_output = Some("count 612".into());
        assert!(cell_matches(&tool, "describe"));
        assert!(cell_matches(&tool, "612"));
        assert!(!cell_matches(&tool, "volcano"));
    }

    #[test]
    fn duration_gantt_places_cells_on_lanes() {
        let user = cell("user", "q", 0, None);
        let assistant = cell("assistant", "a", 100, Some(100));
        let tool = cell("tool", "t", 200, Some(100));
        let rows = vec![
            ("1:0".into(), 1, &user),
            ("1:1".into(), 1, &assistant),
            ("1:2".into(), 1, &tool),
        ];
        let segs = gantt_segments(TimelineAxis::Duration, &rows);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].lane, "input");
        assert_eq!(segs[1].lane, "model");
        assert_eq!(segs[2].lane, "tools");
        assert!(segs[2].left_pct > segs[0].left_pct);
        let span: f64 = segs.iter().map(|s| s.width_pct).sum();
        assert!((span - 100.0).abs() < 0.01);
    }

    #[test]
    fn duration_gantt_fills_track_when_timings_are_missing() {
        let user = cell("user", "q", 0, None);
        let assistant = cell("assistant", "a", 1, None);
        let tool = cell("tool", "t", 2, None);
        let rows = vec![
            ("1:0".into(), 1, &user),
            ("1:1".into(), 1, &assistant),
            ("1:2".into(), 1, &tool),
        ];
        let segs = gantt_segments(TimelineAxis::Duration, &rows);
        assert_eq!(segs.len(), 3);
        assert!((segs[0].width_pct - 100.0 / 3.0).abs() < 0.01);
        let span: f64 = segs.iter().map(|s| s.width_pct).sum();
        assert!((span - 100.0).abs() < 0.01);
    }

    #[test]
    fn gantt_skips_usage_cells() {
        let user = cell("user", "q", 0, None);
        let mut usage = cell("usage", "round 1", 1, None);
        usage.usage = Some(crate::dto::TrajectoryUsageDto {
            round: 1,
            ..Default::default()
        });
        let tool = cell("tool", "t", 2, Some(10));
        let rows = vec![
            ("1:0".into(), 1, &user),
            ("1:1".into(), 1, &usage),
            ("1:2".into(), 1, &tool),
        ];
        let segs = gantt_segments(TimelineAxis::Calls, &rows);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].lane, "input");
        assert_eq!(segs[1].lane, "tools");
    }

    #[test]
    fn calls_gantt_gives_equal_width_slots() {
        let a = cell("user", "q", 0, None);
        let b = cell("tool", "t", 10, Some(5));
        let rows = vec![("1:0".into(), 1, &a), ("1:1".into(), 1, &b)];
        let segs = gantt_segments(TimelineAxis::Calls, &rows);
        assert_eq!(segs.len(), 2);
        assert!((segs[0].width_pct - 50.0).abs() < 0.01);
        assert!((segs[1].left_pct - 50.0).abs() < 0.01);
    }

    #[test]
    fn cell_raw_json_includes_kind_and_summary() {
        let cell = cell("user", "Analyze the ESR1 dataset", 1, None);
        let raw = cell_raw_json(&cell);
        assert!(raw.contains("\"kind\": \"user\""));
        assert!(raw.contains("Analyze the ESR1 dataset"));
    }

    #[test]
    fn failed_tool_status_is_error() {
        let mut tool = cell("tool", "boom", 0, Some(8));
        tool.ok = Some(false);
        tool.is_error = true;
        assert_eq!(cell_status_key(&tool, false), "trajectory.status.error");
        let pending = cell("tool", "wait", 0, None);
        assert_eq!(cell_status_key(&pending, true), "trajectory.status.running");
        assert_eq!(
            cell_status_key(&cell("user", "q", 0, None), false),
            "trajectory.status.completed"
        );
    }
}
