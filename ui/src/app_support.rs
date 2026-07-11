use super::{
    HOME_SEARCH_ARTIFACT_LIMIT, HOME_SEARCH_PROJECT_LIMIT, HOME_SEARCH_SESSION_LIMIT,
    THEME_STORAGE_KEY,
};
use crate::bindings::{
    invoke, invoke_checked, mount_preview, open_external_url, schedule_highlight, upload_files,
    upload_input_files, upload_pasted_images,
};
use crate::dto::*;
use crate::i18n::{localize_backend, tf, t, use_locale, Locale};
use crate::text::{
    dom_value, event_target_value, extract_href_from_tag, fasta_seq_count, fenced_blocks, file_kind,
    format_duration_ms, html_escape, is_external_href, is_separator, is_table_row,
    md_inline_to_html, md_to_html, next_artifact_id,
    normalize_path, opens_in_system_browser, parent_path, parse_csv_line, provider_defaults,
    provider_value, split_row, tool_lang, unique_dom_id,
};
use leptos::{ev, window_event_listener, *};
use serde_wasm_bindgen::to_value;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

pub(super) fn load_theme_mode() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(THEME_STORAGE_KEY).ok().flatten())
        .filter(|mode| matches!(mode.as_str(), "light" | "dark" | "system"))
        .unwrap_or_else(|| "system".into())
}

pub(super) fn apply_theme_mode(mode: &str) {
    let Some(window) = web_sys::window() else { return; };
    if let Some(root) = window.document().and_then(|d| d.document_element()) {
        let _ = root.set_attribute("data-theme", mode);
    }
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item(THEME_STORAGE_KEY, mode);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ComposerSendAction {
    Normal,
    PlanFirst,
    BranchNew,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComposerPickerMode {
    Artifact,
    Session,
    Skill,
}

#[derive(Clone, PartialEq)]
pub(super) enum ComposerPickerItem {
    Artifact(ArtifactInfo),
    Session(SessionSearchInfo),
    Skill(SkillRow),
}

#[derive(Clone)]
pub(super) enum ComposerReferenceChip {
    Artifact { id: String, name: String },
    Session { id: String, title: String, project_name: String },
    Skill { name: String },
}

#[derive(Clone, PartialEq)]
pub(super) enum CommandPaletteItem {
    Project(ProjectSummary),
    Artifact(ArtifactInfo),
    Session(SessionSearchInfo),
    Command(&'static str),
}

#[derive(Clone, PartialEq)]
pub(super) struct CommandAction {
    pub(super) id: &'static str,
    pub(super) icon: &'static str,
    pub(super) title: String,
    pub(super) group: String,
    pub(super) shortcut: &'static str,
}

impl ComposerReferenceChip {
    pub(super) fn key(&self) -> String {
        match self {
            Self::Artifact { id, .. } => format!("artifact:{id}"),
            Self::Session { id, .. } => format!("session:{id}"),
            Self::Skill { name } => format!("skill:{name}"),
        }
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Artifact { name, .. } | Self::Skill { name } => name.clone(),
            Self::Session { title, project_name, .. } => format!("{project_name} / {title}"),
        }
    }

    pub(super) fn arg(&self) -> ComposerReferenceArg {
        match self {
            Self::Artifact { id, .. } => ComposerReferenceArg::Artifact { id: id.clone() },
            Self::Session { id, .. } => ComposerReferenceArg::Session { id: id.clone() },
            Self::Skill { name } => ComposerReferenceArg::Skill { name: name.clone() },
        }
    }
}

#[derive(Clone)]
pub(super) enum FolderModal {
    Create,
    Rename(String),
}

#[derive(Clone)]
pub(super) enum UiConfirm {
    DeleteFolder(String),
    DeleteSession(String),
}

pub(super) fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

pub(super) fn step_tool_meta(
    locale: Locale,
    duration_ms: Option<u64>,
    started_at_ms: Option<u64>,
    ok: Option<bool>,
    lines: usize,
    now: u64,
) -> Option<String> {
    let dur = duration_ms
        .map(format_duration_ms)
        .or_else(|| {
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

pub(super) fn finalize_tool_duration(
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

pub(super) fn allow_drop(ev: &web_sys::DragEvent) {
    ev.prevent_default();
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        let _ = dt.set_drop_effect("move");
    }
}

pub(super) fn drag_session_id(ev: &web_sys::DragEvent, cached: Option<String>) -> Option<String> {
    cached.filter(|s| !s.is_empty()).or_else(|| {
        ev.data_transfer()
            .and_then(|dt| dt.get_data("text/plain").ok())
            .filter(|s| !s.is_empty())
    })
}

pub(super) fn start_session_drag(ev: &web_sys::DragEvent, id: &str) {
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        let _ = dt.set_effect_allowed("move");
        let _ = dt.set_data("text/plain", id);
    }
}

pub(super) fn composer_attachment_key(name: &str, idx: usize) -> String {
    format!("att-{idx}-{name}")
}

pub(super) const COMPOSER_H_DEFAULT: f64 = 220.0;
pub(super) const COMPOSER_H_MIN: f64 = 80.0;
pub(super) const COMPOSER_H_MAX: f64 = 400.0;
pub(super) const COMPOSER_H_KEY: &str = "composerHeight";
pub(super) const COMPOSER_H_SAVED_KEY: &str = "composerHeightCustom";
pub(super) const SIDEBAR_W_DEFAULT: f64 = 248.0;
pub(super) const SIDEBAR_W_MIN: f64 = 200.0;
pub(super) const SIDEBAR_W_MAX: f64 = 520.0;
pub(super) const SIDEBAR_W_KEY: &str = "sidebarWidth";

pub(super) fn load_composer_h() -> f64 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(COMPOSER_H_KEY).ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(COMPOSER_H_DEFAULT)
        .clamp(COMPOSER_H_MIN, COMPOSER_H_MAX)
}

pub(super) fn composer_h_custom() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(COMPOSER_H_SAVED_KEY).ok().flatten())
        .is_some_and(|v| v == "1")
}

pub(super) fn save_composer_h(h: f64) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(COMPOSER_H_KEY, &h.to_string());
        let _ = s.set_item(COMPOSER_H_SAVED_KEY, "1");
    }
}

pub(super) fn load_sidebar_w() -> f64 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SIDEBAR_W_KEY).ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(SIDEBAR_W_DEFAULT)
        .clamp(SIDEBAR_W_MIN, SIDEBAR_W_MAX)
}

pub(super) fn save_sidebar_w(w: f64) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(SIDEBAR_W_KEY, &w.to_string());
    }
}

pub(super) fn parse_upload_results(v: JsValue) -> Vec<UploadFileResult> {
    if v.is_null() || v.is_undefined() {
        return vec![];
    }
    serde_wasm_bindgen::from_value(v).unwrap_or_default()
}

pub(super) fn file_list_len(files: &JsValue) -> usize {
    js_sys::Reflect::get(files, &JsValue::from_str("length"))
        .ok()
        .and_then(|n| n.as_f64())
        .map(|n| n as usize)
        .unwrap_or(0)
}

pub(super) fn begin_uploads(attachments: RwSignal<Vec<ComposerAttachment>>, uploading: RwSignal<bool>, count: usize) {
    if count == 0 {
        return;
    }
    attachments.update(|items| {
        for i in 0..count {
            items.push(ComposerAttachment::Uploading {
                key: format!("up-{}-{i}", js_sys::Date::now()),
                name: String::new(),
            });
        }
    });
    uploading.set(true);
}

pub(super) fn finish_uploads(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    uploading: RwSignal<bool>,
    results: Vec<UploadFileResult>,
) {
    uploading.set(false);
    attachments.update(|items| {
        items.retain(|a| !matches!(a, ComposerAttachment::Uploading { .. }));
        for result in results {
            let name = result
                .info
                .as_ref()
                .map(|i| i.name.clone())
                .or(result.filename.clone())
                .unwrap_or_else(|| "file".into());
            let key = composer_attachment_key(&name, items.len());
            if result.ok {
                if let Some(info) = result.info {
                    items.push(ComposerAttachment::Ready { key, name, path: info.path });
                }
            } else {
                items.push(ComposerAttachment::Error {
                    key,
                    name,
                    error: result.error.unwrap_or_else(|| "Upload failed".into()),
                });
            }
        }
    });
}

// Closes the `<details class="settings-add-menu">` a menu button lives in,
// mirroring native `<select>`-style auto-close so the menu doesn't linger
// open after the user picks an option.
pub(super) fn close_details_ancestor(ev: &web_sys::MouseEvent) {
    let el = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    if let Some(details) = el.and_then(|e| e.closest("details").ok().flatten()) {
        details.remove_attribute("open").ok();
    }
}

pub(super) fn queue_uploads(attachments: RwSignal<Vec<ComposerAttachment>>, uploading: RwSignal<bool>, files: JsValue) {
    let count = file_list_len(&files);
    begin_uploads(attachments, uploading, count);
    spawn_local(async move {
        finish_uploads(attachments, uploading, parse_upload_results(upload_files(files).await));
    });
}

pub(super) fn upload_from_input(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    uploading: RwSignal<bool>,
    input_id: &'static str,
) {
    uploading.set(true);
    spawn_local(async move {
        let v = upload_input_files(input_id).await;
        finish_uploads(attachments, uploading, parse_upload_results(v));
    });
}

pub(super) fn upload_from_paste(
    attachments: RwSignal<Vec<ComposerAttachment>>,
    uploading: RwSignal<bool>,
    event: JsValue,
    count: usize,
) {
    begin_uploads(attachments, uploading, count);
    spawn_local(async move {
        let v = upload_pasted_images(event).await;
        finish_uploads(attachments, uploading, parse_upload_results(v));
    });
}

pub(super) fn attachment_paths(items: &[ComposerAttachment]) -> Vec<String> {
    items
        .iter()
        .filter_map(|a| match a {
            ComposerAttachment::Ready { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn refresh_execution_contexts(into: RwSignal<Vec<ExecutionContext>>) {
    spawn_local(async move {
        let v = invoke("list_execution_contexts", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ExecutionContext>>(v) {
            into.set(list);
        }
    });
}

pub(super) fn refresh_runs(into: RwSignal<Vec<RunRecord>>) {
    spawn_local(async move {
        let v = invoke("list_runs", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<RunRecord>>(v) {
            into.set(list);
        }
    });
}

pub(super) fn context_capability_summary(ctx: &ExecutionContext) -> String {
    let parsed = serde_json::from_str::<serde_json::Value>(&ctx.capabilities_json).ok();
    let mut parts = Vec::new();
    if let Some(v) = parsed.as_ref() {
        let os = v.get("os").and_then(|x| x.as_str()).unwrap_or_default();
        let arch = v.get("arch").and_then(|x| x.as_str()).unwrap_or_default();
        match (os.is_empty(), arch.is_empty()) {
            (false, false) => parts.push(format!("{os}/{arch}")),
            (false, true) => parts.push(os.to_string()),
            (true, false) => parts.push(arch.to_string()),
            (true, true) => {}
        }
        for key in ["gpu_summary", "scheduler", "python"] {
            if let Some(s) = v.get(key).and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                parts.push(s.to_string());
            }
        }
    }
    if parts.is_empty() {
        ctx.last_probe_status.clone().unwrap_or_else(|| "not probed".into())
    } else {
        parts.join(" · ")
    }
}

pub(super) fn run_title(run: &RunRecord) -> String {
    if !run.title.trim().is_empty() {
        run.title.clone()
    } else {
        run.command.clone().unwrap_or_else(|| run.id.clone())
    }
}

pub(super) fn message_with_attachments(text: &str, paths: &[String]) -> String {
    let body = text.trim();
    if paths.is_empty() {
        return body.to_string();
    }
    let files = paths.join(", ");
    if body.is_empty() {
        format!("Uploaded files: {files}")
    } else {
        format!("{body}\n\nUploaded files: {files}")
    }
}

pub(super) fn message_with_references(
    text: &str,
    paths: &[String],
    references: &[ComposerReferenceChip],
) -> String {
    let mut message = message_with_attachments(text, paths);
    for (label, items) in [
        ("Attached artifacts", references.iter().filter_map(|r| match r { ComposerReferenceChip::Artifact { name, .. } => Some(name.clone()), _ => None }).collect::<Vec<_>>()),
        ("Attached sessions", references.iter().filter_map(|r| match r { ComposerReferenceChip::Session { title, project_name, .. } => Some(format!("{project_name} / {title}")), _ => None }).collect::<Vec<_>>()),
        ("Selected skills", references.iter().filter_map(|r| match r { ComposerReferenceChip::Skill { name } => Some(name.clone()), _ => None }).collect::<Vec<_>>()),
    ] {
        if !items.is_empty() {
            message.push_str(&format!("\n\n{label}: {}", items.join(", ")));
        }
    }
    message
}

pub(super) fn plan_first_message(message: &str) -> String {
    format!(
        "Plan first before executing. Write a concise plan for the request, then wait for my confirmation before taking action.\n\nRequest:\n{}",
        message.trim()
    )
}

/// If the composer text ends in an `@`, `#`, or `/` token, return its byte
/// offset, picker mode, and query. ponytail: this is end-of-text only; upgrade
/// to caret-index scanning when editing mentions in the middle matters.
pub(super) fn active_composer_trigger(text: &str) -> Option<(usize, ComposerPickerMode, String)> {
    let (at, trigger) = text
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '@' | '#' | '/'))?;
    if at > 0 && !text[..at].chars().next_back()?.is_whitespace() {
        return None;
    }
    let query = &text[at + 1..];
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    let mode = match trigger {
        '@' => ComposerPickerMode::Artifact,
        '#' => ComposerPickerMode::Session,
        '/' => ComposerPickerMode::Skill,
        _ => return None,
    };
    Some((at, mode, query.to_string()))
}

pub(super) fn scroll_picker_item(selector: &str, index: usize) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else { return; };
    let Ok(items) = document.query_selector_all(selector) else { return; };
    if let Some(item) = items.item(index as u32) {
        item.unchecked_into::<web_sys::Element>().scroll_into_view();
    }
}

pub(super) fn scroll_to_transcript(index: usize) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if let Ok(Some(row)) = document.query_selector(&format!("[data-ui-index=\"{index}\"]")) {
        row.scroll_into_view();
    }
}

#[cfg(test)]
mod mention_tests {
    use super::{active_composer_trigger, ComposerPickerMode};

    #[test]
    fn detects_mention_at_end() {
        assert!(matches!(active_composer_trigger("look at @qc"), Some((8, ComposerPickerMode::Artifact, q)) if q == "qc"));
        assert!(matches!(active_composer_trigger("#old"), Some((0, ComposerPickerMode::Session, q)) if q == "old"));
        assert!(matches!(active_composer_trigger("/boltz"), Some((0, ComposerPickerMode::Skill, q)) if q == "boltz"));
    }

    #[test]
    fn ignores_non_mentions() {
        assert_eq!(active_composer_trigger("no trigger"), None);
        assert_eq!(active_composer_trigger("email a@b.com"), None);
        assert_eq!(active_composer_trigger("@qc then more"), None);
    }
}

pub(super) fn js_error_text(err: JsValue) -> String {
    err.as_string()
        .or_else(|| js_sys::Reflect::get(&err, &JsValue::from_str("message")).ok().and_then(|v| v.as_string()))
        .unwrap_or_else(|| t(Locale::En, "err.unknown").into())
}

pub(super) fn show_copy_toast() {
    let Some(window) = web_sys::window() else { return; };
    let Some(document) = window.document() else { return; };
    if let Some(old) = document.get_element_by_id("copy-toast") { old.remove(); }
    let Ok(toast) = document.create_element("div") else { return; };
    toast.set_id("copy-toast");
    toast.set_class_name("copy-toast");
    toast.set_text_content(Some(if document.document_element().and_then(|el| el.get_attribute("lang")).as_deref() == Some("zh") { "已复制" } else { "Copied" }));
    let Some(body) = document.body() else { return; };
    if body.append_child(&toast).is_err() { return; }
    let remove = Closure::once(move || toast.remove());
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(remove.as_ref().unchecked_ref(), 1_600);
    remove.forget();
}

pub(super) fn copy_text(text: String) {
    if text.is_empty() { return; }
    spawn_local(async move {
        let Some(window) = web_sys::window() else { return; };
        let promise = window.navigator().clipboard().write_text(&text);
        if wasm_bindgen_futures::JsFuture::from(promise).await.is_ok() { show_copy_toast(); }
    });
}

fn normalize_table_copy_cell(text: &str) -> String {
    let mut out = String::new();
    for part in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

fn table_data_to_tsv(t: &TableData) -> String {
    let mut lines = Vec::with_capacity(t.rows.len() + usize::from(!t.headers.is_empty()));
    if !t.headers.is_empty() {
        lines.push(
            t.headers
                .iter()
                .map(|cell| normalize_table_copy_cell(cell))
                .collect::<Vec<_>>()
                .join("\t"),
        );
    }
    lines.extend(t.rows.iter().map(|row| {
        row.iter()
            .map(|cell| normalize_table_copy_cell(cell))
            .collect::<Vec<_>>()
            .join("\t")
    }));
    lines.join("\n")
}

fn html_table_to_tsv(table: &web_sys::HtmlTableElement) -> String {
    let rows = table.rows();
    let mut lines = Vec::with_capacity(rows.length() as usize);
    for i in 0..rows.length() {
        let Some(row) = rows.item(i) else { continue };
        let Ok(row) = row.dyn_into::<web_sys::HtmlTableRowElement>() else { continue };
        let cells = row.cells();
        let mut vals = Vec::with_capacity(cells.length() as usize);
        for j in 0..cells.length() {
            let Some(cell) = cells.item(j) else { continue };
            vals.push(normalize_table_copy_cell(&cell.text_content().unwrap_or_default()));
        }
        if !vals.is_empty() {
            lines.push(vals.join("\t"));
        }
    }
    lines.join("\n")
}

fn wrap_markdown_tables_with_copy_controls(html: String, locale: Locale) -> String {
    let copy_label = html_escape(&t(locale, "table.copy"));
    let mut out = String::with_capacity(html.len());
    let mut rest = html.as_str();
    while let Some(start) = rest.find("<table") {
        out.push_str(&rest[..start]);
        let table_rest = &rest[start..];
        let Some(end) = table_rest.find("</table>") else {
            out.push_str(table_rest);
            return out;
        };
        let table_html = &table_rest[..end + "</table>".len()];
        out.push_str(&format!(
            r#"<div class="md-table-card"><div class="tbl-head"><button type="button" class="tbl-copy md-table-copy" title="{copy_label}" aria-label="{copy_label}">{copy_label}</button></div><div class="tbl-wrap">{table_html}</div></div>"#
        ));
        rest = &table_rest[end + "</table>".len()..];
    }
    out.push_str(rest);
    out
}

pub(super) fn normalize_settings_mut(cfg: &mut Settings) {
    cfg.provider = provider_value(&cfg.provider).into();
    cfg.api_url = cfg.api_url.trim().into();
    cfg.model = cfg.model.trim().into();
}

pub(super) fn normalized_settings(mut cfg: Settings) -> Settings {
    normalize_settings_mut(&mut cfg);
    cfg
}

pub(super) fn settings_required_error_key(cfg: &Settings, key: &str) -> Option<&'static str> {
    if cfg.api_url.trim().is_empty() {
        return Some("err.api_url_required");
    }
    if cfg.model.trim().is_empty() {
        return Some("err.model_required");
    }
    let has_new_key = !key.trim().is_empty();
    if !cfg.has_api_key && !has_new_key {
        return Some("err.api_key_required");
    }
    None
}

/// Single source of truth for `invoke` argument payloads.
///
/// Tauri v2 deserializes command arguments from JS **camelCase** keys onto the
/// Rust **snake_case** parameters. A snake_case key (`session_id`) never binds:
/// an `Option` param silently becomes `None`, which once made every send fork a
/// brand-new conversation instead of continuing the active one. Keep every
/// multi-word key camelCase here; `tauri_args_tests` pins them.
pub(super) mod tauri_args {
    use serde_json::{json, Value};

    pub fn stop_agent(session_id: &Option<String>) -> Value {
        json!({ "sessionId": session_id })
    }
    pub fn review_session(session_id: &Option<String>) -> Value {
        json!({ "sessionId": session_id })
    }
    pub fn branch_session(
        session_id: &Option<String>,
        title: Option<&str>,
        user_index: Option<usize>,
    ) -> Value {
        let mut payload = json!({ "sessionId": session_id });
        if let Some(title) = title.map(str::trim).filter(|s| !s.is_empty()) {
            payload["title"] = json!(title);
        }
        if let Some(user_index) = user_index {
            payload["userIndex"] = json!(user_index);
        }
        payload
    }
    pub fn rewind_session(session_id: &Option<String>, user_index: usize) -> Value {
        json!({ "sessionId": session_id, "userIndex": user_index })
    }
    pub fn confirm_response(
        session_id: &str,
        approved: bool,
        feedback: Option<&str>,
        scope: Option<&str>,
    ) -> Value {
        let mut payload = json!({ "sessionId": session_id, "approved": approved });
        if let Some(feedback) = feedback.map(str::trim).filter(|s| !s.is_empty()) {
            payload["feedback"] = json!(feedback);
        }
        if let Some(scope) = scope.map(str::trim).filter(|s| !s.is_empty()) {
            payload["scope"] = json!(scope);
        }
        payload
    }
    pub fn read_file(path: &str, max_bytes: Option<u64>) -> Value {
        match max_bytes {
            Some(n) => json!({ "path": path, "maxBytes": n }),
            None => json!({ "path": path }),
        }
    }
}

#[cfg(test)]
mod tauri_args_tests {
    use super::*;

    // Guard the exact regression: `session_id` must reach the backend as the
    // camelCase `sessionId`, or `send_message` starts a new conversation.
    #[test]
    fn send_message_args_serialize_camel_case() {
        let v = serde_json::to_value(SendMessageArgs {
            session_id: Some("frame-1".into()),
            message: "hi".into(),
            attachments: vec!["a.png".into()],
            references: vec![],
            resume: false,
        })
        .unwrap();
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["message"], "hi");
        assert_eq!(v["attachments"][0], "a.png");
        assert!(v.get("session_id").is_none(), "snake_case key would bind to None on the backend");
    }

    #[test]
    fn session_command_args_use_camel_case_keys() {
        let sid = Some("frame-1".to_string());

        let v = tauri_args::stop_agent(&sid);
        assert_eq!(v["sessionId"], "frame-1");
        assert!(v.get("session_id").is_none());

        let v = tauri_args::review_session(&sid);
        assert_eq!(v["sessionId"], "frame-1");

        let v = tauri_args::branch_session(&sid, Some("fork here"), Some(2));
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["title"], "fork here");
        assert_eq!(v["userIndex"], 2);
        assert!(v.get("session_id").is_none());
        assert!(v.get("user_index").is_none());

        let v = tauri_args::rewind_session(&sid, 3);
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["userIndex"], 3);
        assert!(v.get("user_index").is_none());

        let v = tauri_args::confirm_response("frame-1", true, None, Some("once"));
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["approved"], true);
        assert_eq!(v["scope"], "once");
        assert!(v.get("feedback").is_none());

        let v = tauri_args::confirm_response("frame-1", false, Some("split the plan"), None);
        assert_eq!(v["feedback"], "split the plan");
        assert!(v.get("scope").is_none());

        let v = tauri_args::read_file("a.txt", Some(1024));
        assert_eq!(v["path"], "a.txt");
        assert_eq!(v["maxBytes"], 1024);
        assert!(v.get("max_bytes").is_none());
    }

    // The agent is told to emit absolute paths, so a clicked file link must reach
    // the backend intact. Stripping the leading slash turned `/Users/…/fig.png`
    // into a bad root-relative path that 404'd on click (#12).
    #[test]
    fn normalize_path_keeps_absolute_paths() {
        assert_eq!(normalize_path("/Users/x/proj/results/fig.png"), "/Users/x/proj/results/fig.png");
        assert_eq!(normalize_path("C:\\proj\\out.csv"), "C:\\proj\\out.csv");
        // Redundant current-dir prefixes are still trimmed; relative stays relative.
        assert_eq!(normalize_path("./results/fig.png"), "results/fig.png");
        assert_eq!(normalize_path(".\\results\\fig.png"), "results\\fig.png");
        assert_eq!(normalize_path("results/fig.png"), "results/fig.png");
        assert_eq!(normalize_path("  /a/b.txt  "), "/a/b.txt");
        assert_eq!(
            normalize_path("figures/panel_I_heatmap_4genes_median.png/.pdf"),
            "figures/panel_I_heatmap_4genes_median.png"
        );
        assert_eq!(normalize_path("./figures/plot.JPG/.PDF"), "figures/plot.JPG");
        assert_eq!(normalize_path("C:\\proj\\fig.png\\.pdf"), "C:\\proj\\fig.png");
    }

    #[test]
    fn collect_artifacts_normalizes_image_pdf_shorthand() {
        let items = vec![ChatItem::Assistant {
            text: "`figures/panel_I_heatmap_4genes_median.png/.pdf`".into(),
            model: None,
        }];
        let arts = collect_artifacts(&items, Locale::En, &mut ProtoCache::new());
        let a = arts.iter().find(|a| a.name == "panel_I_heatmap_4genes_median.png").unwrap();
        assert_eq!(a.kind, "image");
        match &a.data {
            PreviewData::File { path, kind } => {
                assert_eq!(path, "figures/panel_I_heatmap_4genes_median.png");
                assert_eq!(kind, "image");
            }
            _ => panic!("expected file artifact"),
        }
    }
}

pub(super) fn split_tags(raw: &str) -> Vec<String> {
    raw.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect::<BTreeSet<_>>().into_iter().collect()
}

pub(super) fn join_tags(tags: &[String]) -> String {
    tags.join(", ")
}

pub(super) fn skill_matches_filter(skill: &SkillRow, tag: &str, query: &str) -> bool {
    let tag_match = match tag {
        "" => true,
        "__untagged" => skill.tags.is_empty(),
        t => skill.tags.iter().any(|s| s == t),
    };
    let q = query.trim().to_ascii_lowercase();
    tag_match && (q.is_empty() || skill.name.to_ascii_lowercase().contains(&q) || skill.description.to_ascii_lowercase().contains(&q))
}

pub(super) fn refresh_capabilities(caps: RwSignal<Option<Capabilities>>) {
    spawn_local(async move {
        let v = invoke("get_capabilities", JsValue::UNDEFINED).await;
        if let Ok(data) = serde_wasm_bindgen::from_value::<Capabilities>(v) {
            caps.set(Some(data));
        }
    });
}

pub(super) fn begin_pending_turn(pending: RwSignal<HashMap<String, usize>>, running: RwSignal<HashSet<String>>, id: &str) {
    pending.update(|m| {
        *m.entry(id.to_string()).or_insert(0) += 1;
    });
    running.update(|r| {
        r.insert(id.to_string());
    });
}

pub(super) fn finish_pending_turn(pending: RwSignal<HashMap<String, usize>>, running: RwSignal<HashSet<String>>, id: &str) {
    let remaining = pending.with(|m| m.get(id).copied().unwrap_or(0));
    if remaining > 1 {
        pending.update(|m| {
            if let Some(n) = m.get_mut(id) {
                *n -= 1;
            }
        });
        return;
    }
    pending.update(|m| {
        m.remove(id);
    });
    running.update(|r| {
        r.remove(id);
    });
}

pub(super) fn clear_running_if_idle(pending: RwSignal<HashMap<String, usize>>, running: RwSignal<HashSet<String>>, id: &str) {
    if pending.with(|m| m.get(id).copied().unwrap_or(0)) == 0 {
        running.update(|r| {
            r.remove(id);
        });
    }
}

pub(super) fn strip_approval_pending(items: &mut Vec<ChatItem>) {
    items.retain(|i| !matches!(i, ChatItem::ApprovalPending { .. }));
}

pub(super) fn upsert_review(items: &mut Vec<ChatItem>, report: ReviewReport) {
    if let Some(existing) = items.iter_mut().find(|item| {
        matches!(item, ChatItem::Review(current) if current.id == report.id)
    }) {
        *existing = ChatItem::Review(report);
    } else {
        let index = trailing_queue_start(items);
        items.insert(index, ChatItem::Review(report));
    }
}

#[cfg(test)]
mod review_tests {
    use super::upsert_review;
    use crate::dto::{ChatItem, ReviewReport};

    fn report(id: &str, summary: &str) -> ReviewReport {
        ReviewReport {
            id: id.into(),
            summary: summary.into(),
            findings: vec![],
            reviewer_model: "review-model".into(),
        }
    }

    #[test]
    fn follow_up_review_replaces_the_original_card() {
        let mut items = vec![ChatItem::Assistant {
            text: "answer".into(),
            model: None,
        }];
        upsert_review(&mut items, report("r1", "first"));
        upsert_review(&mut items, report("r1", "verified"));

        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[1],
            ChatItem::Review(report) if report.summary == "verified"
        ));
    }
}

pub(super) fn is_error_assistant(item: &ChatItem) -> bool {
    matches!(item, ChatItem::Assistant { text, .. } if text.starts_with("Error: "))
}

pub(super) fn strip_error_at(items: &mut Vec<ChatItem>, idx: usize) {
    if idx < items.len() && is_error_assistant(&items[idx]) {
        items.remove(idx);
    }
}

pub(super) fn ensure_streaming_assistant(items: &mut Vec<ChatItem>, model: Option<String>) {
    let queue_start = trailing_queue_start(items);
    let has_blank = items[..queue_start].iter().rev().any(|i| {
        matches!(i, ChatItem::Assistant { text, .. } if text.trim().is_empty())
    });
    if !has_blank {
        items.insert(
            queue_start,
            ChatItem::Assistant {
                text: String::new(),
                model,
            },
        );
    }
}

pub(super) fn last_tool_input(items: &[ChatItem], tool: &str) -> String {
    items
        .iter()
        .rev()
        .find_map(|i| match i {
            ChatItem::Tool {
                name,
                input,
                ok: None,
                ..
            } if name == tool => Some(input.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

pub(super) fn trailing_queue_start(items: &[ChatItem]) -> usize {
    items
        .iter()
        .rposition(|item| !matches!(item, ChatItem::QueuedUser(_)))
        .map(|i| i + 1)
        .unwrap_or(0)
}

pub(super) fn start_user_turn(items: &mut Vec<ChatItem>, text: String, model: Option<String>) {
    if let Some(idx) = items
        .iter()
        .position(|item| matches!(item, ChatItem::QueuedUser(s) if s == &text))
    {
        items.splice(
            idx..=idx,
            [
                ChatItem::User(text),
                ChatItem::Assistant {
                    text: String::new(),
                    model,
                },
            ],
        );
    } else if items.windows(2).any(|pair| {
        matches!(&pair[0], ChatItem::User(s) if s == &text)
            && matches!(&pair[1], ChatItem::Assistant { text: assistant, .. } if assistant.is_empty())
    }) {
        // Normal sends are rendered optimistically. The backend User event is
        // only an acknowledgement in that case, so do not append a duplicate.
    } else {
        items.push(ChatItem::User(text));
        items.push(ChatItem::Assistant {
            text: String::new(),
            model,
        });
    }
}

pub(super) fn append_assistant_delta(items: &mut Vec<ChatItem>, delta: String, model: Option<String>) {
    let queue_start = trailing_queue_start(items);
    if let Some(idx) = items[..queue_start]
        .iter()
        .rposition(|item| matches!(item, ChatItem::Assistant { .. }))
    {
        if let ChatItem::Assistant { text, .. } = &mut items[idx] {
            text.push_str(&delta);
            return;
        }
    }
    items.insert(queue_start, ChatItem::Assistant { text: delta, model });
}

pub(super) fn append_reasoning_delta(items: &mut Vec<ChatItem>, delta: String) {
    let queue_start = trailing_queue_start(items);
    if let Some(idx) = items[..queue_start]
        .iter()
        .rposition(|item| matches!(item, ChatItem::Reasoning(_)))
    {
        if let ChatItem::Reasoning(text) = &mut items[idx] {
            text.push_str(&delta);
            return;
        }
    }
    items.insert(queue_start, ChatItem::Reasoning(delta));
}

pub(super) fn append_stdout_chunk(items: &mut Vec<ChatItem>, chunk: String) {
    let queue_start = trailing_queue_start(items);
    if let Some(idx) = items[..queue_start]
        .iter()
        .rposition(|item| matches!(item, ChatItem::Tool { .. }))
    {
        if let ChatItem::Tool { output, .. } = &mut items[idx] {
            output.push_str(&chunk);
            return;
        }
    }
    items.insert(queue_start, ChatItem::Tool {
        name: "stdout".into(),
        ok: None,
        input: String::new(),
        output: chunk,
        started_at_ms: None,
        duration_ms: None,
    });
}

// --- Streaming delta batching (#65) ------------------------------------------
//
// The backend emits one `agent` event per LLM/stdout chunk. Applying each one
// writes the `items` signal, and every write re-runs the thread view and the
// artifact scan — O(conversation length) work per token, which freezes long
// conversations. Buffer the append-only deltas and flush them on a short timer
// so the signal is written at most ~20×/s regardless of token rate.

pub(super) enum PendingDelta {
    Text(String),
    Reasoning(String),
    Stdout(String),
}

pub(super) type DeltaBuf = Rc<RefCell<HashMap<String, Vec<PendingDelta>>>>;

/// Append a delta to a session's queue, coalescing consecutive same-kind chunks.
pub(super) fn queue_delta(buf: &DeltaBuf, fid: String, d: PendingDelta) {
    let mut map = buf.borrow_mut();
    let q = map.entry(fid).or_default();
    match (q.last_mut(), d) {
        (Some(PendingDelta::Text(s)), PendingDelta::Text(n)) => s.push_str(&n),
        (Some(PendingDelta::Reasoning(s)), PendingDelta::Reasoning(n)) => s.push_str(&n),
        (Some(PendingDelta::Stdout(s)), PendingDelta::Stdout(n)) => s.push_str(&n),
        (_, d) => q.push(d),
    }
}

/// Apply all buffered deltas to their sessions in arrival order.
pub(super) fn flush_delta_buf(
    buf: &DeltaBuf,
    active: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    models: RwSignal<Vec<ModelProfile>>,
) {
    let drained: Vec<_> = buf.borrow_mut().drain().collect();
    if drained.is_empty() {
        return;
    }
    let fallback_model = active_model_label(&models.get_untracked());
    for (fid, deltas) in drained {
        let model = fallback_model.clone();
        route_items(active, items, transcripts, &fid, move |v| {
            for d in deltas {
                match d {
                    PendingDelta::Text(s) => append_assistant_delta(v, s, model.clone()),
                    PendingDelta::Reasoning(s) => append_reasoning_delta(v, s),
                    PendingDelta::Stdout(s) => append_stdout_chunk(v, s),
                }
            }
        });
    }
}

pub(super) fn schedule_delta_flush(
    buf: &DeltaBuf,
    scheduled: &Rc<Cell<bool>>,
    active: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    models: RwSignal<Vec<ModelProfile>>,
) {
    if scheduled.get() {
        return;
    }
    scheduled.set(true);
    let buf = buf.clone();
    let scheduled = scheduled.clone();
    set_timeout(
        move || {
            scheduled.set(false);
            flush_delta_buf(&buf, active, items, transcripts, models);
        },
        std::time::Duration::from_millis(50),
    );
}

pub(super) fn format_relative_time(ts: i64, locale: Locale) -> String {
    if ts <= 0 { return String::new(); }
    let now_ms = js_sys::Date::now();
    let ts_ms = if ts > 1_000_000_000_000 { ts as f64 } else { ts as f64 * 1000.0 };
    let secs = ((now_ms - ts_ms) / 1000.0).max(0.0) as i64;
    if secs < 45 {
        return t(locale, "time.just_now").into();
    }
    if secs < 3600 {
        return tf(locale, "time.minutes", &[("n", &(secs / 60).max(1).to_string())]);
    }
    if secs < 86_400 {
        return tf(locale, "time.hours", &[("n", &(secs / 3600).to_string())]);
    }
    tf(locale, "time.days", &[("n", &(secs / 86_400).to_string())])
}

#[component]
pub(super) fn SessionStatusBadge(status: SessionStatusKind, locale: RwSignal<Locale>) -> impl IntoView {
    let key = status.i18n_key();
    let class = status.css();
    view! {
        <span class=format!("sess-status sess-status-{class}")>
            {move || t(locale.get(), key)}
        </span>
    }
}

pub(super) fn profile_to_form(m: &ModelProfile) -> ModelForm {
    ModelForm {
        id: Some(m.id.clone()),
        label: m.label.clone(),
        provider: m.provider.clone(),
        api_url: m.api_url.clone(),
        model: m.model.clone(),
        max_tokens: if m.max_tokens >= 16 { m.max_tokens } else { 8192 },
        reasoning_effort: m.reasoning_effort.clone(),
        supports_vision: m.supports_vision,
        use_for_vision: m.use_for_vision,
    }
}

pub(super) fn new_model_form() -> ModelForm {
    let (api_url, model) = provider_defaults("openai");
    ModelForm {
        provider: "openai".into(),
        api_url: api_url.into(),
        model: model.into(),
        max_tokens: 8192,
        ..Default::default()
    }
}

pub(super) fn model_form_to_settings(form: &ModelForm, has_api_key: bool) -> Settings {
    let mut cfg = Settings::default();
    cfg.provider = provider_value(&form.provider).into();
    cfg.api_url = form.api_url.trim().into();
    cfg.model = form.model.trim().into();
    cfg.label = form.label.trim().into();
    cfg.has_api_key = has_api_key;
    cfg.max_tokens = form.max_tokens;
    cfg.reasoning_effort = form.reasoning_effort.clone();
    cfg.supports_vision = form.supports_vision;
    cfg
}

pub(super) fn settings_section_label(loc: Locale, section: &str) -> String {
    match section {
        "models" => t(loc, "settings.nav.models"),
        "specialists" => t(loc, "settings.nav.specialists"),
        "memory" => t(loc, "settings.nav.memory"),
        "skills" => t(loc, "settings.nav.skills"),
        "connections" => t(loc, "settings.nav.connections"),
        "credentials" => t(loc, "settings.nav.credentials"),
        "permissions" => t(loc, "settings.nav.permissions"),
        _ => t(loc, "settings.title"),
    }
    .into()
}

/// A field within a credential service group: (credential id, i18n label key,
/// whether to mask the value like a password).
pub(super) struct CredField {
    pub(super) id: &'static str,
    pub(super) label_key: &'static str,
    pub(super) secret: bool,
}

/// A credential service shown in Settings → Credentials: display name, help
/// text, and its fields. Mirrors the backend `CREDENTIALS` registry in
/// models.rs — keep ids in sync.
pub(super) struct CredGroup {
    pub(super) name_key: &'static str,
    pub(super) hint_key: &'static str,
    pub(super) fields: &'static [CredField],
}

pub(super) const CRED_GROUPS: &[CredGroup] = &[
    CredGroup {
        name_key: "cred.openalex.name",
        hint_key: "cred.openalex.hint",
        fields: &[CredField { id: "openalex_api_key", label_key: "cred.openalex_api_key.label", secret: true }],
    },
    CredGroup {
        name_key: "cred.infinisynapse.name",
        hint_key: "cred.infinisynapse.hint",
        fields: &[CredField { id: "infinisynapse_api_key", label_key: "cred.infinisynapse_api_key.label", secret: true }],
    },
    CredGroup {
        name_key: "cred.scimaster.name",
        hint_key: "cred.scimaster.hint",
        fields: &[CredField { id: "scimaster_api_key", label_key: "cred.scimaster_api_key.label", secret: true }],
    },
    CredGroup {
        name_key: "cred.ncbi.name",
        hint_key: "cred.ncbi.hint",
        fields: &[
            CredField { id: "ncbi_api_key", label_key: "cred.ncbi_api_key.label", secret: true },
            CredField { id: "ncbi_email", label_key: "cred.ncbi_email.label", secret: false },
        ],
    },
];

pub(super) fn settings_subpage_label(
    loc: Locale,
    section: &str,
    model_form: Option<&ModelForm>,
    conn_form: Option<&ConnForm>,
    open_conn: Option<&str>,
    memory_selected: Option<&str>,
    specialist_form: Option<&Specialist>,
) -> Option<String> {
    match section {
        "models" => model_form.map(|f| {
            if f.id.is_some() {
                t(loc, "models.edit").into()
            } else {
                t(loc, "models.add").into()
            }
        }),
        "specialists" => specialist_form.map(|s| {
            if s.id.is_empty() { t(loc, "specialists.add") } else { s.name.clone() }
        }),
        "connections" => conn_form
            .map(|f| {
                if f.id.is_some() {
                    t(loc, "conn.edit").into()
                } else {
                    t(loc, "conn.add").into()
                }
            })
            .or_else(|| open_conn.map(|s| s.to_string())),
        "memory" => memory_selected.map(|s| s.to_string()),
        _ => None,
    }
}

pub(super) fn build_conn_json(f: &ConnForm, assign_id: bool) -> serde_json::Value {
    let id = f.id.clone().unwrap_or_else(|| if assign_id {
        format!("conn-{}", (js_sys::Math::random() * 1e9) as u64)
    } else { "test".into() });
    let transport = if f.kind == "http" {
        let headers: Vec<(String,String)> = f.headers.lines().filter_map(|l| l.split_once(':').map(|(k,v)| (k.trim().to_string(), v.trim().to_string()))).collect();
        serde_json::json!({ "kind": "http", "url": f.url.trim(), "headers": headers })
    } else {
        let args: Vec<String> = f.args.split_whitespace().map(|s| s.to_string()).collect();
        serde_json::json!({ "kind": "stdio", "command": f.command.trim(), "args": args, "env": [], "cwd": null })
    };
    serde_json::json!({ "id": id, "name": f.name.trim(), "enabled": f.enabled, "transport": transport })
}

pub(super) fn conn_form_from_row(row: &ConnRow) -> ConnForm {
    match &row.transport {
        ConnTransport::Stdio { command, args, .. } => ConnForm {
            id: Some(row.id.clone()), name: row.name.clone(), kind: "stdio".into(),
            command: command.clone(), args: args.join(" "), url: String::new(), headers: String::new(),
            enabled: row.enabled,
        },
        ConnTransport::Http { url, headers } => ConnForm {
            id: Some(row.id.clone()), name: row.name.clone(), kind: "http".into(),
            command: String::new(), args: String::new(), url: url.clone(),
            headers: headers.iter().map(|(k,v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"),
            enabled: row.enabled,
        },
    }
}

pub(super) fn refresh_dir(cwd: RwSignal<String>, entries: RwSignal<Vec<DirEntry>>) {
    spawn_local(async move {
        let path = cwd.get();
        let v = invoke("list_dir", to_value(&serde_json::json!({ "path": path })).unwrap()).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DirEntry>>(v) {
            entries.set(list);
        }
    });
}

pub(super) fn refresh_file_search(query: RwSignal<String>, hits: RwSignal<Vec<FileSearchHit>>) {
    spawn_local(async move {
        let q = query.get().trim().to_string();
        if q.is_empty() {
            hits.set(vec![]);
            return;
        }
        let v = invoke(
            "search_files",
            to_value(&serde_json::json!({ "query": q, "limit": 200 })).unwrap(),
        )
        .await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<FileSearchHit>>(v) {
            hits.set(list);
        }
    });
}

pub(super) fn refresh_artifact_search(query: RwSignal<String>, hits: RwSignal<Vec<ArtifactInfo>>) {
    spawn_local(async move {
        let q = query.get().trim().to_string();
        let v = invoke(
            "search_artifacts",
            to_value(&serde_json::json!({ "query": q, "limit": 12 })).unwrap(),
        )
        .await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(v) {
            hits.set(list);
        }
    });
}

pub(super) fn artifact_badge(kind: &str, name: &str) -> String {
    let raw = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty() && ext.len() <= 10)
        .or_else(|| kind.rsplit('/').next())
        .unwrap_or("file");
    raw.to_uppercase()
}

pub(super) fn stored_artifact_path(path: &str) -> String {
    path.strip_prefix("file://").unwrap_or(path).to_string()
}

pub(super) fn contains_search(q: &str, parts: &[&str]) -> bool {
    q.is_empty() || parts.iter().any(|s| s.to_lowercase().contains(q))
}

pub(super) type ModalArtifact = (String, String, String);

pub(super) fn open_workspace_file(path: String, modal_artifact: RwSignal<Option<ModalArtifact>>) {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&path)
        .to_string();
    let kind = file_kind(&path).unwrap_or("text").to_string();
    modal_artifact.set(Some((path, name, kind)));
}

pub(super) fn modal_image_nav_targets(
    artifacts: &[Artifact],
    current_path: &str,
    current_kind: &str,
) -> (Option<ModalArtifact>, Option<ModalArtifact>) {
    if current_kind != "image" {
        return (None, None);
    }
    let images = artifacts
        .iter()
        .filter_map(|artifact| match &artifact.data {
            PreviewData::File { path, kind } if kind == "image" => {
                Some((path.clone(), artifact.name.clone(), kind.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some(index) = images.iter().position(|(path, _, _)| path == current_path) else {
        return (None, None);
    };
    let prev = index.checked_sub(1).and_then(|idx| images.get(idx).cloned());
    let next = images.get(index + 1).cloned();
    (prev, next)
}

pub(super) const ALL_RIGHT_TABS: [RightTab; 6] = [
    RightTab::Artifacts,
    RightTab::Notebook,
    RightTab::File,
    RightTab::Provenance,
    RightTab::Hosts,
    RightTab::SideChat,
];

pub(super) fn ensure_right_tab(
    tab: RightTab,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    show_right.set(true);
    open_right_tabs.update(|tabs| {
        if !tabs.iter().any(|t| *t == tab) {
            tabs.push(tab);
        }
    });
    right_tab.set(tab);
}

pub(super) fn close_right_tab(
    tab: RightTab,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    let was_active = right_tab.get_untracked() == tab;
    let prev_idx = open_right_tabs
        .get_untracked()
        .iter()
        .position(|t| *t == tab);
    open_right_tabs.update(|tabs| tabs.retain(|t| *t != tab));
    let remaining = open_right_tabs.get_untracked();
    if remaining.is_empty() {
        show_right.set(false);
        return;
    }
    if was_active {
        let pick = prev_idx
            .map(|i| if i > 0 { i - 1 } else { 0 })
            .unwrap_or(0)
            .min(remaining.len() - 1);
        right_tab.set(remaining[pick]);
    }
}

pub(super) fn reveal_in_files(
    path: &str,
    file_cwd: RwSignal<String>,
    file_query: RwSignal<String>,
    file_entries: RwSignal<Vec<DirEntry>>,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    file_query.set(String::new());
    file_cwd.set(parent_path(path));
    refresh_dir(file_cwd, file_entries);
    ensure_right_tab(
        RightTab::File,
        show_right,
        open_right_tabs,
        right_tab,
    );
}

pub(super) fn file_dir_label(path: &str) -> String {
    let p = parent_path(path);
    if p == "." {
        String::new()
    } else {
        format!("{p}/")
    }
}

pub(super) fn art_label(a: &Artifact) -> String {
    if a.name.len() <= 28 {
        a.name.clone()
    } else {
        format!("artifact-{}", &a.id[..8.min(a.id.len())])
    }
}

pub(super) fn art_chip(idx: usize, a: &Artifact) -> String {
    let label = html_escape(&art_label(a));
    let title = html_escape(&a.name);
    format!(
        r#"<button type="button" class="art-ref" data-art-idx="{idx}" title="{title}">{label}</button>"#
    )
}

pub(super) fn artifact_file_paths(a: &Artifact) -> Vec<String> {
    match &a.data {
        PreviewData::File { path, .. } => {
            let mut out = vec![normalize_path(path)];
            if let Some(name) = path.rsplit(['/', '\\']).next() {
                let name = normalize_path(name);
                if !out.contains(&name) {
                    out.push(name);
                }
            }
            out
        }
        _ => vec![normalize_path(&a.name)],
    }
}

pub(super) fn href_matches_artifact(href: &str, a: &Artifact) -> bool {
    let h = normalize_path(href);
    artifact_file_paths(a).iter().any(|p| *p == h)
}

pub(super) fn artifact_index_for_href(arts: &[Artifact], href: &str) -> Option<usize> {
    arts.iter()
        .position(|a| href_matches_artifact(href, a))
}

pub(super) fn replace_file_links(html: String, arts: &[Artifact]) -> String {
    let mut out = String::new();
    let mut rest = html.as_str();
    while let Some(ai) = rest.find("<a ") {
        out.push_str(&rest[..ai]);
        rest = &rest[ai..];
        let Some(gt) = rest.find('>') else {
            out.push_str(rest);
            break;
        };
        let tag = &rest[..=gt];
        let after = &rest[gt + 1..];
        let Some(end) = after.find("</a>") else {
            out.push_str(rest);
            break;
        };
        let inner = &after[..end];
        rest = &after[end + 4..];

        if let Some(href) = extract_href_from_tag(tag) {
            if !is_external_href(&href) {
                if let Some(idx) = artifact_index_for_href(arts, &href) {
                    out.push_str(&art_chip(idx, &arts[idx]));
                    continue;
                }
            }
        }
        out.push_str(tag);
        out.push_str(inner);
        out.push_str("</a>");
    }
    out.push_str(rest);
    out
}

pub(super) fn artifact_matches_token(token: &str, id: &str) -> bool {
    let t = token.trim();
    t == id
        || t.starts_with(id)
        || id.starts_with(&t[..t.len().min(8)])
        || t.starts_with(&id[..id.len().min(8)])
}

pub(super) fn replace_artifact_tokens(mut html: String, arts: &[Artifact]) -> String {
    while let Some(start) = html.find("{{artifact:") {
        let (head, rest) = html.split_at(start);
        let rest = &rest["{{artifact:".len()..];
        let Some(end) = rest.find("}}") else { break; };
        let token = rest[..end].trim();
        let tail = &rest[end + 2..];
        let chip = arts.iter().enumerate().find_map(|(i, a)| {
            if artifact_matches_token(token, &a.id) {
                Some(art_chip(i, a))
            } else {
                None
            }
        }).unwrap_or_else(|| {
            let short = &token[..token.len().min(8)];
            format!(r#"<span class="art-ref dead" title="{token}">artifact-{short}</span>"#)
        });
        html = format!("{head}{chip}{tail}");
    }
    html
}

/// Drop stray list markers left in front of artifact chips.
/// Models often write `- \`file\`` inside table cells; after chip promotion the
/// leading `- ` remains as a dash beside the pill.
pub(super) fn strip_list_markers_before_art_refs(html: &str) -> String {
    const CHIPS: &[&str] = &[
        r#"<button type="button" class="art-ref""#,
        r#"<span class="art-ref"#,
    ];
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some((idx, needle)) = CHIPS
        .iter()
        .filter_map(|n| rest.find(n).map(|i| (i, *n)))
        .min_by_key(|(i, _)| *i)
    {
        let (before, after) = rest.split_at(idx);
        out.push_str(strip_trailing_list_marker(before));
        out.push_str(needle);
        rest = &after[needle.len()..];
    }
    out.push_str(rest);
    out
}

fn strip_trailing_list_marker(before: &str) -> &str {
    let trimmed = before.trim_end_matches([' ', '\t']);
    let Some(without) = trimmed.strip_suffix(['-', '*', '•', '–', '—']) else {
        return before;
    };
    let boundary = without.trim_end_matches([' ', '\t']);
    if boundary.is_empty() || boundary.ends_with('>') || boundary.ends_with('\n') {
        boundary
    } else {
        before
    }
}

/// Post-process rendered Markdown: artifact chips, code wrappers, filename links.
pub(super) fn enrich_md_html(mut html: String, arts: &[Artifact], locale: Locale) -> String {
    html = replace_artifact_tokens(html, arts);
    html = replace_file_links(html, arts);
    for (i, a) in arts.iter().enumerate() {
        let chip = art_chip(i, a);
        let marker = format!("{{{{artifact:{}}}}}", a.id);
        html = html.replace(&marker, &chip);
        let fname = html_escape(&a.name);
        html = html.replace(
            &format!("<code>{fname}</code>"),
            &format!(r#"<button type="button" class="art-ref" data-art-idx="{i}" title="{fname}"><code>{fname}</code></button>"#),
        );
    }
    html = strip_list_markers_before_art_refs(&html);
    html = html.replace("<pre><code", "<pre class=\"md-code\"><code");
    html = wrap_markdown_tables_with_copy_controls(html, locale);
    html
}

#[cfg(test)]
mod art_ref_marker_tests {
    use super::*;

    #[test]
    fn strips_list_dashes_before_chips_in_table_cells() {
        let html = r#"<td> - <button type="button" class="art-ref" data-art-idx="0">a.fasta</button> - <button type="button" class="art-ref" data-art-idx="1">b.fasta</button></td>"#;
        let out = strip_list_markers_before_art_refs(html);
        assert_eq!(
            out,
            r#"<td><button type="button" class="art-ref" data-art-idx="0">a.fasta</button><button type="button" class="art-ref" data-art-idx="1">b.fasta</button></td>"#
        );
    }

    #[test]
    fn keeps_dashes_that_are_part_of_prose() {
        let html = r#"see range 1 - <button type="button" class="art-ref" data-art-idx="0">x.csv</button>"#;
        assert_eq!(strip_list_markers_before_art_refs(html), html);
    }

    #[test]
    fn wraps_markdown_tables_with_copy_controls() {
        let html = "<p>Summary</p><table><thead><tr><th>a</th></tr></thead><tbody><tr><td>1</td></tr></tbody></table>";
        let out = wrap_markdown_tables_with_copy_controls(html.into(), Locale::En);
        assert!(out.contains("md-table-card"));
        assert!(out.contains("md-table-copy"));
        assert!(out.contains("Copy table"));
    }

    #[test]
    fn table_data_to_tsv_uses_tabs_and_newlines() {
        let t = TableData {
            headers: vec!["Gene".into(), "TPM".into()],
            rows: vec![vec!["A".into(), "2.62".into()], vec!["B".into(), "1.81".into()]],
        };
        assert_eq!(table_data_to_tsv(&t), "Gene\tTPM\nA\t2.62\nB\t1.81");
    }
}

pub(super) fn handle_md_click(
    ev: &web_sys::MouseEvent,
    arts: &[Artifact],
    on_artifact: &Callback<usize>,
    on_file: &Callback<(String, String)>,
) {
    use wasm_bindgen::JsCast;
    let mut el = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    while let Some(n) = el {
        if n.class_list().contains("md-table-copy") {
            if let Ok(Some(card)) = n.closest(".md-table-card") {
                if let Ok(Some(table)) = card.query_selector("table") {
                    if let Ok(table) = table.dyn_into::<web_sys::HtmlTableElement>() {
                        ev.prevent_default();
                        ev.stop_propagation();
                        copy_text(html_table_to_tsv(&table));
                    }
                }
            }
            return;
        }
        if n.class_list().contains("art-ref") {
            if let Ok(idx) = n.get_attribute("data-art-idx").unwrap_or_default().parse::<usize>() {
                ev.prevent_default();
                ev.stop_propagation();
                on_artifact.call(idx);
            }
            return;
        }
        if n.tag_name().eq_ignore_ascii_case("a") {
            if let Some(href) = n.get_attribute("href") {
                if opens_in_system_browser(&href) {
                    ev.prevent_default();
                    ev.stop_propagation();
                    open_external_url(href);
                    return;
                }
                if !is_external_href(&href) {
                    ev.prevent_default();
                    ev.stop_propagation();
                    let path = normalize_path(&href);
                    if let Some(idx) = artifact_index_for_href(arts, &path) {
                        on_artifact.call(idx);
                    } else {
                        let kind = file_kind(&path).unwrap_or("text").to_string();
                        on_file.call((path, kind));
                    }
                    return;
                }
            }
        }
        el = n.parent_element();
    }
}

pub(super) fn refresh_sessions(sessions: RwSignal<Vec<SessionInfo>>) {
    spawn_local(async move {
        let v = invoke("list_sessions", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SessionInfo>>(v) {
            sessions.set(list);
        }
    });
}

pub(super) fn refresh_folders(folders: RwSignal<Vec<FolderInfo>>) {
    spawn_local(async move {
        let v = invoke("list_folders", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<FolderInfo>>(v) {
            folders.set(list);
        }
    });
}

pub(super) fn bucket_sessions_by_date(list: &[SessionInfo]) -> (Vec<SessionInfo>, Vec<SessionInfo>) {
    let now_ms = js_sys::Date::now();
    let (mut today, mut earlier) = (Vec::new(), Vec::new());
    for s in list {
        let ts_ms = if s.ts > 1_000_000_000_000 {
            s.ts as f64
        } else {
            s.ts as f64 * 1000.0
        };
        if s.ts > 0 && ts_ms >= now_ms - 86_400_000.0 {
            today.push(s.clone());
        } else {
            earlier.push(s.clone());
        }
    }
    (today, earlier)
}

// --- Artifact detection (Markdown tables + fenced CSV) -----------------------

/// Segment assistant text into plain-text and rendered Markdown-table chunks.
pub(super) enum Seg { Text, Table(TableData) }

pub(super) fn split_segments(text: &str) -> Vec<Seg> {
    let lines: Vec<&str> = text.lines().collect();
    let mut segs: Vec<Seg> = vec![];
    let mut buf: Vec<&str> = vec![];
    let mut i = 0;
    while i < lines.len() {
        if is_table_row(lines[i]) && i + 1 < lines.len() && is_separator(lines[i + 1]) {
            if !buf.is_empty() { segs.push(Seg::Text); buf.clear(); }
            let headers = split_row(lines[i]);
            let mut rows = vec![];
            let mut j = i + 2;
            while j < lines.len() && is_table_row(lines[j]) {
                rows.push(split_row(lines[j]));
                j += 1;
            }
            segs.push(Seg::Table(TableData { headers, rows }));
            i = j;
        } else {
            buf.push(lines[i]);
            i += 1;
        }
    }
    if !buf.is_empty() { segs.push(Seg::Text); }
    segs
}

/// Locale-free artifact material extracted from one chat item. Ids, localized
/// names, per-kind numbering, and cross-item file dedup are assigned later in
/// `assemble_artifacts`, so this half is cacheable per item and streaming only
/// re-extracts the tail message instead of rescanning the whole transcript.
pub(super) enum ProtoArtifact {
    Table(TableData),
    Csv(TableData),
    Fasta(String),
    Latex(String),
    File { path: String, kind: &'static str },
}

pub(super) fn file_proto(word: &str) -> Option<ProtoArtifact> {
    let p = normalize_path(word.trim().trim_matches('`').trim_matches('"').trim_matches('\''));
    if p.is_empty() { return None; }
    let kind = file_kind(&p)?;
    Some(ProtoArtifact::File { path: p, kind })
}

pub(super) struct ArtifactScan {
    tbl_n: usize,
    csv_n: usize,
    tex_n: usize,
}

pub(super) fn extract_markdown_protos(out: &mut Vec<ProtoArtifact>, s: &str) {
    for seg in split_segments(s) {
        if let Seg::Table(t) = seg {
            out.push(ProtoArtifact::Table(t));
        }
    }
    for (lang, body) in fenced_blocks(s) {
        if lang == "csv" || lang == "tsv" {
            let lines: Vec<&str> = body.lines().collect();
            if let Some(header) = lines.first() {
                let headers = parse_csv_line(header);
                let rows = lines[1..].iter().map(|line| parse_csv_line(line)).collect();
                out.push(ProtoArtifact::Csv(TableData { headers, rows }));
            }
        } else if lang == "fasta" || lang == "fa" {
            out.push(ProtoArtifact::Fasta(body));
        }
    }
    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim().starts_with("```") {
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                i += 1;
            }
            i = i.saturating_add(1);
            continue;
        }
        if lines[i].trim().starts_with("$") {
            let mut body = vec![];
            let mut j = i + 1;
            while j < lines.len() && !lines[j].trim().ends_with("$") { body.push(lines[j]); j += 1; }
            if j < lines.len() { body.push(lines[j].trim().trim_end_matches("$")); }
            out.push(ProtoArtifact::Latex(body.join("\n")));
            i = j + 1;
            continue;
        }
        i += 1;
    }
    for word in s.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']') {
        if let Some(p) = file_proto(word) { out.push(p); }
    }
}

/// Extraction half of the artifact scan: pure function of one item's content.
pub(super) fn extract_protos(it: &ChatItem) -> Vec<ProtoArtifact> {
    let mut out = vec![];
    match it {
        // Uploaded files live only in the user turn ("Uploaded files: a, b").
        ChatItem::User(s) => {
            for word in s.split(|c: char| c.is_whitespace() || c == ',' || c == '"' || c == '\'') {
                if let Some(p) = file_proto(word) { out.push(p); }
            }
        }
        ChatItem::Assistant { text: s, .. } => extract_markdown_protos(&mut out, s),
        ChatItem::Tool { name, input, output, .. } => {
            if name == "attempt_completion" && !output.is_empty() {
                extract_markdown_protos(&mut out, output);
            } else {
                let text = if output.is_empty() { input.as_str() } else { output.as_str() };
                for word in text.split(|c: char| c.is_whitespace() || c == '\n' || c == '"' || c == '\'') {
                    if let Some(p) = file_proto(word) { out.push(p); }
                }
            }
        }
        _ => {}
    }
    out
}

/// Numbering half: assign ids, localized names, and cross-item file dedup.
/// O(artifact count) per run — cheap next to re-scanning message text.
pub(super) fn assemble_artifacts(per_item: &[Rc<Vec<ProtoArtifact>>], locale: Locale) -> Vec<Artifact> {
    let mut out: Vec<Artifact> = vec![];
    let mut seen = std::collections::HashSet::<String>::new();
    let mut scan = ArtifactScan { tbl_n: 0, csv_n: 0, tex_n: 0 };
    for protos in per_item {
        for p in protos.iter() {
            match p {
                ProtoArtifact::Table(t) => {
                    scan.tbl_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: tf(locale, "artifact.table", &[("n", &scan.tbl_n.to_string())]),
                        kind: "table",
                        data: PreviewData::Table(t.clone()),
                    });
                }
                ProtoArtifact::Csv(t) => {
                    scan.csv_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact { id, name: format!("data-{}.csv", scan.csv_n), kind: "csv", data: PreviewData::Table(t.clone()) });
                }
                ProtoArtifact::Fasta(body) => {
                    let id = next_artifact_id(out.len());
                    out.push(Artifact { id, name: format!("alignment-{}.fasta", scan.csv_n), kind: "fasta", data: PreviewData::Fasta(body.clone()) });
                }
                ProtoArtifact::Latex(tex) => {
                    scan.tex_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: tf(locale, "artifact.equation", &[("n", &scan.tex_n.to_string())]),
                        kind: "latex",
                        data: PreviewData::Latex { tex: tex.clone(), display: true },
                    });
                }
                ProtoArtifact::File { path, kind } => {
                    if seen.contains(path.as_str()) { continue; }
                    seen.insert(path.clone());
                    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
                    let id = next_artifact_id(out.len());
                    out.push(Artifact { id, name, kind, data: PreviewData::File { path: path.clone(), kind: kind.to_string() } });
                }
            }
        }
    }
    out
}

/// Promote `attempt_completion` output into the assistant bubble (web-dist renders
/// completion as the final markdown response, not a collapsed tool row).
pub(super) fn promote_assistant_text(items: &mut Vec<ChatItem>, text: &str) {
    if text.trim().is_empty() { return; }
    if let Some(i) = items.iter().rposition(|i| matches!(i, ChatItem::Assistant { .. })) {
        if let ChatItem::Assistant { text: s, .. } = &mut items[i] {
            if s.is_empty() {
                s.push_str(text);
                return;
            }
        }
    }
    items.push(ChatItem::Assistant { text: text.to_string(), model: None });
}

/// Identity hash of the artifact list as seen by assistant markdown (chip
/// index, id, and label). Mixed into assistant row keys so chips re-render
/// when the artifact list changes, and nothing re-renders when it doesn't.
pub(super) fn artifacts_fingerprint(arts: &[Artifact]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for a in arts {
        (&a.id, &a.name).hash(&mut h);
    }
    h.finish()
}

pub(super) type ProtoCache = HashMap<(usize, u64), Rc<Vec<ProtoArtifact>>>;

/// Collect tables, data blocks, equations, and file-path artifacts from the transcript.
/// Extraction is cached per (index, content hash): a streaming flush only
/// re-extracts the message it touched, then renumbering runs over the cached
/// protos — instead of rescanning every message on every signal write.
pub(super) fn collect_artifacts(items: &[ChatItem], locale: Locale, cache: &mut ProtoCache) -> Vec<Artifact> {
    let mut next: ProtoCache = HashMap::with_capacity(items.len());
    let mut per_item: Vec<Rc<Vec<ProtoArtifact>>> = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        let key = (i, it.fingerprint());
        let protos = cache.remove(&key).unwrap_or_else(|| Rc::new(extract_protos(it)));
        next.insert(key, protos.clone());
        per_item.push(protos);
    }
    *cache = next;
    assemble_artifacts(&per_item, locale)
}

#[cfg(test)]
mod artifact_scan_tests {
    use super::*;

    fn fresh(items: &[ChatItem], locale: Locale) -> Vec<Artifact> {
        collect_artifacts(items, locale, &mut ProtoCache::new())
    }

    /// Streaming reuses cached extractions for untouched messages; the result
    /// must be identical to a from-scratch scan (ids, names, order, dedup).
    #[test]
    fn cached_scan_matches_fresh_scan() {
        let mut cache = ProtoCache::new();
        let mut items = vec![
            ChatItem::User("check data.csv and data.csv".into()),
            ChatItem::Assistant { text: "| a | b |\n|---|---|\n| 1 | 2 |\n\n```csv\nx,y\n1,2\n```".into(), model: None },
        ];
        let a1 = collect_artifacts(&items, Locale::En, &mut cache);
        assert!(a1 == fresh(&items, Locale::En));
        // csv file (deduped once) + table + fenced csv
        assert_eq!(a1.len(), 3);

        // Simulate a streaming flush: the tail message grows a code fence.
        if let ChatItem::Assistant { text, .. } = &mut items[1] {
            text.push_str("\n```py\nprint(1)\n```");
        }
        items.push(ChatItem::Tool {
            name: "write".into(), ok: Some(true), input: String::new(),
            output: "wrote out/result.csv".into(), started_at_ms: None, duration_ms: Some(1),
        });
        let a2 = collect_artifacts(&items, Locale::En, &mut cache);
        assert!(a2 == fresh(&items, Locale::En));
        assert_eq!(a2.len(), 4); // code moves to Notebook; result.csv remains an artifact
    }
}

pub(super) fn table_view(table: &TableData, locale: Locale) -> impl IntoView {
    let total = table.rows.len();
    let truncated = total > 500;
    let copy = table_data_to_tsv(table);
    let headers: Vec<String> = table.headers.iter().map(|h| md_inline_to_html(h)).collect();
    let rows: Vec<Vec<String>> = table.rows.iter().take(500)
        .map(|r| r.iter().map(|c| md_inline_to_html(c)).collect())
        .collect();
    view! {
        <div class="tbl-card">
            <div class="tbl-head">
                {truncated.then(|| view! {
                    <span class="tbl-note">{tf(locale, "table.rows_note", &[("total", &total.to_string())])}</span>
                })}
                <button type="button" class="tbl-copy" on:click=move |_| copy_text(copy.clone())>
                    {move || crate::i18n::t(locale, "table.copy")}
                </button>
            </div>
            <div class="tbl-wrap">
                <table class="tbl">
                    <thead><tr>{headers.into_iter().map(|h| view! { <th inner_html=h></th> }).collect_view()}</tr></thead>
                    <tbody>
                        {rows.into_iter().map(|r| view! {
                            <tr>{r.into_iter().map(|c| view! { <td inner_html=c></td> }).collect_view()}</tr>
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
        </div>
    }
}

pub(super) fn artifact_group_label(key: &str, locale: Locale) -> String {
    if let Some(kind) = key.strip_prefix('@') {
        let i18n = match kind {
            "table" => "artifact.group.table",
            "latex" => "artifact.group.latex",
            "csv" => "artifact.group.csv",
            "fasta" => "artifact.group.fasta",
            "msa" => "artifact.group.msa",
            "text" | "markdown" => "artifact.group.text",
            _ => return kind.to_string(),
        };
        t(locale, i18n).into()
    } else if key == "." {
        t(locale, "artifact.group.root").into()
    } else {
        key.to_string()
    }
}

pub(super) fn artifact_meta(a: &Artifact, locale: Locale) -> String {
    match &a.data {
        PreviewData::Table(t) => tf(locale, "artifact.meta.table", &[
            ("rows", &t.rows.len().to_string()),
            ("cols", &t.headers.len().to_string()),
        ]),
        PreviewData::File { path, kind } => {
            if kind == "fasta" {
                t(locale, "artifact.kind.fasta").into()
            } else if kind == "msa" {
                t(locale, "artifact.kind.msa").into()
            } else if let Some(parent) = path.rsplit(['/', '\\']).nth(1) {
                if parent.is_empty() {
                    tf(locale, "artifact.meta.file", &[("kind", kind)])
                } else {
                    format!("{parent}/")
                }
            } else {
                tf(locale, "artifact.meta.file", &[("kind", kind)])
            }
        }
        PreviewData::Latex { .. } => t(locale, "artifact.latex").into(),
        PreviewData::Fasta(s) => tf(locale, "artifact.meta.fasta", &[("seqs", &fasta_seq_count(s).max(1).to_string())]),
        PreviewData::Smiles(s) => s.chars().take(28).collect(),
        PreviewData::Text(s) | PreviewData::Markdown(s) => tf(locale, "artifact.meta.text", &[("chars", &s.len().to_string())]),
    }
}

#[component]
pub(super) fn HeavyPreview(dom_id: String, kind: String, payload: String) -> impl IntoView {
    let id_for_effect = dom_id.clone();
    let kind_for_effect = kind.clone();
    let payload_for_effect = payload.clone();
    create_effect(move |_| {
        let dom_id = id_for_effect.clone();
        let kind = kind_for_effect.clone();
        let payload = payload_for_effect.clone();
        spawn_local(async move { let _ = mount_preview(&kind, &dom_id, &payload).await; });
    });
    view! { <div class="rp-heavy" id=dom_id></div> }
}

pub(super) fn parse_csv_text(text: &str) -> Option<TableData> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() { return None; }
    let headers = parse_csv_line(lines[0]);
    let rows: Vec<Vec<String>> = lines[1..].iter().map(|l| parse_csv_line(l)).collect();
    Some(TableData { headers, rows })
}

pub(super) fn artifact_id_path(path: &str) -> Option<&str> {
    path.strip_prefix("artifact:").filter(|id| !id.is_empty())
}

#[component]
pub(super) fn CsvFilePreview(path: String) -> impl IntoView {
    let locale = use_locale();
    let table = create_rw_signal::<Option<TableData>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    create_effect(move |_| {
        let path = path.clone();
        let loc = locale.get();
        spawn_local(async move {
            table.set(None);
            err.set(None);
            let v = match artifact_id_path(&path) {
                Some(id) => invoke("read_artifact", to_value(&serde_json::json!({ "id": id })).unwrap()).await,
                None => invoke("read_file", to_value(&serde_json::json!({ "path": path })).unwrap()).await,
            };
            let Ok(fc) = serde_wasm_bindgen::from_value::<FileContent>(v) else {
                err.set(Some(tf(loc, "err.file_not_found", &[("path", &path)])));
                return;
            };
            match fc.text.as_deref().and_then(parse_csv_text) {
                Some(t) => table.set(Some(t)),
                None => err.set(Some(tf(loc, "err.file_not_found", &[("path", &path)]))),
            }
        });
    });
    move || match (table.get(), err.get()) {
        (Some(t), _) => table_view(&t, locale.get()).into_view(),
        (_, Some(e)) => view! { <div class="rp-error">{e}</div> }.into_view(),
        _ => view! { <div class="rp-heavy">{move || t(locale.get(), "loading")}</div> }.into_view(),
    }
}

#[component]
pub(super) fn FilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
    let locale = use_locale();
    let id_for_effect = dom_id.clone();
    let path_for_effect = path.clone();
    create_effect(move |_| {
        let path = path_for_effect.clone();
        let kind = kind.clone();
        let dom_id = id_for_effect.clone();
        let loc = locale.get();
        spawn_local(async move {
            let doc = web_sys::window().and_then(|w| w.document());
            let el = doc.as_ref().and_then(|d| d.get_element_by_id(&dom_id));
            // Allow up to the backend's 32 MB ceiling so a large produced figure or
            // PDF still renders (the default 8 MB cap silently rejected them, #35).
            // On failure, surface the real backend error (size limit / outside project
            // root / …) instead of a blanket "file not found".
            let result = match artifact_id_path(&path) {
                Some(id) => invoke_checked("read_artifact", to_value(&serde_json::json!({ "id": id })).unwrap()).await,
                None => invoke_checked("read_file", to_value(&tauri_args::read_file(&path, Some(32 * 1024 * 1024))).unwrap()).await,
            };
            let fc = match result {
                Ok(v) => match serde_wasm_bindgen::from_value::<FileContent>(v) {
                    Ok(fc) => fc,
                    Err(_) => {
                        if let Some(el) = el {
                            el.set_class_name("rp-heavy rp-error");
                            el.set_text_content(Some(&tf(loc, "err.file_not_found", &[("path", &path)])));
                        }
                        return;
                    }
                },
                Err(err) => {
                    if let Some(el) = el {
                        el.set_class_name("rp-heavy rp-error");
                        el.set_text_content(Some(&localize_backend(loc, &js_error_text(err))));
                    }
                    return;
                }
            };
            if kind == "markdown" {
                if let Some(el) = el {
                    el.set_class_name("rp-heavy md");
                    el.set_inner_html(&md_to_html(fc.text.as_deref().unwrap_or("")));
                }
                return;
            }
            let (mount_kind, payload) = match kind.as_str() {
                "pdf" => ("pdf", serde_json::json!({ "b64": fc.base64 }).to_string()),
                "image" => ("image", serde_json::json!({ "b64": fc.base64, "mime": fc.mime }).to_string()),
                "structure" => ("structure", serde_json::json!({ "text": fc.text, "format": "pdb" }).to_string()),
                "molecule" | "smiles" => ("molecule", serde_json::json!({ "text": fc.text, "smiles": fc.text }).to_string()),
                "fasta" => ("fasta", serde_json::json!({ "text": fc.text }).to_string()),
                "msa" => ("msa", serde_json::json!({ "text": fc.text }).to_string()),
                _ => ("text", serde_json::json!({ "text": fc.text }).to_string()),
            };
            let _ = mount_preview(mount_kind, &dom_id, &payload).await;
        });
    });
    view! { <div class="rp-heavy" id=dom_id>{move || t(locale.get(), "loading")}</div> }
}

pub(super) fn artifact_preview(a: &Artifact, dom_id: String, locale: Locale) -> impl IntoView {
    match &a.data {
        PreviewData::Table(t) => table_view(t, locale).into_view(),
        PreviewData::Text(s) => view! { <pre class="rp-pre">{s.clone()}</pre> }.into_view(),
        PreviewData::Markdown(s) => view! { <div class="md rp-md" inner_html=md_to_html(s)></div> }.into_view(),
        PreviewData::Latex { tex, display } => {
            let payload = serde_json::json!({ "tex": tex, "display": display }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="latex".to_string() payload=payload /> }.into_view()
        }
        PreviewData::Fasta(text) => {
            let payload = serde_json::json!({ "text": text }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="fasta".to_string() payload=payload /> }.into_view()
        }
        PreviewData::Smiles(s) => {
            let payload = serde_json::json!({ "smiles": s }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="molecule".to_string() payload=payload /> }.into_view()
        }
        PreviewData::File { path, kind } => {
            if kind == "csv" {
                view! {
                    <p class="rp-path hint">{path.clone()}</p>
                    <CsvFilePreview path=path.clone() />
                }.into_view()
            } else {
                view! {
                    <p class="rp-path hint">{path.clone()}</p>
                    <FilePreview dom_id=dom_id path=path.clone() kind=kind.clone() />
                }.into_view()
            }
        }
    }
}

#[component]
pub(super) fn CodeBlock(lang: String, body: String) -> impl IntoView {
    let lang_class = if lang.is_empty() { "plaintext".to_string() } else { lang.clone() };
    let hid = unique_dom_id("code");
    let hid_for_effect = hid.clone();
    let lang_track = lang_class.clone();
    let body_track = body.clone();
    create_effect(move |_| {
        let _ = (&lang_track, &body_track);
        schedule_highlight(hid_for_effect.clone());
    });
    view! {
        <div class="code-block" id=hid.clone()>
            {(!lang.is_empty()).then(|| view! { <div class="code-lang">{lang.clone()}</div> })}
            <pre class="md-code"><code class=format!("language-{lang_class}")>{body.clone()}</code></pre>
        </div>
    }
}

/// Right-pane code view with a line-number gutter (Claude Science style).
/// The gutter is a plain <pre> (no <code>) so highlight.js skips it.
#[component]
pub(super) fn RpCodeView(lang: String, body: String) -> impl IntoView {
    let lang_class = if lang.is_empty() { "plaintext".to_string() } else { lang.clone() };
    let hid = unique_dom_id("rpcode");
    let hid_for_effect = hid.clone();
    let body_track = body.clone();
    create_effect(move |_| {
        let _ = &body_track;
        schedule_highlight(hid_for_effect.clone());
    });
    // split('\n') matches how <pre> renders a trailing newline, keeping the
    // gutter aligned with the body line-for-line.
    let n = body.split('\n').count().max(1);
    let gutter = (1..=n).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
    view! {
        <div class="rp-code" id=hid.clone()>
            <pre class="rp-code-gutter">{gutter}</pre>
            <pre class="rp-code-body"><code class=format!("language-{lang_class}")>{body.clone()}</code></pre>
        </div>
    }
}

/// File kinds that open in the full ArtifactModal viewer on click (image/pdf
/// full-size, csv as a dataset table) rather than rendering inline in the pane.
pub(super) fn opens_in_modal(kind: &str) -> bool {
    matches!(kind, "image" | "pdf" | "csv")
}

/// Fire the native save dialog to download a workspace file (backend copies it).
pub(super) fn download_artifact(path: String) {
    spawn_local(async move {
        let arg = to_value(&serde_json::json!({ "path": path })).unwrap();
        let _ = invoke("download_file", arg).await;
    });
}

pub(super) fn keyboard_event_targets_text_entry(ev: &web_sys::KeyboardEvent) -> bool {
    let mut el = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    while let Some(node) = el {
        if node.dyn_ref::<web_sys::HtmlInputElement>().is_some()
            || node.dyn_ref::<web_sys::HtmlTextAreaElement>().is_some()
            || node.tag_name().eq_ignore_ascii_case("select")
            || node.has_attribute("contenteditable")
        {
            return true;
        }
        el = node.parent_element();
    }
    false
}

/// Click-to-expand modal for a produced artifact: shows the full-size
/// image/PDF (or a CSV as a dataset table) plus tabbed provenance
/// (Code/Log/Inputs/Environment) fetched from `get_artifact_provenance`.
/// Provenance is best-effort — a `None` result (or any empty field within it)
/// renders an empty state; the figure never depends on provenance being present.
#[component]
pub(super) fn ArtifactModal(
    path: String,
    name: String,
    kind: String,
    session: Option<String>,
    can_prev: bool,
    can_next: bool,
    on_prev: Callback<()>,
    on_next: Callback<()>,
    on_close: Callback<()>,
    on_open_path: Callback<(String, String)>, // open an input file (path, kind)
) -> impl IntoView {
    let locale = use_locale();
    let prov = create_rw_signal(None::<ArtifactProvenance>);
    let loaded = create_rw_signal(false);
    let tab = create_rw_signal("code");
    let dom_id = unique_dom_id("amodal");
    {
        let path = path.clone();
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "sessionId": session, "path": path })).unwrap();
            let v = invoke("get_artifact_provenance", arg).await;
            prov.set(serde_wasm_bindgen::from_value::<Option<ArtifactProvenance>>(v).ok().flatten());
            loaded.set(true);
        });
    }
    let path_head = path.clone();
    let path_dl = path.clone();
    view! {
        <div class="overlay" on:click=move |_| on_close.call(())>
            <div class="modal artifact-modal" on:click=|ev| ev.stop_propagation()>
                <div class="am-head">
                    <span class="am-name">{name.clone()}</span>
                    {(can_prev || can_next).then(|| view! {
                        <div class="am-nav">
                            <button type="button" class="icon-btn am-nav-btn"
                                disabled=!can_prev
                                aria-label=move || t(locale.get(), "artifact.prev_image")
                                title=move || format!("{} (←)", t(locale.get(), "artifact.prev_image"))
                                on:click=move |_| on_prev.call(())>{compose_icon("chevron-left")}</button>
                            <button type="button" class="icon-btn am-nav-btn"
                                disabled=!can_next
                                aria-label=move || t(locale.get(), "artifact.next_image")
                                title=move || format!("{} (→)", t(locale.get(), "artifact.next_image"))
                                on:click=move |_| on_next.call(())>{compose_icon("chevron-right")}</button>
                        </div>
                    })}
                    <div class="spacer"></div>
                    <button class="icon-btn" title=move || t(locale.get(), "artifact.download")
                        on:click=move |_| download_artifact(path_dl.clone())>{compose_icon("download")}</button>
                    <button class="icon-btn" title=move || t(locale.get(), "right.close")
                        on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
                </div>
                <div class="am-figure">
                    {if kind == "csv" {
                        view! { <CsvFilePreview path=path_head.clone() /> }.into_view()
                    } else if kind == "image" || kind == "pdf" {
                        view! { <FilePreview dom_id=dom_id path=path_head.clone() kind=kind.clone() /> }.into_view()
                    } else {
                        view! { <FilePreview dom_id=dom_id path=path_head.clone() kind=kind.clone() /> }.into_view()
                    }}
                </div>
                <div class="am-tabs">
                    {["code","log","inputs","env"].iter().map(|k| {
                        let k = *k;
                        let label_key = format!("artifact.tab.{k}");
                        view! {
                            <button class="am-tab" class:active=move || tab.get()==k
                                on:click=move |_| tab.set(k)>
                                {move || t(locale.get(), &label_key)}</button>
                        }
                    }).collect_view()}
                </div>
                <div class="am-panel">
                    {move || {
                        let loc = locale.get();
                        if !loaded.get() { return view! { <div class="rp-heavy">{t(loc,"loading")}</div> }.into_view(); }
                        let Some(p) = prov.get() else {
                            return view! { <div class="am-empty">{t(loc,"artifact.none")}</div> }.into_view();
                        };
                        match tab.get() {
                            "code" => view! { <RpCodeView lang=p.language.clone() body=p.code.clone() /> }.into_view(),
                            "log" => view! { <pre class="am-log">{p.output.clone()}</pre> }.into_view(),
                            "inputs" => view! {
                                <div class="am-inputs">
                                    {p.inputs.iter().map(|i| {
                                        let ip = i.path.clone();
                                        let linkable = i.produced_here;
                                        let open = on_open_path;
                                        view! {
                                            <button class="am-input" class:linkable=linkable
                                                on:click=move |_| if linkable {
                                                    let kind = file_kind(&ip).unwrap_or("text").to_string();
                                                    open.call((ip.clone(), kind));
                                                }>
                                                {i.path.clone()}</button>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_view(),
                            _ => match p.env.clone() {
                                None => view! { <div class="am-empty">{t(loc,"artifact.env.none")}</div> }.into_view(),
                                Some(env) => view! {
                                    <table class="am-env">
                                        {env.packages.iter().map(|pk| view! {
                                            <tr><td>{pk.name.clone()}</td><td>{pk.version.clone()}</td></tr>
                                        }).collect_view()}
                                    </table>
                                }.into_view(),
                            },
                        }
                    }}
                </div>
            </div>
        </div>
    }
}

pub(super) fn composer_text_from_user_message(text: &str) -> String {
    ["\n\nUploaded files: ", "\n\nAttached artifacts: ", "\n\nAttached sessions: ", "\n\nSelected skills: "]
        .iter()
        .filter_map(|marker| text.find(marker))
        .min()
        .map(|idx| text[..idx].trim().to_string())
        .unwrap_or_else(|| text.to_string())
}

pub(super) fn user_message_index(items: &[ChatItem], ui_index: usize) -> Option<usize> {
    if !matches!(items.get(ui_index), Some(ChatItem::User(_))) {
        return None;
    }
    Some(
        items
            .iter()
            .take(ui_index + 1)
            .filter(|item| matches!(item, ChatItem::User(_)))
            .count()
            .saturating_sub(1),
    )
}

pub(super) fn focus_composer() {
    focus_element("composer-input");
}

pub(super) fn focus_element(id: &str) {
    focus_element_inner(id, false);
}

pub(super) fn focus_and_select_element(id: &str) {
    focus_element_inner(id, true);
}

fn focus_element_inner(id: &str, select_all: bool) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return; };
    let Some(el) = doc.get_element_by_id(id) else { return; };
    let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|e| e.focus());
    if !select_all {
        return;
    }
    if let Some(input) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        input.select();
    } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        ta.select();
    }
}

pub(super) fn focus_element_soon(id: &'static str) {
    schedule_focus(id, false);
}

/// Focus a text field after the next paint and select its contents.
/// Used by rename/create modals so Ctrl/⌘A and typing work immediately.
pub(super) fn focus_and_select_soon(id: &'static str) {
    schedule_focus(id, true);
}

fn schedule_focus(id: &'static str, select_all: bool) {
    let focus = Closure::once(move || {
        if select_all {
            focus_and_select_element(id);
        } else {
            focus_element(id);
        }
    });
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(focus.as_ref().unchecked_ref(), 0);
    }
    focus.forget();
}

/// Shared Lucide-style UI icons. Interactive controls must use these SVGs,
/// never font glyphs whose shape varies by platform and fallback font.
pub(super) fn compose_icon(kind: &str) -> impl IntoView {
    let body = match kind {
        "attach" => view! { <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/> }.into_view(),
        "folder" => view! { <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/> }.into_view(),
        "plan" => view! { <path d="M8 6h13"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M3 6l1 1 2-2"/><path d="M3 12l1 1 2-2"/><path d="M3 18l1 1 2-2"/> }.into_view(),
        "chat" => view! { <path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"/><path d="M8 10h8"/><path d="M8 14h5"/> }.into_view(),
        "branch" => view! { <path d="M6 3v6a4 4 0 0 0 4 4h8"/><path d="M18 7v12"/><path d="M14 15l4 4 4-4"/><circle cx="6" cy="3" r="2"/> }.into_view(),
        "chevron-down" => view! { <path d="m6 9 6 6 6-6"/> }.into_view(),
        "chevron-left" => view! { <path d="m15 18-6-6 6-6"/> }.into_view(),
        "chevron-right" => view! { <path d="m9 18 6-6-6-6"/> }.into_view(),
        "download" => view! { <path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/> }.into_view(),
        "close" => view! { <path d="M18 6 6 18"/><path d="m6 6 12 12"/> }.into_view(),
        "more" => view! { <circle cx="12" cy="5" r="1" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="12" cy="19" r="1" fill="currentColor" stroke="none"/> }.into_view(),
        "plus" => view! { <path d="M12 5v14"/><path d="M5 12h14"/> }.into_view(),
        "up" => view! { <path d="m18 15-6-6-6 6"/> }.into_view(),
        "copy" => view! { <rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/> }.into_view(),
        "edit" => view! { <path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4Z"/> }.into_view(),
        "doc" => view! { <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6"/> }.into_view(),
        "review" => view! { <circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 1 0 18Z" fill="currentColor" stroke="none"/> }.into_view(),
        "skill" => view! { <path d="M19 17V5a2 2 0 0 0-2-2H4"/><path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3"/> }.into_view(),
        "server" => view! { <rect x="3" y="4" width="18" height="7" rx="1"/><rect x="3" y="13" width="18" height="7" rx="1"/><circle cx="7" cy="7.5" r="0.5" fill="currentColor"/><circle cx="7" cy="16.5" r="0.5" fill="currentColor"/> }.into_view(),
        "grid" => view! { <rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/> }.into_view(),
        "list" => view! { <path d="M8 6h13"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M3 6h.01"/><path d="M3 12h.01"/><path d="M3 18h.01"/> }.into_view(),
        _ => view! { <path d="M9 18l6-6-6-6"/> }.into_view(), // chevron
    };
    let size = if matches!(kind, "chevron" | "chevron-down" | "chevron-left" | "chevron-right") { "16" } else { "18" };
    view! {
        <svg width=size height=size viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>
    }
}

#[component]
pub(super) fn UserMessage(
    text: String,
    ui_index: usize,
    busy: ReadSignal<bool>,
    on_copy: Callback<String>,
    on_edit: Callback<usize>,
    on_branch: Callback<usize>,
) -> impl IntoView {
    let locale = use_locale();
    view! {
        <div class="role">{move || t(locale.get(), "chat.you")}</div>
        <div class="user-bubble">
            <div class="body">{text.clone()}</div>
            <div class="msg-actions">
                <button
                    type="button"
                    class="msg-btn"
                    disabled=move || busy.get()
                    title=move || t(locale.get(), "msg.copy")
                    on:click=move |_| on_copy.call(text.clone())
                >{move || t(locale.get(), "msg.copy")}</button>
                <button
                    type="button"
                    class="msg-btn"
                    disabled=move || busy.get()
                    title=move || t(locale.get(), "msg.edit")
                    on:click=move |_| on_edit.call(ui_index)
                >{move || t(locale.get(), "msg.edit")}</button>
                <button
                    type="button"
                    class="msg-btn"
                    title=move || t(locale.get(), "msg.branch")
                    on:click=move |_| on_branch.call(ui_index)
                >{move || t(locale.get(), "msg.branch")}</button>
            </div>
        </div>
    }
}

#[component]
pub(super) fn AssistantMessage(
    text: String,
    model: Option<String>,
    artifacts: Vec<Artifact>,
    on_artifact: Callback<usize>,
    on_file: Callback<(String, String)>,
    on_copy: Callback<String>,
) -> impl IntoView {
    let locale = use_locale();
    let arts_for_html = artifacts.clone();
    let text_for_html = text.clone();
    let html = create_memo(move |_| enrich_md_html(md_to_html(&text_for_html), &arts_for_html, locale.get()));
    let hid = unique_dom_id("md");
    let hid_for_effect = hid.clone();
    create_effect(move |_| {
        let _ = html.get();
        schedule_highlight(hid_for_effect.clone());
    });
    let on_artifact = on_artifact.clone();
    let on_file = on_file.clone();
    let arts_for_click = artifacts.clone();
    let text_for_disabled = text.clone();
    let text_for_click_copy = text;
    view! {
        <div class="role">
            <span class="role-brand">{move || t(locale.get(), "chat.assistant")}</span>
            {move || model.clone().filter(|m| !m.is_empty()).map(|m| view! {
                <span class="role-model">{m}</span>
            })}
        </div>
        <div class="assistant-wrap">
            <div class="body md" id=hid.clone()
                inner_html=move || html.get()
                on:click=move |ev: web_sys::MouseEvent| {
                    handle_md_click(&ev, &arts_for_click, &on_artifact, &on_file)
                }></div>
            <div class="msg-actions">
                <button
                    type="button"
                    class="msg-icon-btn"
                    title=move || t(locale.get(), "ctx.copy_message")
                    aria-label=move || t(locale.get(), "ctx.copy_message")
                    disabled=move || text_for_disabled.trim().is_empty()
                    on:click=move |_| on_copy.call(text_for_click_copy.clone())
                >
                    <span class="gi copy" aria-hidden="true"></span>
                </button>
            </div>
        </div>
    }
}

#[component]
pub(super) fn ToolBlock(name: String, ok: Option<bool>, input: String, output: String) -> impl IntoView {
    let locale = use_locale();
    let open = ok != Some(true);
    let lang = tool_lang(&name).to_string();
    let hid = unique_dom_id("tool");
    let hid_for_effect = hid.clone();
    let has_input = !input.is_empty();
    let has_output = !output.is_empty();
    let input_track = input.clone();
    let output_track = output.clone();
    let lang_track = lang.clone();
    create_effect(move |_| {
        let _ = (&input_track, &output_track, &lang_track);
        schedule_highlight(hid_for_effect.clone());
    });
    let name_for_label = name.clone();
    let input_label = move || {
        if name_for_label == "python" { t(locale.get(), "tool.copy_code") } else { t(locale.get(), "tool.copy_input") }
    };

    view! {
        <details class="tool" open=open>
            <summary class="head">
                <span>{name.clone()}</span>
                {match ok {
                    Some(true) => view!{ <span class="ok">"✓"</span> }.into_view(),
                    Some(false) => view!{ <span class="fail">"✗"</span> }.into_view(),
                    None => view!{ <span class="run"><span class="run-dot"></span>{move || t(locale.get(), "tool.running")}</span> }.into_view(),
                }}
            </summary>
            <div class="tool-panel" id=hid.clone()>
                <div class="tool-actions">
                    {has_input.then(|| {
                        let text = input.clone();
                        view! {
                            <button type="button" class="tool-btn" on:click=move |_| copy_text(text.clone())>
                                {input_label}
                            </button>
                        }
                    })}
                    {has_output.then(|| {
                        let text = output.clone();
                        view! {
                            <button type="button" class="tool-btn" on:click=move |_| copy_text(text.clone())>{move || t(locale.get(), "tool.copy_output")}</button>
                        }
                    })}
                </div>
                {has_input.then(|| view! {
                    <pre class="tool-input md-code"><code class=format!("language-{lang}")>{input.clone()}</code></pre>
                })}
                {has_output.then(|| view! {
                    <pre class="tool-output md-code"><code class="language-plaintext">{output.clone()}</code></pre>
                })}
            </div>
        </details>
    }
}

/// Parse a rendered plan checklist line (`[x] text` / `[~] text` / `[ ] text`)
/// into (status_class, text). Mirrors `update_plan`'s render in wisp-tools.
pub(super) fn plan_step_line(line: &str) -> Option<(&'static str, &str)> {
    for (prefix, cls) in [("[x] ", "done"), ("[~] ", "running"), ("[ ] ", "pending")] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some((cls, rest));
        }
    }
    None
}

pub(super) fn approval_allow_label_key(scope: &str) -> &'static str {
    match scope {
        "session" => "approval.allow_session",
        "project" => "approval.allow_project",
        "global" => "approval.allow_global",
        _ => "approval.allow_once",
    }
}

#[component]
pub(super) fn ApprovalCard(
    tool: String,
    preview: String,
    session_id: String,
    on_decide: Callback<(String, bool, Option<String>, String)>,
) -> impl IntoView {
    let locale = use_locale();
    let is_plan = tool == "update_plan";
    let show_feedback = create_rw_signal(false);
    let feedback = create_rw_signal(String::new());
    let approval_scope = create_rw_signal(String::from("once"));
    let feedback_ready = move || !feedback.get().trim().is_empty();
    create_effect(move |_| {
        if show_feedback.get() {
            focus_element_soon("plan-feedback-input");
        }
    });
    let lang = tool_lang(&tool).to_string();
    // For the plan card, `preview` is the rendered checklist; parse it into rows.
    let plan_steps: Vec<(&'static str, String)> = if is_plan {
        preview
            .lines()
            .filter_map(|l| plan_step_line(l).map(|(c, t)| (c, t.to_string())))
            .collect()
    } else {
        vec![]
    };
    let tool_for_title = tool.clone();
    let title = move || {
        let loc = locale.get();
        match tool_for_title.as_str() {
            _ if is_plan => t(loc, "approval.review_plan"),
            "python" => t(loc, "approval.run_python"),
            "shell" => t(loc, "approval.run_shell"),
            _ => tf(loc, "approval.run_tool", &[("tool", &tool_for_title)]),
        }
    };
    let sid_allow = session_id.clone();
    let sid_deny = session_id.clone();
    let sid_feedback = create_rw_signal(session_id);
    view! {
        <div class="approval-wrap">
            <div class="approval-wait-line">{move || t(locale.get(), "approval.waiting_line")}</div>
            <div class="approval-card" class:plan=is_plan>
                <div class="approval-head">
                    <span class="approval-title">{title}</span>
                    <span class="approval-status">
                        <span class="approval-dot"></span>
                        {move || t(locale.get(), "approval.waiting")}
                    </span>
                </div>
                {if is_plan {
                    view! {
                        <div class="plan-steps">
                            {plan_steps.into_iter().map(|(cls, text)| view! {
                                <div class=format!("plan-step {cls}")>
                                    <span class="plan-step-mark"></span>
                                    <span class="plan-step-text">{text}</span>
                                </div>
                            }).collect_view()}
                        </div>
                    }.into_view()
                } else {
                    let show_tag = !tool.is_empty();
                    let tag = tool.clone();
                    let show_code = !preview.is_empty();
                    let p = preview.clone();
                    let lang = lang.clone();
                    view! {
                        {show_tag.then(|| view! {
                            <div class="approval-tags"><span class="approval-tag">{tag}</span></div>
                        })}
                        {show_code.then(|| view! {
                            <details class="approval-code" open=true>
                                <summary>{move || t(locale.get(), "approval.code")}</summary>
                                <pre><code class=format!("language-{lang}")>{p}</code></pre>
                            </details>
                        })}
                    }.into_view()
                }}
                <p class="approval-hint">{move || t(locale.get(), if is_plan { "approval.plan_hint" } else { "approval.hint" })}</p>
                <div class="approval-actions">
                    {(!is_plan).then(|| view! {
                        <label class="approval-scope">
                            <span>{move || t(locale.get(), "approval.scope")}</span>
                            <select
                                aria-label=move || t(locale.get(), "approval.scope")
                                prop:value=move || approval_scope.get()
                                on:change=move |ev| approval_scope.set(dom_value(&ev))>
                                <option value="once">{move || t(locale.get(), "approval.scope.once")}</option>
                                <option value="session">{move || t(locale.get(), "approval.scope.session")}</option>
                                <option value="project">{move || t(locale.get(), "approval.scope.project")}</option>
                                <option value="global">{move || t(locale.get(), "approval.scope.global")}</option>
                            </select>
                        </label>
                    })}
                    <button type="button" class="primary"
                        on:click=move |_| {
                            let scope = if is_plan { "once".into() } else { approval_scope.get() };
                            on_decide.call((sid_allow.clone(), true, None, scope));
                        }>
                        {move || {
                            if is_plan {
                                t(locale.get(), "approval.plan_approve").to_string()
                            } else {
                                t(locale.get(), approval_allow_label_key(&approval_scope.get())).to_string()
                            }
                        }}
                    </button>
                    <button type="button"
                        on:click=move |_| on_decide.call((sid_deny.clone(), false, None, "once".into()))>
                        {move || t(locale.get(), if is_plan { "approval.plan_reject" } else { "confirm.deny" })}
                    </button>
                    {is_plan.then(|| view! {
                        <button type="button" on:click=move |_| show_feedback.update(|open| *open = !*open)>
                            {move || t(locale.get(), "approval.plan_other")}
                        </button>
                    })}
                </div>
                {is_plan.then(move || {
                    view! {
                        <Show when=move || show_feedback.get()>
                            <div class="plan-feedback"
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Escape" && !ev.is_composing() {
                                        // Collapse feedback before the window-level
                                        // handler rejects the whole plan.
                                        ev.prevent_default();
                                        feedback.set(String::new());
                                        show_feedback.set(false);
                                    }
                                }>
                                <textarea
                                    id="plan-feedback-input"
                                    class="plan-feedback-input"
                                    rows="3"
                                    prop:value=move || feedback.get()
                                    placeholder=move || t(locale.get(), "approval.plan_feedback_placeholder")
                                    on:input=move |ev| feedback.set(event_target_value(&ev))
                                ></textarea>
                                <div class="plan-feedback-actions">
                                    <button
                                        type="button"
                                        class="primary"
                                        disabled=move || !feedback_ready()
                                        on:click=move |_| {
                                            let text = feedback.get().trim().to_string();
                                            if !text.is_empty() {
                                                on_decide.call((sid_feedback.get_untracked(), false, Some(text), "once".into()));
                                            }
                                        }
                                    >
                                        {move || t(locale.get(), "approval.plan_feedback_submit")}
                                    </button>
                                    <button
                                        type="button"
                                        on:click=move |_| {
                                            feedback.set(String::new());
                                            show_feedback.set(false);
                                        }
                                    >
                                        {move || t(locale.get(), "approval.plan_feedback_cancel")}
                                    </button>
                                </div>
                            </div>
                        </Show>
                    }
                })}
            </div>
        </div>
    }
}

#[component]
pub(super) fn ProjectsScreen(
    locale: RwSignal<Locale>,
    running: RwSignal<HashSet<String>>,
    approval_pending: ReadSignal<HashSet<String>>,
    on_open: Callback<String>,
    on_open_session: Callback<(String, String)>,
    on_open_artifact: Callback<(String, String, String)>,
    on_open_settings: Callback<()>,
    on_open_demo: Callback<()>,
    on_search: Callback<()>,
) -> impl IntoView {
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let recent = create_rw_signal(Vec::<RecentSession>::new());
    let artifact_hits = create_rw_signal(Vec::<ArtifactInfo>::new());
    let search_open = create_rw_signal(false);
    let search_query = create_rw_signal(String::new());
    let search_active = create_rw_signal(0usize);
    let demo_count = create_rw_signal(0usize);
    let creating = create_rw_signal(false);
    let new_name = create_rw_signal(String::new());
    let new_dir = create_rw_signal(String::new());
    let new_desc = create_rw_signal(String::new());
    let new_ctx = create_rw_signal(String::new());
    // Pending project deletion, awaiting in-app confirmation. Native
    // `window.confirm()` is a no-op in this webview (wry's WKUIDelegate doesn't
    // implement the JS confirm panel), so it always returned false and the ✕
    // did nothing — use an in-app modal instead.
    let pending_delete = create_rw_signal(None::<String>);

    let reload = move || {
        spawn_local(async move {
            let v = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) { projects.set(list); }
            let r = invoke("list_recent_sessions", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<RecentSession>>(r) { recent.set(list); }
            let dm = invoke("list_demos", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DemoInfo>>(dm) { demo_count.set(list.len()); }
        });
    };
    reload();

    // Refresh dashboard when a background turn starts/finishes or waits on approval.
    create_effect(move |_| {
        running.get();
        approval_pending.get();
        reload();
    });

    create_effect(move |_| {
        if search_open.get() {
            search_query.get();
            refresh_artifact_search(search_query, artifact_hits);
            focus_element_soon("project-search-input");
        }
    });

    create_effect(move |_| {
        if creating.get() {
            focus_and_select_soon("new-project-name");
        }
    });

    create_effect(move |_| {
        search_open.get();
        search_query.get();
        search_active.set(0);
    });

    let search_count = move || {
        let q = search_query.get().trim().to_lowercase();
        let projects_n = projects
            .get()
            .into_iter()
            .filter(|p| contains_search(&q, &[&p.name, &p.description]))
            .take(HOME_SEARCH_PROJECT_LIMIT)
            .count();
        let artifacts_n = artifact_hits.get().into_iter().take(HOME_SEARCH_ARTIFACT_LIMIT).count();
        let sessions_n = recent
            .get()
            .into_iter()
            .filter(|s| contains_search(&q, &[&s.title]))
            .take(HOME_SEARCH_SESSION_LIMIT)
            .count();
        projects_n + artifacts_n + sessions_n + 1
    };

    let run_search_action = Callback::new(move |idx: usize| {
        let q = search_query.get().trim().to_lowercase();
        let mut pos = 0usize;
        for p in projects
            .get()
            .into_iter()
            .filter(|p| contains_search(&q, &[&p.name, &p.description]))
            .take(HOME_SEARCH_PROJECT_LIMIT)
        {
            if pos == idx {
                search_open.set(false);
                on_open.call(p.id);
                return;
            }
            pos += 1;
        }
        for a in artifact_hits.get().into_iter().take(HOME_SEARCH_ARTIFACT_LIMIT) {
            if pos == idx {
                search_open.set(false);
                let path = stored_artifact_path(&a.path);
                let kind = file_kind(&a.name)
                    .or_else(|| file_kind(&path))
                    .unwrap_or_else(|| {
                        if a.kind.starts_with("image/") {
                            "image"
                        } else if a.kind.contains("pdf") {
                            "pdf"
                        } else if a.kind.contains("csv") {
                            "csv"
                        } else {
                            "text"
                        }
                    })
                    .to_string();
                on_open_artifact.call((path, a.name, kind));
                return;
            }
            pos += 1;
        }
        for s in recent
            .get()
            .into_iter()
            .filter(|s| contains_search(&q, &[&s.title]))
            .take(HOME_SEARCH_SESSION_LIMIT)
        {
            if pos == idx {
                search_open.set(false);
                on_open_session.call((s.project_id, s.id));
                return;
            }
            pos += 1;
        }
        if pos == idx {
            search_open.set(false);
            creating.set(true);
        }
    });

    let choose_dir = move |_| spawn_local(async move {
        let v = invoke("pick_directory", JsValue::UNDEFINED).await;
        if let Ok(Some(p)) = serde_wasm_bindgen::from_value::<Option<String>>(v) { new_dir.set(p); }
    });

    let submit = move |_| {
        let (n, d, desc, ctx) = (new_name.get(), new_dir.get(), new_desc.get(), new_ctx.get());
        if n.trim().is_empty() || d.trim().is_empty() { return; }
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "name": n, "workspaceDir": d, "description": desc, "agentContext": ctx,
            })).unwrap();
            let v = invoke("create_project", arg).await;
            if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectSummary>(v) {
                new_name.set(String::new()); new_dir.set(String::new());
                new_desc.set(String::new()); new_ctx.set(String::new());
                creating.set(false);
                on_open.call(p.id);
            }
        });
    };

    let delete = move |id: String| {
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
            let _ = invoke("delete_project", arg).await;
            let v = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) { projects.set(list); }
        });
    };
    let delete_confirmed = delete.clone(); // used by the confirm modal below

    // Local Escape stack — ProjectsScreen owns its own modals, so the App
    // window listener cannot see `creating` / `pending_delete`.
    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else { return };
        if ev.key() != "Escape" || ev.default_prevented() || ev.is_composing() {
            return;
        }
        if pending_delete.get().is_some() {
            ev.prevent_default();
            pending_delete.set(None);
            return;
        }
        if search_open.get() {
            ev.prevent_default();
            search_open.set(false);
            return;
        }
        if creating.get() {
            ev.prevent_default();
            creating.set(false);
        }
    });

    view! {
        <div class="projects-screen">
            <div class="projects-head">
                <div class="projects-title">"Wisp Science"<span class="beta">"Beta"</span></div>
                <div class="projects-actions">
                    <button type="button" class="projects-icon-btn"
                        title=move || t(locale.get(), "projects.search")
                        aria-label=move || t(locale.get(), "projects.search")
                        on:click=move |_| on_search.call(())>
                        <span class="gi search"></span>
                    </button>
                    <button type="button" class="projects-icon-btn"
                        title=move || t(locale.get(), "sidebar.settings")
                        aria-label=move || t(locale.get(), "sidebar.settings")
                        on:click=move |_| on_open_settings.call(())>
                        <span class="gi gear"></span>
                    </button>
                    <button class="btn-primary" on:click=move |_| creating.set(true)>
                        <span class="new-plus">"+"</span>{move || t(locale.get(), "projects.new")}
                    </button>
                </div>
            </div>
            {move || search_open.get().then(|| view! {
                <div class="project-search-overlay" on:click=move |_| search_open.set(false)>
                    <div class="project-search-dialog" role="dialog" aria-label=move || t(locale.get(), "projects.search")
                        on:click=|ev| ev.stop_propagation()>
                        <div class="project-search-input">
                            <span class="gi search"></span>
                            <input id="project-search-input" type="text" inputmode="search" autofocus=true
                                autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                                placeholder=move || t(locale.get(), "projects.search_ph")
                                prop:value=move || search_query.get()
                                on:input=move |ev| search_query.set(event_target_value(&ev))
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.is_composing() { return; }
                                    let key = ev.key();
                                    let last = search_count().saturating_sub(1);
                                    match key.as_str() {
                                        "Escape" => {
                                            ev.prevent_default();
                                            search_open.set(false);
                                        }
                                        "ArrowDown" => {
                                            ev.prevent_default();
                                            search_active.update(|i| *i = (*i + 1).min(last));
                                        }
                                        "ArrowUp" => {
                                            ev.prevent_default();
                                            search_active.update(|i| *i = i.saturating_sub(1));
                                        }
                                        "Enter" => {
                                            ev.prevent_default();
                                            run_search_action.call(search_active.get().min(last));
                                        }
                                        _ => {}
                                    }
                                } />
                        </div>
                        <div class="project-search-results">
                            {move || {
                                let loc = locale.get();
                                let q = search_query.get().trim().to_lowercase();
                                let mut idx = 0usize;

                                let project_start = idx;
                                let project_rows = projects
                                    .get()
                                    .into_iter()
                                    .filter(|p| contains_search(&q, &[&p.name, &p.description]))
                                    .take(HOME_SEARCH_PROJECT_LIMIT)
                                    .map(|p| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let sessions = tf(loc, "projects.sessions_n", &[("n", &p.session_count.to_string())]);
                                        let when = format_relative_time(p.updated_at, loc);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                <span class="gi folder"></span>
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{p.name.clone()}</span>
                                                    <span class="project-search-sub">
                                                        {sessions}{(!when.is_empty()).then(|| format!(" · {when}")).unwrap_or_default()}
                                                    </span>
                                                </span>
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_project_rows = idx > project_start;
                                let artifact_start = idx;
                                let artifact_rows = artifact_hits
                                    .get()
                                    .into_iter()
                                    .take(HOME_SEARCH_ARTIFACT_LIMIT)
                                    .map(|a| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let badge = artifact_badge(&a.kind, &a.name);
                                        let when = format_relative_time(a.ts, loc);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                <span class="gi doc"></span>
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{a.name.clone()}</span>
                                                    <span class="project-search-sub">
                                                        {a.path.clone()}{(!when.is_empty()).then(|| format!(" · {when}")).unwrap_or_default()}
                                                    </span>
                                                </span>
                                                <span class="project-search-badge">{badge}</span>
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_artifact_rows = idx > artifact_start;
                                let session_start = idx;
                                let session_rows = recent
                                    .get()
                                    .into_iter()
                                    .filter(|s| contains_search(&q, &[&s.title]))
                                    .take(HOME_SEARCH_SESSION_LIMIT)
                                    .map(|s| {
                                        let row_idx = idx;
                                        idx += 1;
                                        let open = run_search_action;
                                        let status = SessionStatusKind::from_str(&s.status);
                                        view! {
                                            <button type="button" class="project-search-row" class:active=move || search_active.get() == row_idx
                                                on:click=move |_| open.call(row_idx)>
                                                <span class="gi bubble"></span>
                                                <span class="project-search-main">
                                                    <span class="project-search-title">{s.title.clone()}</span>
                                                    <span class="project-search-sub">{format_relative_time(s.ts, loc)}</span>
                                                </span>
                                                <SessionStatusBadge status=status locale=locale />
                                            </button>
                                        }
                                    })
                                    .collect_view();
                                let has_session_rows = idx > session_start;
                                view! {
                                    {has_project_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.title")}</div>
                                            {project_rows}
                                        </div>
                                    })}
                                    {has_artifact_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.search_artifacts")}</div>
                                            {artifact_rows}
                                        </div>
                                    })}
                                    {has_session_rows.then(|| view! {
                                        <div class="project-search-section">
                                            <div class="project-search-label">{move || t(locale.get(), "projects.recent")}</div>
                                            {session_rows}
                                        </div>
                                    })}
                                }.into_view()
                            }}
                        </div>
                        <button type="button" class="project-search-new"
                            class:active=move || search_active.get() + 1 == search_count()
                            on:click=move |_| {
                                search_open.set(false);
                                creating.set(true);
                            }>
                            <span class="gi plus"></span>
                            <span>{move || t(locale.get(), "projects.new")}</span>
                        </button>
                        <div class="project-search-foot">
                            <span><kbd>"↑↓"</kbd>{move || t(locale.get(), "projects.search_nav")}</span>
                            <span><kbd>"↵"</kbd>{move || t(locale.get(), "projects.search_open")}</span>
                            <span><kbd>"esc"</kbd>{move || t(locale.get(), "projects.search_close")}</span>
                        </div>
                    </div>
                </div>
            })}
            <div class="projects-cols">
                <div class="projects-col">
                    <h2>{move || t(locale.get(), "projects.title")}</h2>
                    {move || creating.get().then(|| view! {
                        <div class="overlay">
                            <div class="modal proj-settings-modal">
                                <div class="ps-head">
                                    <h2>{move || t(locale.get(), "projects.new")}</h2>
                                    <button type="button" class="ps-close"
                                        title=move || t(locale.get(), "projects.cancel")
                                        on:click=move |_| creating.set(false)>{compose_icon("close")}</button>
                                </div>
                                <label>
                                    {move || t(locale.get(), "proj_settings.name")}
                                    <input id="new-project-name" autofocus=true
                                        placeholder=move || t(locale.get(), "projects.name_ph")
                                        prop:value=move || new_name.get()
                                        on:input=move |e| new_name.set(event_target_value(&e)) />
                                </label>
                                <label>
                                    {move || t(locale.get(), "projects.directory")}
                                    <div class="pn-dir">
                                        <button type="button" class="btn-ghost" on:click=choose_dir>
                                            {move || t(locale.get(), "projects.choose_dir")}</button>
                                        <span class="path">{move || new_dir.get()}</span>
                                    </div>
                                </label>
                                <label>
                                    {move || t(locale.get(), "proj_settings.description")}
                                    <span class="ps-hint">{move || t(locale.get(), "proj_settings.description_hint")}</span>
                                    <textarea class="ps-textarea" rows="2"
                                        prop:value=move || new_desc.get()
                                        on:input=move |ev| new_desc.set(event_target_value(&ev))></textarea>
                                </label>
                                <label>
                                    {move || t(locale.get(), "proj_settings.agent_context")}
                                    <span class="ps-hint">{move || t(locale.get(), "proj_settings.agent_context_hint")}</span>
                                    <textarea class="ps-textarea ps-ctx" rows="8"
                                        prop:value=move || new_ctx.get()
                                        on:input=move |ev| new_ctx.set(event_target_value(&ev))></textarea>
                                </label>
                                <div class="row">
                                    <button type="button" on:click=move |_| creating.set(false)>
                                        {move || t(locale.get(), "projects.cancel")}</button>
                                    <button type="button" class="primary"
                                        disabled=move || new_name.get().trim().is_empty() || new_dir.get().trim().is_empty()
                                        on:click=submit>{move || t(locale.get(), "projects.create")}</button>
                                </div>
                            </div>
                        </div>
                    })}
                    <button type="button" class="proj-card proj-example" on:click=move |_| on_open_demo.call(())>
                        <div>
                            <div class="pc-name">
                                {move || t(locale.get(), "projects.example")}
                                <span class="pc-tag">{move || t(locale.get(), "projects.example_tag")}</span>
                            </div>
                            <div class="pc-meta">{move || tf(locale.get(), "projects.sessions_n", &[("n", &demo_count.get().to_string())])}</div>
                        </div>
                    </button>
                    {move || {
                        let loc = locale.get();
                        let list = projects.get();
                        if list.is_empty() && !creating.get() {
                            return view! {}.into_view();
                        }
                        list.into_iter().map(|p| {
                            let id_open = p.id.clone();
                            let id_del = p.id.clone();
                            let id_win = p.id.clone();
                            let meta = tf(loc, "projects.sessions_n", &[("n", &p.session_count.to_string())]);
                            let active = p.running_count + p.needs_you_count;
                            let dot_class = if p.running_count > 0 { "running" } else { "ready" };
                            let when = format_relative_time(p.updated_at, loc);
                            view! {
                                <div class="proj-card">
                                    <button type="button" class="proj-card-main" on:click=move |_| on_open.call(id_open.clone())>
                                    <div class="pc-main">
                                        <div class="pc-name-row">
                                            <div class="pc-name">{p.name.clone()}</div>
                                            {(active > 0).then(|| view! {
                                                <span class=format!("pc-dot {dot_class}")>
                                                    <span class="pc-dot-mark"></span>
                                                    <span class="pc-dot-n">{active}</span>
                                                </span>
                                            })}
                                        </div>
                                        <div class="pc-meta-row">
                                            <span class="pc-meta">{meta}</span>
                                            {(!when.is_empty()).then(|| view! { <span class="pc-when">{when.clone()}</span> })}
                                        </div>
                                    </div>
                                    </button>
                                    <div class="pc-actions">
                                    <button class="pc-window" title=t(loc, "projects.new_window")
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            let id = id_win.clone();
                                            spawn_local(async move {
                                                let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                let _ = invoke("open_project_window", arg).await;
                                            });
                                        }>{compose_icon("copy")}</button>
                                    <button class="pc-del" title=t(loc, "projects.delete")
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            pending_delete.set(Some(id_del.clone()));
                                        }>{compose_icon("close")}</button>
                                    </div>
                                </div>
                            }
                        }).collect_view()
                    }}
                </div>
                <div class="projects-col">
                    <h2>{move || t(locale.get(), "projects.recent")}</h2>
                    {move || recent.get().into_iter().map(|s| {
                        let (pid, sid) = (s.project_id.clone(), s.id.clone());
                        let status = SessionStatusKind::from_str(&s.status);
                        view! {
                            <button type="button" class="proj-card proj-recent" data-testid="recent-session-card"
                                on:click=move |_| on_open_session.call((pid.clone(), sid.clone()))>
                                <div class="pc-main">
                                    <div class="pc-name-row">
                                        <div class="pc-name">{s.title.clone()}</div>
                                        <SessionStatusBadge status=status locale=locale />
                                    </div>
                                </div>
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>
            {move || pending_delete.get().map(|id| {
                let confirm_del = delete_confirmed.clone();
                view! {
                    <div class="overlay">
                        <div class="modal confirm-modal">
                            <h2>{move || t(locale.get(), "confirm.title")}</h2>
                            <div class="hint">{move || t(locale.get(), "projects.delete_confirm")}</div>
                            <div class="row">
                                <button on:click=move |_| pending_delete.set(None)>
                                    {move || t(locale.get(), "settings.cancel")}</button>
                                <button class="primary" on:click=move |_| {
                                    pending_delete.set(None);
                                    confirm_del(id.clone());
                                }>{move || t(locale.get(), "confirm.approve")}</button>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
    }
}

/// Apply a transcript mutation to the right session: the live `items` view when
/// `fid` is the active session, otherwise the background cache keyed by `fid`.
/// This is what lets a second conversation stream while the user views another.
pub(super) fn route_items(
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

/// A dedicated project window (#52) carries `?project=<id>` in its URL. Returns
/// that id so the window opens straight into the project and skips the landing.
/// Project ids are UUIDs or "default" — no percent-decoding needed.
pub(super) fn url_project_param() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let q = search.strip_prefix('?').unwrap_or(&search);
    q.split('&')
        .find_map(|p| p.strip_prefix("project="))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

#[component]
pub(super) fn CommandPalette(
    open: RwSignal<bool>,
    current_project_id: Signal<Option<String>>,
    on_open_project: Callback<String>,
    on_open_session: Callback<(String, String)>,
    on_open_artifact: Callback<(String, String, String)>,
    on_new_session: Callback<()>,
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
        if !open.get() { return; }
        let q = query.get();
        spawn_local(async move {
            let p = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(p) { projects.set(rows); }
            let a = invoke("search_artifacts", to_value(&serde_json::json!({ "query": q, "limit": 12, "allProjects": true })).unwrap()).await;
            if query.get_untracked() == q {
                if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(a) { artifacts.set(rows); }
            }
            let s = invoke("search_sessions", to_value(&serde_json::json!({ "query": q, "limit": 12 })).unwrap()).await;
            if query.get_untracked() == q {
                if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SessionSearchInfo>>(s) { sessions.set(rows); }
            }
        });
    });
    create_effect(move |_| { open.get(); query.get(); active.set(0); });
    create_effect(move |_| {
        if open.get() { focus_element_soon("command-palette-input"); }
    });
    let items = create_memo(move |_| {
        let q = query.get().trim().to_lowercase();
        let current = current_project_id.get();
        let mut out = Vec::new();
        let mut ps: Vec<_> = projects.get().into_iter().filter(|p| contains_search(&q, &[&p.name, &p.description])).collect();
        ps.sort_by_key(|p| (current.as_deref() != Some(p.id.as_str()), p.name.clone()));
        out.extend(ps.into_iter().map(CommandPaletteItem::Project));
        let mut ars = artifacts.get();
        ars.sort_by_key(|a| (current.as_deref() != a.project_id.as_deref(), std::cmp::Reverse(a.ts)));
        out.extend(ars.into_iter().map(CommandPaletteItem::Artifact));
        let mut ss = sessions.get();
        ss.sort_by_key(|s| (current.as_deref() != Some(s.project_id.as_str()), std::cmp::Reverse(s.activity_at)));
        out.extend(ss.into_iter().map(CommandPaletteItem::Session));
        out.push(CommandPaletteItem::Command("new"));
        if current.is_some() {
            out.push(CommandPaletteItem::Command("settings"));
            out.push(CommandPaletteItem::Command("skills"));
        }
        out
    });
    let open_item = Callback::new(move |idx: usize| {
        let Some(item) = items.get().get(idx).cloned() else { return; };
        open.set(false);
        match item {
            CommandPaletteItem::Project(p) => on_open_project.call(p.id),
            CommandPaletteItem::Artifact(a) => {
                let kind = file_kind(&a.name).or_else(|| file_kind(&a.path)).unwrap_or("text").to_string();
                on_open_artifact.call((format!("artifact:{}", a.id), a.name, kind));
            }
            CommandPaletteItem::Session(s) => on_open_session.call((s.project_id, s.id)),
            CommandPaletteItem::Command("new") => on_new_session.call(()),
            CommandPaletteItem::Command("settings") => on_project_settings.call(()),
            CommandPaletteItem::Command("skills") => on_manage_skills.call(()),
            CommandPaletteItem::Command(_) => {},
        }
    });
    let attach_item = Callback::new(move |idx: usize| {
        let list = items.get();
        let item = list.get(idx).cloned().filter(|item| matches!(item, CommandPaletteItem::Artifact(_) | CommandPaletteItem::Session(_)))
            .or_else(|| list.into_iter().find(|item| matches!(item, CommandPaletteItem::Artifact(_) | CommandPaletteItem::Session(_))));
        let Some(item) = item else { return; };
        match item {
            CommandPaletteItem::Artifact(a) => on_attach.call(ComposerReferenceChip::Artifact { id: a.id, name: a.name }),
            CommandPaletteItem::Session(s) => on_attach.call(ComposerReferenceChip::Session { id: s.id, title: s.title, project_name: s.project_name }),
            _ => return,
        }
        open.set(false);
        focus_composer();
    });
    view! {
        {move || open.get().then(|| view! {
            <div class="project-search-overlay" on:click=move |_| open.set(false)>
                <div class="project-search-dialog" role="dialog" aria-label="Search"
                    on:click=|ev| ev.stop_propagation()>
                    <div class="project-search-input">
                        <span class="gi search"></span>
                        <input id="command-palette-input" type="text" inputmode="search" autofocus=true
                            autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                            placeholder="Search this project…"
                            prop:value=move || query.get()
                            on:input=move |ev| query.set(event_target_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ev.is_composing() { return; }
                                let n = items.get().len();
                                match ev.key().as_str() {
                                    "Escape" => { ev.prevent_default(); open.set(false); }
                                    "ArrowDown" => { ev.prevent_default(); if n > 0 { let next = (active.get() + 1) % n; active.set(next); scroll_picker_item(".project-search-dialog:not(.action-palette) .project-search-row", next); } }
                                    "ArrowUp" => { ev.prevent_default(); if n > 0 { let next = (active.get() + n - 1) % n; active.set(next); scroll_picker_item(".project-search-dialog:not(.action-palette) .project-search-row", next); } }
                                    "Enter" if ev.shift_key() => { ev.prevent_default(); attach_item.call(active.get()); }
                                    "Enter" => { ev.prevent_default(); open_item.call(active.get()); }
                                    _ => {}
                                }
                            } />
                    </div>
                    <div class="project-search-results">
                        {move || items.get().into_iter().enumerate().map(|(i, item)| {
                            let (icon, title, sub) = match item {
                                CommandPaletteItem::Project(p) => ("folder", p.name, p.description),
                                CommandPaletteItem::Artifact(a) => ("doc", a.name, a.project_name.unwrap_or_default()),
                                CommandPaletteItem::Session(s) => ("bubble", s.title, s.project_name),
                                CommandPaletteItem::Command("new") => ("plus", t(locale.get(), "projects.new").to_string(), "Command".into()),
                                CommandPaletteItem::Command("settings") => ("gear", t(locale.get(), "proj_settings.title").to_string(), "Command".into()),
                                CommandPaletteItem::Command("skills") => ("grid", t(locale.get(), "skills.title").to_string(), "Command".into()),
                                CommandPaletteItem::Command(_) => ("doc", String::new(), String::new()),
                            };
                            view! {
                                <button type="button" class="project-search-row" class:active=move || active.get() == i
                                    on:mousemove=move |_| active.set(i)
                                    on:click=move |_| open_item.call(i)>
                                    <span class=format!("gi {icon}")></span>
                                    <span class="project-search-main">
                                        <span class="project-search-title">{title}</span>
                                        {(!sub.trim().is_empty()).then(|| view! { <span class="project-search-sub">{sub}</span> })}
                                    </span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                    <div class="project-search-foot"><span><kbd>"↑↓"</kbd>"navigate"</span><span><kbd>"↵"</kbd>"open"</span><span><kbd>"⇧↵"</kbd>"attach"</span><span><kbd>"esc"</kbd>"close"</span></div>
                </div>
            </div>
        })}
    }
}

#[component]
pub(super) fn ActionPalette(open: RwSignal<bool>, on_action: Callback<&'static str>) -> impl IntoView {
    let locale = use_locale();
    let query = create_rw_signal(String::new());
    let active = create_rw_signal(0usize);
    create_effect(move |_| {
        if !open.get() { return; }
        query.set(String::new());
        active.set(0);
        let focus = Closure::once(|| {
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return; };
            let Some(input) = doc.get_element_by_id("action-palette-input") else { return; };
            let _ = input.dyn_ref::<web_sys::HtmlElement>().map(|el| el.focus());
        });
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(focus.as_ref().unchecked_ref(), 0);
        }
        focus.forget();
    });
    let actions = create_memo(move |_| {
        let loc = locale.get();
        let general = t(loc, "command.group.general").to_string();
        let navigate = t(loc, "command.group.navigate").to_string();
        let appearance = t(loc, "command.group.appearance").to_string();
        let entries = [
            ("new", "plus", "command.new_session", general.clone(), "Ctrl/⌘ N"),
            ("search", "search", "command.search", general.clone(), "Ctrl/⌘ K"),
            ("settings", "gear", "command.settings", general.clone(), "Ctrl/⌘ ,"),
            ("project-settings", "gear", "command.project_settings", general.clone(), ""),
            ("skills", "grid", "command.skills", general, ""),
            ("projects", "folder", "command.projects", navigate.clone(), ""),
            ("toggle-sidebar", "panel", "command.toggle_sidebar", navigate.clone(), "Ctrl/⌘ B"),
            ("artifacts", "grid", "command.artifacts", navigate.clone(), ""),
            ("notebook", "doc", "command.notebook", navigate.clone(), ""),
            ("files", "doc", "command.files", navigate.clone(), ""),
            ("provenance", "copy", "command.provenance", navigate.clone(), ""),
            ("contexts", "server", "command.contexts", navigate.clone(), ""),
            ("side-chat", "bubble", "command.side_chat", navigate.clone(), ""),
            ("close-panel", "panel", "command.close_panel", navigate, ""),
            ("theme-light", "gear", "command.theme_light", appearance.clone(), ""),
            ("theme-dark", "gear", "command.theme_dark", appearance.clone(), ""),
            ("theme-system", "gear", "command.theme_system", appearance, ""),
        ];
        let q = query.get().trim().to_lowercase();
        entries.into_iter().filter_map(|(id, icon, key, group, shortcut)| {
            let title = t(loc, key).to_string();
            contains_search(&q, &[id, &title, &group]).then_some(CommandAction { id, icon, title, group, shortcut })
        }).collect::<Vec<_>>()
    });
    let run = Callback::new(move |index: usize| {
        let Some(action) = actions.get().get(index).cloned() else { return; };
        open.set(false);
        on_action.call(action.id);
    });
    view! {
        {move || open.get().then(|| view! {
            <div class="project-search-overlay action-palette-overlay" on:click=move |_| open.set(false)>
                <div class="project-search-dialog action-palette" role="dialog" aria-label="Command Palette"
                    on:click=|ev| ev.stop_propagation()>
                    <div class="project-search-input">
                        <span class="gi search"></span>
                        <input id="action-palette-input" type="text" inputmode="search" autofocus=true
                            autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                            placeholder=move || t(locale.get(), "command.placeholder")
                            prop:value=move || query.get()
                            on:input=move |ev| { query.set(event_target_value(&ev)); active.set(0); }
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ev.is_composing() { return; }
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
                            rows.into_iter().enumerate().map(|(i, action)| {
                                let previous_group = (i > 0).then(|| actions.get().get(i - 1).map(|a| a.group.clone())).flatten();
                                let show_group = previous_group.as_deref() != Some(action.group.as_str());
                                view! {
                                    {show_group.then(|| view! { <div class="action-palette-group">{action.group.clone()}</div> })}
                                    <button type="button" class="project-search-row action-palette-row" class:active=move || active.get() == i
                                        on:mousemove=move |_| active.set(i)
                                        on:click=move |_| run.call(i)>
                                        <span class=format!("gi {}", action.icon)></span>
                                        <span class="project-search-main"><span class="project-search-title">{action.title}</span></span>
                                        {(!action.shortcut.is_empty()).then(|| view! { <kbd class="action-shortcut">{action.shortcut}</kbd> })}
                                    </button>
                                }
                            }).collect_view()
                        }}
                    </div>
                    <div class="project-search-foot"><span><kbd>"↑↓"</kbd>"navigate"</span><span><kbd>"↵"</kbd>"run"</span><span><kbd>"esc"</kbd>"close"</span></div>
                </div>
            </div>
        })}
    }
}
