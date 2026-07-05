mod bindings;
mod context_menu;
mod dto;
mod i18n;
mod text;

use bindings::{
    attach_chat_autoscroll, force_chat_bottom, invoke, invoke_checked, invoke_timeout, listen,
    mount_preview, open_external_url, schedule_chat_follow, schedule_highlight, upload_files,
    upload_input_files, CHAT_SCROLLER_ID, CHAT_THREAD_ID,
};
use context_menu::{ContextMenuPortal, CtxMenu};
use dto::*;
use i18n::{localize_backend, set_document_lang, tab_count, tf, t, use_locale, Locale};
use leptos::{ev, window_event_listener, *};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use text::{
    dom_value, event_target_checked, event_target_input, event_target_value, extract_href_from_tag,
    fasta_seq_count, file_kind, format_bytes, html_escape, is_external_href, is_separator,
    is_table_row, join_path, md_inline_to_html, md_to_html, next_artifact_id, normalize_path,
    opens_in_system_browser, parent_path, parse_csv_line, provider_defaults, provider_value, split_row, tool_lang,
    unique_dom_id,
};
use serde_wasm_bindgen::to_value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Stable substring of the backend's missing-key error (`src-tauri` `send_message`),
/// used to turn that failure into an actionable "open Settings" prompt.
const NO_API_KEY_MARK: &str = "No API key set";

#[derive(Clone)]
enum FolderModal {
    Create,
    Rename(String),
}

#[derive(Clone)]
enum UiConfirm {
    DeleteFolder(String),
    DeleteSession(String),
}

fn allow_drop(ev: &web_sys::DragEvent) {
    ev.prevent_default();
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        let _ = dt.set_drop_effect("move");
    }
}

fn drag_session_id(ev: &web_sys::DragEvent, cached: Option<String>) -> Option<String> {
    cached.filter(|s| !s.is_empty()).or_else(|| {
        ev.data_transfer()
            .and_then(|dt| dt.get_data("text/plain").ok())
            .filter(|s| !s.is_empty())
    })
}

fn start_session_drag(ev: &web_sys::DragEvent, id: &str) {
    ev.stop_propagation();
    if let Some(dt) = ev.data_transfer() {
        let _ = dt.set_effect_allowed("move");
        let _ = dt.set_data("text/plain", id);
    }
}

fn composer_attachment_key(name: &str, idx: usize) -> String {
    format!("att-{idx}-{name}")
}

fn parse_upload_results(v: JsValue) -> Vec<UploadFileResult> {
    if v.is_null() || v.is_undefined() {
        return vec![];
    }
    serde_wasm_bindgen::from_value(v).unwrap_or_default()
}

fn file_list_len(files: &JsValue) -> usize {
    js_sys::Reflect::get(files, &JsValue::from_str("length"))
        .ok()
        .and_then(|n| n.as_f64())
        .map(|n| n as usize)
        .unwrap_or(0)
}

fn begin_uploads(attachments: RwSignal<Vec<ComposerAttachment>>, uploading: RwSignal<bool>, count: usize) {
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

fn finish_uploads(
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

fn queue_uploads(attachments: RwSignal<Vec<ComposerAttachment>>, uploading: RwSignal<bool>, files: JsValue) {
    let count = file_list_len(&files);
    begin_uploads(attachments, uploading, count);
    spawn_local(async move {
        finish_uploads(attachments, uploading, parse_upload_results(upload_files(files).await));
    });
}

fn upload_from_input(
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

fn attachment_paths(items: &[ComposerAttachment]) -> Vec<String> {
    items
        .iter()
        .filter_map(|a| match a {
            ComposerAttachment::Ready { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn message_with_attachments(text: &str, paths: &[String]) -> String {
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

/// If the composer text ends in an `@…` mention token, return (byte offset of `@`,
/// the query after it). The `@` must be at the string start or follow whitespace,
/// and no whitespace may come between it and the end of the string.
/// ponytail: only matches a mention at the end of the text (the typing caret),
/// not one edited into the middle — upgrade to caret-index scanning if that matters.
fn active_mention(text: &str) -> Option<(usize, String)> {
    let at = text.rfind('@')?;
    if at > 0 && !text[..at].chars().next_back()?.is_whitespace() {
        return None;
    }
    let query = &text[at + 1..];
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    Some((at, query.to_string()))
}

#[cfg(test)]
mod mention_tests {
    use super::active_mention;

    #[test]
    fn detects_mention_at_end() {
        assert_eq!(active_mention("look at @qc"), Some((8, "qc".into())));
        assert_eq!(active_mention("@qc"), Some((0, "qc".into())));
        assert_eq!(active_mention("@"), Some((0, String::new())));
    }

    #[test]
    fn ignores_non_mentions() {
        assert_eq!(active_mention("no at sign"), None);
        assert_eq!(active_mention("email a@b.com"), None); // '@' not after whitespace
        assert_eq!(active_mention("@qc then more"), None); // whitespace after query
    }
}

fn js_error_text(err: JsValue) -> String {
    err.as_string()
        .or_else(|| js_sys::Reflect::get(&err, &JsValue::from_str("message")).ok().and_then(|v| v.as_string()))
        .unwrap_or_else(|| t(Locale::En, "err.unknown").into())
}

fn copy_text(text: String) {
    if text.is_empty() {
        return;
    }
    spawn_local(async move {
        let Some(window) = web_sys::window() else { return; };
        let promise = window.navigator().clipboard().write_text(&text);
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    });
}

fn normalize_settings_mut(cfg: &mut Settings) {
    cfg.provider = provider_value(&cfg.provider).into();
    cfg.api_url = cfg.api_url.trim().into();
    cfg.model = cfg.model.trim().into();
}

fn normalized_settings(mut cfg: Settings) -> Settings {
    normalize_settings_mut(&mut cfg);
    cfg
}

fn settings_required_error_key(cfg: &Settings, key: &str) -> Option<&'static str> {
    if cfg.api_url.trim().is_empty() {
        return Some("err.api_url_required");
    }
    if cfg.model.trim().is_empty() {
        return Some("err.model_required");
    }
    let stored = t(Locale::En, "settings.stored_key");
    let has_new_key = !key.trim().is_empty() && !key.starts_with(&stored) && !key.starts_with("(stored");
    if !cfg.has_api_key && !has_new_key {
        return Some("err.api_key_required");
    }
    None
}

fn is_stored_key_placeholder(key: &str, locale: Locale) -> bool {
    let stored = t(locale, "settings.stored_key");
    key.starts_with(&stored) || key.starts_with("(stored")
}

fn should_close_right_pane_on_escape(ev: &web_sys::KeyboardEvent) -> bool {
    if ev.default_prevented() || ev.is_composing() {
        return false;
    }
    let Some(window) = web_sys::window() else { return false };
    let Some(document) = window.document() else { return false };
    let target = ev.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok());
    let Some(node) = target.as_ref() else { return true };
    if !node.is_connected() {
        return false;
    }
    if let Ok(Some(panel)) = document.query_selector(".rightpane") {
        if panel.contains(Some(node)) {
            return true;
        }
    }
    document.body().as_ref().is_some_and(|body| node.is_same_node(Some(body)))
        || document.document_element().as_ref().is_some_and(|html| node.is_same_node(Some(html)))
}

/// Single source of truth for `invoke` argument payloads.
///
/// Tauri v2 deserializes command arguments from JS **camelCase** keys onto the
/// Rust **snake_case** parameters. A snake_case key (`session_id`) never binds:
/// an `Option` param silently becomes `None`, which once made every send fork a
/// brand-new conversation instead of continuing the active one. Keep every
/// multi-word key camelCase here; `tauri_args_tests` pins them.
mod tauri_args {
    use serde_json::{json, Value};

    pub fn stop_agent(session_id: &Option<String>) -> Value {
        json!({ "sessionId": session_id })
    }
    pub fn review_session(session_id: &Option<String>) -> Value {
        json!({ "sessionId": session_id })
    }
    pub fn rewind_session(session_id: &Option<String>, user_index: usize) -> Value {
        json!({ "sessionId": session_id, "userIndex": user_index })
    }
    pub fn confirm_response(session_id: &str, approved: bool) -> Value {
        json!({ "sessionId": session_id, "approved": approved })
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
        })
        .unwrap();
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["message"], "hi");
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

        let v = tauri_args::rewind_session(&sid, 3);
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["userIndex"], 3);
        assert!(v.get("user_index").is_none());

        let v = tauri_args::confirm_response("frame-1", true);
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["approved"], true);

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
    }
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect::<BTreeSet<_>>().into_iter().collect()
}

fn join_tags(tags: &[String]) -> String {
    tags.join(", ")
}

fn skill_matches_filter(skill: &SkillRow, tag: &str, query: &str) -> bool {
    let tag_match = match tag {
        "" => true,
        "__untagged" => skill.tags.is_empty(),
        t => skill.tags.iter().any(|s| s == t),
    };
    let q = query.trim().to_ascii_lowercase();
    tag_match && (q.is_empty() || skill.name.to_ascii_lowercase().contains(&q) || skill.description.to_ascii_lowercase().contains(&q))
}

fn refresh_capabilities(caps: RwSignal<Option<Capabilities>>) {
    spawn_local(async move {
        let v = invoke("get_capabilities", JsValue::UNDEFINED).await;
        if let Ok(data) = serde_wasm_bindgen::from_value::<Capabilities>(v) {
            caps.set(Some(data));
        }
    });
}

fn begin_pending_turn(pending: RwSignal<HashMap<String, usize>>, running: RwSignal<HashSet<String>>, id: &str) {
    pending.update(|m| {
        *m.entry(id.to_string()).or_insert(0) += 1;
    });
    running.update(|r| {
        r.insert(id.to_string());
    });
}

fn finish_pending_turn(pending: RwSignal<HashMap<String, usize>>, running: RwSignal<HashSet<String>>, id: &str) {
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

fn clear_running_if_idle(pending: RwSignal<HashMap<String, usize>>, running: RwSignal<HashSet<String>>, id: &str) {
    if pending.with(|m| m.get(id).copied().unwrap_or(0)) == 0 {
        running.update(|r| {
            r.remove(id);
        });
    }
}

fn strip_approval_pending(items: &mut Vec<ChatItem>) {
    items.retain(|i| !matches!(i, ChatItem::ApprovalPending { .. }));
}

fn last_tool_input(items: &[ChatItem], tool: &str) -> String {
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

fn trailing_queue_start(items: &[ChatItem]) -> usize {
    items
        .iter()
        .rposition(|item| !matches!(item, ChatItem::QueuedUser(_)))
        .map(|i| i + 1)
        .unwrap_or(0)
}

fn start_user_turn(items: &mut Vec<ChatItem>, text: String, model: Option<String>) {
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
    } else {
        items.push(ChatItem::User(text));
        items.push(ChatItem::Assistant {
            text: String::new(),
            model,
        });
    }
}

fn append_assistant_delta(items: &mut Vec<ChatItem>, delta: String, model: Option<String>) {
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

fn append_reasoning_delta(items: &mut Vec<ChatItem>, delta: String) {
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

fn append_stdout_chunk(items: &mut Vec<ChatItem>, chunk: String) {
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
    items.insert(queue_start, ChatItem::Tool { name: "stdout".into(), ok: None, input: String::new(), output: chunk });
}

// --- Streaming delta batching (#65) ------------------------------------------
//
// The backend emits one `agent` event per LLM/stdout chunk. Applying each one
// writes the `items` signal, and every write re-runs the thread view and the
// artifact scan — O(conversation length) work per token, which freezes long
// conversations. Buffer the append-only deltas and flush them on a short timer
// so the signal is written at most ~20×/s regardless of token rate.

enum PendingDelta {
    Text(String),
    Reasoning(String),
    Stdout(String),
}

type DeltaBuf = Rc<RefCell<HashMap<String, Vec<PendingDelta>>>>;

/// Append a delta to a session's queue, coalescing consecutive same-kind chunks.
fn queue_delta(buf: &DeltaBuf, fid: String, d: PendingDelta) {
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
fn flush_delta_buf(
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

fn schedule_delta_flush(
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

fn format_relative_time(ts: i64, locale: Locale) -> String {
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
fn SessionStatusBadge(status: SessionStatusKind, locale: RwSignal<Locale>) -> impl IntoView {
    let key = status.i18n_key();
    let class = status.css();
    view! {
        <span class=format!("sess-status sess-status-{class}")>
            {move || t(locale.get(), key)}
        </span>
    }
}

fn profile_to_form(m: &ModelProfile) -> ModelForm {
    ModelForm {
        id: Some(m.id.clone()),
        label: m.label.clone(),
        provider: m.provider.clone(),
        api_url: m.api_url.clone(),
        model: m.model.clone(),
        max_tokens: if m.max_tokens >= 16 { m.max_tokens } else { 8192 },
        reasoning_effort: m.reasoning_effort.clone(),
    }
}

fn new_model_form() -> ModelForm {
    let (api_url, model) = provider_defaults("openai");
    ModelForm {
        provider: "openai".into(),
        api_url: api_url.into(),
        model: model.into(),
        max_tokens: 8192,
        ..Default::default()
    }
}

fn model_form_to_settings(form: &ModelForm, has_api_key: bool) -> Settings {
    let mut cfg = Settings::default();
    cfg.provider = provider_value(&form.provider).into();
    cfg.api_url = form.api_url.trim().into();
    cfg.model = form.model.trim().into();
    cfg.label = form.label.trim().into();
    cfg.has_api_key = has_api_key;
    cfg.max_tokens = form.max_tokens;
    cfg.reasoning_effort = form.reasoning_effort.clone();
    cfg
}

fn settings_section_label(loc: Locale, section: &str) -> String {
    match section {
        "models" => t(loc, "settings.nav.models"),
        "memory" => t(loc, "settings.nav.memory"),
        "skills" => t(loc, "settings.nav.skills"),
        "connections" => t(loc, "settings.nav.connections"),
        _ => t(loc, "settings.title"),
    }
    .into()
}

fn settings_subpage_label(
    loc: Locale,
    section: &str,
    model_form: Option<&ModelForm>,
    conn_form: Option<&ConnForm>,
    open_conn: Option<&str>,
    memory_selected: Option<&str>,
) -> Option<String> {
    match section {
        "models" => model_form.map(|f| {
            if f.id.is_some() {
                t(loc, "models.edit").into()
            } else {
                t(loc, "models.add").into()
            }
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

fn build_conn_json(f: &ConnForm, assign_id: bool) -> serde_json::Value {
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

fn refresh_dir(cwd: RwSignal<String>, entries: RwSignal<Vec<DirEntry>>) {
    spawn_local(async move {
        let path = cwd.get();
        let v = invoke("list_dir", to_value(&serde_json::json!({ "path": path })).unwrap()).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DirEntry>>(v) {
            entries.set(list);
        }
    });
}

fn art_label(a: &Artifact) -> String {
    if a.name.len() <= 28 {
        a.name.clone()
    } else {
        format!("artifact-{}", &a.id[..8.min(a.id.len())])
    }
}

fn art_chip(idx: usize, a: &Artifact) -> String {
    let label = html_escape(&art_label(a));
    let title = html_escape(&a.name);
    format!(
        r#"<button type="button" class="art-ref" data-art-idx="{idx}" title="{title}">{label}</button>"#
    )
}

fn artifact_file_paths(a: &Artifact) -> Vec<String> {
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

fn href_matches_artifact(href: &str, a: &Artifact) -> bool {
    let h = normalize_path(href);
    artifact_file_paths(a).iter().any(|p| *p == h)
}

fn artifact_index_for_href(arts: &[Artifact], href: &str) -> Option<usize> {
    arts.iter()
        .position(|a| href_matches_artifact(href, a))
}

fn replace_file_links(html: String, arts: &[Artifact]) -> String {
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

fn artifact_matches_token(token: &str, id: &str) -> bool {
    let t = token.trim();
    t == id
        || t.starts_with(id)
        || id.starts_with(&t[..t.len().min(8)])
        || t.starts_with(&id[..id.len().min(8)])
}

fn replace_artifact_tokens(mut html: String, arts: &[Artifact]) -> String {
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

/// Post-process rendered Markdown: artifact chips, code wrappers, filename links.
fn enrich_md_html(mut html: String, arts: &[Artifact]) -> String {
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
    html = html.replace("<pre><code", "<pre class=\"md-code\"><code");
    html
}

fn handle_md_click(
    ev: &web_sys::MouseEvent,
    arts: &[Artifact],
    on_artifact: &Callback<usize>,
    on_file: &Callback<(String, String)>,
) {
    use wasm_bindgen::JsCast;
    let mut el = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    while let Some(n) = el {
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

fn refresh_sessions(sessions: RwSignal<Vec<SessionInfo>>) {
    spawn_local(async move {
        let v = invoke("list_sessions", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SessionInfo>>(v) {
            sessions.set(list);
        }
    });
}

fn refresh_folders(folders: RwSignal<Vec<FolderInfo>>) {
    spawn_local(async move {
        let v = invoke("list_folders", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<FolderInfo>>(v) {
            folders.set(list);
        }
    });
}

fn bucket_sessions_by_date(list: &[SessionInfo]) -> (Vec<SessionInfo>, Vec<SessionInfo>) {
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
enum Seg { Text, Table(TableData) }

fn split_segments(text: &str) -> Vec<Seg> {
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

fn push_file_artifact(out: &mut Vec<Artifact>, seen: &mut std::collections::HashSet<String>, path: &str) {
    let p = path.trim().trim_matches('`').trim_matches('"').trim_matches('\'');
    if p.is_empty() || seen.contains(p) { return; }
    let Some(kind) = file_kind(p) else { return; };
    seen.insert(p.to_string());
    let name = p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
    let id = next_artifact_id(out.len());
    out.push(Artifact { id, name, kind, data: PreviewData::File { path: p.to_string(), kind: kind.to_string() } });
}

struct ArtifactScan {
    tbl_n: usize,
    csv_n: usize,
    code_n: usize,
    tex_n: usize,
}

fn collect_markdown_artifacts(
    out: &mut Vec<Artifact>,
    seen: &mut std::collections::HashSet<String>,
    s: &str,
    locale: Locale,
    scan: &mut ArtifactScan,
) {
    for seg in split_segments(s) {
        if let Seg::Table(t) = seg {
            scan.tbl_n += 1;
            let id = next_artifact_id(out.len());
            out.push(Artifact {
                id,
                name: tf(locale, "artifact.table", &[("n", &scan.tbl_n.to_string())]),
                kind: "table",
                data: PreviewData::Table(t),
            });
        }
    }
    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let f = lines[i].trim().to_ascii_lowercase();
        if f.starts_with("```") {
            let lang = f.trim_start_matches('`').split_whitespace().next().unwrap_or("").to_string();
            let mut body = vec![];
            let mut j = i + 1;
            while j < lines.len() && !lines[j].trim().starts_with("```") { body.push(lines[j]); j += 1; }
            if !body.is_empty() {
                if lang == "csv" || lang == "tsv" {
                    let headers = parse_csv_line(body[0]);
                    let rows: Vec<Vec<String>> = body[1..].iter().map(|l| parse_csv_line(l)).collect();
                    scan.csv_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact { id, name: format!("data-{}.csv", scan.csv_n), kind: "csv", data: PreviewData::Table(TableData { headers, rows }) });
                } else if lang == "fasta" || lang == "fa" {
                    let id = next_artifact_id(out.len());
                    out.push(Artifact { id, name: format!("alignment-{}.fasta", scan.csv_n), kind: "fasta", data: PreviewData::Fasta(body.join("\n")) });
                } else {
                    scan.code_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: tf(locale, "artifact.code", &[("n", &scan.code_n.to_string())]),
                        kind: "code",
                        data: PreviewData::Code { lang, body: body.join("\n") },
                    });
                }
            }
            i = j + 1;
            continue;
        }
        if lines[i].trim().starts_with("$") {
            let mut body = vec![];
            let mut j = i + 1;
            while j < lines.len() && !lines[j].trim().ends_with("$") { body.push(lines[j]); j += 1; }
            if j < lines.len() { body.push(lines[j].trim().trim_end_matches("$")); }
            scan.tex_n += 1;
            let id = next_artifact_id(out.len());
            out.push(Artifact {
                id,
                name: tf(locale, "artifact.equation", &[("n", &scan.tex_n.to_string())]),
                kind: "latex",
                data: PreviewData::Latex { tex: body.join("\n"), display: true },
            });
            i = j + 1;
            continue;
        }
        i += 1;
    }
    for word in s.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']') {
        push_file_artifact(out, seen, word);
    }
}

/// Promote `attempt_completion` output into the assistant bubble (web-dist renders
/// completion as the final markdown response, not a collapsed tool row).
fn promote_assistant_text(items: &mut Vec<ChatItem>, text: &str) {
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
fn artifacts_fingerprint(arts: &[Artifact]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for a in arts {
        (&a.id, &a.name).hash(&mut h);
    }
    h.finish()
}

/// Collect tables, code, latex, and file-path artifacts from the transcript.
fn collect_artifacts(items: &[ChatItem], locale: Locale) -> Vec<Artifact> {
    let mut out: Vec<Artifact> = vec![];
    let mut seen = std::collections::HashSet::<String>::new();
    let mut scan = ArtifactScan { tbl_n: 0, csv_n: 0, code_n: 0, tex_n: 0 };

    for it in items {
        match it {
            // Uploaded files live only in the user turn ("Uploaded files: a, b").
            ChatItem::User(s) => {
                for word in s.split(|c: char| c.is_whitespace() || c == ',' || c == '"' || c == '\'') {
                    push_file_artifact(&mut out, &mut seen, word);
                }
            }
            ChatItem::Assistant { text: s, .. } => collect_markdown_artifacts(&mut out, &mut seen, s, locale, &mut scan),
            ChatItem::Tool { name, input, output, .. } => {
                if name == "attempt_completion" && !output.is_empty() {
                    collect_markdown_artifacts(&mut out, &mut seen, output, locale, &mut scan);
                } else {
                    let text = if output.is_empty() { input.as_str() } else { output.as_str() };
                    for word in text.split(|c: char| c.is_whitespace() || c == '\n' || c == '"' || c == '\'') {
                        push_file_artifact(&mut out, &mut seen, word);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn table_view(t: &TableData, locale: Locale) -> impl IntoView {
    let total = t.rows.len();
    let truncated = total > 500;
    let headers: Vec<String> = t.headers.iter().map(|h| md_inline_to_html(h)).collect();
    let rows: Vec<Vec<String>> = t.rows.iter().take(500)
        .map(|r| r.iter().map(|c| md_inline_to_html(c)).collect())
        .collect();
    view! {
        <div class="tbl-wrap">
            {truncated.then(|| view! {
                <div class="tbl-note">{tf(locale, "table.rows_note", &[("total", &total.to_string())])}</div>
            })}
            <table class="tbl">
                <thead><tr>{headers.into_iter().map(|h| view! { <th inner_html=h></th> }).collect_view()}</tr></thead>
                <tbody>
                    {rows.into_iter().map(|r| view! {
                        <tr>{r.into_iter().map(|c| view! { <td inner_html=c></td> }).collect_view()}</tr>
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

fn artifact_meta(a: &Artifact, locale: Locale) -> String {
    match &a.data {
        PreviewData::Table(t) => tf(locale, "artifact.meta.table", &[
            ("rows", &t.rows.len().to_string()),
            ("cols", &t.headers.len().to_string()),
        ]),
        PreviewData::Code { lang, body } => tf(locale, "artifact.meta.code", &[
            ("lang", lang),
            ("lines", &body.lines().count().to_string()),
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
fn HeavyPreview(dom_id: String, kind: String, payload: String) -> impl IntoView {
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

fn parse_csv_text(text: &str) -> Option<TableData> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() { return None; }
    let headers = parse_csv_line(lines[0]);
    let rows: Vec<Vec<String>> = lines[1..].iter().map(|l| parse_csv_line(l)).collect();
    Some(TableData { headers, rows })
}

#[component]
fn CsvFilePreview(path: String) -> impl IntoView {
    let locale = use_locale();
    let table = create_rw_signal::<Option<TableData>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    create_effect(move |_| {
        let path = path.clone();
        let loc = locale.get();
        spawn_local(async move {
            table.set(None);
            err.set(None);
            let v = invoke("read_file", to_value(&serde_json::json!({ "path": path })).unwrap()).await;
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
fn FilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
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
            let arg = to_value(&tauri_args::read_file(&path, Some(32 * 1024 * 1024))).unwrap();
            let fc = match invoke_checked("read_file", arg).await {
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

fn artifact_preview(a: &Artifact, dom_id: String, locale: Locale) -> impl IntoView {
    match &a.data {
        PreviewData::Table(t) => table_view(t, locale).into_view(),
        PreviewData::Text(s) => view! { <pre class="rp-pre">{s.clone()}</pre> }.into_view(),
        PreviewData::Markdown(s) => view! { <div class="md rp-md" inner_html=md_to_html(s)></div> }.into_view(),
        PreviewData::Code { lang, body } => view! {
            <RpCodeView lang=lang.clone() body=body.clone() />
        }.into_view(),
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
fn CodeBlock(lang: String, body: String) -> impl IntoView {
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
fn RpCodeView(lang: String, body: String) -> impl IntoView {
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
fn opens_in_modal(kind: &str) -> bool {
    matches!(kind, "image" | "pdf" | "csv")
}

/// Fire the native save dialog to download a workspace file (backend copies it).
fn download_artifact(path: String) {
    spawn_local(async move {
        let arg = to_value(&serde_json::json!({ "path": path })).unwrap();
        let _ = invoke("download_file", arg).await;
    });
}

/// Click-to-expand modal for a produced artifact: shows the full-size
/// image/PDF (or a CSV as a dataset table) plus tabbed provenance
/// (Code/Log/Inputs/Environment) fetched from `get_artifact_provenance`.
/// Provenance is best-effort — a `None` result (or any empty field within it)
/// renders an empty state; the figure never depends on provenance being present.
#[component]
fn ArtifactModal(
    path: String,
    name: String,
    kind: String,
    session: Option<String>,
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
                    <div class="spacer"></div>
                    <button class="icon-btn" title=move || t(locale.get(), "artifact.download")
                        on:click=move |_| download_artifact(path_dl.clone())>"↓"</button>
                    <button class="icon-btn" title=move || t(locale.get(), "right.close")
                        on:click=move |_| on_close.call(())>"×"</button>
                </div>
                <div class="am-figure">
                    {if kind == "csv" {
                        view! { <CsvFilePreview path=path_head.clone() /> }.into_view()
                    } else if kind == "image" || kind == "pdf" {
                        view! { <FilePreview dom_id=dom_id path=path_head.clone() kind=kind.clone() /> }.into_view()
                    } else {
                        view! { <p class="rp-path hint">{path_head.clone()}</p> }.into_view()
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

fn composer_text_from_user_message(text: &str) -> String {
    const SUFFIX: &str = "\n\nUploaded files: ";
    text.split_once(SUFFIX)
        .map(|(body, _)| body.trim())
        .unwrap_or(text)
        .to_string()
}

fn user_message_index(items: &[ChatItem], ui_index: usize) -> Option<usize> {
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

fn focus_composer() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return; };
    if let Some(el) = doc.get_element_by_id("composer-input") {
        let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|e| e.focus());
    }
}

/// Compose-menu icons: lucide stroke SVGs (paperclip, folder, contrast, scroll, chevron).
fn compose_icon(kind: &str) -> impl IntoView {
    let body = match kind {
        "attach" => view! { <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/> }.into_view(),
        "folder" => view! { <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/> }.into_view(),
        "review" => view! { <circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 1 0 18Z" fill="currentColor" stroke="none"/> }.into_view(),
        "skill" => view! { <path d="M19 17V5a2 2 0 0 0-2-2H4"/><path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3"/> }.into_view(),
        "server" => view! { <rect x="3" y="4" width="18" height="7" rx="1"/><rect x="3" y="13" width="18" height="7" rx="1"/><circle cx="7" cy="7.5" r="0.5" fill="currentColor"/><circle cx="7" cy="16.5" r="0.5" fill="currentColor"/> }.into_view(),
        _ => view! { <path d="M9 18l6-6-6-6"/> }.into_view(), // chevron
    };
    let size = if kind == "chevron" { "16" } else { "18" };
    view! {
        <svg width=size height=size viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>
    }
}

#[component]
fn UserMessage(
    text: String,
    ui_index: usize,
    busy: ReadSignal<bool>,
    on_copy: Callback<String>,
    on_edit: Callback<usize>,
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
            </div>
        </div>
    }
}

#[component]
fn AssistantMessage(
    text: String,
    model: Option<String>,
    artifacts: Vec<Artifact>,
    on_artifact: Callback<usize>,
    on_file: Callback<(String, String)>,
    on_copy: Callback<String>,
) -> impl IntoView {
    let arts_for_html = artifacts.clone();
    let text_for_html = text.clone();
    let html = create_memo(move |_| enrich_md_html(md_to_html(&text_for_html), &arts_for_html));
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
    let locale = use_locale();
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
fn ToolBlock(name: String, ok: Option<bool>, input: String, output: String) -> impl IntoView {
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
fn plan_step_line(line: &str) -> Option<(&'static str, &str)> {
    for (prefix, cls) in [("[x] ", "done"), ("[~] ", "running"), ("[ ] ", "pending")] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some((cls, rest));
        }
    }
    None
}

#[component]
fn ApprovalCard(
    tool: String,
    preview: String,
    session_id: String,
    on_decide: Callback<(String, bool)>,
) -> impl IntoView {
    let locale = use_locale();
    let is_plan = tool == "update_plan";
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
    let sid_deny = session_id;
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
                    <button type="button" class="primary"
                        on:click=move |_| on_decide.call((sid_allow.clone(), true))>
                        {move || t(locale.get(), if is_plan { "approval.plan_approve" } else { "approval.allow_session" })}
                    </button>
                    <button type="button"
                        on:click=move |_| on_decide.call((sid_deny.clone(), false))>
                        {move || t(locale.get(), if is_plan { "approval.plan_reject" } else { "confirm.deny" })}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
fn ProjectsScreen(
    locale: RwSignal<Locale>,
    running: RwSignal<HashSet<String>>,
    approval_pending: ReadSignal<HashSet<String>>,
    on_open: Callback<String>,
    on_open_session: Callback<(String, String)>,
    on_open_demo: Callback<()>,
) -> impl IntoView {
    let projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let recent = create_rw_signal(Vec::<RecentSession>::new());
    let demo_count = create_rw_signal(0usize);
    let creating = create_rw_signal(false);
    let new_name = create_rw_signal(String::new());
    let new_dir = create_rw_signal(String::new());
    let new_desc = create_rw_signal(String::new());
    let new_ctx = create_rw_signal(String::new());

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

    view! {
        <div class="projects-screen">
            <div class="projects-head">
                <div class="projects-title">"Wisp Science"<span class="beta">"Beta"</span></div>
                <button class="btn-primary" on:click=move |_| creating.set(true)>
                    {move || t(locale.get(), "projects.new")}
                </button>
            </div>
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
                                        on:click=move |_| creating.set(false)>"×"</button>
                                </div>
                                <label>
                                    {move || t(locale.get(), "proj_settings.name")}
                                    <input placeholder=move || t(locale.get(), "projects.name_ph")
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
                    <div class="proj-card proj-example" on:click=move |_| on_open_demo.call(())>
                        <div>
                            <div class="pc-name">
                                {move || t(locale.get(), "projects.example")}
                                <span class="pc-tag">{move || t(locale.get(), "projects.example_tag")}</span>
                            </div>
                            <div class="pc-meta">{move || tf(locale.get(), "projects.sessions_n", &[("n", &demo_count.get().to_string())])}</div>
                        </div>
                    </div>
                    {move || {
                        let loc = locale.get();
                        let list = projects.get();
                        if list.is_empty() && !creating.get() {
                            return view! {}.into_view();
                        }
                        list.into_iter().map(|p| {
                            let id_open = p.id.clone();
                            let id_del = p.id.clone();
                            let del = delete.clone();
                            let meta = tf(loc, "projects.sessions_n", &[("n", &p.session_count.to_string())]);
                            let active = p.running_count + p.needs_you_count;
                            let dot_class = if p.running_count > 0 { "running" } else { "ready" };
                            let when = format_relative_time(p.updated_at, loc);
                            view! {
                                <div class="proj-card" on:click=move |_| on_open.call(id_open.clone())>
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
                                    <button class="pc-del" title=t(loc, "projects.delete")
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            if web_sys::window().and_then(|w| w.confirm_with_message(&t(loc, "projects.delete_confirm")).ok()).unwrap_or(false) {
                                                del(id_del.clone());
                                            }
                                        }>"✕"</button>
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
                            <div class="proj-card proj-recent" data-testid="recent-session-card"
                                on:click=move |_| on_open_session.call((pid.clone(), sid.clone()))>
                                <div class="pc-main">
                                    <div class="pc-name-row">
                                        <div class="pc-name">{s.title.clone()}</div>
                                        <SessionStatusBadge status=status locale=locale />
                                    </div>
                                </div>
                            </div>
                        }
                    }).collect_view()}
                </div>
            </div>
        </div>
    }
}

/// Apply a transcript mutation to the right session: the live `items` view when
/// `fid` is the active session, otherwise the background cache keyed by `fid`.
/// This is what lets a second conversation stream while the user views another.
fn route_items(
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

#[component]
fn App() -> impl IntoView {
    let locale = create_rw_signal(Locale::detect_browser());
    provide_context(locale.read_only());

    let items = create_rw_signal::<Vec<ChatItem>>(vec![]);
    let input = create_rw_signal(String::new());
    let attachments = create_rw_signal::<Vec<ComposerAttachment>>(vec![]);
    let uploading = create_rw_signal(false);
    let drag_over = create_rw_signal(false);
    // Per-session streaming state. `running` is the set of session ids with an
    // in-flight turn; `transcripts` caches the live transcript of background
    // (non-active) sessions so switching to them shows streaming progress.
    let running = create_rw_signal::<HashSet<String>>(HashSet::new());
    let approval_pending = create_rw_signal::<HashSet<String>>(HashSet::new());
    let pending_turns = create_rw_signal::<HashMap<String, usize>>(HashMap::new());
    let transcripts = create_rw_signal::<HashMap<String, Vec<ChatItem>>>(HashMap::new());
    let busy = create_rw_signal(false);
    // Interrupting a running turn (esp. the python kernel) is not instant, so
    // keep track of the session whose Stop click is waiting for the backend.
    let stopping_session = create_rw_signal::<Option<String>>(None);
    let show_settings = create_rw_signal(false);
    let settings_section = create_rw_signal(String::from("general"));
    let skills_list = create_rw_signal(Vec::<SkillRow>::new());
    let skills_search = create_rw_signal(String::new());
    let model_form = create_rw_signal(None::<ModelForm>);
    let model_form_key = create_rw_signal(String::new());
    let model_form_msg = create_rw_signal(None::<(bool, String)>);
    let memory_view = create_rw_signal(None::<MemoryView>);
    let memory_selected = create_rw_signal(None::<String>);
    let memory_editor = create_rw_signal(String::new());
    let memory_msg = create_rw_signal(None::<(bool, String)>);
    let conns_view = create_rw_signal(None::<ConnView>);
    let connectors = create_rw_signal(None::<ConnectorsView>);
    let open_conn_key = create_rw_signal(None::<String>);
    let conn_form = create_rw_signal(None::<ConnForm>);
    let conn_test_msg = create_rw_signal(None::<(bool,String)>);
    // Gate the settings sub-form panes on whether a form is open — NOT on its
    // contents. A closure that reads the whole form signal re-runs on every
    // keystroke (each `on:input` calls `.update`), rebuilding the inputs and
    // dropping focus after each character (#62). A memo only notifies when the
    // Some/None state flips, so the inputs stay mounted while editing.
    let model_form_open = create_memo(move |_| model_form.get().is_some());
    let conn_form_open = create_memo(move |_| conn_form.get().is_some());
    // Same reason, one level deeper: the connection form swaps stdio/http fields
    // on `kind`; track just `kind` so editing command/url doesn't rebuild them.
    let conn_form_kind = create_memo(move |_| conn_form.get().map(|f| f.kind).unwrap_or_default());
    let settings = create_rw_signal(Settings::default());
    // Configured model profiles + the composer's bottom-right picker state.
    let models = create_rw_signal::<Vec<ModelProfile>>(vec![]);
    let model_menu_open = create_rw_signal(false);
    let settings_busy = create_rw_signal(false);
    let settings_message = create_rw_signal::<Option<(bool, String)>>(None);
    let status = create_rw_signal(String::new());
    // Set when a send fails because no API key is configured, so the status bar
    // can offer a one-click jump to Settings instead of a dead-end message.
    let needs_api_key = create_rw_signal(false);
    let refresh_models = move || spawn_local(async move {
        let v = invoke("list_models", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) { models.set(list); }
    });
    let demos = create_rw_signal::<Vec<DemoInfo>>(vec![]);
    let show_projects = create_rw_signal(true); // app lands on the Projects screen
    let demo_mode = create_rw_signal(false); // true = the synthetic "Example project" is open
    // Top-nav project switcher dropdown + Project Settings modal.
    let show_proj_menu = create_rw_signal(false);
    let proj_list = create_rw_signal::<Vec<ProjectSummary>>(vec![]);
    let show_proj_settings = create_rw_signal(false);
    let proj_settings = create_rw_signal(ProjectSettings::default());
    let proj_settings_busy = create_rw_signal(false);

    // Session history (left sidebar).
    let sessions = create_rw_signal::<Vec<SessionInfo>>(vec![]);
    let folders = create_rw_signal::<Vec<FolderInfo>>(vec![]);
    let collapsed_folders = create_rw_signal::<HashSet<String>>(HashSet::new());
    let drag_session = create_rw_signal::<Option<String>>(None);
    let drop_target = create_rw_signal::<Option<String>>(None);
    let active_session = create_rw_signal::<Option<String>>(None);
    refresh_sessions(sessions);
    refresh_folders(folders);

    // `busy` is "the active session is currently streaming" — derived from the
    // per-session `running` set so it stays correct when the user switches
    // conversations or a background turn finishes.
    create_effect(move |_| {
        let r = running.get();
        let b = active_session.get().map(|id| r.contains(&id)).unwrap_or(false);
        busy.set(b);
    });

    // Three-pane layout state (mirrors web-dist: sidebar / conversation / right pane).
    let show_sidebar = create_rw_signal(true);
    let show_right = create_rw_signal(false);
    let right_w = create_rw_signal(440.0_f64);
    let dragging = create_rw_signal(false);
    let drag_start_x = create_rw_signal(0.0_f64);
    let drag_start_w = create_rw_signal(0.0_f64);

    // Artifacts (right pane): tables + CSV detected in the transcript.
    let artifacts_all = create_memo(move |_| collect_artifacts(&items.get(), locale.get()));
    // File-backed artifacts are scraped from chat text, so a file that was
    // renamed or overwritten still lingers and 404s on click (#41). Ask the
    // backend which referenced files are gone and drop them from the list.
    let missing_paths = create_rw_signal(std::collections::HashSet::<String>::new());
    create_effect(move |_| {
        let paths: Vec<String> = artifacts_all.get().iter()
            .filter_map(|a| match &a.data { PreviewData::File { path, .. } => Some(path.clone()), _ => None })
            .collect();
        if paths.is_empty() { missing_paths.set(std::collections::HashSet::new()); return; }
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "paths": paths })).unwrap();
            let v = invoke("missing_files", arg).await;
            if let Ok(m) = serde_wasm_bindgen::from_value::<Vec<String>>(v) {
                missing_paths.set(m.into_iter().collect());
            }
        });
    });
    let artifacts = create_memo(move |_| {
        let miss = missing_paths.get();
        artifacts_all.get().into_iter()
            .filter(|a| match &a.data { PreviewData::File { path, .. } => !miss.contains(path), _ => true })
            .collect::<Vec<_>>()
    });
    let sel_artifact = create_rw_signal(0usize);
    let modal_artifact = create_rw_signal(None::<(String, String, String)>); // (path, name, kind)
    let artifact_menu = create_rw_signal(None::<(usize, i32, i32)>); // (open tile idx, cursor x, y) — fixed-positioned so the `.rp-tiles` overflow doesn't clip it
    let right_tab = create_rw_signal(RightTab::Artifacts);
    let show_files = create_rw_signal(false);
    let file_query = create_rw_signal(String::new());
    let file_cwd = create_rw_signal(".".to_string());
    let file_entries = create_rw_signal::<Vec<DirEntry>>(vec![]);
    let open_file = create_rw_signal::<Option<(String, String)>>(None);
    let project_info = create_rw_signal::<Option<ProjectInfo>>(None);
    let show_capabilities = create_rw_signal(false);
    let skill_filter_tag = create_rw_signal(String::new());
    let caps = create_rw_signal::<Option<Capabilities>>(None);
    let bootstrap = create_rw_signal::<Option<BootstrapStatus>>(None);
    let show_onboarding = create_rw_signal(false);
    let onboard_step = create_rw_signal(0usize);

    let on_artifact_select = Callback::new(move |idx: usize| {
        let arts = artifacts.get();
        if let Some(a) = arts.get(idx) {
            if let PreviewData::File { path, kind } = &a.data {
                // Images, PDFs and CSVs open in the modal viewer, not the pane.
                if opens_in_modal(kind) {
                    modal_artifact.set(Some((path.clone(), a.name.clone(), kind.clone())));
                    return;
                }
                show_right.set(true);
                right_tab.set(RightTab::File);
                open_file.set(Some((path.clone(), kind.clone())));
            } else {
                show_right.set(true);
                right_tab.set(RightTab::Artifacts);
                sel_artifact.set(idx);
            }
        }
    });

    let on_file_link = Callback::new(move |(path, kind): (String, String)| {
        show_right.set(true);
        right_tab.set(RightTab::File);
        open_file.set(Some((path, kind)));
    });

    // `@`-mention: type `@` in the composer to pick a session file. Picking one
    // reuses the existing attachment pipeline (chip + `Uploaded files:` on send).
    let mention_active = create_rw_signal(false);
    let mention_query = create_rw_signal(String::new());
    let mention_index = create_rw_signal(0usize);
    // (name, path, in_view) — in-view file first, then file-backed artifacts, deduped.
    let mention_matches = create_memo(move |_| {
        let q = mention_query.get().to_lowercase();
        let mut out: Vec<(String, String, bool)> = Vec::new();
        if let Some((path, _)) = open_file.get() {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            if name.to_lowercase().contains(&q) {
                out.push((name, path, true));
            }
        }
        for a in artifacts.get() {
            if let PreviewData::File { path, .. } = &a.data {
                if out.iter().any(|(_, p, _)| p == path) {
                    continue;
                }
                if a.name.to_lowercase().contains(&q) {
                    out.push((a.name.clone(), path.clone(), false));
                }
            }
        }
        out
    });
    let mention_show = create_memo(move |_| mention_active.get() && !mention_matches.get().is_empty());
    let select_mention = Callback::new(move |i: usize| {
        let Some((name, path, _)) = mention_matches.get().get(i).cloned() else { return; };
        input.update(|s| {
            if let Some((at, _)) = active_mention(s) {
                s.truncate(at);
            }
        });
        attachments.update(|items| {
            if items.iter().any(|a| matches!(a, ComposerAttachment::Ready { path: p, .. } if *p == path)) {
                return;
            }
            let key = composer_attachment_key(&name, items.len());
            items.push(ComposerAttachment::Ready { key, name, path });
        });
        mention_active.set(false);
        focus_composer();
    });

    spawn_local(async move {
        let v = invoke("get_project_info", JsValue::UNDEFINED).await;
        if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
            project_info.set(Some(p));
        }
        let v = invoke("get_settings", JsValue::UNDEFINED).await;
        if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
            let loc = Locale::from_code(&cfg.locale);
            locale.set(loc);
            set_document_lang(loc);
        }
        let v = invoke("get_onboarding_state", JsValue::UNDEFINED).await;
        if let Ok(s) = serde_wasm_bindgen::from_value::<OnboardingState>(v) {
            if s.show { show_onboarding.set(true); }
        }
        let b = invoke("get_bootstrap_status", JsValue::UNDEFINED).await;
        if let Ok(st) = serde_wasm_bindgen::from_value::<BootstrapStatus>(b) {
            bootstrap.set(Some(st));
        }
        refresh_models();
    });

    create_effect(move |_| {
        attach_chat_autoscroll();
    });
    create_effect(move |_| {
        let _ = items.get();
        schedule_chat_follow();
    });

    // Wire the agent event stream once. Every event carries the session frame
    // id; route transcript mutations to `items` (active session) or the
    // `transcripts` cache (background session) so parallel conversations don't
    // interleave in the view.
    let items_cb = items;
    let active_cb = active_session;
    let transcripts_cb = transcripts;
    let running_cb = running;
    let pending_cb = pending_turns;
    let approval_cb = approval_pending;
    let status_cb = status;
    let locale_cb = locale;
    let models_cb = models;
    // Streaming deltas are buffered and flushed on a timer (~20 fps) instead of
    // being applied per token; see the "Streaming delta batching" block above.
    let delta_buf: DeltaBuf = Rc::new(RefCell::new(HashMap::new()));
    let flush_scheduled = Rc::new(Cell::new(false));
    let cb_buf = delta_buf.clone();
    let cb_scheduled = flush_scheduled.clone();
    let cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let ev: AgentEvent = match serde_wasm_bindgen::from_value(payload) {
            Ok(e) => e,
            Err(err) => {
                web_sys::console::log_1(&format!("agent event decode error: {err:?}").into());
                return;
            }
        };
        // Ordered, non-delta events (tool calls, results, done…) must observe
        // every delta buffered before them, so drain the buffer first.
        let flush_now = || flush_delta_buf(&cb_buf, active_cb, items_cb, transcripts_cb, models_cb);
        let queue = |fid: String, d: PendingDelta| {
            queue_delta(&cb_buf, fid, d);
            schedule_delta_flush(&cb_buf, &cb_scheduled, active_cb, items_cb, transcripts_cb, models_cb);
        };
        match ev {
            AgentEvent::User { frame_id, text } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    let model = active_model_label(&models_cb.get());
                    start_user_turn(v, text, model);
                })
            }
            AgentEvent::Text { frame_id, delta } => queue(frame_id, PendingDelta::Text(delta)),
            AgentEvent::Reasoning { frame_id, delta } => queue(frame_id, PendingDelta::Reasoning(delta)),
            AgentEvent::ToolCall { frame_id, name, preview } => { flush_now(); route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                let idx = trailing_queue_start(v);
                v.insert(idx, ChatItem::Tool { name, ok: None, input: preview, output: String::new() });
            }) }
            AgentEvent::ToolResult { frame_id, name, ok, content } => { flush_now(); route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                let queue_start = trailing_queue_start(v);
                let idx = v[..queue_start].iter().rposition(|c| matches!(c, ChatItem::Tool { name: n, ok: None, .. } if n == &name));
                if let Some(i) = idx {
                    if let ChatItem::Tool { ok: o, output, .. } = &mut v[i] {
                        *o = Some(ok);
                        *output = content.clone();
                    }
                } else {
                    v.insert(queue_start, ChatItem::Tool { name: name.clone(), ok: Some(ok), input: String::new(), output: content.clone() });
                }
                if name == "attempt_completion" && ok {
                    promote_assistant_text(v, &content);
                }
            }) }
            AgentEvent::Usage { frame_id, input, output, ctx_tokens, max_context, .. } => {
                // Status bar reflects only the active session's usage.
                if active_cb.get().as_deref() == Some(&frame_id) {
                    let pct = if max_context > 0 { ctx_tokens * 100 / max_context } else { 0 };
                    let loc = locale_cb.get();
                    status_cb.set(tf(loc, "status.usage", &[
                        ("in", &format!("{:.1}", input as f64 / 1000.0)),
                        ("out", &format!("{:.1}", output as f64 / 1000.0)),
                        ("pct", &pct.to_string()),
                    ]));
                }
            }
            AgentEvent::Compaction { frame_id, before, after, .. } => {
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(tf(locale_cb.get(), "status.compact", &[
                        ("before", &before.to_string()),
                        ("after", &after.to_string()),
                    ]));
                }
            }
            AgentEvent::Stdout { frame_id, chunk } => queue(frame_id, PendingDelta::Stdout(chunk)),
            AgentEvent::Done { frame_id } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, strip_approval_pending);
                approval_cb.update(|s| { s.remove(&frame_id); });
                clear_running_if_idle(pending_cb, running_cb, &frame_id);
                if stopping_session.get().as_deref() == Some(&frame_id) {
                    stopping_session.set(None);
                }
                refresh_sessions(sessions);
            }
            AgentEvent::Error { frame_id, message } => {
                flush_now();
                let model = active_model_label(&models_cb.get());
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| {
                    strip_approval_pending(v);
                    v.push(ChatItem::Assistant { text: format!("Error: {message}"), model });
                });
                approval_cb.update(|s| { s.remove(&frame_id); });
                clear_running_if_idle(pending_cb, running_cb, &frame_id);
                if stopping_session.get().as_deref() == Some(&frame_id) {
                    stopping_session.set(None);
                }
            }
            AgentEvent::Review { frame_id, markdown } => {
                flush_now();
                route_items(active_cb, items_cb, transcripts_cb, &frame_id, |v| v.push(ChatItem::Review(markdown)));
                if active_cb.get().as_deref() == Some(&frame_id) {
                    status_cb.set(t(locale_cb.get(), "status.review_done"));
                }
            }
            AgentEvent::Diff { .. } => {}
        }
    }) as Box<dyn FnMut(JsValue)>);
    let agent_js = cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(cb);
    // wasm-bindgen only runs an async extern's JS body when the returned
    // future is polled, so we must await `listen` (not fire-and-forget it).
    spawn_local(async move { let _ = listen("agent", &agent_js).await; });

    // Confirm handler: render an inline approval card in the session thread
    // (not a global modal — see README inline tool-approval card).
    let confirm_active = active_session;
    let confirm_items = items;
    let confirm_transcripts = transcripts;
    let confirm_pending = approval_pending;
    let confirm_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        if let Ok(v) = serde_wasm_bindgen::from_value::<serde_json::Value>(payload) {
            let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
            let fid = v.get("frame_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
            if msg.is_empty() || fid.is_empty() {
                return;
            }
            let mut tool = v.get("tool").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let mut preview = v.get("preview").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if tool.is_empty() {
                if let Some(rest) = msg.strip_prefix("Run tool '") {
                    if let Some((t, _)) = rest.split_once("'?") {
                        tool = t.to_string();
                    }
                } else if msg.starts_with("Dangerous command detected") {
                    tool = "shell".into();
                }
            }
            route_items(confirm_active, confirm_items, confirm_transcripts, &fid, |v| {
                strip_approval_pending(v);
                if preview.is_empty() {
                    preview = last_tool_input(v, &tool);
                }
                v.push(ChatItem::ApprovalPending {
                    tool,
                    preview,
                    message: msg,
                });
            });
            confirm_pending.update(|s| {
                s.insert(fid);
            });
            force_chat_bottom();
        }
    }) as Box<dyn FnMut(JsValue)>);
    let confirm_js = confirm_cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(confirm_cb);
    spawn_local(async move { let _ = listen("confirm-request", &confirm_js).await; });

    let stop = move |_| {
        if stopping_session.get().is_some() { return; }
        // Stop only the active session's turn; background conversations keep running.
        let sid = active_session.get();
        stopping_session.set(sid.clone());
        spawn_local(async move {
            let arg = to_value(&tauri_args::stop_agent(&sid)).unwrap();
            let _ = invoke("stop_agent", arg).await;
        });
    };

    let send = move || {
        let text = input.get();
        let paths = attachment_paths(&attachments.get());
        let message = message_with_attachments(&text, &paths);
        if message.trim().is_empty() || uploading.get() { return; }
        let active = active_session.get();
        if active.as_ref().is_some_and(|id| running.get().contains(id)) {
            items.update(|v| v.push(ChatItem::QueuedUser(message.clone())));
            force_chat_bottom();
        }
        needs_api_key.set(false);
        input.set(String::new());
        attachments.set(vec![]);
        let locale = locale;
        let status = status;
        let running = running;
        let active_session = active_session;
        let items = items;
        let transcripts = transcripts;
        let sessions = sessions;
        let stopping_session = stopping_session;
        let pending_turns = pending_turns;
        spawn_local(async move {
            // Resolve the target session: use the active one, or create a fresh
            // frame up front so streamed events can be routed before the first delta.
            let id = match active.clone() {
                Some(id) => id,
                None => {
                    let v = invoke("new_session", JsValue::UNDEFINED).await;
                    match v.as_string() {
                        Some(s) => s,
                        None => {
                            // Bridge returned no id (e.g. legacy mock); bail without
                            // flipping running so the user can retry.
                            let loc = locale.get();
                            status.set(t(loc, "status.send_failed").into());
                            return;
                        }
                    }
                }
            };
            active_session.set(Some(id.clone()));
            begin_pending_turn(pending_turns, running, &id);
            let arg = to_value(&SendMessageArgs { session_id: Some(id.clone()), message }).unwrap();
            match invoke_checked("send_message", arg).await {
                Ok(_) => {
                    // send_message is awaited for the whole turn, so it resolves only
                    // once the turn has finished AND been persisted. Clear `running`
                    // here rather than trusting the separate `Done` broadcast — a
                    // dropped broadcast used to pin the session on "运行中" until an
                    // app restart (#34).
                    finish_pending_turn(pending_turns, running, &id);
                    if stopping_session.get().as_deref() == Some(&id) {
                        stopping_session.set(None);
                    }
                    // If the live view desynced (a tool row left unresolved by a
                    // missed event), reconcile it from the authoritative DB so the
                    // completed result shows without a restart. Healthy turns keep
                    // their richer streamed view (incl. tool inputs) untouched.
                    let is_active = active_session.get().as_deref() == Some(&id);
                    let stranded = if is_active {
                        items.with(|v| v.iter().any(|c| matches!(c, ChatItem::Tool { ok: None, .. })))
                    } else {
                        transcripts.with(|m| m.get(&id).map_or(false, |v| v.iter().any(|c| matches!(c, ChatItem::Tool { ok: None, .. }))))
                    };
                    if stranded {
                        let v = invoke("load_session", to_value(&serde_json::json!({ "id": id })).unwrap()).await;
                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<LoadedItem>>(v) {
                            let chats: Vec<ChatItem> = list.into_iter().map(LoadedItem::into_chat).collect();
                            transcripts.update(|m| { m.insert(id.clone(), chats.clone()); });
                            if active_session.get().as_deref() == Some(&id) {
                                items.set(chats);
                                force_chat_bottom();
                            }
                        }
                    }
                    refresh_sessions(sessions);
                }
                Err(err) => {
                    let loc = locale.get();
                    let raw = js_error_text(err);
                    if raw.contains(NO_API_KEY_MARK) { needs_api_key.set(true); }
                    status.set(tf(loc, "status.send_failed", &[("msg", &localize_backend(loc, &raw))]));
                    finish_pending_turn(pending_turns, running, &id);
                    if stopping_session.get().as_deref() == Some(&id) {
                        stopping_session.set(None);
                    }
                }
            }
        });
    };

    let on_send = move |ev: web_sys::KeyboardEvent| {
        if mention_show.get() {
            match ev.key().as_str() {
                "ArrowDown" => {
                    ev.prevent_default();
                    let n = mention_matches.get().len().max(1);
                    mention_index.update(|i| *i = (*i + 1) % n);
                }
                "ArrowUp" => {
                    ev.prevent_default();
                    let n = mention_matches.get().len().max(1);
                    mention_index.update(|i| *i = (*i + n - 1) % n);
                }
                "Enter" | "Tab" => { ev.prevent_default(); select_mention.call(mention_index.get()); }
                "Escape" => { ev.prevent_default(); mention_active.set(false); }
                _ => {}
            }
            return;
        }
        if ev.key() == "Enter" && !ev.shift_key() { ev.prevent_default(); send(); }
    };

    let edit_message = move |ui_index: usize| {
        if busy.get() {
            return;
        }
        let list = items.get();
        let Some(user_idx) = user_message_index(&list, ui_index) else {
            return;
        };
        let Some(ChatItem::User(text)) = list.get(ui_index) else {
            return;
        };
        let draft = composer_text_from_user_message(text);
        items.set(list.into_iter().take(ui_index).collect());
        input.set(draft);
        focus_composer();
        let sid = active_session.get();
        spawn_local(async move {
            let arg = to_value(&tauri_args::rewind_session(&sid, user_idx)).unwrap();
            let _ = invoke("rewind_session", arg).await;
        });
    };

    let pick_files = move |_| {
        if uploading.get() {
            return;
        }
        let Some(window) = web_sys::window() else { return; };
        let Some(doc) = window.document() else { return; };
        let Some(el) = doc.get_element_by_id("composer-file-input") else { return; };
        let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|e| e.click());
    };

    let on_files_selected = move |_ev: web_sys::Event| {
        if uploading.get() {
            return;
        }
        upload_from_input(attachments, uploading, "composer-file-input");
    };

    let on_drag_over = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        if !uploading.get() {
            drag_over.set(true);
        }
    };

    let on_drag_leave = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        drag_over.set(false);
    };

    let on_drop = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        drag_over.set(false);
        if uploading.get() {
            return;
        }
        if let Some(dt) = ev.data_transfer() {
            if let Some(files) = dt.files() {
                queue_uploads(attachments, uploading, files.into());
            }
        }
    };

    let composer_blocked = move || uploading.get();

    let check_updates = move |_| {
        if settings_busy.get() { return; }
        settings_busy.set(true);
        settings_message.set(Some((true, t(locale.get(), "status.checking_updates").into())));
        let msg = settings_message;
        let busy = settings_busy;
        let loc = locale;
        spawn_local(async move {
            match invoke_checked("check_for_updates", JsValue::UNDEFINED).await {
                Ok(v) => {
                    let text = v.as_string().unwrap_or_else(|| t(loc.get(), "status.update_check_complete").into());
                    msg.set(Some((true, localize_backend(loc.get(), &text))));
                }
                Err(err) => msg.set(Some((false, localize_backend(loc.get(), &js_error_text(err))))),
            }
            busy.set(false);
        });
    };

    let refresh_skills = move || {
        spawn_local(async move {
            let v = invoke("list_skills", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) {
                skills_list.set(rows);
            }
        });
    };

    let refresh_conns = move || {
        spawn_local(async move {
            let v = invoke("list_mcp_connections", JsValue::UNDEFINED).await;
            if let Ok(view) = serde_wasm_bindgen::from_value::<ConnView>(v) { conns_view.set(Some(view)); }
            let c = invoke("list_connectors", JsValue::UNDEFINED).await;
            if let Ok(view) = serde_wasm_bindgen::from_value::<ConnectorsView>(c) { connectors.set(Some(view)); }
        });
    };

    let refresh_memory = move || {
        spawn_local(async move {
            let v = invoke("get_memory_view", JsValue::UNDEFINED).await;
            if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                memory_view.set(Some(view));
            }
        });
    };

    let load_memory_file = move |name: String| {
        memory_selected.set(Some(name.clone()));
        memory_msg.set(None);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "name": name })).unwrap();
            let v = invoke("read_memory_file", arg).await;
            memory_editor.set(v.as_string().unwrap_or_default());
        });
    };

    let close_settings_subpage = move || {
        model_form.set(None);
        model_form_key.set(String::new());
        model_form_msg.set(None);
        conn_form.set(None);
        open_conn_key.set(None);
        conn_test_msg.set(None);
        memory_selected.set(None);
        memory_editor.set(String::new());
        memory_msg.set(None);
    };

    let go_settings_section = move |sec: &str| {
        close_settings_subpage();
        settings_section.set(sec.into());
        match sec {
            "models" => refresh_models(),
            "memory" => refresh_memory(),
            "skills" => refresh_skills(),
            "connections" => refresh_conns(),
            _ => {}
        }
    };

    let open_settings_fn = move |section: Option<String>| {
        show_settings.set(true);
        settings_message.set(None);
        needs_api_key.set(false);
        close_settings_subpage();
        if let Some(sec) = section {
            settings_section.set(sec);
        }
        let s = settings;
        let msg = settings_message;
        let loc = locale;
        refresh_skills();
        refresh_conns();
        refresh_models();
        refresh_memory();
        spawn_local(async move {
            let v = invoke("get_settings", JsValue::UNDEFINED).await;
            if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                let cfg = normalized_settings(cfg);
                let l = Locale::from_code(&cfg.locale);
                loc.set(l);
                set_document_lang(l);
                s.set(cfg);
            } else {
                msg.set(Some((false, t(loc.get(), "status.failed_load_settings").into())));
            }
        });
    };
    let open_settings = move |_| open_settings_fn(None);

    let save_settings = move |_| {
        if settings_busy.get() { return; }
        let mut cfg = normalized_settings(settings.get());
        cfg.locale = locale.get().code().into();
        let s = settings;
        let show = show_settings;
        let busy = settings_busy;
        let msg = settings_message;
        let status_msg = status;
        let loc = locale;
        busy.set(true);
        let saving = t(loc.get(), "status.saving_settings").to_string();
        msg.set(Some((true, saving.clone())));
        status_msg.set(saving);
        spawn_local(async move {
            let settings_result = invoke_checked(
                "set_settings",
                to_value(&serde_json::json!({ "settings": cfg.clone() })).unwrap(),
            ).await;
            if let Err(err) = settings_result {
                let l = loc.get();
                let text = tf(l, "status.save_failed", &[("msg", &localize_backend(l, &js_error_text(err)))]);
                msg.set(Some((false, text.clone())));
                status_msg.set(text);
                busy.set(false);
                return;
            }
            busy.set(false);
            show.set(false);
            status_msg.set(t(loc.get(), "status.settings_saved").into());
            s.set(cfg);
        });
    };

    let save_model_form = move |_| {
        if settings_busy.get() { return; }
        let Some(form) = model_form.get() else { return; };
        let loc = locale.get();
        let key_raw = model_form_key.get();
        let key = if is_stored_key_placeholder(&key_raw, loc) { String::new() } else { key_raw };
        let has_key = form.id.as_ref()
            .and_then(|id| models.get().iter().find(|m| &m.id == id).map(|m| m.has_api_key))
            .unwrap_or(false);
        let cfg = model_form_to_settings(&form, has_key && key.is_empty());
        if let Some(err_key) = settings_required_error_key(&cfg, &key) {
            let err = t(loc, err_key);
            let text = tf(loc, "status.save_failed", &[("msg", &err)]);
            model_form_msg.set(Some((false, text)));
            return;
        }
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.saving_settings").into())));
        let profile = serde_json::json!({
            "id": form.id.clone().unwrap_or_default(),
            "label": form.label.trim(),
            "provider": provider_value(&form.provider),
            "api_url": form.api_url.trim(),
            "model": form.model.trim(),
            "max_tokens": form.max_tokens,
            "reasoning_effort": form.reasoning_effort.trim(),
        });
        let key_arg = if key.is_empty() { None } else { Some(key) };
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "profile": profile, "key": key_arg })).unwrap();
            match invoke_checked("save_model", arg).await {
                Ok(v) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                        models.set(list);
                    }
                    let v = invoke("get_settings", JsValue::UNDEFINED).await;
                    if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                        settings.set(normalized_settings(cfg));
                    }
                    model_form.set(None);
                    model_form_key.set(String::new());
                    model_form_msg.set(Some((true, t(loc, "status.settings_saved").into())));
                }
                Err(err) => {
                    model_form_msg.set(Some((false, localize_backend(loc, &js_error_text(err)))));
                }
            }
            settings_busy.set(false);
        });
    };

    let validate_model_form = move |_| {
        if settings_busy.get() { return; }
        let Some(form) = model_form.get() else { return; };
        let loc = locale.get();
        let key_raw = model_form_key.get();
        let key = if is_stored_key_placeholder(&key_raw, loc) { String::new() } else { key_raw };
        let has_key = models.get().iter().find(|m| Some(m.id.as_str()) == form.id.as_deref()).map(|m| m.has_api_key).unwrap_or(false);
        let cfg = model_form_to_settings(&form, has_key);
        if let Some(err_key) = settings_required_error_key(&cfg, &key) {
            let err = t(loc, err_key);
            model_form_msg.set(Some((false, tf(loc, "status.validation_failed", &[("msg", &err)]))));
            return;
        }
        settings_busy.set(true);
        model_form_msg.set(Some((true, t(loc, "status.validating").into())));
        spawn_local(async move {
            let res = invoke_timeout(
                "validate_settings",
                to_value(&serde_json::json!({ "settings": cfg, "key": key })).unwrap(),
                35_000,
            ).await;
            match res {
                Ok(v) => {
                    let raw = v.as_string().unwrap_or_else(|| t(loc, "status.validation_succeeded").into());
                    model_form_msg.set(Some((true, localize_backend(loc, &raw))));
                }
                Err(err) => {
                    model_form_msg.set(Some((false, tf(loc, "status.validation_failed", &[("msg", &localize_backend(loc, &js_error_text(err)))]))));
                }
            }
            settings_busy.set(false);
        });
    };

    let new_session = move |_| {
        demo_mode.set(false); // starting a fresh chat leaves the demo view
        // Stash the current transcript under its id so a running turn keeps
        // streaming into the cache, then create a fresh frame and show it.
        // We do NOT cancel any running turn — parallel conversations keep going.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| { m.insert(old, items.get()); });
        }
        attachments.set(vec![]);
        sel_artifact.set(0);
        open_file.set(None);
        right_tab.set(RightTab::Artifacts);
        spawn_local(async move {
            let v = invoke("new_session", JsValue::UNDEFINED).await;
            // Guard the malformed-response case: a `None` id would blank the active
            // session and strand the user on an empty, unusable view (#15). The old
            // transcript is already stashed above, so bailing keeps it reachable.
            let Some(id) = v.as_string() else {
                status.set(t(locale.get(), "status.send_failed").into());
                return;
            };
            active_session.set(Some(id));
            items.set(vec![]);
            refresh_sessions(sessions);
        });
    };

    let start_env_setup = {
        let items = items;
        let running = running;
        let status = status;
        let locale = locale;
        let show_capabilities = show_capabilities;
        let active_session = active_session;
        let sel_artifact = sel_artifact;
        let open_file = open_file;
        let right_tab = right_tab;
        let sessions = sessions;
        let models = models;
        move |_| {
            if busy.get() { return; }
            show_capabilities.set(false);
            attachments.set(vec![]);
            sel_artifact.set(0);
            open_file.set(None);
            right_tab.set(RightTab::Artifacts);
            let text: String = t(locale.get(), "caps.env_setup_prompt").into();
            let turn_model = active_model_label(&models.get());
            items.set(vec![
                ChatItem::User(text.clone()),
                ChatItem::Assistant { text: String::new(), model: turn_model },
            ]);
            force_chat_bottom();
            spawn_local(async move {
                // Fresh frame for the setup turn; route events to it.
                let v = invoke("new_session", JsValue::UNDEFINED).await;
                let id = v.as_string().unwrap_or_default();
                if id.is_empty() {
                    let loc = locale.get();
                    status.set(t(loc, "status.send_failed").into());
                    return;
                }
                active_session.set(Some(id.clone()));
                running.update(|r| { r.insert(id.clone()); });
                refresh_sessions(sessions);
                let arg = to_value(&SendMessageArgs { session_id: Some(id.clone()), message: text }).unwrap();
                match invoke_checked("send_message", arg).await {
                    // The awaited command resolving is the reliable turn-complete
                    // signal; clear `running` here so a dropped `Done` broadcast
                    // can't pin the session on "运行中" (#34).
                    Ok(_) => { running.update(|r| { r.remove(&id); }); refresh_sessions(sessions); }
                    Err(err) => {
                        let loc = locale.get();
                        let raw = js_error_text(err);
                        if raw.contains(NO_API_KEY_MARK) { needs_api_key.set(true); }
                        status.set(tf(loc, "status.send_failed", &[("msg", &localize_backend(loc, &raw))]));
                        running.update(|r| { r.clear(); });
                    }
                }
            });
        }
    };

    let load_session = Callback::new(move |id: String| {
        attachments.set(vec![]);
        sel_artifact.set(0);
        open_file.set(None);
        right_tab.set(RightTab::Artifacts);
        // Stash the transcript we're leaving under its id.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| { m.insert(old, items.get()); });
        }
        let is_running = running.get().contains(&id);
        active_session.set(Some(id.clone()));
        if is_running {
            // Mid-stream: render the cached transcript (live), no DB load needed.
            items.set(transcripts.with(|m| m.get(&id).cloned().unwrap_or_default()));
            force_chat_bottom();
            return;
        }
        // Idle session: load from DB and overwrite any stale cache entry.
        spawn_local(async move {
            let v = invoke("load_session", to_value(&serde_json::json!({ "id": id })).unwrap()).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<LoadedItem>>(v) {
                let chats: Vec<ChatItem> = list.into_iter().map(LoadedItem::into_chat).collect();
                transcripts.update(|m| { m.insert(id.clone(), chats.clone()); });
                // Only repaint the view if we're still on this session — a rapid
                // switch could have moved on while the load was in flight, and an
                // unguarded set would clobber the newer view with stale rows (#53).
                if active_session.get().as_deref() == Some(&id) {
                    items.set(chats);
                    force_chat_bottom();
                }
            }
        });
    });

    let load_demo = move |info: DemoInfo| {
        let id = info.id.clone();
        let items = items;
        // Demos are read-only transcripts; they don't stream, so we don't touch
        // `running`. We do stash the current chat so returning to it is possible.
        if let Some(old) = active_session.get() {
            transcripts.update(|m| { m.insert(old, items.get()); });
        }
        attachments.set(vec![]);
        sel_artifact.set(0);
        open_file.set(None);
        right_tab.set(RightTab::Artifacts);
        active_session.set(None);
        spawn_local(async move {
            // Fresh session so the demo doesn't mix into a real conversation.
            let _ = invoke("new_session", JsValue::UNDEFINED).await;
            let v = invoke("load_demo", to_value(&serde_json::json!({ "id": id })).unwrap()).await;
            if let Ok(demo) = serde_wasm_bindgen::from_value::<Demo>(v) {
                let mut view = vec![ChatItem::User(demo.request.clone())];
                if let Some(t) = &demo.thinking {
                    if !t.is_empty() { view.push(ChatItem::Reasoning(t.clone())); }
                }
                view.push(ChatItem::Assistant { text: demo.response.clone(), model: None });
                items.set(view);
                force_chat_bottom();
                status_cb.set(tf(locale.get(), "status.demo", &[("title", &demo.title)]));
            }
        });
    };

    let respond_confirm = {
        let active_session = active_session;
        let items = items;
        let transcripts = transcripts;
        let approval_pending = approval_pending;
        Callback::new(move |(sid, approved): (String, bool)| {
            route_items(active_session, items, transcripts, &sid, strip_approval_pending);
            approval_pending.update(|s| {
                s.remove(&sid);
            });
            let arg = to_value(&tauri_args::confirm_response(&sid, approved)).unwrap();
            spawn_local(async move { let _ = invoke("confirm_response", arg).await; });
        })
    };

    let on_resize_start = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        dragging.set(true);
        drag_start_x.set(ev.client_x() as f64);
        drag_start_w.set(right_w.get());
    };
    let on_resize_move = move |ev: web_sys::MouseEvent| {
        if dragging.get() {
            let dx = drag_start_x.get() - ev.client_x() as f64;
            right_w.set((drag_start_w.get() + dx).clamp(320.0, 900.0));
        }
    };

    let open_files = move |_| {
        file_query.set(String::new());
        show_files.set(true);
        refresh_dir(file_cwd, file_entries);
    };

    let open_capabilities = move |_| {
        show_capabilities.set(true);
        refresh_capabilities(caps);
    };

    let save_skill_tags = Callback::new(move |(name, raw): (String, String)| {
        let tags = split_tags(&raw);
        spawn_local(async move {
            let _ = invoke_checked("set_skill_tags", to_value(&serde_json::json!({ "name": name, "tags": tags })).unwrap()).await;
            refresh_skills();
        });
    });

    let set_visible_skills_enabled = Callback::new(move |enabled: bool| {
        let tag = skill_filter_tag.get();
        let query = skills_search.get();
        let names = skills_list.get().into_iter()
            .filter(|s| skill_matches_filter(s, &tag, &query))
            .map(|s| s.name)
            .collect::<Vec<_>>();
        if names.is_empty() {
            return;
        }
        let names_for_update = names.clone();
        skills_list.update(|list| {
            for skill in list {
                if names_for_update.contains(&skill.name) {
                    skill.enabled = enabled;
                }
            }
        });
        spawn_local(async move {
            let _ = invoke_checked("set_skills_enabled", to_value(&serde_json::json!({ "names": names, "enabled": enabled })).unwrap()).await;
            refresh_skills();
        });
    });

    let dismiss_onboarding = Callback::new(move |_| {
        show_onboarding.set(false);
        spawn_local(async move { let _ = invoke("dismiss_onboarding", JsValue::UNDEFINED).await; });
    });
    let dismiss_onboard = move |_| dismiss_onboarding.call(());

    let ctx_menu = create_rw_signal::<Option<CtxMenu>>(None);
    let rename_session_target = create_rw_signal::<Option<(String, String)>>(None);
    let rename_session_input = create_rw_signal(String::new());
    let folder_modal = create_rw_signal::<Option<FolderModal>>(None);
    let folder_modal_input = create_rw_signal(String::new());
    let ui_confirm = create_rw_signal::<Option<UiConfirm>>(None);
    let compose_menu_open = create_rw_signal(false);
    let compute_menu_open = create_rw_signal(false);
    let ssh_hosts = create_rw_signal::<Vec<SshHost>>(vec![]);
    let show_add_host = create_rw_signal(false);
    let config_aliases = create_rw_signal::<Vec<String>>(vec![]);
    let host_alias = create_rw_signal(String::new());
    let host_user = create_rw_signal(String::new());
    let host_port = create_rw_signal(String::new());
    let host_identity = create_rw_signal(String::new());
    let host_notes = create_rw_signal(String::new());

    // Load persisted hosts once at startup.
    {
        let ssh_hosts = ssh_hosts;
        spawn_local(async move {
            let v = invoke("list_ssh_hosts", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) {
                ssh_hosts.set(list);
            }
        });
    }
    let open_session = load_session.clone();
    let on_ctx_pick = {
        let open_session = open_session.clone();
        let sessions = sessions;
        let rename_session_target = rename_session_target;
        let rename_session_input = rename_session_input;
        let folder_modal = folder_modal;
        let folder_modal_input = folder_modal_input;
        let ui_confirm = ui_confirm;
        Callback::new(move |(action, payload): (String, String)| {
            if let Some(act) = context_menu::folder_action(&action, &payload) {
                match act {
                    context_menu::FolderAction::Rename { id, name } => {
                        folder_modal_input.set(name);
                        folder_modal.set(Some(FolderModal::Rename(id)));
                    }
                    context_menu::FolderAction::Delete(id) => {
                        ui_confirm.set(Some(UiConfirm::DeleteFolder(id)));
                    }
                }
                return;
            }
            if let Some(act) = context_menu::session_action(&action, &payload) {
                match act {
                    context_menu::SessionAction::Open(id) => open_session.call(id),
                    context_menu::SessionAction::Rename { id, title } => {
                        rename_session_input.set(title.clone());
                        rename_session_target.set(Some((id, title)));
                    }
                    context_menu::SessionAction::Move { id, folder_id } => {
                        let sessions = sessions;
                        spawn_local(async move {
                            let arg = to_value(&serde_json::json!({ "id": id, "folderId": folder_id })).unwrap();
                            if invoke_checked("move_session", arg).await.is_ok() {
                                refresh_sessions(sessions);
                            }
                        });
                    }
                    context_menu::SessionAction::Delete(id) => {
                        ui_confirm.set(Some(UiConfirm::DeleteSession(id)));
                    }
                }
            }
            context_menu::run_action(&action, &payload, copy_text);
        })
    };
    let on_context_menu = move |ev: web_sys::MouseEvent| {
        let loc = locale.get();
        if let Some(menu) = context_menu::build(&ev, loc) {
            if !menu.items.is_empty() {
                ev.prevent_default();
                ctx_menu.set(Some(menu));
                return;
            }
        }
        ctx_menu.set(None);
        if !context_menu::dev_mode() {
            ev.prevent_default();
        }
    };

    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else { return };
        if ev.key() != "Escape" || ev.default_prevented() || ev.is_composing() {
            return;
        }

        if active_session
            .get()
            .is_some_and(|_sid| items.get().iter().any(|i| matches!(i, ChatItem::ApprovalPending { .. })))
        {
            ev.prevent_default();
            if let Some(sid) = active_session.get() {
                respond_confirm.call((sid, false));
            }
            return;
        }
        if ctx_menu.get().is_some() {
            ev.prevent_default();
            ctx_menu.set(None);
            return;
        }
        if rename_session_target.get().is_some() {
            ev.prevent_default();
            rename_session_target.set(None);
            return;
        }
        if folder_modal.get().is_some() {
            ev.prevent_default();
            folder_modal.set(None);
            return;
        }
        if ui_confirm.get().is_some() {
            ev.prevent_default();
            ui_confirm.set(None);
            return;
        }
        if show_onboarding.get() {
            ev.prevent_default();
            if onboard_step.get() > 0 {
                onboard_step.update(|s| *s = s.saturating_sub(1));
            } else {
                dismiss_onboarding.call(());
            }
            return;
        }
        if show_settings.get() && !settings_busy.get() {
            ev.prevent_default();
            show_settings.set(false);
            return;
        }
        if show_files.get() {
            ev.prevent_default();
            show_files.set(false);
            return;
        }
        if show_capabilities.get() {
            ev.prevent_default();
            show_capabilities.set(false);
            return;
        }
        if dragging.get() {
            ev.prevent_default();
            dragging.set(false);
            return;
        }
        if show_right.get() && should_close_right_pane_on_escape(ev) {
            ev.prevent_default();
            show_right.set(false);
        }
    });

    // --- Top-nav project switcher + Project Settings ---
    // Switch the active project inline (same flow as the Projects screen).
    let switch_project = Callback::new(move |id: String| {
        show_proj_menu.set(false);
        show_projects.set(false);
        demo_mode.set(false);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
            let _ = invoke("open_project", arg).await;
            items.set(vec![]);
            active_session.set(None);
            collapsed_folders.set(HashSet::new());
            refresh_sessions(sessions);
            refresh_folders(folders);
            let v = invoke("get_project_info", JsValue::UNDEFINED).await;
            if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) { project_info.set(Some(p)); }
        });
    });
    let toggle_proj_menu = move |_| {
        let opening = !show_proj_menu.get();
        show_proj_menu.set(opening);
        if opening {
            spawn_local(async move {
                let v = invoke("list_projects", JsValue::UNDEFINED).await;
                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) { proj_list.set(list); }
            });
        }
    };
    let open_proj_settings = move |_| {
        show_proj_menu.set(false);
        spawn_local(async move {
            let v = invoke("get_project_settings", JsValue::UNDEFINED).await;
            if let Ok(s) = serde_wasm_bindgen::from_value::<ProjectSettings>(v) {
                proj_settings.set(s);
                show_proj_settings.set(true);
            }
        });
    };
    let save_proj_settings = move |_| {
        if proj_settings_busy.get() { return; }
        let form = proj_settings.get();
        if form.name.trim().is_empty() { return; }
        proj_settings_busy.set(true);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({
                "name": form.name, "description": form.description, "agentContext": form.agent_context,
            })).unwrap();
            let res = invoke_checked("update_project", arg).await;
            proj_settings_busy.set(false);
            if res.is_ok() {
                show_proj_settings.set(false);
                let v = invoke("get_project_info", JsValue::UNDEFINED).await;
                if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) { project_info.set(Some(p)); }
            }
        });
    };

    let move_session_to = {
        let sessions = sessions;
        Callback::new(move |(session_id, folder_id): (String, Option<String>)| {
            spawn_local(async move {
                let arg = to_value(&serde_json::json!({ "id": session_id, "folderId": folder_id })).unwrap();
                if invoke_checked("move_session", arg).await.is_ok() {
                    refresh_sessions(sessions);
                }
            });
        })
    };

    let new_folder = move |_| {
        folder_modal_input.set(String::new());
        folder_modal.set(Some(FolderModal::Create));
    };

    let save_folder_modal = {
        let folders = folders;
        move |mode: FolderModal| {
            let name = folder_modal_input.get().trim().to_string();
            if name.is_empty() {
                return;
            }
            folder_modal.set(None);
            match mode {
                FolderModal::Create => spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "name": name })).unwrap();
                    if invoke_checked("create_folder", arg).await.is_ok() {
                        refresh_folders(folders);
                    }
                }),
                FolderModal::Rename(id) => spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "id": id, "name": name })).unwrap();
                    if invoke_checked("rename_folder", arg).await.is_ok() {
                        refresh_folders(folders);
                    }
                }),
            }
        }
    };

    view! {
        {move || show_projects.get().then(|| {
            let open = Callback::new(move |id: String| {
                show_projects.set(false);
                demo_mode.set(false);
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                    let _ = invoke("open_project", arg).await;
                    // Reset the chat view for the newly-opened project, then reload
                    // its project info + session list (reuses the existing helpers).
                    items.set(vec![]);
                    active_session.set(None);
                    collapsed_folders.set(HashSet::new());
                    refresh_sessions(sessions);
                    refresh_folders(folders);
                    let v = invoke("get_project_info", JsValue::UNDEFINED).await;
                    if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
                        project_info.set(Some(p));
                    }
                });
            });
            let open_session = load_session.clone();
            let on_open_session = Callback::new(move |(project_id, session_id): (String, String)| {
                show_projects.set(false);
                demo_mode.set(false);
                let open_session = open_session.clone();
                spawn_local(async move {
                    let arg = to_value(&serde_json::json!({ "id": project_id })).unwrap();
                    let _ = invoke("open_project", arg).await;
                    // Project swap must land before loading the session (it switches
                    // the backend's active project + session frame out from under us).
                    open_session.call(session_id);
                    refresh_sessions(sessions);
                    let v = invoke("get_project_info", JsValue::UNDEFINED).await;
                    if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
                        project_info.set(Some(p));
                    }
                });
            });
            let on_open_demo = Callback::new(move |_: ()| {
                show_projects.set(false);
                demo_mode.set(true);
                items.set(vec![]);
                active_session.set(None);
                spawn_local(async move {
                    let v = invoke("list_demos", JsValue::UNDEFINED).await;
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DemoInfo>>(v) { demos.set(list); }
                });
            });
            view! { <ProjectsScreen locale=locale running=running approval_pending=approval_pending.read_only() on_open=open on_open_session=on_open_session on_open_demo=on_open_demo /> }
        })}
        <div class="app" class:app-hidden=move || show_projects.get() on:contextmenu=on_context_menu>
        <aside class="sidebar" class:collapsed=move || !show_sidebar.get()>
            <div class="sidebar-head">
                <button class="side-back" title=move || t(locale.get(), "sidebar.back_projects")
                    on:click=move |_| { show_proj_menu.set(false); demo_mode.set(false); show_projects.set(true); }>"←"</button>
                <button class="proj-switch" class:active=move || show_proj_menu.get() on:click=toggle_proj_menu>
                    <span class="proj-name">{move || if demo_mode.get() { t(locale.get(), "projects.example").to_string() } else { project_info.get().map(|p| p.name.clone()).unwrap_or_else(|| "wisp-science".into()) }}</span>
                    <span class="caret">"▾"</span>
                </button>
                <button class="icon-btn" title=move || t(locale.get(), "sidebar.collapse") on:click=move |_| show_sidebar.set(false)>"‹"</button>
            </div>
            {move || show_proj_menu.get().then(|| view! {
                <div class="proj-menu-backdrop" on:click=move |_| show_proj_menu.set(false)></div>
                <div class="proj-menu">
                    <button type="button" class="proj-menu-item" on:click=open_proj_settings>
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
                <button class="side-btn primary" on:click=new_session><span class="gi plus"></span>{move || t(locale.get(), "sidebar.new_session")}</button>
                <button class="side-btn" on:click=new_folder><span class="gi folder"></span>{move || t(locale.get(), "sidebar.new_folder")}</button>
                <button class="side-btn" on:click=open_files><span class="gi doc"></span>{move || t(locale.get(), "sidebar.files")}</button>
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
                                <button class="side-item ses" on:click=move |_| load_demo(d_click.clone())>
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
                        let open = load_session.clone();
                        let is_dragging = dragging_for_make.as_deref() == Some(id_drag.as_str());
                        let id_click = id.clone();
                        let id_key = id.clone();
                        let id_rename = id.clone();
                        let title_rename = title.clone();
                        view! {
                            <button type="button" class="side-item ses"
                                class:active=move || active_session.get().as_deref() == Some(id_active.as_str())
                                class:running=move || running.get().contains(&id_running)
                                class:dragging=is_dragging
                                attr:draggable="true"
                                data-session-id=id_attr
                                data-session-title=title_attr
                                on:click=move |_| {
                                    open.call(id_click.clone());
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
                                        open.call(id_key.clone());
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
                <button class="side-btn" on:click=open_capabilities><span class="gi grid"></span>{move || t(locale.get(), "sidebar.capabilities")}</button>
                <button class="side-btn" on:click=open_settings><span class="gi gear"></span>{move || t(locale.get(), "sidebar.settings")}</button>
            </div>
        </aside>

        <main class="center">
            <div class="topbar">
                {move || (!show_sidebar.get()).then(|| view! {
                    <button class="icon-btn" title=move || t(locale.get(), "sidebar.show") on:click=move |_| show_sidebar.set(true)>"›"</button>
                })}
                <span class="center-title">{move || {
                    let loc = locale.get();
                    if let Some(id) = active_session.get() {
                        if let Some(s) = sessions.get().iter().find(|s| s.id == id) {
                            let t = s.title.trim();
                            if !t.is_empty() { return s.title.clone(); }
                        }
                    }
                    items.get().iter().find_map(|i| match i {
                        ChatItem::User(msg) => {
                            let t = msg.trim();
                            if t.is_empty() { None }
                            else if t.chars().count() > 48 {
                                Some(format!("{}…", t.chars().take(48).collect::<String>()))
                            } else { Some(t.to_string()) }
                        }
                        _ => None,
                    }).unwrap_or_else(|| i18n::t(loc, "center.new_session").into())
                }}</span>
                {move || if needs_api_key.get() {
                    view! {
                        <span class="hint hint-action">
                            {move || t(locale.get(), "err.no_api_key")}" "
                            <button type="button" class="link-inline" on:click=move |_| open_settings_fn(Some("models".into()))>
                                {move || t(locale.get(), "status.open_settings")}
                            </button>
                        </span>
                    }.into_view()
                } else {
                    view! { <span class="hint">{move || status.get()}</span> }.into_view()
                }}
                <div class="spacer"></div>
                <button class="icon-btn" title=move || t(locale.get(), "center.toggle_panel")
                    class:active=move || show_right.get()
                    on:click=move |_| show_right.update(|v| *v = !*v)><span class="gi panel"></span></button>
            </div>

            <div class="chat" id=CHAT_SCROLLER_ID>
                <div class="thread" id=CHAT_THREAD_ID>
                    {move || items.get().is_empty().then(|| view! {
                        <div class="empty">
                            <span class="empty-logo"></span>
                            <h1>{move || t(locale.get(), "empty.title")}</h1>
                            <p>{move || t(locale.get(), "empty.subtitle")}</p>
                        </div>
                    })}
                    // Keyed rows (#65): the key is a content fingerprint, so a
                    // streaming delta rebuilds only the message it touched, not
                    // the whole thread (which froze long conversations).
                    <For
                        each=move || {
                            let arts_fp = artifacts.with(|a| artifacts_fingerprint(a));
                            let busy_now = busy.get();
                            let list = items.get();
                            let last = list.len().saturating_sub(1);
                            // Skip items that render nothing (empty streaming placeholder,
                            // attempt_completion) so their wrapper <div> doesn't leave a
                            // `.thread` gap between real messages (#19).
                            list.into_iter().enumerate()
                                .filter(|(_, item)| !renders_nothing(item))
                                .map(|(i, item)| {
                                    let is_last = i == last;
                                    let mut fp = item.fingerprint();
                                    match &item {
                                        // Assistant markdown embeds artifact chips (index + label).
                                        ChatItem::Assistant { .. } => fp ^= arts_fp,
                                        // The live reasoning block auto-opens while streaming (#31).
                                        ChatItem::Reasoning(_) => fp ^= (is_last && busy_now) as u64,
                                        _ => {}
                                    }
                                    (i, fp, item, is_last)
                                })
                                .collect::<Vec<_>>()
                        }
                        key=|(i, fp, _, _)| (*i, *fp)
                        children=move |(i, _, item, is_last)| {
                            let arts = artifacts.get_untracked();
                            let sid = active_session.get().unwrap_or_default();
                            view! {
                                <div class=class_for(&item)>
                                    {render_item(i, &item, &arts, on_artifact_select, on_file_link, busy.read_only(), is_last, edit_message, sid, respond_confirm)}
                                </div>
                            }
                        }
                    />
                </div>
            </div>

            <div class="composer">
                <div class="composer-inner"
                    class:composer-dragover=move || drag_over.get()
                    on:dragover=on_drag_over
                    on:dragleave=on_drag_leave
                    on:drop=on_drop>
                    <input id="composer-file-input" type="file" multiple=true class="composer-file-input"
                        on:change=on_files_selected />
                    {move || (!attachments.get().is_empty()).then(|| view! {
                        <div class="composer-attachments">
                            {attachments.get().into_iter().map(|att| {
                                let remove_key = match &att {
                                    ComposerAttachment::Uploading { key, .. }
                                    | ComposerAttachment::Ready { key, .. }
                                    | ComposerAttachment::Error { key, .. } => key.clone(),
                                };
                                let att_view = match att {
                                    ComposerAttachment::Uploading { name, .. } => {
                                        let label = if name.is_empty() {
                                            t(locale.get(), "composer.uploading").into()
                                        } else {
                                            name
                                        };
                                        view! { <span class="composer-attachment uploading">{label}</span> }.into_view()
                                    }
                                    ComposerAttachment::Ready { name, .. } => {
                                        view! { <span class="composer-attachment ready">{name}</span> }.into_view()
                                    }
                                    ComposerAttachment::Error { name, error, .. } => {
                                        view! {
                                            <span class="composer-attachment error" title=error.clone()>{name}</span>
                                        }.into_view()
                                    }
                                };
                                view! {
                                    <div class="composer-attachment-row">
                                        {att_view}
                                        <button type="button" class="composer-attachment-remove"
                                            title=move || t(locale.get(), "composer.remove_attachment")
                                            on:click=move |_| attachments.update(|items| {
                                                items.retain(|a| match a {
                                                    ComposerAttachment::Uploading { key, .. }
                                                    | ComposerAttachment::Ready { key, .. }
                                                    | ComposerAttachment::Error { key, .. } => key != &remove_key,
                                                });
                                            })>"×"</button>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })}
                    <div class="composer-mention-anchor">
                        <textarea
                            id="composer-input"
                            prop:value={move || input.get()}
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                match active_mention(&v) {
                                    Some((_, q)) => { mention_query.set(q); mention_index.set(0); mention_active.set(true); }
                                    None => mention_active.set(false),
                                }
                                input.set(v);
                            }
                            on:keydown=on_send
                            prop:placeholder=move || t(locale.get(), "composer.placeholder")
                        ></textarea>
                        {move || mention_show.get().then(|| {
                            let loc = locale.get();
                            let matches = mention_matches.get();
                            let row = |i: usize, name: String| view! {
                                <button type="button" class="mention-item" class:active=move || mention_index.get() == i
                                    on:mouseenter=move |_| mention_index.set(i)
                                    on:mousedown=move |ev| { ev.prevent_default(); select_mention.call(i); }>
                                    <span class="mention-item-icon">{compose_icon("attach")}</span>
                                    <span class="mention-item-name">{name}</span>
                                </button>
                            };
                            let in_view = matches.iter().enumerate()
                                .filter(|(_, (_, _, iv))| *iv)
                                .map(|(i, (n, _, _))| row(i, n.clone())).collect_view();
                            let session = matches.iter().enumerate()
                                .filter(|(_, (_, _, iv))| !*iv)
                                .map(|(i, (n, _, _))| row(i, n.clone())).collect_view();
                            let has_in_view = matches.iter().any(|(_, _, iv)| *iv);
                            let has_session = matches.iter().any(|(_, _, iv)| !*iv);
                            view! {
                                <div class="mention-backdrop" on:mousedown=move |_| mention_active.set(false)></div>
                                <div class="mention-menu">
                                    {has_in_view.then(|| view! {
                                        <div class="mention-group-label">{t(loc, "composer.mention_in_view")}</div>
                                    })}
                                    {in_view}
                                    {has_session.then(|| view! {
                                        <div class="mention-group-label">{t(loc, "composer.mention_session")}</div>
                                    })}
                                    {session}
                                    <div class="mention-menu-hint">{t(loc, "composer.mention_hint")}</div>
                                </div>
                            }
                        })}
                    </div>
                    <div class="composer-actions">
                        <div class="composer-tools">
                            <button type="button" class="composer-plus"
                                class:active=move || compose_menu_open.get()
                                title=move || t(locale.get(), "composer.add")
                                on:click=move |_| compose_menu_open.update(|o| *o = !*o)>
                                <span class="gi plus"></span>
                            </button>
                            {move || compose_menu_open.get().then(|| view! {
                                <div class="compose-backdrop" on:click=move |_| compose_menu_open.set(false)></div>
                                <div class="compose-menu">
                                    <div class="compose-menu-title">{move || t(locale.get(), "composer.compose")}</div>
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "composer.group_add")}</div>
                                        <button type="button" class="compose-item" disabled=composer_blocked
                                            on:click=move |ev| { compose_menu_open.set(false); pick_files(ev); }>
                                            <span class="compose-item-icon">{compose_icon("attach")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.attach_files")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.attach_files_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                        <button type="button" class="compose-item"
                                            on:click=move |ev| { compose_menu_open.set(false); open_files(ev); }>
                                            <span class="compose-item-icon">{compose_icon("folder")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.your_files")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.your_files_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                    </div>
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "composer.group_session")}</div>
                                        <button type="button" class="compose-item"
                                            on:click=move |_| {
                                                compose_menu_open.set(false);
                                                let loc = locale.get();
                                                status.set(t(loc, "status.reviewing"));
                                                let sid = active_session.get();
                                                spawn_local(async move {
                                                    let arg = to_value(&tauri_args::review_session(&sid)).unwrap();
                                                    if let Err(err) = invoke_checked("review_session", arg).await {
                                                        status.set(tf(loc, "status.review_failed", &[("msg", &localize_backend(loc, &js_error_text(err)))]));
                                                    }
                                                });
                                            }>
                                            <span class="compose-item-icon">{compose_icon("review")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.request_review")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.request_review_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                        <button type="button" class="compose-item"
                                            on:click=move |_| {
                                                compose_menu_open.set(false);
                                                input.set(t(locale.get(), "composer.skill_prompt").into());
                                                focus_composer();
                                            }>
                                            <span class="compose-item-icon">{compose_icon("skill")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "composer.save_skill")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "composer.save_skill_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                        <button type="button" class="compose-item"
                                            on:click=move |_| {
                                                compose_menu_open.set(false);
                                                open_settings_fn(Some("skills".into()));
                                            }>
                                            <span class="compose-item-icon">{compose_icon("skill")}</span>
                                            <span class="compose-item-text">
                                                <span class="compose-item-label">{move || t(locale.get(), "skills.manage")}</span>
                                                <span class="compose-item-sub">{move || t(locale.get(), "skills.manage_sub")}</span>
                                            </span>
                                            <span class="compose-item-chevron">{compose_icon("chevron")}</span>
                                        </button>
                                    </div>
                                </div>
                            })}
                            <button type="button" class="composer-compute"
                                class:active=move || compute_menu_open.get()
                                title=move || t(locale.get(), "compute.button")
                                on:click=move |_| compute_menu_open.update(|o| *o = !*o)>
                                {compose_icon("server")}
                            </button>
                            {move || compute_menu_open.get().then(|| view! {
                                <div class="compose-backdrop" on:click=move |_| compute_menu_open.set(false)></div>
                                <div class="compose-menu compute-menu">
                                    <button type="button" class="compose-item" on:click=move |_| {
                                        compute_menu_open.set(false);
                                        show_add_host.set(true);
                                        spawn_local(async move {
                                            let v = invoke("list_ssh_config_aliases", JsValue::UNDEFINED).await;
                                            if let Ok(a) = serde_wasm_bindgen::from_value::<Vec<String>>(v) { config_aliases.set(a); }
                                        });
                                    }>
                                        <span class="compose-item-icon">{compose_icon("server")}</span>
                                        <span class="compose-item-text">
                                            <span class="compose-item-label">{move || t(locale.get(), "compute.add_host")}</span>
                                        </span>
                                    </button>
                                    <div class="compose-group">
                                        <div class="compose-group-label">{move || t(locale.get(), "hosts.title")}</div>
                                        {move || {
                                            let hs = ssh_hosts.get();
                                            if hs.is_empty() {
                                                view! { <div class="compose-item-sub" style="padding:6px 18px">{move || t(locale.get(), "compute.none")}</div> }.into_view()
                                            } else {
                                                hs.into_iter().map(|h| view! {
                                                    <button type="button" class="compose-item" on:click=move |_| {
                                                        compute_menu_open.set(false); right_tab.set(RightTab::Hosts); show_right.set(true);
                                                    }>
                                                        <span class="compose-item-icon">{compose_icon("server")}</span>
                                                        <span class="compose-item-text"><span class="compose-item-label">{h.alias.clone()}</span></span>
                                                    </button>
                                                }.into_view()).collect_view()
                                            }
                                        }}
                                    </div>
                                </div>
                            })}
                        </div>
                        <div class="composer-buttons">
                            {move || (!models.get().is_empty()).then(|| view! {
                                <div class="model-picker">
                                    <button type="button" class="model-picker-btn" class:active=move || model_menu_open.get()
                                        on:click=move |_| model_menu_open.update(|o| *o = !*o)>
                                        <span class="model-picker-label">{move || {
                                            let l = models.get();
                                            l.iter().find(|m| m.active).or_else(|| l.first()).map(|m| m.label.clone()).unwrap_or_default()
                                        }}</span>
                                        <span class="model-picker-chev">"▾"</span>
                                    </button>
                                    {move || model_menu_open.get().then(|| view! {
                                        <div class="model-menu-backdrop" on:click=move |_| model_menu_open.set(false)></div>
                                        <div class="model-menu">
                                            {move || {
                                                let list = models.get();
                                                let can_delete = list.len() > 1;
                                                list.into_iter().map(|m| {
                                                    let pick_id = m.id.clone();
                                                    let del_id = m.id.clone();
                                                    let is_active = m.active;
                                                    let show_sub = !m.model.is_empty() && m.model != m.label;
                                                    view! {
                                                        <div class="model-menu-row" class:active=is_active>
                                                            <button type="button" class="model-menu-pick" on:click=move |_| {
                                                                model_menu_open.set(false);
                                                                let id = pick_id.clone();
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                    match invoke_checked("set_active_model", arg).await {
                                                                        Ok(v) => {
                                                                            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                                models.set(list);
                                                                            }
                                                                        }
                                                                        Err(err) => {
                                                                            web_sys::console::warn_1(&format!("set_active_model failed: {:?}", err).into());
                                                                        }
                                                                    }
                                                                });
                                                            }>
                                                                <span class="model-menu-text">
                                                                    <span class="model-menu-label">{m.label.clone()}</span>
                                                                    {show_sub.then(|| view! { <span class="model-menu-sub">{m.model.clone()}</span> })}
                                                                </span>
                                                                {is_active.then(|| view! { <span class="model-menu-check">"✓"</span> })}
                                                            </button>
                                                            {(can_delete && !is_active).then(|| { let id = del_id.clone(); view! {
                                                                <button type="button" class="model-menu-del"
                                                                    title=move || t(locale.get(), "models.remove")
                                                                    on:click=move |_| {
                                                                        let id = id.clone();
                                                                        spawn_local(async move {
                                                                            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                            let v = invoke("remove_model", arg).await;
                                                                            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) { models.set(list); }
                                                                        });
                                                                    }>"×"</button>
                                                            }})}
                                                        </div>
                                                    }
                                                }).collect_view()
                                            }}
                                            <button type="button" class="model-menu-add" on:click=move |_| {
                                                model_menu_open.set(false);
                                                model_form.set(Some(new_model_form()));
                                                model_form_key.set(String::new());
                                                model_form_msg.set(None);
                                                open_settings_fn(Some("models".into()));
                                            }>{move || t(locale.get(), "models.add")}</button>
                                        </div>
                                    })}
                                </div>
                            })}
                            {move || busy.get().then(|| view! {
                                <button type="button" class="stop"
                                    disabled=move || active_session.get() == stopping_session.get()
                                    on:click=stop>
                                    {move || t(locale.get(), if active_session.get() == stopping_session.get() { "composer.stopping" } else { "composer.stop" })}
                                </button>
                            })}
                            <button class="send" disabled=composer_blocked on:click=move |_| send()>
                                {move || t(locale.get(), if busy.get() { "composer.queue" } else { "composer.send" })}
                            </button>
                        </div>
                    </div>
                    <div class="composer-hint">{move || t(locale.get(), "composer.hint")}</div>
                </div>
            </div>
        </main>

        {move || show_right.get().then(|| view! {
            <div class="resizer" on:mousedown=on_resize_start></div>
            <section class="rightpane" style=move || format!("width:{}px", right_w.get())>
                <div class="rp-tabs">
                    <button class="rp-tab" class:active=move || right_tab.get() == RightTab::Artifacts
                        on:click=move |_| right_tab.set(RightTab::Artifacts)>
                        {move || {
                            let n = artifacts.get().len();
                            tab_count(locale.get(), "right.artifacts", n)
                        }}
                    </button>
                    <button class="rp-tab" class:active=move || right_tab.get() == RightTab::File
                        on:click=move |_| right_tab.set(RightTab::File)>{move || t(locale.get(), "right.file")}</button>
                    <button class="rp-tab" class:active=move || right_tab.get() == RightTab::Provenance
                        on:click=move |_| right_tab.set(RightTab::Provenance)>
                        {move || {
                            let n = items.get().iter().filter(|i| matches!(i, ChatItem::Tool { .. })).count();
                            tab_count(locale.get(), "right.provenance", n)
                        }}
                    </button>
                    <button class="rp-tab" class:active=move || right_tab.get() == RightTab::Hosts
                        on:click=move |_| right_tab.set(RightTab::Hosts)>
                        {move || t(locale.get(), "hosts.title")}
                    </button>
                    <div class="spacer"></div>
                    <button class="icon-btn" title=move || t(locale.get(), "right.close") on:click=move |_| show_right.set(false)>"×"</button>
                </div>
                <div class="rp-doc">
                    {move || match right_tab.get() {
                        RightTab::Artifacts => {
                            let arts = artifacts.get();
                            let loc = locale.get();
                            if arts.is_empty() {
                                view! {
                                    <div class="rp-empty">
                                        <span class="rp-empty-icon"></span>
                                        <div class="rp-empty-title">{t(loc, "right.no_artifacts.title")}</div>
                                        <p>{t(loc, "right.no_artifacts.body")}</p>
                                    </div>
                                }.into_view()
                            } else {
                                // Build the tile list from `arts` only — do NOT read
                                // `sel_artifact` in this (outer) scope, or selecting a
                                // tile re-runs the whole branch and rebuilds `.rp-tiles`,
                                // resetting its scroll to the top (#25). Selection is
                                // isolated to the `.active` class and the nested `.rp-view`
                                // closure below, so the scroll container is preserved.
                                let tiles = arts.iter().enumerate().map(|(i, a)| {
                                    let name = a.name.clone();
                                    let kind = a.kind.to_string();
                                    let meta = artifact_meta(a, loc);
                                    // File artifacts (images, csv, datasets) get download + ⋮ tools;
                                    // inline artifacts (md tables/code) keep the plain tile.
                                    let file = if let PreviewData::File { path, kind } = &a.data {
                                        Some((path.clone(), kind.clone()))
                                    } else {
                                        None
                                    };
                                    let file_click = file.clone();
                                    let name_click = name.clone();
                                    let tools = file.map(|(path, fkind)| {
                                        let (dl, vn) = (path.clone(), name.clone());
                                        view! {
                                            <div class="rp-tile-tools">
                                                <button type="button" class="rp-tile-tool"
                                                    title=move || t(locale.get(), "artifact.download")
                                                    on:click=move |ev| { ev.stop_propagation(); download_artifact(dl.clone()); }>"↓"</button>
                                                <button type="button" class="rp-tile-tool"
                                                    title=move || t(locale.get(), "artifact.more")
                                                    on:click=move |ev: web_sys::MouseEvent| {
                                                        ev.stop_propagation();
                                                        let open = matches!(artifact_menu.get(), Some((mi, _, _)) if mi == i);
                                                        artifact_menu.set(if open { None } else { Some((i, ev.client_x(), ev.client_y())) });
                                                    }>"⋮"</button>
                                            </div>
                                            {move || {
                                                let (mi, cx, cy) = artifact_menu.get()?;
                                                (mi == i).then(|| {
                                                let (p, n, k) = (path.clone(), vn.clone(), fkind.clone());
                                                let (mv, sp, dw) = (p.clone(), p.clone(), p.clone());
                                                let (mvn, mvk) = (n.clone(), k.clone());
                                                let spk = k.clone();
                                                view! {
                                                    <div class="rp-tile-menu-backdrop" on:click=move |_| artifact_menu.set(None)></div>
                                                    <div class="rp-tile-menu"
                                                        style=format!("right:calc(100vw - {cx}px);top:{cy}px")>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| { artifact_menu.set(None); modal_artifact.set(Some((mv.clone(), mvn.clone(), mvk.clone()))); }>
                                                            {move || t(locale.get(), "artifact.open_viewer")}</button>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| { artifact_menu.set(None); show_right.set(true); right_tab.set(RightTab::File); open_file.set(Some((sp.clone(), spk.clone()))); }>
                                                            {move || t(locale.get(), "artifact.open_split")}</button>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| { artifact_menu.set(None); show_right.set(true); right_tab.set(RightTab::Provenance); }>
                                                            {move || t(locale.get(), "artifact.provenance")}</button>
                                                        <button type="button" class="rp-tile-menu-item"
                                                            on:click=move |_| { artifact_menu.set(None); download_artifact(dw.clone()); }>
                                                            {move || t(locale.get(), "artifact.download")}</button>
                                                    </div>
                                                }
                                            })
                                            }}
                                        }.into_view()
                                    });
                                    view! {
                                        <div class="rp-tile" class:active=move || sel_artifact.get() == i
                                            data-artifact-name=name.clone()>
                                            <button type="button" class="rp-tile-main"
                                                on:click=move |_| {
                                                    artifact_menu.set(None);
                                                    if let Some((path, kind)) = &file_click {
                                                        if opens_in_modal(kind) {
                                                            modal_artifact.set(Some((path.clone(), name_click.clone(), kind.clone())));
                                                            return;
                                                        }
                                                    }
                                                    sel_artifact.set(i);
                                                }>
                                                <span class="rp-tile-text">
                                                    <span class="rp-tile-name">{name}</span>
                                                    <span class="rp-tile-meta">{meta}</span>
                                                </span>
                                                <span class=format!("rp-badge {}", kind)>{kind.clone()}</span>
                                            </button>
                                            {tools}
                                        </div>
                                    }.into_view()
                                }).collect_view();
                                let arts_for_view = arts.clone();
                                view! {
                                    <div class="rp-artifacts-body">
                                        <div class="rp-tiles">{tiles}</div>
                                        {move || {
                                            let arts = arts_for_view.clone();
                                            let sel = sel_artifact.get().min(arts.len().saturating_sub(1));
                                            let cur = arts[sel].clone();
                                            let dom_id = format!("rp-{sel}");
                                            // image/pdf/csv aren't rendered inline — offer the modal viewer.
                                            let modal_file = if let PreviewData::File { path, kind } = &cur.data {
                                                opens_in_modal(kind).then(|| (path.clone(), cur.name.clone(), kind.clone()))
                                            } else {
                                                None
                                            };
                                            view! {
                                                <div class="rp-view">
                                                    <div class="rp-view-head">
                                                        <span class=format!("rp-badge {}", cur.kind)>{cur.kind.to_string()}</span>
                                                        <span class="rp-view-name">{cur.name.clone()}</span>
                                                    </div>
                                                    {match modal_file {
                                                        Some((p, n, k)) => view! {
                                                            <button class="rp-open-viewer" type="button"
                                                                on:click=move |_| modal_artifact.set(Some((p.clone(), n.clone(), k.clone())))>
                                                                {move || t(locale.get(), "artifact.open_viewer")}
                                                            </button>
                                                        }.into_view(),
                                                        None => artifact_preview(&cur, dom_id, loc).into_view(),
                                                    }}
                                                </div>
                                            }
                                        }}
                                    </div>
                                }.into_view()
                            }
                        }
                        RightTab::File => {
                            let loc = locale.get();
                            match open_file.get() {
                                None => view! {
                                    <button type="button" class="rp-empty rp-empty-clickable"
                                        title=t(loc, "right.browse_files")
                                        on:click=open_files>
                                        <span class="rp-empty-icon"></span>
                                        <div class="rp-empty-title">{t(loc, "right.no_file.title")}</div>
                                        <p>{t(loc, "right.no_file.body")}</p>
                                        <span class="rp-empty-action">{t(loc, "right.browse_files")}</span>
                                    </button>
                                }.into_view(),
                                Some((path, kind)) => {
                                    let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
                                    let dom_id = "rp-file".to_string();
                                    view! {
                                        <div class="rp-view">
                                            <div class="rp-view-head">
                                                <span class=format!("rp-badge {}", kind)>{kind.clone()}</span>
                                                <span class="rp-view-name">{name.clone()}</span>
                                                <div class="spacer"></div>
                                                <button class="icon-btn" type="button"
                                                    title=move || t(locale.get(), "right.close_file")
                                                    on:click=move |_| open_file.set(None)>"×"</button>
                                            </div>
                                            <p class="rp-path hint">{path.clone()}</p>
                                            {if kind == "csv" {
                                                view! { <CsvFilePreview path=path.clone() /> }.into_view()
                                            } else {
                                                view! { <FilePreview dom_id=dom_id path=path kind=kind /> }.into_view()
                                            }}
                                        </div>
                                    }.into_view()
                                }
                            }
                        }
                        RightTab::Provenance => {
                            let loc = locale.get();
                            let tools: Vec<_> = items.get().iter().filter_map(|it| match it {
                                ChatItem::Tool { name, ok, input, output } => Some((name.clone(), *ok, input.clone(), output.clone())),
                                _ => None,
                            }).collect();
                            if tools.is_empty() {
                                view! {
                                    <div class="rp-empty">
                                        <span class="rp-empty-icon"></span>
                                        <div class="rp-empty-title">{t(loc, "right.no_tools.title")}</div>
                                        <p>{t(loc, "right.no_tools.body")}</p>
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                                    <div class="prov-list">
                                        {tools.into_iter().map(|(name, ok, input, output)| view! {
                                            <details class="prov-item" open=ok != Some(true)>
                                                <summary class="prov-head">
                                                    <span class="prov-name">{name.clone()}</span>
                                                    {match ok {
                                                        Some(true) => view! { <span class="ok">"✓"</span> }.into_view(),
                                                        Some(false) => view! { <span class="fail">"✗"</span> }.into_view(),
                                                        None => view! { <span class="run">"…"</span> }.into_view(),
                                                    }}
                                                </summary>
                                                {(!input.is_empty()).then(|| view! {
                                                    <div class="prov-label">{move || t(locale.get(), "right.input")}</div>
                                                    <pre class="prov-body">{input.clone()}</pre>
                                                })}
                                                {(!output.is_empty()).then(|| view! {
                                                    <div class="prov-label">{move || t(locale.get(), "right.output")}</div>
                                                    <pre class="prov-body">{output.clone()}</pre>
                                                })}
                                            </details>
                                        }).collect_view()}
                                    </div>
                                }.into_view()
                            }
                        }
                        RightTab::Hosts => {
                            let loc = locale.get();
                            let hs = ssh_hosts.get();
                            if hs.is_empty() {
                                view! {
                                    <div class="rp-hosts">
                                        <button type="button" class="rp-empty rp-empty-clickable"
                                            title=t(loc, "hosts.add")
                                            on:click=move |_| {
                                                show_add_host.set(true);
                                                spawn_local(async move {
                                                    let v = invoke("list_ssh_config_aliases", JsValue::UNDEFINED).await;
                                                    if let Ok(a) = serde_wasm_bindgen::from_value::<Vec<String>>(v) { config_aliases.set(a); }
                                                });
                                            }>
                                            <span class="rp-empty-icon host"><span class="gi server"></span></span>
                                            <div class="rp-empty-title">{t(loc, "hosts.empty.title")}</div>
                                            <p>{t(loc, "hosts.empty")}</p>
                                            <span class="rp-empty-action">{t(loc, "hosts.add")}</span>
                                        </button>
                                        <button type="button" class="rp-hosts-add"
                                            on:click=move |_| {
                                                spawn_local(async move {
                                                    let v = invoke("import_ssh_config_hosts", JsValue::UNDEFINED).await;
                                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) { ssh_hosts.set(list); }
                                                });
                                            }><span class="gi server"></span>{t(loc, "hosts.import")}</button>
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                                <div class="rp-hosts">
                                    <button type="button" class="rp-hosts-add"
                                        on:click=move |_| {
                                            show_add_host.set(true);
                                            spawn_local(async move {
                                                let v = invoke("list_ssh_config_aliases", JsValue::UNDEFINED).await;
                                                if let Ok(a) = serde_wasm_bindgen::from_value::<Vec<String>>(v) { config_aliases.set(a); }
                                            });
                                        }><span class="gi plus"></span>{t(loc, "hosts.add")}</button>
                                    <button type="button" class="rp-hosts-add"
                                        on:click=move |_| {
                                            spawn_local(async move {
                                                let v = invoke("import_ssh_config_hosts", JsValue::UNDEFINED).await;
                                                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) { ssh_hosts.set(list); }
                                            });
                                        }><span class="gi server"></span>{t(loc, "hosts.import")}</button>
                                    {
                                        hs.into_iter().map(|h| {
                                            let alias = h.alias.clone();
                                            let conn = {
                                                let mut c = String::new();
                                                if let Some(u) = &h.user { c.push_str(u); c.push('@'); }
                                                c.push_str(&h.alias);
                                                if let Some(p) = h.port { c.push_str(&format!(":{p}")); }
                                                c
                                            };
                                            view! {
                                                <div class="host-card">
                                                    <div class="host-card-head">
                                                        <span class="host-card-alias">{h.alias.clone()}</span>
                                                        <button type="button" class="host-card-remove"
                                                            on:click=move |_| {
                                                                let alias = alias.clone();
                                                                let arg = to_value(&serde_json::json!({ "alias": alias })).unwrap();
                                                                spawn_local(async move {
                                                                    let v = invoke("remove_ssh_host", arg).await;
                                                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) { ssh_hosts.set(list); }
                                                                });
                                                            }>"×"</button>
                                                    </div>
                                                    <div class="host-card-conn">{conn}</div>
                                                    {h.notes.clone().map(|n| view! { <div class="host-card-notes">{n}</div> })}
                                                </div>
                                            }
                                        }).collect_view()
                                    }
                                </div>
                                }.into_view()
                            }
                        }
                    }}
                </div>
            </section>
        }.into_view())}

        {move || dragging.get().then(|| view! {
            <div class="drag-overlay"
                on:mousemove=on_resize_move
                on:mouseup=move |_| dragging.set(false)></div>
        })}

        {move || rename_session_target.get().map(|(id, _)| {
            let id_key = id.clone();
            let id_btn = id.clone();
            view! {
            <div class="overlay">
                <div class="modal">
                    <h2>{move || t(locale.get(), "session.rename_title")}</h2>
                    <label>
                        <input
                            type="text"
                            prop:value=move || rename_session_input.get()
                            on:input=move |ev| rename_session_input.set(dom_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    let title = rename_session_input.get().trim().to_string();
                                    if title.is_empty() { return; }
                                    let id = id_key.clone();
                                    let sessions = sessions;
                                    rename_session_target.set(None);
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id, "title": title })).unwrap();
                                        if invoke_checked("rename_session", arg).await.is_ok() {
                                            refresh_sessions(sessions);
                                        }
                                    });
                                }
                            }
                        />
                    </label>
                    <div class="row">
                        <button on:click=move |_| rename_session_target.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| {
                            let title = rename_session_input.get().trim().to_string();
                            if title.is_empty() { return; }
                            let id = id_btn.clone();
                            let sessions = sessions;
                            rename_session_target.set(None);
                            spawn_local(async move {
                                let arg = to_value(&serde_json::json!({ "id": id, "title": title })).unwrap();
                                if invoke_checked("rename_session", arg).await.is_ok() {
                                    refresh_sessions(sessions);
                                }
                            });
                        }>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}

        {move || folder_modal.get().map(|mode| {
            let mode_save = mode.clone();
            let mode_enter = mode.clone();
            let title_key = match &mode {
                FolderModal::Create => "folder.new_title",
                FolderModal::Rename(_) => "folder.rename_prompt",
            };
            let label_key = match &mode {
                FolderModal::Create => "folder.new_prompt",
                FolderModal::Rename(_) => "folder.new_prompt",
            };
            view! {
            <div class="overlay">
                <div class="modal">
                    <h2>{move || t(locale.get(), title_key)}</h2>
                    <label>
                        {move || t(locale.get(), label_key)}
                        <input
                            type="text"
                            prop:value=move || folder_modal_input.get()
                            on:input=move |ev| folder_modal_input.set(dom_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    save_folder_modal(mode_enter.clone());
                                }
                            }
                        />
                    </label>
                    <div class="row">
                        <button on:click=move |_| folder_modal.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| save_folder_modal(mode_save.clone())>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}

        {move || ui_confirm.get().map(|action| {
            let action_ok = action.clone();
            let msg_key = match &action {
                UiConfirm::DeleteFolder(_) => "folder.delete_confirm",
                UiConfirm::DeleteSession(_) => "session.delete_confirm",
            };
            view! {
            <div class="overlay">
                <div class="modal confirm-modal">
                    <h2>{move || t(locale.get(), "confirm.title")}</h2>
                    <div class="hint">{move || t(locale.get(), msg_key)}</div>
                    <div class="row">
                        <button on:click=move |_| ui_confirm.set(None)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button class="primary" on:click=move |_| {
                            ui_confirm.set(None);
                            match action_ok.clone() {
                                UiConfirm::DeleteFolder(id) => {
                                    let folders = folders;
                                    let sessions = sessions;
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                        if invoke_checked("delete_folder", arg).await.is_ok() {
                                            refresh_folders(folders);
                                            refresh_sessions(sessions);
                                        }
                                    });
                                }
                                UiConfirm::DeleteSession(id) => {
                                    let sessions = sessions;
                                    let active_session = active_session;
                                    let items = items;
                                    let transcripts = transcripts;
                                    let running = running;
                                    let pending_turns = pending_turns;
                                    spawn_local(async move {
                                        let arg = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                        if invoke_checked("delete_session", arg).await.is_ok() {
                                            transcripts.update(|m| { m.remove(&id); });
                                            running.update(|r| { r.remove(&id); });
                                            pending_turns.update(|m| { m.remove(&id); });
                                            if active_session.get().as_deref() == Some(id.as_str()) {
                                                active_session.set(None);
                                                items.set(vec![]);
                                            }
                                            refresh_sessions(sessions);
                                        }
                                    });
                                }
                            }
                        }>{move || t(locale.get(), "confirm.approve")}</button>
                    </div>
                </div>
            </div>
        }.into_view()
        })}

        {move || show_proj_settings.get().then(|| view! {
            <div class="overlay">
                <div class="modal proj-settings-modal">
                    <div class="ps-head">
                        <h2>{move || t(locale.get(), "proj_settings.title")}</h2>
                        <button type="button" class="ps-close"
                            title=move || t(locale.get(), "settings.cancel")
                            on:click=move |_| show_proj_settings.set(false)>"×"</button>
                    </div>
                    <label>
                        {move || t(locale.get(), "proj_settings.name")}
                        <input prop:value=move || proj_settings.get().name
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.name = v); } />
                    </label>
                    <label>
                        {move || t(locale.get(), "proj_settings.description")}
                        <span class="ps-hint">{move || t(locale.get(), "proj_settings.description_hint")}</span>
                        <textarea class="ps-textarea" rows="2"
                            prop:value=move || proj_settings.get().description
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.description = v); }></textarea>
                    </label>
                    <label>
                        {move || t(locale.get(), "proj_settings.agent_context")}
                        <span class="ps-hint">{move || t(locale.get(), "proj_settings.agent_context_hint")}</span>
                        <textarea class="ps-textarea ps-ctx" rows="8"
                            prop:value=move || proj_settings.get().agent_context
                            on:input=move |ev| { let v = event_target_value(&ev); proj_settings.update(|s| s.agent_context = v); }></textarea>
                    </label>
                    <div class="row">
                        <button type="button" disabled=move || proj_settings_busy.get()
                            on:click=move |_| show_proj_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button type="button" class="primary"
                            disabled=move || proj_settings_busy.get() || proj_settings.get().name.trim().is_empty()
                            on:click=save_proj_settings>{move || t(locale.get(), "settings.save")}</button>
                    </div>
                </div>
            </div>
        })}

        {move || modal_artifact.get().map(|(path, name, kind)| {
            let session = active_session.get();
            view! {
                <ArtifactModal path=path name=name kind=kind session=session
                    on_close=Callback::new(move |_| modal_artifact.set(None))
                    on_open_path=Callback::new(move |(p, k): (String, String)| {
                        open_file.set(Some((p, k)));
                        right_tab.set(RightTab::File);
                        modal_artifact.set(None);
                    }) />
            }
        })}
        {move || show_settings.get().then(|| view! {
            <div class="overlay">
                <div class="modal settings-modal">
                    <div class="settings-nav">
                        <div class="settings-nav-group">
                            <span class="settings-nav-label">{move || t(locale.get(), "settings.nav.workspace")}</span>
                            <button class:active=move || settings_section.get()=="general"
                                on:click=move |_| go_settings_section("general")>
                                {move || t(locale.get(), "settings.nav.general")}</button>
                        </div>
                        <div class="settings-nav-group">
                            <span class="settings-nav-label">{move || t(locale.get(), "settings.nav.capabilities")}</span>
                            <button class:active=move || settings_section.get()=="models"
                                on:click=move |_| go_settings_section("models")>
                                {move || t(locale.get(), "settings.nav.models")}</button>
                            <button class:active=move || settings_section.get()=="memory"
                                on:click=move |_| go_settings_section("memory")>
                                {move || t(locale.get(), "settings.nav.memory")}</button>
                            <button class:active=move || settings_section.get()=="skills"
                                on:click=move |_| go_settings_section("skills")>
                                {move || t(locale.get(), "settings.nav.skills")}</button>
                            <button class:active=move || settings_section.get()=="connections"
                                on:click=move |_| go_settings_section("connections")>
                                {move || t(locale.get(), "settings.nav.connections")}</button>
                        </div>
                    </div>
                    <div class="settings-content">
                        {move || {
                            let sec = settings_section.get();
                            let loc = locale.get();
                            let parent = settings_section_label(loc, &sec);
                            let open_conn_name = open_conn_key.get().and_then(|k| {
                                connectors.get().and_then(|v| {
                                    v.connectors.into_iter().find(|c| c.key == k).map(|c| c.name)
                                })
                            });
                            let sub = settings_subpage_label(
                                loc,
                                &sec,
                                model_form.get().as_ref(),
                                conn_form.get().as_ref(),
                                open_conn_name.as_deref(),
                                memory_selected.get().as_deref(),
                            );
                            view! {
                                <div class="settings-head">
                                    <div class="settings-head-main">
                                        {sub.is_some().then(|| view! {
                                            <button type="button" class="settings-head-back"
                                                title=move || t(locale.get(), "settings.back")
                                                on:click=move |_| close_settings_subpage()>"‹"</button>
                                        })}
                                        {move || if let Some(child) = sub.clone() {
                                            view! {
                                                <div class="settings-breadcrumb">
                                                    <button type="button" class="settings-crumb-link"
                                                        on:click=move |_| close_settings_subpage()>{parent.clone()}</button>
                                                    <span class="settings-crumb-sep">"›"</span>
                                                    <span class="settings-crumb-current">{child}</span>
                                                </div>
                                            }.into_view()
                                        } else {
                                            view! { <h2>{parent.clone()}</h2> }.into_view()
                                        }}
                                    </div>
                                    <button type="button" class="settings-head-close icon-btn"
                                        title=move || t(locale.get(), "settings.cancel")
                                        on:click=move |_| show_settings.set(false)>"×"</button>
                                </div>
                            }
                        }}
                        {move || (settings_section.get() == "general").then(|| view! {
                            <div class="settings-pane">
                                <div class="settings-form-grid">
                                <label class="span-2">{move || t(locale.get(), "settings.language")}
                                    <select
                                        on:change=move|ev| {
                                            let code = dom_value(&ev);
                                            let loc = Locale::from_code(&code);
                                            locale.set(loc);
                                            set_document_lang(loc);
                                            settings.update(|s| s.locale = code);
                                        }
                                        prop:value=move || locale.get().code().to_string()>
                                        <option value="en">{move || t(locale.get(), "settings.language.en")}</option>
                                        <option value="zh">{move || t(locale.get(), "settings.language.zh")}</option>
                                    </select>
                                </label>
                                <label class="span-2">{move || t(locale.get(), "settings.workspace_dir")}
                                    <input class="settings-path-input" on:input=move|ev| settings.update(|s| {
                                            s.workspace_dir = event_target_input(&ev).value();
                                        })
                                        prop:value={move || settings.get().workspace_dir}
                                        placeholder=move || bootstrap.get().map(|b| b.workspace).unwrap_or_default() />
                                </label>
                                </div>
                                {move || settings_message.get().map(|(ok, text)| view! {
                                    <div class="settings-status"
                                        class:ok=move || ok
                                        class:fail=move || !ok>{text}</div>
                                })}
                                <div class="row settings-footer">
                                    <button type="button" disabled=move || settings_busy.get() on:click=check_updates>{move || t(locale.get(), "settings.check_updates")}</button>
                                    <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                                    <button type="button" class="primary" disabled=move || settings_busy.get() on:click=save_settings>{move || t(locale.get(), "settings.save")}</button>
                                </div>
                            </div>
                        }.into_view())}
                        {move || (settings_section.get() == "models").then(|| {
                            if model_form_open.get() {
                                view! {
                                    <div class="settings-pane settings-pane-subpage">
                                        <div class="conn-form model-form">
                                            <div class="settings-form-grid">
                                                <label class="span-2">{move || t(locale.get(), "settings.provider")}
                                                    <select data-testid="settings-provider"
                                                        on:change=move|ev| {
                                                            let p = dom_value(&ev);
                                                            model_form.update(|o| if let Some(o)=o {
                                                                let (api_url, model) = provider_defaults(&p);
                                                                o.provider = provider_value(&p).into();
                                                                o.api_url = api_url.into();
                                                                o.model = model.into();
                                                            });
                                                        }
                                                        prop:value=move || model_form.get().map(|f| provider_value(&f.provider).to_string()).unwrap_or_else(|| "openai".into())>
                                                        <option value="openai">{move || t(locale.get(), "settings.provider.openai")}</option>
                                                        <option value="openai_responses">{move || t(locale.get(), "settings.provider.openai_responses")}</option>
                                                        <option value="anthropic">{move || t(locale.get(), "settings.provider.anthropic")}</option>
                                                    </select>
                                                </label>
                                                <label class="span-2">{move || t(locale.get(), "settings.api_url")}
                                                    <input prop:value=move || model_form.get().map(|f| f.api_url.clone()).unwrap_or_default()
                                                        on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.api_url = event_target_input(&ev).value(); }) /></label>
                                                <label>{move || t(locale.get(), "settings.label")}
                                                    <input prop:value=move || model_form.get().map(|f| f.label.clone()).unwrap_or_default()
                                                        placeholder=move || t(locale.get(), "settings.label_ph")
                                                        on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.label = event_target_input(&ev).value(); }) /></label>
                                                <label>{move || t(locale.get(), "settings.model")}
                                                    <input prop:value=move || model_form.get().map(|f| f.model.clone()).unwrap_or_default()
                                                        placeholder=move || t(locale.get(), "settings.model_ph")
                                                        on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.model = event_target_input(&ev).value(); }) /></label>
                                                <label>{move || t(locale.get(), "settings.max_tokens")}
                                                    <input type="number" min="16" step="1"
                                                        on:input=move|ev| model_form.update(|o| if let Some(o)=o {
                                                            o.max_tokens = dom_value(&ev).parse().unwrap_or(0);
                                                        })
                                                        prop:value=move || model_form.get().map(|f| f.max_tokens.to_string()).unwrap_or_else(|| "8192".into()) />
                                                </label>
                                                <label>{move || t(locale.get(), "settings.reasoning_effort")}
                                                    <select
                                                        on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                            let v = dom_value(&ev);
                                                            o.reasoning_effort = if v == "default" { String::new() } else { v };
                                                        })
                                                        prop:value=move || {
                                                            let v = model_form.get().map(|f| f.reasoning_effort).unwrap_or_default();
                                                            if v.is_empty() { "default".to_string() } else { v }
                                                        }>
                                                        <option value="default">{move || t(locale.get(), "settings.reasoning_effort.default")}</option>
                                                        <option value="none">"none"</option>
                                                        <option value="minimal">"minimal"</option>
                                                        <option value="low">"low"</option>
                                                        <option value="medium">"medium"</option>
                                                        <option value="high">"high"</option>
                                                        <option value="xhigh">"xhigh"</option>
                                                    </select>
                                                </label>
                                                <label class="span-2">{move || t(locale.get(), "settings.api_key")}
                                                    <input type="password" prop:value=move || model_form_key.get()
                                                        on:input=move |ev| model_form_key.set(event_target_input(&ev).value()) /></label>
                                            </div>
                                            <span class="hint">{move || t(locale.get(), "settings.tip")}</span>
                                            {move || model_form_msg.get().map(|(ok, text)| view! {
                                                <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                            })}
                                            <div class="row settings-footer">
                                                <button type="button" disabled=move || settings_busy.get() on:click=validate_model_form>{move || t(locale.get(), "settings.validate")}</button>
                                                <button type="button" disabled=move || settings_busy.get() on:click=move |_| close_settings_subpage()>{move || t(locale.get(), "settings.cancel")}</button>
                                                <button type="button" class="primary" disabled=move || settings_busy.get() on:click=save_model_form>{move || t(locale.get(), "settings.save")}</button>
                                            </div>
                                        </div>
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                                <div class="settings-pane settings-pane-list">
                                    <div class="settings-toolbar settings-toolbar-end">
                                        <span class="settings-filter">{move || {
                                            let n = models.get().len();
                                            format!("{} ({n})", t(locale.get(), "settings.nav.models"))
                                        }}</span>
                                        <button type="button" class="settings-add-btn" on:click=move |_| {
                                            model_form.set(Some(new_model_form()));
                                            model_form_key.set(String::new());
                                            model_form_msg.set(None);
                                        }>{move || t(locale.get(), "models.add")}</button>
                                    </div>
                                    <div class="settings-list">
                                        <For each=move || models.get() key=|m| m.id.clone() let:m>
                                            {
                                                let pick_id = m.id.clone();
                                                let del_id = m.id.clone();
                                                let edit = m.clone();
                                                let is_active = m.active;
                                                let can_delete = models.get().len() > 1;
                                                let show_sub = !m.model.is_empty() && m.model != m.label;
                                                view! {
                                                    <div class="settings-list-row settings-list-row-link"
                                                        class:settings-list-row-active=is_active
                                                        on:click=move |_| {
                                                            model_form.set(Some(profile_to_form(&edit)));
                                                            model_form_key.set(if edit.has_api_key {
                                                                t(locale.get(), "settings.stored_key").into()
                                                            } else {
                                                                String::new()
                                                            });
                                                            model_form_msg.set(None);
                                                        }>
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">{m.label.clone()}</span>
                                                            {show_sub.then(|| view! {
                                                                <span class="settings-list-sub">{m.model.clone()}</span>
                                                            })}
                                                        </div>
                                                        <div class="settings-list-actions">
                                                            {is_active.then(|| view! {
                                                                <span class="settings-active-mark" title="active">"✓"</span>
                                                            })}
                                                            {(can_delete && !is_active).then(|| { let id = del_id.clone(); view! {
                                                                <button class="settings-list-remove" type="button" title=move || t(locale.get(), "models.remove")
                                                                    on:click=move |ev| {
                                                                        ev.stop_propagation();
                                                                        let id = id.clone();
                                                                        spawn_local(async move {
                                                                            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                            if let Ok(v) = invoke_checked("remove_model", arg).await {
                                                                                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                                    models.set(list);
                                                                                }
                                                                            }
                                                                        });
                                                                    }>"×"</button>
                                                            }})}
                                                            {(!is_active).then(|| { let id = pick_id.clone(); view! {
                                                                <button class="settings-list-use" type="button"
                                                                    on:click=move |ev| {
                                                                        ev.stop_propagation();
                                                                        let id = id.clone();
                                                                        spawn_local(async move {
                                                                            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                            if let Ok(v) = invoke_checked("set_active_model", arg).await {
                                                                                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                                    models.set(list);
                                                                                }
                                                                            }
                                                                        });
                                                                    }>{move || t(locale.get(), "models.use")}</button>
                                                            }})}
                                                            <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                        </div>
                                                    </div>
                                                }
                                            }
                                        </For>
                                    </div>
                                    {move || models.get().is_empty().then(|| view! {
                                        <p class="model-empty-hint">{move || t(locale.get(), "models.empty")}</p>
                                    })}
                                </div>
                                }.into_view()
                            }
                        })}
                        {move || (settings_section.get() == "memory").then(|| {
                            if memory_selected.get().is_some() {
                                view! {
                                    <div class="settings-pane settings-pane-subpage">
                                        {move || memory_selected.get().map(|name| {
                                            let name_del = name.clone();
                                            let name_save = name.clone();
                                            view! {
                                                <div class="memory-editor-inner memory-editor-page">
                                                    <textarea class="memory-editor-text" prop:value=move || memory_editor.get()
                                                        on:input=move |ev| memory_editor.set(event_target_value(&ev))></textarea>
                                                    {move || memory_msg.get().map(|(ok, text)| view! {
                                                        <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                                    })}
                                                    <div class="row settings-footer">
                                                        <button type="button" class="memory-delete-btn"
                                                            on:click=move |_| {
                                                                let n = name_del.clone();
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "name": n })).unwrap();
                                                                    if let Ok(files) = invoke_checked("delete_memory_file", arg).await {
                                                                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(files) {
                                                                            memory_view.update(|o| if let Some(o)=o { o.files = list; });
                                                                            close_settings_subpage();
                                                                        }
                                                                    }
                                                                });
                                                            }>{move || t(locale.get(), "memory.delete")}</button>
                                                        <button type="button" class="primary" on:click=move |_| {
                                                            let n = name_save.clone();
                                                            let content = memory_editor.get();
                                                            spawn_local(async move {
                                                                let arg = to_value(&serde_json::json!({ "name": n, "content": content })).unwrap();
                                                                match invoke_checked("write_memory_file", arg).await {
                                                                    Ok(v) => {
                                                                        if let Ok(files) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(v) {
                                                                            memory_view.update(|o| if let Some(o)=o { o.files = files; });
                                                                        }
                                                                        memory_msg.set(Some((true, t(locale.get(), "memory.save").into())));
                                                                    }
                                                                    Err(e) => memory_msg.set(Some((false, js_error_text(e)))),
                                                                }
                                                            });
                                                        }>{move || t(locale.get(), "memory.save")}</button>
                                                    </div>
                                                </div>
                                            }
                                        })}
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                                <div class="settings-pane settings-pane-memory">
                                    <div class="settings-toolbar settings-toolbar-end memory-toolbar">
                                        <span class="settings-filter">{move || {
                                            let n = memory_view.get().map(|v| v.files.len()).unwrap_or(0);
                                            format!("{} ({n})", t(locale.get(), "memory.notes"))
                                        }}</span>
                                        <div class="memory-toolbar-actions">
                                            <label class="toggle" title=move || t(locale.get(), "settings.nav.memory")>
                                                <input type="checkbox" prop:checked=move || memory_view.get().map(|v| v.enabled).unwrap_or(true)
                                                    on:change=move |ev| {
                                                        let on = event_target_checked(&ev);
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "enabled": on })).unwrap();
                                                            if let Ok(v) = invoke_checked("set_memory_enabled", arg).await {
                                                                if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                                                                    memory_view.set(Some(view));
                                                                }
                                                            }
                                                        });
                                                    } />
                                                <span class="toggle-track" aria-hidden="true"></span>
                                            </label>
                                            <button type="button" class="settings-add-btn" on:click=move |_| {
                                                if let Some(today) = memory_view.get().map(|v| v.today_file) {
                                                    load_memory_file(today);
                                                }
                                            }>{move || t(locale.get(), "memory.add")}</button>
                                            <button type="button" class="memory-clear-btn" on:click=move |_| {
                                                spawn_local(async move {
                                                    let v = invoke("clear_memory", JsValue::UNDEFINED).await;
                                                    if let Ok(files) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(v) {
                                                        memory_view.update(|o| if let Some(o)=o { o.files = files; });
                                                        memory_selected.set(None);
                                                        memory_editor.set(String::new());
                                                    }
                                                });
                                            }>{move || t(locale.get(), "memory.clear_all")}</button>
                                        </div>
                                    </div>
                                    {move || {
                                        let off = memory_view.get().map(|v| !v.enabled).unwrap_or(false);
                                        off.then(|| view! {
                                        <div class="memory-off-banner">
                                            <span>{move || t(locale.get(), "memory.off_banner")}</span>
                                            <button type="button" class="settings-add-btn" on:click=move |_| {
                                                spawn_local(async move {
                                                    let arg = to_value(&serde_json::json!({ "enabled": true })).unwrap();
                                                    if let Ok(v) = invoke_checked("set_memory_enabled", arg).await {
                                                        if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                                                            memory_view.set(Some(view));
                                                        }
                                                    }
                                                });
                                            }>{move || t(locale.get(), "memory.turn_on")}</button>
                                        </div>
                                        })
                                    }}
                                    <div class="settings-list memory-file-list">
                                        <For each=move || memory_view.get().map(|v| v.files).unwrap_or_default() key=|f| f.name.clone() let:f>
                                            {
                                                let pick = f.name.clone();
                                                view! {
                                                    <div class="settings-list-row settings-list-row-link"
                                                        on:click=move |_| load_memory_file(pick.clone())>
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">{f.name.clone()}</span>
                                                            <span class="settings-list-sub">{format_bytes(f.bytes)}</span>
                                                        </div>
                                                        <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                    </div>
                                                }
                                            }
                                        </For>
                                    </div>
                                    {move || memory_view.get().map(|v| v.files.is_empty().then(|| view! {
                                        <p class="memory-empty">{move || t(locale.get(), "memory.empty")}</p>
                                    })).into_view()}
                                </div>
                                }.into_view()
                            }
                        })}
                        {move || (settings_section.get() == "skills").then(|| view! {
                            <div class="settings-pane settings-pane-list">
                                <div class="settings-toolbar">
                                    <span class="settings-filter">{move || {
                                        let q = skills_search.get().trim().to_lowercase();
                                        let tag = skill_filter_tag.get();
                                        let n = skills_list.get().iter().filter(|s| {
                                            skill_matches_filter(s, &tag, &q)
                                        }).count();
                                        format!("{} ({n})", t(locale.get(), "skills.all"))
                                    }}</span>
                                    <input class="settings-search" type="search"
                                        placeholder=move || t(locale.get(), "skills.search_ph")
                                        prop:value=move || skills_search.get()
                                        on:input=move |ev| skills_search.set(event_target_input(&ev).value()) />
                                    <button type="button" on:click=move |_| set_visible_skills_enabled.call(true)>
                                        {move || t(locale.get(), "skills.enable_visible")}
                                    </button>
                                    <button type="button" on:click=move |_| set_visible_skills_enabled.call(false)>
                                        {move || t(locale.get(), "skills.disable_visible")}
                                    </button>
                                    <details class="settings-add-menu">
                                        <summary>{move || t(locale.get(), "skills.add")}</summary>
                                        <button type="button" on:click=move |_| {
                                            spawn_local(async move {
                                                let picked = invoke("pick_skill_source", JsValue::UNDEFINED).await;
                                                if let Some(path) = picked.as_string() {
                                                    let arg = to_value(&serde_json::json!({ "srcPath": path })).unwrap();
                                                    let _ = invoke_checked("install_skill", arg).await;
                                                    refresh_skills();
                                                }
                                            });
                                        }>{move || t(locale.get(), "skills.add_file")}</button>
                                        <button type="button" on:click=move |_| {
                                            spawn_local(async move {
                                                let picked = invoke("pick_directory", JsValue::UNDEFINED).await;
                                                if let Some(path) = picked.as_string() {
                                                    let arg = to_value(&serde_json::json!({ "srcPath": path })).unwrap();
                                                    let _ = invoke_checked("install_skill", arg).await;
                                                    refresh_skills();
                                                }
                                            });
                                        }>{move || t(locale.get(), "skills.add_folder")}</button>
                                    </details>
                                </div>
                                <div class="skill-tags-filter">
                                    <button class:active=move || skill_filter_tag.get().is_empty()
                                        on:click=move |_| skill_filter_tag.set(String::new())>
                                        {move || t(locale.get(), "skills.all")}
                                    </button>
                                    <button class:active=move || skill_filter_tag.get() == "__untagged"
                                        on:click=move |_| skill_filter_tag.set("__untagged".into())>
                                        {move || t(locale.get(), "skills.untagged")}
                                    </button>
                                    {move || {
                                        let tags = skills_list.get().iter()
                                            .flat_map(|s| s.tags.iter().cloned())
                                            .collect::<BTreeSet<_>>()
                                            .into_iter()
                                            .collect::<Vec<_>>();
                                        tags.into_iter().map(|tag| {
                                            let active_tag = tag.clone();
                                            let set_tag = tag.clone();
                                            view! {
                                                <button class:active=move || skill_filter_tag.get() == active_tag
                                                    on:click=move |_| skill_filter_tag.set(set_tag.clone())>
                                                    {tag}
                                                </button>
                                            }
                                        }).collect_view()
                                    }}
                                </div>
                                <p class="settings-note">{move || t(locale.get(), "settings.applies_new_session")}</p>
                                <div class="settings-list">
                                    <For each=move || {
                                        let q = skills_search.get().trim().to_lowercase();
                                        let tag = skill_filter_tag.get();
                                        skills_list.get().into_iter().filter(|s| {
                                            skill_matches_filter(s, &tag, &q)
                                        }).collect::<Vec<_>>()
                                    } key=|s| format!("{}:{}:{}", s.name, s.enabled, join_tags(&s.tags)) let:s>
                                        {
                                            let name_toggle = s.name.clone();
                                            let name_remove = s.name.clone();
                                            let name_tags = s.name.clone();
                                            let enabled = s.enabled;
                                            let builtin = s.builtin;
                                            let tags_text = join_tags(&s.tags);
                                            let tags_cb = save_skill_tags.clone();
                                            view! {
                                                <div class="settings-list-row" data-skill-name=s.name.clone()>
                                                    <div class="settings-list-main">
                                                        <span class="settings-list-title">{s.name.clone()}</span>
                                                        {(!s.description.is_empty() && s.description != ">").then(|| {
                                                            let desc = s.description.clone();
                                                            view! { <span class="settings-list-sub">{desc}</span> }
                                                        })}
                                                        <input class="skill-tags-input"
                                                            prop:value=tags_text
                                                            prop:placeholder=move || t(locale.get(), "skills.tags_placeholder")
                                                            on:change=move |ev| tags_cb.call((name_tags.clone(), event_target_value(&ev))) />
                                                    </div>
                                                    <div class="settings-list-actions">
                                                        {(!builtin).then(|| { let n = name_remove.clone(); view! {
                                                            <button class="settings-list-remove" type="button" title="remove" on:click=move |_| {
                                                                let n = n.clone();
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "name": n })).unwrap();
                                                                    let _ = invoke_checked("remove_skill", arg).await;
                                                                    refresh_skills();
                                                                });
                                                            }>"×"</button>
                                                        }})}
                                                        <label class="toggle">
                                                            <input type="checkbox" prop:checked=enabled on:change=move |ev| {
                                                                let n = name_toggle.clone();
                                                                let on = event_target_checked(&ev);
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "name": n, "enabled": on })).unwrap();
                                                                    let _ = invoke_checked("set_skill_enabled", arg).await;
                                                                    refresh_skills();
                                                                });
                                                            } />
                                                            <span class="toggle-track" aria-hidden="true"></span>
                                                        </label>
                                                    </div>
                                                </div>
                                            }
                                        }
                                    </For>
                                </div>
                            </div>
                        }.into_view())}
                        {move || (settings_section.get() == "connections").then(|| {
                            if conn_form_open.get() {
                                view! {
                                    <div class="settings-pane settings-pane-subpage">
                                        <div class="conn-form">
                                            <label>{move || t(locale.get(),"conn.name")}
                                                <input prop:value=move || conn_form.get().map(|f| f.name.clone()).unwrap_or_default()
                                                    on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.name = event_target_input(&ev).value(); }) /></label>
                                            <label>{move || t(locale.get(),"conn.kind")}
                                                <select prop:value=move || conn_form.get().map(|f| f.kind.clone()).unwrap_or_else(|| "stdio".into())
                                                    on:change=move |ev| conn_form.update(|o| if let Some(o)=o { o.kind = event_target_value(&ev); })>
                                                    <option value="stdio">{move || t(locale.get(),"conn.kind.stdio")}</option>
                                                    <option value="http">{move || t(locale.get(),"conn.kind.http")}</option>
                                                </select></label>
                                            {move || (conn_form_kind.get() == "stdio").then(|| view!{
                                                <label>{move || t(locale.get(),"conn.command")}
                                                    <input prop:value=move || conn_form.get().map(|f| f.command.clone()).unwrap_or_default()
                                                        on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.command = event_target_input(&ev).value(); }) /></label>
                                                <label>{move || t(locale.get(),"conn.args")}
                                                    <input placeholder="arg1 arg2" prop:value=move || conn_form.get().map(|f| f.args.clone()).unwrap_or_default()
                                                        on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.args = event_target_input(&ev).value(); }) /></label>
                                            })}
                                            {move || (conn_form_kind.get() == "http").then(|| view!{
                                                <label>{move || t(locale.get(),"conn.url")}
                                                    <input placeholder="https://host/mcp" prop:value=move || conn_form.get().map(|f| f.url.clone()).unwrap_or_default()
                                                        on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.url = event_target_input(&ev).value(); }) /></label>
                                                <label>{move || t(locale.get(),"conn.headers")}
                                                    <input placeholder="Authorization: Bearer xxx" prop:value=move || conn_form.get().map(|f| f.headers.clone()).unwrap_or_default()
                                                        on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.headers = event_target_input(&ev).value(); }) /></label>
                                            })}
                                            {move || conn_test_msg.get().map(|(ok,msg)| view!{
                                                <div class="settings-status" class:ok=ok class:fail=move||!ok>{msg}</div>
                                            })}
                                            <div class="row settings-footer">
                                                <button type="button" on:click=move |_| { let f = conn_form.get().unwrap_or_default();
                                                    spawn_local(async move {
                                                        let conn = build_conn_json(&f, false);
                                                        match invoke_checked("test_mcp_connection", to_value(&serde_json::json!({"conn": conn})).unwrap()).await {
                                                            Ok(v) => { let n = v.as_f64().unwrap_or(0.0) as i64; conn_test_msg.set(Some((true, format!("OK — {n} tools")))); }
                                                            Err(e) => conn_test_msg.set(Some((false, format!("{e:?}")))),
                                                        }
                                                    });
                                                }>{move || t(locale.get(),"conn.test")}</button>
                                                <button type="button" on:click=move |_| close_settings_subpage()>{move || t(locale.get(),"settings.cancel")}</button>
                                                <button type="button" class="primary" on:click=move |_| { let f = conn_form.get().unwrap_or_default();
                                                    spawn_local(async move {
                                                        let editing = f.id.is_some();
                                                        let conn = build_conn_json(&f, true);
                                                        let cmd = if editing { "update_mcp_connection" } else { "add_mcp_connection" };
                                                        if invoke_checked(cmd, to_value(&serde_json::json!({"conn": conn})).unwrap()).await.is_ok() {
                                                            conn_form.set(None); conn_test_msg.set(None); refresh_conns();
                                                        }
                                                    });
                                                }>{move || t(locale.get(),"settings.save")}</button>
                                            </div>
                                        </div>
                                    </div>
                                }.into_view()
                            } else if open_conn_key.get().is_some() {
                                // Level 2 — bundled connector detail: Skip-approvals + per-tool approval.
                                view! {
                                    <div class="settings-pane settings-pane-subpage">
                                        <p class="settings-note">{move || t(locale.get(), "settings.applies_new_session")}</p>
                                        {move || {
                                            let key = open_conn_key.get();
                                            let conn = key.and_then(|k| connectors.get().and_then(|v| v.connectors.into_iter().find(|c| c.key == k)));
                                            conn.map(|c| {
                                                let skip_on = c.skip_approvals;
                                                let key_skip = c.key.clone();
                                                view! {
                                                    <div class="settings-list">
                                                        <div class="settings-list-row">
                                                            <div class="settings-list-main">
                                                                <span class="settings-list-title">{move || t(locale.get(), "conn.skip_approvals")}</span>
                                                                <span class="settings-list-sub">{move || t(locale.get(), "conn.skip_approvals.desc")}</span>
                                                            </div>
                                                            <label class="toggle">
                                                                <input type="checkbox" prop:checked=skip_on on:change=move |ev| {
                                                                    let key = key_skip.clone();
                                                                    let on = event_target_checked(&ev);
                                                                    spawn_local(async move {
                                                                        let arg = to_value(&serde_json::json!({ "key": key, "enabled": on })).unwrap();
                                                                        let _ = invoke_checked("set_connector_skip_approvals", arg).await;
                                                                        refresh_conns();
                                                                    });
                                                                } />
                                                                <span class="toggle-track" aria-hidden="true"></span>
                                                            </label>
                                                        </div>
                                                    </div>
                                                    <div class="conn-group-label">{move || t(locale.get(), "conn.tools")}</div>
                                                    <div class="settings-list">
                                                        {c.tools.iter().map(|tool| {
                                                            let name = tool.name.clone();
                                                            let mode = tool.mode.clone();
                                                            let seg = |m: &'static str, glyph: &'static str, key: &'static str| {
                                                                let name2 = name.clone();
                                                                let active = mode.as_str() == m;
                                                                view! {
                                                                    <button type="button" class=format!("approval-btn approval-{m}") class:active=active
                                                                        disabled=skip_on
                                                                        title=move || t(locale.get(), key)
                                                                        on:click=move |_| {
                                                                            let name = name2.clone();
                                                                            spawn_local(async move {
                                                                                let arg = to_value(&serde_json::json!({ "tool": name, "mode": m })).unwrap();
                                                                                let _ = invoke_checked("set_tool_approval", arg).await;
                                                                                refresh_conns();
                                                                            });
                                                                        }>{glyph}</button>
                                                                }
                                                            };
                                                            view! {
                                                                <div class="settings-list-row">
                                                                    <div class="settings-list-main">
                                                                        <span class="settings-list-title">{tool.name.clone()}</span>
                                                                    </div>
                                                                    <div class="approval-seg" class:disabled=skip_on>
                                                                        {seg("allow", "✓", "conn.approval.allow")}
                                                                        {seg("ask", "?", "conn.approval.ask")}
                                                                        {seg("deny", "✕", "conn.approval.deny")}
                                                                    </div>
                                                                </div>
                                                            }
                                                        }).collect_view()}
                                                    </div>
                                                }
                                            })
                                        }}
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                            <div class="settings-pane settings-pane-list">
                                <div class="settings-toolbar settings-toolbar-end">
                                    <span class="settings-filter">{move || {
                                        let nb = connectors.get().map(|v| v.connectors.iter().filter(|c| c.kind == "bundled").count()).unwrap_or(0);
                                        let nc = conns_view.get().map(|v| v.connections.len()).unwrap_or(0);
                                        format!("{} ({})", t(locale.get(), "settings.nav.connections"), nb + nc)
                                    }}</span>
                                    <button type="button" class="settings-add-btn" on:click=move |_| {
                                        conn_form.set(Some(ConnForm { kind: "stdio".into(), enabled: true, ..Default::default() }));
                                        conn_test_msg.set(None);
                                    }>{move || t(locale.get(), "conn.add")}</button>
                                </div>
                                <p class="settings-note">{move || t(locale.get(), "settings.applies_new_session")}</p>
                                <div class="conn-group-label">{move || t(locale.get(), "conn.featured")}</div>
                                <div class="settings-list">
                                    <For each=move || connectors.get().map(|v| v.connectors.into_iter().filter(|c| c.kind == "bundled").collect::<Vec<_>>()).unwrap_or_default() key=|c| c.key.clone() let:c>
                                        {
                                            let key_open = c.key.clone();
                                            let key_toggle = c.key.clone();
                                            let n_tools = c.tools.len();
                                            let enabled = c.enabled;
                                            view! {
                                                <div class="settings-list-row settings-list-row-link"
                                                    on:click=move |_| open_conn_key.set(Some(key_open.clone()))>
                                                    <div class="settings-list-main">
                                                        <span class="settings-list-title">{c.name.clone()}</span>
                                                        <span class="settings-list-sub">{move || tf(locale.get(), "conn.tools_count", &[("n", &n_tools.to_string())])}</span>
                                                    </div>
                                                    <div class="settings-list-actions">
                                                        <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                                            <input type="checkbox" prop:checked=enabled on:change=move |ev| {
                                                                let key = key_toggle.clone();
                                                                let on = event_target_checked(&ev);
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "key": key, "enabled": on })).unwrap();
                                                                    let _ = invoke_checked("set_connector_enabled", arg).await;
                                                                    refresh_conns();
                                                                });
                                                            } />
                                                            <span class="toggle-track" aria-hidden="true"></span>
                                                        </label>
                                                        <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                    </div>
                                                </div>
                                            }
                                        }
                                    </For>
                                </div>
                                {move || conns_view.get().map(|v| v.connections.len()).unwrap_or(0).gt(&0).then(|| view! {
                                    <div class="conn-group-label">{move || t(locale.get(), "conn.custom")}</div>
                                })}
                                <div class="settings-list">
                                    <For each=move || conns_view.get().map(|v| v.connections).unwrap_or_default() key=|c| c.id.clone() let:c>
                                        {
                                            let id_del = c.id.clone();
                                            let id_toggle = c.id.clone();
                                            let row = c.clone();
                                            let kind_badge = match &c.transport {
                                                ConnTransport::Stdio { .. } => "stdio",
                                                ConnTransport::Http { .. } => "http",
                                            };
                                            view! {
                                                <div class="settings-list-row settings-list-row-link"
                                                    on:click=move |_| {
                                                        let form = match &row.transport {
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
                                                        };
                                                        conn_form.set(Some(form));
                                                        conn_test_msg.set(None);
                                                    }>
                                                    <div class="settings-list-main">
                                                        <span class="settings-list-title">{c.name.clone()} <span class="badge">{kind_badge}</span></span>
                                                        <span class="settings-list-sub">
                                                            {match &c.transport {
                                                                ConnTransport::Stdio { command, .. } => command.clone(),
                                                                ConnTransport::Http { url, .. } => url.clone(),
                                                            }}
                                                        </span>
                                                    </div>
                                                    <div class="settings-list-actions">
                                                        <button class="settings-list-remove" type="button" title="remove" on:click=move |ev| {
                                                            ev.stop_propagation();
                                                            let id = id_del.clone();
                                                            spawn_local(async move {
                                                                let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                let _ = invoke_checked("delete_mcp_connection", arg).await;
                                                                refresh_conns();
                                                            });
                                                        }>"×"</button>
                                                        <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                                            <input type="checkbox" prop:checked=c.enabled on:change=move |ev| {
                                                                let id = id_toggle.clone();
                                                                let on = event_target_checked(&ev);
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "id": id, "enabled": on })).unwrap();
                                                                    let _ = invoke_checked("set_mcp_connection_enabled", arg).await;
                                                                    refresh_conns();
                                                                });
                                                            } />
                                                            <span class="toggle-track" aria-hidden="true"></span>
                                                        </label>
                                                        <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                    </div>
                                                </div>
                                            }
                                        }
                                    </For>
                                </div>
                            </div>
                                }.into_view()
                            }
                        })}
                    </div>
                </div>
            </div>
        }.into_view())}

        {move || stopping_session.get().is_some().then(|| view! {
            <div class="stopping-toast">
                <span class="stopping-spinner"></span>
                <div class="stopping-text">
                    <strong>{move || t(locale.get(), "composer.stopping")}</strong>
                    <span>{move || t(locale.get(), "composer.stopping_hint")}</span>
                </div>
            </div>
        })}

        {move || show_add_host.get().then(|| view! {
            <div class="overlay">
                <div class="modal host-modal">
                    <h2>{move || t(locale.get(), "hosts.add")}</h2>
                    <label class="host-label">{move || t(locale.get(), "hosts.from_config")}</label>
                    <select class="host-input" on:change=move |ev| host_alias.set(event_target_value(&ev))>
                        <option value="">{move || t(locale.get(), "hosts.pick")}</option>
                        {move || config_aliases.get().into_iter().map(|a| view! { <option value=a.clone()>{a}</option> }).collect_view()}
                    </select>
                    <label class="host-label">{move || t(locale.get(), "hosts.or_type")}</label>
                    <input class="host-input" prop:value=move || host_alias.get() on:input=move |ev| host_alias.set(event_target_value(&ev)) />
                    <label class="host-label">{move || t(locale.get(), "hosts.notes")}</label>
                    <textarea class="host-input" prop:value=move || host_notes.get()
                        placeholder=move || t(locale.get(), "hosts.notes_ph")
                        on:input=move |ev| host_notes.set(event_target_value(&ev))></textarea>
                    <details class="host-advanced">
                        <summary>{move || t(locale.get(), "hosts.advanced")}</summary>
                        <label class="host-label">{move || t(locale.get(), "hosts.user")}</label>
                        <input class="host-input" prop:value=move || host_user.get() on:input=move |ev| host_user.set(event_target_value(&ev)) />
                        <label class="host-label">{move || t(locale.get(), "hosts.port")}</label>
                        <input class="host-input" prop:value=move || host_port.get() on:input=move |ev| host_port.set(event_target_value(&ev)) />
                        <label class="host-label">{move || t(locale.get(), "hosts.identity")}</label>
                        <input class="host-input" prop:value=move || host_identity.get() on:input=move |ev| host_identity.set(event_target_value(&ev)) />
                    </details>
                    <div class="row">
                        <button type="button" on:click=move |_| show_add_host.set(false)>{move || t(locale.get(), "hosts.cancel")}</button>
                        <button type="button" class="primary" disabled=move || host_alias.get().trim().is_empty()
                            on:click=move |_| {
                                let opt = |s: String| { let s = s.trim().to_string(); if s.is_empty() { None } else { Some(s) } };
                                let host = SshHost {
                                    alias: host_alias.get().trim().to_string(),
                                    user: opt(host_user.get()),
                                    port: host_port.get().trim().parse::<u16>().ok(),
                                    identity_file: opt(host_identity.get()),
                                    notes: opt(host_notes.get()),
                                };
                                let arg = to_value(&serde_json::json!({ "host": host })).unwrap();
                                spawn_local(async move {
                                    let v = invoke("add_ssh_host", arg).await;
                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SshHost>>(v) { ssh_hosts.set(list); }
                                });
                                host_alias.set(String::new()); host_user.set(String::new()); host_port.set(String::new());
                                host_identity.set(String::new()); host_notes.set(String::new());
                                show_add_host.set(false);
                            }>{move || t(locale.get(), "hosts.save")}</button>
                    </div>
                </div>
            </div>
        }.into_view())}

        {move || show_files.get().then(|| {
            let cwd = file_cwd.get();
            let parent = if cwd == "." { None } else { Some(parent_path(&cwd)) };
            view! {
                <div class="overlay">
                    <div class="modal modal-wide">
                        <div class="fb-head">
                            <h2>{move || t(locale.get(), "files.title")}</h2>
                            <button class="icon-btn" on:click=move |_| show_files.set(false)>"×"</button>
                        </div>
                        <div class="fb-crumb">
                            {parent.map(|p| {
                                let p_click = p.clone();
                                view! {
                                    <button class="fb-up" on:click=move |_| { file_query.set(String::new()); file_cwd.set(p_click.clone()); refresh_dir(file_cwd, file_entries); }>"↑"</button>
                                }.into_view()
                            })}
                            <span class="fb-path">{cwd.clone()}</span>
                        </div>
                        <input class="fb-search" type="text"
                            placeholder=move || t(locale.get(), "files.search")
                            prop:value=move || file_query.get()
                            on:input=move |ev| file_query.set(event_target_value(&ev)) />
                        <div class="fb-list">
                            {move || {
                                let q = file_query.get().to_lowercase();
                                file_entries.get().into_iter()
                                    .filter(move |e| q.is_empty() || e.name.to_lowercase().contains(&q))
                                    .map(|e| {
                                let name = e.name.clone();
                                let full = join_path(&file_cwd.get(), &name);
                                if e.is_dir {
                                    let full_click = full.clone();
                                    view! {
                                        <button class="fb-row dir" on:click=move |_| {
                                            file_query.set(String::new());
                                            file_cwd.set(full_click.clone());
                                            refresh_dir(file_cwd, file_entries);
                                        }>
                                            <span class="fb-icon">"📁"</span>
                                            <span class="fb-name">{name}</span>
                                        </button>
                                    }.into_view()
                                } else {
                                    let full_open = full.clone();
                                    let kind = file_kind(&full).unwrap_or("text").to_string();
                                    view! {
                                        <button class="fb-row" on:click=move |_| {
                                            open_file.set(Some((full_open.clone(), kind.clone())));
                                            show_files.set(false);
                                            show_right.set(true);
                                            right_tab.set(RightTab::File);
                                        }>
                                            <span class="fb-icon">"📄"</span>
                                            <span class="fb-name">{name}</span>
                                            <span class="fb-size">{format_bytes(e.size)}</span>
                                        </button>
                                    }.into_view()
                                }
                            }).collect_view()
                            }}
                        </div>
                        {move || project_info.get().map(|p| {
                            let loc = locale.get();
                            view! {
                            <div class="hint fb-root">{tf(loc, "files.root", &[("path", &p.root)])}</div>
                        }})}
                    </div>
                </div>
            }.into_view()
        })}

        {move || show_capabilities.get().then(|| view! {
            <div class="overlay">
                <div class="modal modal-wide">
                    <div class="fb-head">
                        <h2>{move || t(locale.get(), "caps.title")}</h2>
                        <button class="icon-btn" on:click=move |_| show_capabilities.set(false)>"×"</button>
                    </div>
                    {move || bootstrap.get().map(|b| {
                        let loc = locale.get();
                        view! {
                        <div class="cap-section">
                            <h3>{tf(loc, "caps.runtime", &[("version", &b.app_version)])}</h3>
                            <p class="hint">{tf(loc, "caps.workspace", &[("path", &b.workspace)])}</p>
                            <p class="hint">{{
                                let ready = t(loc, "caps.ready");
                                let missing = t(loc, "caps.missing");
                                tf(loc, "caps.runtime_status", &[
                                ("py", if b.python_ok { &ready } else { &missing }),
                                ("uv", if b.uv_ok { &ready } else { &missing }),
                                ("node", if b.node_ok { &ready } else { &missing }),
                                ("sci", if b.sci_ok { &ready } else { &missing }),
                                ("pixi", if b.pixi_ok { &ready } else { &missing }),
                                ("skills", &b.skills_loaded.to_string()),
                                ("mcp", &b.mcp_catalog.to_string()),
                            ])}}</p>
                            {(!b.errors.is_empty()).then(|| view! {
                                <div class="settings-status fail">
                                    {b.errors.join("\n")}
                                </div>
                            })}
                        </div>
                    }})}
                    {move || caps.get().map(|c| view! {
                        // ponytail: counts only — detail lists (bio-tool tags, skill list,
                        // permissions hint) live in Settings, not this read-only summary.
                        <div class="cap-grid">
                            <div class="cap-stat"><span class="cap-num">{c.project.skill_count}</span><span class="cap-label">{move || t(locale.get(), "caps.skills")}</span></div>
                            <div class="cap-stat"><span class="cap-num">{c.mcp_servers.len()}</span><span class="cap-label">{move || t(locale.get(), "caps.mcp_servers")}</span></div>
                            <div class="cap-stat"><span class="cap-num">{c.memory_files.len()}</span><span class="cap-label">{move || t(locale.get(), "caps.memory_files")}</span></div>
                        </div>
                    })}
                    <div class="row">
                        <button on:click=move |_| show_capabilities.set(false)>{move || t(locale.get(), "caps.close")}</button>
                        {move || bootstrap.get().filter(|b| !b.python_ok || !b.uv_ok || !b.node_ok || !b.sci_ok || !b.pixi_ok).map(|_| view! {
                            <button class="primary" disabled=move || busy.get() on:click=start_env_setup.clone()>
                                {move || t(locale.get(), "caps.setup_env")}
                            </button>
                        })}
                    </div>
                </div>
            </div>
        }.into_view())}

        {move || show_onboarding.get().then(|| {
            let step = onboard_step.get();
            let loc = locale.get();
            view! {
                <div class="overlay onboard-overlay">
                    <div class="modal onboard">
                        {match step {
                            0 => view! {
                                <h2>{t(loc, "onboard.welcome.title")}</h2>
                                <p class="hint">{t(loc, "onboard.welcome.body")}</p>
                            }.into_view(),
                            1 => view! {
                                <h2>{t(loc, "onboard.connect.title")}</h2>
                                <p class="hint">{t(loc, "onboard.connect.body")}</p>
                            }.into_view(),
                            _ => view! {
                                <h2>{t(loc, "onboard.features.title")}</h2>
                                <p class="hint">{t(loc, "onboard.features.body")}</p>
                            }.into_view(),
                        }}
                        <div class="onboard-dots">
                            {(0..3).map(|i| view! {
                                <span class="onboard-dot" class:active=move || onboard_step.get() == i></span>
                            }).collect_view()}
                        </div>
                        <div class="row">
                            {if step > 0 {
                                view! { <button on:click=move |_| onboard_step.update(|s| *s = s.saturating_sub(1))>{move || t(locale.get(), "onboard.back")}</button> }.into_view()
                            } else { view! { <span></span> }.into_view() }}
                            {if step < 2 {
                                view! { <button class="primary" on:click=move |_| onboard_step.update(|s| *s += 1)>{move || t(locale.get(), "onboard.next")}</button> }.into_view()
                            } else {
                                view! {
                                    <button class="primary" on:click=dismiss_onboard>{move || t(locale.get(), "onboard.start")}</button>
                                }.into_view()
                            }}
                        </div>
                    </div>
                </div>
            }.into_view()
        })}
        <ContextMenuPortal menu=ctx_menu.read_only() set_menu=ctx_menu.write_only() on_pick=on_ctx_pick />
        </div>
    }
}

/// True for items whose `render_item` produces an empty view, so the thread
/// loop can drop their wrapper `<div>` and avoid a dangling `.thread` gap (#19).
fn renders_nothing(item: &ChatItem) -> bool {
    matches!(item, ChatItem::Assistant { text, .. } if text.trim().is_empty())
        || matches!(item, ChatItem::Tool { name, .. } if name == "attempt_completion")
}

fn class_for(item: &ChatItem) -> &'static str {
    match item {
        ChatItem::User(_) => "msg user",
        ChatItem::QueuedUser(_) => "msg user queued",
        ChatItem::Assistant { text, .. } if text.starts_with("Error: ") => "tool-wrap",
        ChatItem::Assistant { .. } => "msg assistant",
        ChatItem::Reasoning(_) => "msg reasoning",
        ChatItem::Tool { .. } => "tool-wrap",
        ChatItem::ApprovalPending { .. } => "tool-wrap approval-wrap-row",
        ChatItem::Review(_) => "tool-wrap",
    }
}

fn render_item(
    ui_index: usize,
    item: &ChatItem,
    artifacts: &[Artifact],
    on_artifact: Callback<usize>,
    on_file: Callback<(String, String)>,
    busy: ReadSignal<bool>,
    is_last: bool,
    on_edit: impl Fn(usize) + Clone + 'static,
    session_id: String,
    on_approval: Callback<(String, bool)>,
) -> impl IntoView {
    let locale = use_locale();
    match item {
        ChatItem::User(s) => view! {
            <UserMessage
                text=s.clone()
                ui_index=ui_index
                busy=busy
                on_copy=Callback::new(copy_text)
                on_edit=Callback::new(on_edit)
            />
        }.into_view(),
        ChatItem::QueuedUser(s) => view! {
            <div class="role">{move || t(locale.get(), "composer.queued")}</div>
            <div class="user-bubble queued-bubble">
                <div class="body">{s.clone()}</div>
            </div>
        }.into_view(),
        ChatItem::Assistant { text, .. } if text.trim().is_empty() => view! {}.into_view(),
        ChatItem::Assistant { text, .. } if text.starts_with("Error: ") => {
            let msg = text.strip_prefix("Error: ").unwrap_or(text.as_str()).to_string();
            let copy = msg.clone();
            view! {
                <div class="finding err">
                    <div class="finding-head">
                        <span class="finding-tag">{move || format!("● {}", t(locale.get(), "chat.error"))}</span>
                        <span class="finding-title">{msg}</span>
                        <button type="button" class="tool-btn card-copy"
                            title=move || t(locale.get(), "ctx.copy_message")
                            on:click=move |_| copy_text(copy.clone())>
                            {move || t(locale.get(), "msg.copy")}
                        </button>
                    </div>
                </div>
            }.into_view()
        }
        ChatItem::Assistant { text, model } => view! {
            <AssistantMessage
                text=text.clone()
                model=model.clone()
                artifacts=artifacts.to_vec()
                on_artifact=on_artifact
                on_file=on_file
                on_copy=Callback::new(copy_text)
            />
        }.into_view(),
        ChatItem::Tool { name, .. } if name == "attempt_completion" => view! {}.into_view(),
        ChatItem::Reasoning(s) => {
            // Auto-expand the block while it is the live, streaming item. The thread
            // is a non-keyed re-render, so every reasoning delta rebuilds this
            // <details> from scratch; a DOM-only open state would snap shut on the
            // next chunk and the user could never watch the live thinking (#31).
            let live = is_last && busy.get();
            view! {
                <details class="rz" open=live>
                    <summary>{move || t(locale.get(), "chat.thinking")}</summary>
                    <div class="body">{s.clone()}</div>
                </details>
            }.into_view()
        }
        ChatItem::Tool { name, ok, input, output } => view! {
            <ToolBlock name=name.clone() ok=*ok input=input.clone() output=output.clone() />
        }.into_view(),
        ChatItem::ApprovalPending { tool, preview, message: _ } => view! {
            <ApprovalCard tool=tool.clone() preview=preview.clone() session_id=session_id.clone() on_decide=on_approval />
        }.into_view(),
        ChatItem::Review(md) => {
            let copy = md.clone();
            view! {
                <div class="review-card">
                    <div class="review-head">
                        <span class="review-badge">"🔍"</span>
                        {move || t(locale.get(), "review.title")}
                        <button type="button" class="tool-btn card-copy"
                            title=move || t(locale.get(), "ctx.copy_message")
                            on:click=move |_| copy_text(copy.clone())>
                            {move || t(locale.get(), "msg.copy")}
                        </button>
                    </div>
                    <div class="md review-md" inner_html=md_to_html(md)></div>
                </div>
            }.into_view()
        }
    }
}

pub fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}
