use super::*;

// Only our file panel sets this type. Plain text/session drags must not become files.
pub(crate) const WORKSPACE_PATH_DRAG_TYPE: &str = "application/x-wisp-workspace-path";

pub(crate) fn start_workspace_path_drag(ev: &web_sys::DragEvent, path: &str) {
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        dt.set_effect_allowed("copy");
        let _ = dt.set_data(WORKSPACE_PATH_DRAG_TYPE, path);
    }
}

pub(crate) fn has_drag_type(dt: &web_sys::DataTransfer, kind: &str) -> bool {
    dt.types()
        .iter()
        .any(|value| value.as_string().as_deref() == Some(kind))
}

pub(crate) fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

pub(crate) fn now_secs() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

pub(crate) fn step_tool_meta(
    locale: Locale,
    duration_ms: Option<u64>,
    started_at_ms: Option<u64>,
    ok: Option<bool>,
    lines: usize,
    now: u64,
) -> Option<String> {
    let dur = duration_ms.map(format_duration_ms).or_else(|| {
        (ok.is_none())
            .then_some(started_at_ms?)
            .map(|start| format_duration_ms(now.saturating_sub(start)))
    });
    let line_label = (lines > 0 && ok != Some(false))
        .then(|| tf(locale, "chat.step_lines", &[("n", &lines.to_string())]));
    match (dur, line_label) {
        (Some(d), Some(l)) => Some(format!("{d} · {l}")),
        (Some(d), None) => Some(d),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    }
}

pub(crate) fn finalize_tool_duration(
    started_at_ms: &mut Option<u64>,
    store: &mut Option<u64>,
    event_ms: u64,
) {
    let elapsed = if event_ms > 0 {
        event_ms
    } else if let Some(start) = started_at_ms.take() {
        now_ms().saturating_sub(start)
    } else {
        0
    };
    if elapsed > 0 {
        *store = Some(elapsed);
    }
    started_at_ms.take();
}

pub(crate) fn allow_drop(ev: &web_sys::DragEvent) {
    ev.prevent_default();
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        let _ = dt.set_drop_effect("move");
    }
}

pub(crate) fn drag_session_id(ev: &web_sys::DragEvent, cached: Option<String>) -> Option<String> {
    cached.filter(|s| !s.is_empty()).or_else(|| {
        ev.data_transfer()
            .and_then(|dt| dt.get_data("text/plain").ok())
            .filter(|s| !s.is_empty())
    })
}

pub(crate) fn start_session_drag(ev: &web_sys::DragEvent, id: &str) {
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        let _ = dt.set_effect_allowed("move");
        let _ = dt.set_data("text/plain", id);
    }
}
