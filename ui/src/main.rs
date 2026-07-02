mod context_menu;
mod i18n;

use context_menu::{ContextMenuPortal, CtxMenu};
use i18n::{localize_backend, set_document_lang, tab_count, tf, t, use_locale, Locale};
use leptos::{ev, window_event_listener, *};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DOM_ID: AtomicUsize = AtomicUsize::new(0);

fn unique_dom_id(prefix: &str) -> String {
    format!("{prefix}-{}", NEXT_DOM_ID.fetch_add(1, Ordering::Relaxed))
}
use serde::{Deserialize, Serialize};
use serde_wasm_bindgen::to_value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen(module = "/src/highlight.js")]
extern "C" {
    async fn highlight_by_id(id: &str) -> JsValue;
}

#[wasm_bindgen(module = "/src/api.js")]
extern "C" {
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
    #[wasm_bindgen(catch, js_name = invoke_strict)]
    async fn invoke_checked(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = invoke_timeout)]
    async fn invoke_timeout(cmd: &str, args: JsValue, timeout_ms: u32) -> Result<JsValue, JsValue>;
    async fn listen(event: &str, cb: &js_sys::Function) -> JsValue;
    async fn mount_preview(kind: &str, el_id: &str, payload: &str) -> JsValue;
    async fn upload_files(files: JsValue) -> JsValue;
    #[wasm_bindgen(js_name = upload_input_files)]
    async fn upload_input_files(input_id: &str) -> JsValue;
}

#[wasm_bindgen(module = "/src/scroll.js")]
extern "C" {
    fn attach_chat_scroll(scroller_id: &str, content_id: &str);
    fn notify_chat_scroll(scroller_id: &str);
    fn force_chat_scroll_bottom(scroller_id: &str);
}

const CHAT_SCROLLER_ID: &str = "chat-scroller";
const CHAT_THREAD_ID: &str = "chat-thread";

fn schedule_chat_follow() {
    notify_chat_scroll(CHAT_SCROLLER_ID);
}

fn force_chat_bottom() {
    force_chat_scroll_bottom(CHAT_SCROLLER_ID);
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
#[serde(tag = "kind")]
enum AgentEvent {
    Text { frame_id: String, delta: String },
    Reasoning { frame_id: String, delta: String },
    ToolCall { frame_id: String, name: String, preview: String },
    ToolResult { frame_id: String, name: String, ok: bool, content: String },
    Usage { frame_id: String, round: u64, input: u64, output: u64, ctx_tokens: usize, max_context: usize },
    Compaction { frame_id: String, before: usize, after: usize, strategy: String },
    Diff { frame_id: String, path: String },
    Stdout { frame_id: String, chunk: String },
    Done { frame_id: String },
    Error { frame_id: String, message: String },
}

#[derive(Clone)]
enum ChatItem {
    User(String),
    Assistant(String),
    Reasoning(String),
    Tool { name: String, ok: Option<bool>, input: String, output: String },
}

#[derive(Serialize, Deserialize, Clone)]
struct ArtifactInfo {
    id: String,
    name: String,
    kind: String,
    path: String,
    ts: i64,
}

#[derive(Clone)]
enum ComposerAttachment {
    Uploading { key: String, name: String },
    Ready { key: String, name: String, path: String },
    Error { key: String, name: String, error: String },
}

#[derive(Deserialize)]
struct UploadFileResult {
    ok: bool,
    info: Option<ArtifactInfo>,
    filename: Option<String>,
    error: Option<String>,
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

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    provider: String,
    api_url: String,
    model: String,
    has_api_key: bool,
    #[serde(default)]
    locale: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            api_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            has_api_key: false,
            locale: Locale::En.code().into(),
        }
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

fn dom_value(ev: &web_sys::Event) -> String {
    ev.target()
        .and_then(|target| js_sys::Reflect::get(&target, &JsValue::from_str("value")).ok())
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

fn provider_value(provider: &str) -> &'static str {
    match provider.trim() {
        "anthropic" => "anthropic",
        "openai_responses" | "openai-responses" | "responses" => "openai_responses",
        _ => "openai",
    }
}

fn provider_defaults(provider: &str) -> (&'static str, &'static str) {
    match provider_value(provider) {
        "anthropic" => ("https://api.anthropic.com", "claude-sonnet-5"),
        "openai_responses" => ("https://api.openai.com/v1", "gpt-5.5"),
        _ => ("https://api.deepseek.com", "deepseek-v4-pro"),
    }
}

fn apply_provider_defaults(settings: RwSignal<Settings>, provider: String) {
    settings.update(|s| {
        let provider = provider_value(&provider);
        let (api_url, model) = provider_defaults(provider);
        s.provider = provider.into();
        s.api_url = api_url.into();
        s.model = model.into();
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

#[derive(Serialize, Deserialize, Clone)]
struct DemoInfo {
    id: String,
    title: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Demo {
    id: String,
    title: String,
    request: String,
    response: String,
    thinking: Option<String>,
}

#[derive(Serialize)]
struct SendArgs<'a> { message: &'a str }

#[derive(Deserialize, Clone)]
struct SessionInfo {
    id: String,
    title: String,
    #[allow(dead_code)]
    ts: i64,
}

/// A transcript row returned by `load_session`.
#[derive(Deserialize, Clone)]
struct LoadedItem {
    role: String,
    text: String,
    tool_name: Option<String>,
    ok: Option<bool>,
}

impl LoadedItem {
    fn into_chat(self) -> ChatItem {
        match self.role.as_str() {
            "user" => ChatItem::User(self.text),
            "reasoning" => ChatItem::Reasoning(self.text),
            "tool" => ChatItem::Tool {
                name: self.tool_name.unwrap_or_else(|| "tool".into()),
                ok: self.ok,
                input: String::new(),
                output: self.text,
            },
            _ => ChatItem::Assistant(self.text),
        }
    }
}

#[derive(Clone, PartialEq)]
struct TableData {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

#[derive(Clone, PartialEq)]
#[allow(dead_code)]
enum PreviewData {
    Table(TableData),
    Text(String),
    Markdown(String),
    Code { lang: String, body: String },
    Latex { tex: String, display: bool },
    File { path: String, kind: String },
    Smiles(String),
    Fasta(String),
}

#[derive(Clone, PartialEq)]
struct Artifact {
    id: String,
    name: String,
    kind: &'static str,
    data: PreviewData,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct FileContent {
    path: String,
    mime: String,
    text: Option<String>,
    base64: Option<String>,
}

#[derive(Deserialize, Clone)]
struct DirEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
struct ProjectInfo {
    name: String,
    root: String,
    skill_count: usize,
    mcp_server_count: usize,
    memory_file_count: usize,
    has_api_key: bool,
}

#[derive(Deserialize, Clone)]
struct SkillInfo {
    name: String,
    description: String,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
struct MemoryFile {
    name: String,
    preview: String,
    bytes: u64,
}

#[derive(Deserialize, Clone)]
struct BootstrapStatus {
    skills_loaded: usize,
    python_ok: bool,
    mcp_catalog: usize,
    uv_ok: bool,
    app_version: String,
    workspace: String,
    errors: Vec<String>,
}

#[derive(Deserialize, Clone)]
struct Capabilities {
    skills: Vec<SkillInfo>,
    mcp_servers: Vec<String>,
    memory_files: Vec<MemoryFile>,
    project: ProjectInfo,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
struct OnboardingState {
    show: bool,
    has_api_key: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RightTab { Artifacts, File, Provenance }

fn join_path(base: &str, name: &str) -> String {
    if base == "." || base.is_empty() { name.to_string() }
    else { format!("{}/{}", base.trim_end_matches(['/', '\\']), name) }
}

fn parent_path(path: &str) -> String {
    if path == "." || path.is_empty() { return ".".into(); }
    let p = path.replace('\\', "/");
    match p.rsplit_once('/') {
        None | Some(("", _)) => ".".into(),
        Some((a, _)) if a.is_empty() => ".".into(),
        Some((a, _)) => a.to_string(),
    }
}

fn format_bytes(n: u64) -> String {
    if n < 1024 { format!("{n} B") }
    else if n < 1024 * 1024 { format!("{:.1} KB", n as f64 / 1024.0) }
    else { format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)) }
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

fn event_target_value(ev: &web_sys::Event) -> String {
    use wasm_bindgen::JsCast;
    ev.target().unwrap().dyn_into::<web_sys::HtmlTextAreaElement>().unwrap().value()
}
fn event_target_input(ev: &web_sys::Event) -> web_sys::HtmlInputElement {
    use wasm_bindgen::JsCast;
    ev.target().unwrap().dyn_into::<web_sys::HtmlInputElement>().unwrap()
}

/// Render agent/assistant Markdown to HTML for `inner_html`. GFM tables,
/// strikethrough, task lists and footnotes are on; the source is trusted
/// (local agent output rendered in the desktop WebView).
fn md_to_html(src: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(src, opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Inline Markdown for table cells (bold, code, links, etc.).
fn md_inline_to_html(src: &str) -> String {
    if src.is_empty() { return String::new(); }
    let html = md_to_html(src);
    let s = html.trim();
    if let Some(inner) = s.strip_prefix("<p>").and_then(|rest| rest.strip_suffix("</p>")) {
        if !inner.contains("<p>") { return inner.to_string(); }
    }
    html
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn next_artifact_id(n: usize) -> String {
    format!("{:08x}", n + 1)
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

fn normalize_path(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .trim_start_matches('/')
        .trim_start_matches('\\')
        .to_string()
}

fn is_external_href(href: &str) -> bool {
    let h = href.trim();
    h.starts_with("http://")
        || h.starts_with("https://")
        || h.starts_with("mailto:")
        || h.starts_with('#')
        || h.starts_with("javascript:")
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

fn extract_href_from_tag(tag: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let i = lower.find("href=")?;
    let rest = &tag[i + 5..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
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

fn tool_lang(name: &str) -> &'static str {
    let n = name.trim().to_ascii_lowercase();
    match n.as_str() {
        "python" | "python3" => "python",
        "bash" | "shell" | "sh" => "bash",
        "javascript" | "js" => "javascript",
        "json" => "json",
        "sql" => "sql",
        "rust" => "rust",
        "r" => "r",
        _ => "plaintext",
    }
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

fn schedule_highlight(id: String) {
    spawn_local(async move {
        let _ = highlight_by_id(&id).await;
    });
}

fn refresh_sessions(sessions: RwSignal<Vec<SessionInfo>>) {
    spawn_local(async move {
        let v = invoke("list_sessions", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<SessionInfo>>(v) {
            sessions.set(list);
        }
    });
}

// --- Artifact detection (Markdown tables + fenced CSV) -----------------------

fn split_row(line: &str) -> Vec<String> {
    line.trim().trim_start_matches('|').trim_end_matches('|')
        .split('|').map(|c| c.trim().to_string()).collect()
}
fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.contains('|')
}
fn is_separator(line: &str) -> bool {
    let cells = split_row(line);
    !cells.is_empty() && cells.iter().all(|c| {
        let c = c.trim();
        !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':') && c.contains('-')
    })
}

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

fn parse_csv_line(line: &str) -> Vec<String> {
    line.split(',').map(|c| c.trim().trim_matches('"').to_string()).collect()
}

fn file_kind(path: &str) -> Option<&'static str> {
    let (_, ext) = path.rsplit_once('.')?;
    if ext.is_empty() { return None; }
    let ext = ext.to_ascii_lowercase();
    Some(match ext.as_str() {
        "csv" | "tsv" => "csv",
        "pdf" => "pdf",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "image",
        "pdb" | "mol2" | "cif" => "structure",
        "sdf" | "mol" => "molecule",
        "smi" | "smiles" => "smiles",
        // Alignment formats → interactive MSA viewer (web-dist Vae)
        "aln" | "clustal" | "clustalw" | "sto" | "stockholm" | "stk" | "afa" | "mfa" => "msa",
        // Plain FASTA → syntax-highlighted text (web-dist Hae → text preview)
        "fasta" | "fa" | "fas" | "fna" | "faa" | "ffn" | "frn" => "fasta",
        "md" => "markdown",
        "nwk" | "newick" | "treefile" | "tre" => "text",
        "txt" | "log" | "json" => "text",
        _ => return None,
    })
}

fn fasta_seq_count(text: &str) -> usize {
    text.lines().filter(|l| l.trim_start().starts_with('>')).count()
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
    if let Some(i) = items.iter().rposition(|i| matches!(i, ChatItem::Assistant(_))) {
        if let ChatItem::Assistant(s) = &mut items[i] {
            if s.is_empty() {
                s.push_str(text);
                return;
            }
        }
    }
    items.push(ChatItem::Assistant(text.to_string()));
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
            ChatItem::Assistant(s) => collect_markdown_artifacts(&mut out, &mut seen, s, locale, &mut scan),
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
            let v = invoke("read_file", to_value(&serde_json::json!({ "path": path })).unwrap()).await;
            let Ok(fc) = serde_wasm_bindgen::from_value::<FileContent>(v) else {
                if let Some(el) = el {
                    el.set_class_name("rp-heavy rp-error");
                    el.set_text_content(Some(&tf(loc, "err.file_not_found", &[("path", &path)])));
                }
                return;
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
        <div class="role">{move || t(locale.get(), "chat.assistant")}</div>
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

#[component]
fn App() -> impl IntoView {
    let locale = create_rw_signal(Locale::detect_browser());
    provide_context(locale.read_only());

    let items = create_rw_signal::<Vec<ChatItem>>(vec![]);
    let input = create_rw_signal(String::new());
    let attachments = create_rw_signal::<Vec<ComposerAttachment>>(vec![]);
    let uploading = create_rw_signal(false);
    let drag_over = create_rw_signal(false);
    let busy = create_rw_signal(false);
    let show_settings = create_rw_signal(false);
    let settings = create_rw_signal(Settings::default());
    let api_key_input = create_rw_signal(String::new());
    let settings_busy = create_rw_signal(false);
    let settings_message = create_rw_signal::<Option<(bool, String)>>(None);
    let status = create_rw_signal(String::new());
    let show_demos = create_rw_signal(false);
    let demos = create_rw_signal::<Vec<DemoInfo>>(vec![]);

    // Session history (left sidebar).
    let sessions = create_rw_signal::<Vec<SessionInfo>>(vec![]);
    let active_session = create_rw_signal::<Option<String>>(None);
    refresh_sessions(sessions);

    // Three-pane layout state (mirrors web-dist: sidebar / conversation / right pane).
    let show_sidebar = create_rw_signal(true);
    let show_right = create_rw_signal(true);
    let right_w = create_rw_signal(440.0_f64);
    let dragging = create_rw_signal(false);
    let drag_start_x = create_rw_signal(0.0_f64);
    let drag_start_w = create_rw_signal(0.0_f64);

    // Artifacts (right pane): tables + CSV detected in the transcript.
    let artifacts = create_memo(move |_| collect_artifacts(&items.get(), locale.get()));
    let sel_artifact = create_rw_signal(0usize);
    let right_tab = create_rw_signal(RightTab::Artifacts);
    let show_files = create_rw_signal(false);
    let file_cwd = create_rw_signal(".".to_string());
    let file_entries = create_rw_signal::<Vec<DirEntry>>(vec![]);
    let open_file = create_rw_signal::<Option<(String, String)>>(None);
    let project_info = create_rw_signal::<Option<ProjectInfo>>(None);
    let show_capabilities = create_rw_signal(false);
    let caps = create_rw_signal::<Option<Capabilities>>(None);
    let bootstrap = create_rw_signal::<Option<BootstrapStatus>>(None);
    let show_onboarding = create_rw_signal(false);
    let onboard_step = create_rw_signal(0usize);

    let on_artifact_select = Callback::new(move |idx: usize| {
        let arts = artifacts.get();
        if let Some(a) = arts.get(idx) {
            show_right.set(true);
            if let PreviewData::File { path, kind } = &a.data {
                right_tab.set(RightTab::File);
                open_file.set(Some((path.clone(), kind.clone())));
            } else {
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
    });

    create_effect(move |_| {
        attach_chat_scroll(CHAT_SCROLLER_ID, CHAT_THREAD_ID);
    });
    create_effect(move |_| {
        let _ = items.get();
        schedule_chat_follow();
    });

    // Wire the agent event stream once.
    let items_cb = items;
    let busy_cb = busy;
    let status_cb = status;
    let locale_cb = locale;
    let cb = Closure::wrap(Box::new(move |payload: JsValue| {
        let ev: AgentEvent = match serde_wasm_bindgen::from_value(payload) {
            Ok(e) => e,
            Err(err) => {
                web_sys::console::log_1(&format!("agent event decode error: {err:?}").into());
                return;
            }
        };
        match ev {
            AgentEvent::Text { delta, .. } => items_cb.update(|v| {
                match v.last_mut() {
                    Some(ChatItem::Assistant(s)) => s.push_str(&delta),
                    _ => v.push(ChatItem::Assistant(delta)),
                }
            }),
            AgentEvent::Reasoning { delta, .. } => items_cb.update(|v| {
                match v.last_mut() {
                    Some(ChatItem::Reasoning(s)) => s.push_str(&delta),
                    _ => v.push(ChatItem::Reasoning(delta)),
                }
            }),
            AgentEvent::ToolCall { name, preview, .. } => items_cb.update(|v| {
                v.push(ChatItem::Tool { name, ok: None, input: preview, output: String::new() })
            }),
            AgentEvent::ToolResult { name, ok, content, .. } => items_cb.update(|v| {
                let idx = v.iter().rposition(|c| matches!(c, ChatItem::Tool { name: n, ok: None, .. } if n == &name));
                if let Some(i) = idx {
                    if let ChatItem::Tool { ok: o, output, .. } = &mut v[i] {
                        *o = Some(ok);
                        *output = content.clone();
                    }
                } else {
                    v.push(ChatItem::Tool { name: name.clone(), ok: Some(ok), input: String::new(), output: content.clone() });
                }
                if name == "attempt_completion" && ok {
                    promote_assistant_text(v, &content);
                }
            }),
            AgentEvent::Usage { input, output, ctx_tokens, max_context, .. } => {
                let pct = if max_context > 0 { ctx_tokens * 100 / max_context } else { 0 };
                let loc = locale_cb.get();
                status_cb.set(tf(loc, "status.usage", &[
                    ("in", &format!("{:.1}", input as f64 / 1000.0)),
                    ("out", &format!("{:.1}", output as f64 / 1000.0)),
                    ("pct", &pct.to_string()),
                ]));
            }
            AgentEvent::Compaction { before, after, .. } => {
                status_cb.set(tf(locale_cb.get(), "status.compact", &[
                    ("before", &before.to_string()),
                    ("after", &after.to_string()),
                ]));
            }
            AgentEvent::Stdout { chunk, .. } => items_cb.update(|v| match v.last_mut() {
                Some(ChatItem::Tool { output, .. }) => output.push_str(&chunk),
                _ => v.push(ChatItem::Tool { name: "stdout".into(), ok: None, input: String::new(), output: chunk }),
            }),
            AgentEvent::Done { .. } => { busy_cb.set(false); refresh_sessions(sessions); }
            AgentEvent::Error { message, .. } => { items_cb.update(|v| v.push(ChatItem::Assistant(format!("Error: {message}")))); busy_cb.set(false); }
            AgentEvent::Diff { .. } => {}
        }
    }) as Box<dyn FnMut(JsValue)>);
    let agent_js = cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(cb);
    // wasm-bindgen only runs an async extern's JS body when the returned
    // future is polled, so we must await `listen` (not fire-and-forget it).
    spawn_local(async move { let _ = listen("agent", &agent_js).await; });

    // Confirm handler: the backend denies on timeout, so the UI MUST surface
    // confirm-request. We render an inline Approve/Deny and call
    // confirm_response.
    let confirm_state = create_rw_signal::<Option<(String, String)>>(None);
    let confirm_cb = Closure::wrap(Box::new(move |payload: JsValue| {
        if let Ok(v) = serde_wasm_bindgen::from_value::<serde_json::Value>(payload) {
            let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
            let fid = v.get("frame_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
            if !msg.is_empty() { confirm_state.set(Some((fid, msg))); }
        }
    }) as Box<dyn FnMut(JsValue)>);
    let confirm_js = confirm_cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    std::mem::forget(confirm_cb);
    spawn_local(async move { let _ = listen("confirm-request", &confirm_js).await; });

    let stop = move |_| {
        spawn_local(async {
            let _ = invoke("stop_agent", JsValue::UNDEFINED).await;
        });
    };

    let send = move || {
        let text = input.get();
        let paths = attachment_paths(&attachments.get());
        let message = message_with_attachments(&text, &paths);
        if message.trim().is_empty() || busy.get() || uploading.get() { return; }
        items.update(|v| { v.push(ChatItem::User(message.clone())); v.push(ChatItem::Assistant(String::new())); });
        force_chat_bottom();
        input.set(String::new());
        attachments.set(vec![]);
        busy.set(true);
        let args = to_value(&SendArgs { message: "" }).unwrap();
        // Re-serialize with the real text (SendArgs borrows; build a fresh value).
        let arg = to_value(&serde_json::json!({ "message": message })).unwrap();
        let _ = args;
        spawn_local(async move {
            if let Err(err) = invoke_checked("send_message", arg).await {
                let loc = locale.get();
                status.set(tf(loc, "status.send_failed", &[("msg", &localize_backend(loc, &js_error_text(err)))]));
                busy.set(false);
            }
        });
    };

    let on_send = move |_ev: web_sys::KeyboardEvent| {
        if _ev.key() == "Enter" && !_ev.shift_key() { _ev.prevent_default(); send(); }
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
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "user_index": user_idx })).unwrap();
            let _ = invoke("rewind_session", arg).await;
        });
    };

    let pick_files = move |_| {
        if busy.get() || uploading.get() {
            return;
        }
        let Some(window) = web_sys::window() else { return; };
        let Some(doc) = window.document() else { return; };
        let Some(el) = doc.get_element_by_id("composer-file-input") else { return; };
        let _ = el.dyn_ref::<web_sys::HtmlElement>().map(|e| e.click());
    };

    let on_files_selected = move |_ev: web_sys::Event| {
        if busy.get() || uploading.get() {
            return;
        }
        upload_from_input(attachments, uploading, "composer-file-input");
    };

    let on_drag_over = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        if !busy.get() && !uploading.get() {
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
        if busy.get() || uploading.get() {
            return;
        }
        if let Some(dt) = ev.data_transfer() {
            if let Some(files) = dt.files() {
                queue_uploads(attachments, uploading, files.into());
            }
        }
    };

    let composer_blocked = move || busy.get() || uploading.get();

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

    let open_settings = move |_| {
        show_settings.set(true);
        settings_message.set(None);
        let s = settings;
        let api_key_input = api_key_input;
        let msg = settings_message;
        let loc = locale;
        spawn_local(async move {
            let v = invoke("get_settings", JsValue::UNDEFINED).await;
            if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                let cfg = normalized_settings(cfg);
                let l = Locale::from_code(&cfg.locale);
                loc.set(l);
                set_document_lang(l);
                api_key_input.set(if cfg.has_api_key { t(l, "settings.stored_key").into() } else { String::new() });
                s.set(cfg);
            } else {
                msg.set(Some((false, t(loc.get(), "status.failed_load_settings").into())));
            }
        });
    };

    let save_settings = move |_| {
        if settings_busy.get() { return; }
        let mut cfg = normalized_settings(settings.get());
        cfg.locale = locale.get().code().into();
        let key = api_key_input.get();
        let s = settings;
        let show = show_settings;
        let busy = settings_busy;
        let msg = settings_message;
        let status_msg = status;
        let loc = locale;
        if let Some(err_key) = settings_required_error_key(&cfg, &key) {
            let err = t(loc.get(), err_key);
            let text = tf(loc.get(), "status.save_failed", &[("msg", &err)]);
            msg.set(Some((false, text.clone())));
            status_msg.set(text);
            return;
        }
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
            let saved_new_key = !key.is_empty() && !is_stored_key_placeholder(&key, loc.get());
            if saved_new_key {
                if let Err(err) = invoke_checked("set_api_key", to_value(&serde_json::json!({ "key": key })).unwrap()).await {
                    let l = loc.get();
                    let text = tf(l, "status.api_key_save_failed", &[("msg", &localize_backend(l, &js_error_text(err)))]);
                    msg.set(Some((false, text.clone())));
                    status_msg.set(text);
                    busy.set(false);
                    return;
                }
            }
            busy.set(false);
            show.set(false);
            status_msg.set(t(loc.get(), "status.settings_saved").into());
            if saved_new_key {
                cfg.has_api_key = true;
            }
            s.set(cfg);
        });
    };

    let validate_settings = move |_| {
        if settings_busy.get() { return; }
        let cfg = normalized_settings(settings.get());
        let key = api_key_input.get();
        let busy = settings_busy;
        let msg = settings_message;
        let status_msg = status;
        let loc = locale;
        if let Some(err_key) = settings_required_error_key(&cfg, &key) {
            let err = t(loc.get(), err_key);
            let text = tf(loc.get(), "status.validation_failed", &[("msg", &err)]);
            msg.set(Some((false, text.clone())));
            status_msg.set(text);
            return;
        }
        busy.set(true);
        let validating = t(loc.get(), "status.validating").to_string();
        msg.set(Some((true, validating.clone())));
        status_msg.set(validating);
        spawn_local(async move {
            let res = invoke_timeout(
                "validate_settings",
                to_value(&serde_json::json!({ "settings": cfg, "key": key })).unwrap(),
                35_000,
            ).await;
            match res {
                Ok(v) => {
                    let l = loc.get();
                    let raw = v.as_string().unwrap_or_else(|| t(l, "status.validation_succeeded").into());
                    let text = localize_backend(l, &raw);
                    msg.set(Some((true, text.clone())));
                    status_msg.set(text);
                }
                Err(err) => {
                    let l = loc.get();
                    let text = tf(l, "status.validation_failed", &[("msg", &localize_backend(l, &js_error_text(err)))]);
                    msg.set(Some((false, text.clone())));
                    status_msg.set(text);
                }
            }
            busy.set(false);
        });
    };

    let new_session = move |_| {
        items.set(vec![]);
        // ponytail: mid-upload switch can still re-add chips when the upload
        // finishes; add a generation guard if that ever bites.
        attachments.set(vec![]);
        active_session.set(None);
        sel_artifact.set(0);
        open_file.set(None);
        right_tab.set(RightTab::Artifacts);
        spawn_local(async move {
            let _ = invoke("new_session", JsValue::UNDEFINED).await;
            refresh_sessions(sessions);
        });
    };

    let start_env_setup = {
        let items = items;
        let busy = busy;
        let status = status;
        let locale = locale;
        let show_capabilities = show_capabilities;
        let active_session = active_session;
        let sel_artifact = sel_artifact;
        let open_file = open_file;
        let right_tab = right_tab;
        let sessions = sessions;
        move |_| {
            if busy.get() { return; }
            show_capabilities.set(false);
            attachments.set(vec![]);
            active_session.set(None);
            sel_artifact.set(0);
            open_file.set(None);
            right_tab.set(RightTab::Artifacts);
            let text: String = t(locale.get(), "caps.env_setup_prompt").into();
            items.set(vec![
                ChatItem::User(text.clone()),
                ChatItem::Assistant(String::new()),
            ]);
            force_chat_bottom();
            busy.set(true);
            spawn_local(async move {
                let _ = invoke("new_session", JsValue::UNDEFINED).await;
                refresh_sessions(sessions);
                let arg = to_value(&serde_json::json!({ "message": text })).unwrap();
                if let Err(err) = invoke_checked("send_message", arg).await {
                    let loc = locale.get();
                    status.set(tf(loc, "status.send_failed", &[("msg", &localize_backend(loc, &js_error_text(err)))]));
                    busy.set(false);
                }
            });
        }
    };

    let load_session = Callback::new(move |id: String| {
        attachments.set(vec![]);
        active_session.set(Some(id.clone()));
        sel_artifact.set(0);
        open_file.set(None);
        right_tab.set(RightTab::Artifacts);
        busy.set(true);
        spawn_local(async move {
            let v = invoke("load_session", to_value(&serde_json::json!({ "id": id })).unwrap()).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<LoadedItem>>(v) {
                items.set(list.into_iter().map(LoadedItem::into_chat).collect());
                force_chat_bottom();
            }
            busy.set(false);
        });
    });

    let open_demos = move |_| {
        let d = demos;
        let show = show_demos;
        spawn_local(async move {
            let v = invoke("list_demos", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DemoInfo>>(v) {
                d.set(list);
                show.set(true);
            }
        });
    };

    let load_demo = move |info: DemoInfo| {
        let id = info.id.clone();
        let show = show_demos;
        let items = items;
        let busy = busy;
        show.set(false);
        busy.set(true);
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
                view.push(ChatItem::Assistant(demo.response.clone()));
                items.set(view);
                force_chat_bottom();
                status_cb.set(tf(locale.get(), "status.demo", &[("title", &demo.title)]));
            }
            busy.set(false);
        });
    };

    let respond_confirm = Callback::new(move |approved: bool| {
        confirm_state.set(None);
        let arg = to_value(&serde_json::json!({ "approved": approved })).unwrap();
        spawn_local(async move { let _ = invoke("confirm_response", arg).await; });
    });

    let approve = move |v: bool| move |_| respond_confirm.call(v);

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
        show_files.set(true);
        refresh_dir(file_cwd, file_entries);
    };

    let open_capabilities = move |_| {
        show_capabilities.set(true);
        let c = caps;
        spawn_local(async move {
            let v = invoke("get_capabilities", JsValue::UNDEFINED).await;
            if let Ok(data) = serde_wasm_bindgen::from_value::<Capabilities>(v) {
                c.set(Some(data));
            }
        });
    };

    let dismiss_onboarding = Callback::new(move |_| {
        show_onboarding.set(false);
        spawn_local(async move { let _ = invoke("dismiss_onboarding", JsValue::UNDEFINED).await; });
    });
    let dismiss_onboard = move |_| dismiss_onboarding.call(());

    let ctx_menu = create_rw_signal::<Option<CtxMenu>>(None);
    let open_session = load_session.clone();
    let on_ctx_pick = Callback::new(move |(action, payload): (String, String)| {
        if let Some(id) = context_menu::session_action(&action, &payload) {
            open_session.call(id);
        }
        context_menu::run_action(&action, &payload, copy_text);
    });
    let on_context_menu = move |ev: web_sys::MouseEvent| {
        if context_menu::dev_mode() {
            return;
        }
        ev.prevent_default();
        let loc = locale.get();
        if let Some(menu) = context_menu::build(&ev, loc) {
            ctx_menu.set(if menu.items.is_empty() { None } else { Some(menu) });
        }
    };

    window_event_listener(ev::keydown, move |ev| {
        let Some(ev) = ev.dyn_ref::<web_sys::KeyboardEvent>() else { return };
        if ev.key() != "Escape" || ev.default_prevented() || ev.is_composing() {
            return;
        }

        if confirm_state.get().is_some() {
            ev.prevent_default();
            respond_confirm.call(false);
            return;
        }
        if ctx_menu.get().is_some() {
            ev.prevent_default();
            ctx_menu.set(None);
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
        if show_demos.get() {
            ev.prevent_default();
            show_demos.set(false);
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

    view! {
        <div class="app" on:contextmenu=on_context_menu>
        <aside class="sidebar" class:collapsed=move || !show_sidebar.get()>
            <div class="brand">
                <span class="brand-name">"Wisp Science"</span>
                <span class="brand-beta">"Beta"</span>
                <button class="icon-btn" title=move || t(locale.get(), "sidebar.collapse") on:click=move |_| show_sidebar.set(false)>"‹"</button>
            </div>
            <button class="proj-switch" on:click=open_capabilities>
                <span class="proj-name">{move || project_info.get().map(|p| p.name.clone()).unwrap_or_else(|| "wisp-science".into())}</span>
                <span class="caret">"▾"</span>
            </button>
            <nav class="nav">
                <button class="side-btn primary" on:click=new_session><span class="gi plus"></span>{move || t(locale.get(), "sidebar.new_session")}</button>
                <button class="side-btn" on:click=open_demos><span class="gi grid"></span>{move || t(locale.get(), "sidebar.open_demo")}</button>
                <button class="side-btn" on:click=open_files><span class="gi doc"></span>{move || t(locale.get(), "sidebar.files")}</button>
            </nav>
            <div class="side-list">
                {move || {
                    let list = sessions.get();
                    let loc = locale.get();
                    if list.is_empty() {
                        return view! { <div class="side-hint">{t(loc, "sidebar.no_sessions")}</div> }.into_view();
                    }
                    let make = move |s: &SessionInfo| {
                        let id = s.id.clone();
                        let id_active = id.clone();
                        let id_attr = id.clone();
                        let title = if s.title.trim().is_empty() { t(loc, "sidebar.untitled").into() } else { s.title.clone() };
                        let title_attr = title.clone();
                        let open = load_session.clone();
                        view! {
                            <button class="side-item ses"
                                class:active=move || active_session.get().as_deref() == Some(id_active.as_str())
                                data-session-id=id_attr
                                data-session-title=title_attr
                                on:click=move |_| open.call(id.clone())>
                                <span class="dot"></span>
                                <span class="ses-title">{title}</span>
                            </button>
                        }.into_view()
                    };
                    // ponytail: bucket by 24h recency (Today / Earlier); calendar-day
                    // grouping if session timestamps ever gain finer granularity.
                    let now_ms = js_sys::Date::now();
                    let (mut today, mut earlier) = (Vec::new(), Vec::new());
                    for s in &list {
                        let ts_ms = if s.ts > 1_000_000_000_000 { s.ts as f64 } else { s.ts as f64 * 1000.0 };
                        if s.ts > 0 && ts_ms >= now_ms - 86_400_000.0 { today.push(s.clone()); }
                        else { earlier.push(s.clone()); }
                    }
                    view! {
                        {(!today.is_empty()).then(|| view! {
                            <div class="side-group-title">{t(loc, "sidebar.today")}</div>
                            {today.iter().map(&make).collect_view()}
                        })}
                        {(!earlier.is_empty()).then(|| view! {
                            <div class="side-group-title">{t(loc, "sidebar.earlier")}</div>
                            {earlier.iter().map(&make).collect_view()}
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
                <span class="hint">{move || status.get()}</span>
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
                    {move || {
                        let arts = artifacts.get();
                        let pick = on_artifact_select.clone();
                        let open_link = on_file_link.clone();
                        let is_busy = busy.read_only();
                        items.get().into_iter().enumerate().map(|(i, item)| view! {
                            <div class=format!("{}", class_for(&item)) key=i>
                                {render_item(i, &item, &arts, pick.clone(), open_link.clone(), is_busy, edit_message)}
                            </div>
                        }.into_view()).collect_view()
                    }}
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
                    <textarea
                        id="composer-input"
                        prop:value={move || input.get()}
                        on:input=move|ev| input.set(event_target_value(&ev))
                        on:keydown=on_send
                        prop:placeholder=move || t(locale.get(), "composer.placeholder")
                    ></textarea>
                    <div class="composer-actions">
                        <span class="composer-hint">{move || t(locale.get(), "composer.hint")}</span>
                        <div class="composer-buttons">
                            <button type="button" class="attach" disabled=composer_blocked
                                title=move || t(locale.get(), "composer.attach")
                                on:click=pick_files>{move || t(locale.get(), "composer.attach")}</button>
                            {move || busy.get().then(|| view! {
                                <button type="button" class="stop" on:click=stop>{move || t(locale.get(), "composer.stop")}</button>
                            })}
                            <button class="send" disabled=composer_blocked on:click=move |_| send()>{move || t(locale.get(), "composer.send")}</button>
                        </div>
                    </div>
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
                                let sel = sel_artifact.get().min(arts.len() - 1);
                                let tiles = arts.iter().enumerate().map(|(i, a)| {
                                    let name = a.name.clone();
                                    let kind = a.kind.to_string();
                                    let meta = artifact_meta(a, loc);
                                    view! {
                                        <button class="rp-tile" class:active=move || sel_artifact.get() == i
                                            data-artifact-name=name.clone()
                                            on:click=move |_| sel_artifact.set(i)>
                                            <span class="rp-tile-text">
                                                <span class="rp-tile-name">{name}</span>
                                                <span class="rp-tile-meta">{meta}</span>
                                            </span>
                                            <span class=format!("rp-badge {}", kind)>{kind.clone()}</span>
                                        </button>
                                    }.into_view()
                                }).collect_view();
                                let cur = arts[sel].clone();
                                let dom_id = format!("rp-{sel}");
                                view! {
                                    <div class="rp-artifacts-body">
                                        <div class="rp-tiles">{tiles}</div>
                                        <div class="rp-view">
                                            <div class="rp-view-head">
                                                <span class=format!("rp-badge {}", cur.kind)>{cur.kind.to_string()}</span>
                                                <span class="rp-view-name">{cur.name.clone()}</span>
                                            </div>
                                            {artifact_preview(&cur, dom_id, loc)}
                                        </div>
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
                    }}
                </div>
            </section>
        }.into_view())}

        {move || dragging.get().then(|| view! {
            <div class="drag-overlay"
                on:mousemove=on_resize_move
                on:mouseup=move |_| dragging.set(false)></div>
        })}

        {move || confirm_state.get().map(|(fid, msg)| view! {
            <div class="overlay" key=fid>
                <div class="modal">
                    <h2>{move || t(locale.get(), "confirm.title")}</h2>
                    <div class="hint">{msg}</div>
                    <div class="row">
                        <button on:click=approve(false)>{move || t(locale.get(), "confirm.deny")}</button>
                        <button class="primary" on:click=approve(true)>{move || t(locale.get(), "confirm.approve")}</button>
                    </div>
                </div>
            </div>
        }.into_view())}

        {move || show_demos.get().then(|| view! {
            <div class="overlay">
                <div class="modal">
                    <h2>{move || t(locale.get(), "demos.title")}</h2>
                    <span class="hint">{move || t(locale.get(), "demos.hint")}</span>
                    <div class="demo-list">
                        {move || demos.get().into_iter().map(|d| {
                            let d_click = d.clone();
                            view! {
                                <button class="demo-item" key=d.id.clone() on:click=move |_| load_demo(d_click.clone())>
                                    {d.title.clone()}
                                </button>
                            }.into_view()
                        }).collect_view()}
                    </div>
                    <div class="row">
                        <button on:click=move |_| show_demos.set(false)>{move || t(locale.get(), "demos.close")}</button>
                    </div>
                </div>
            </div>
        }.into_view())}

        {move || show_settings.get().then(|| view! {
            <div class="overlay">
                <div class="modal">
                    <h2>{move || t(locale.get(), "settings.title")}</h2>
                    <label>{move || t(locale.get(), "settings.language")}
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
                    <label>{move || t(locale.get(), "settings.provider")}
                        <select data-testid="settings-provider"
                            on:input=move|ev| apply_provider_defaults(settings, dom_value(&ev))
                            on:change=move|ev| apply_provider_defaults(settings, dom_value(&ev))
                            prop:value={move || provider_value(&settings.get().provider).to_string()}>
                            <option value="openai">{move || t(locale.get(), "settings.provider.openai")}</option>
                            <option value="openai_responses">{move || t(locale.get(), "settings.provider.openai_responses")}</option>
                            <option value="anthropic">{move || t(locale.get(), "settings.provider.anthropic")}</option>
                        </select>
                    </label>
                    <label>{move || t(locale.get(), "settings.api_url")}
                        <input on:input=move|ev| settings.update(|s| {
                                normalize_settings_mut(s);
                                s.api_url = event_target_input(&ev).value();
                            })
                            prop:value={move || settings.get().api_url} />
                    </label>
                    <label>{move || t(locale.get(), "settings.model")}
                        <input on:input=move|ev| settings.update(|s| {
                                normalize_settings_mut(s);
                                s.model = event_target_input(&ev).value();
                            })
                            prop:value={move || settings.get().model} />
                    </label>
                    <label>{move || t(locale.get(), "settings.api_key")}
                        <input on:input=move|ev| api_key_input.set(event_target_input(&ev).value())
                            prop:value={move || api_key_input.get()} type="password" />
                    </label>
                    <span class="hint">{move || t(locale.get(), "settings.tip")}</span>
                    {move || settings_message.get().map(|(ok, text)| view! {
                        <div class="settings-status"
                            class:ok=move || ok
                            class:fail=move || !ok>{text}</div>
                    })}
                    <div class="row">
                        <button type="button" disabled=move || settings_busy.get() on:click=check_updates>{move || t(locale.get(), "settings.check_updates")}</button>
                        <button type="button" disabled=move || settings_busy.get() on:click=validate_settings>{move || t(locale.get(), "settings.validate")}</button>
                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                        <button type="button" class="primary" disabled=move || settings_busy.get() on:click=save_settings>{move || t(locale.get(), "settings.save")}</button>
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
                                    <button class="fb-up" on:click=move |_| { file_cwd.set(p_click.clone()); refresh_dir(file_cwd, file_entries); }>"↑"</button>
                                }.into_view()
                            })}
                            <span class="fb-path">{cwd.clone()}</span>
                        </div>
                        <div class="fb-list">
                            {move || file_entries.get().into_iter().map(|e| {
                                let name = e.name.clone();
                                let full = join_path(&file_cwd.get(), &name);
                                if e.is_dir {
                                    let full_click = full.clone();
                                    view! {
                                        <button class="fb-row dir" on:click=move |_| {
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
                            }).collect_view()}
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
                        <div class="cap-grid">
                            <div class="cap-stat"><span class="cap-num">{c.project.skill_count}</span><span class="cap-label">{move || t(locale.get(), "caps.skills")}</span></div>
                            <div class="cap-stat"><span class="cap-num">{c.mcp_servers.len()}</span><span class="cap-label">{move || t(locale.get(), "caps.mcp_servers")}</span></div>
                            <div class="cap-stat"><span class="cap-num">{c.memory_files.len()}</span><span class="cap-label">{move || t(locale.get(), "caps.memory_files")}</span></div>
                        </div>
                        <div class="cap-section">
                            <h3>{move || t(locale.get(), "caps.mcp_bio")}</h3>
                            <div class="cap-tags">
                                {c.mcp_servers.iter().map(|s| view! { <span class="cap-tag">{s.clone()}</span> }).collect_view()}
                            </div>
                        </div>
                        <div class="cap-section">
                            <h3>{move || t(locale.get(), "caps.skills_section")}</h3>
                            <div class="cap-skills">
                                {c.skills.iter().map(|s| view! {
                                    <div class="cap-skill">
                                        <span class="cap-skill-name">{s.name.clone()}</span>
                                        <span class="cap-skill-desc">{s.description.clone()}</span>
                                    </div>
                                }).collect_view()}
                            </div>
                        </div>
                        <div class="cap-section">
                            <h3>{move || t(locale.get(), "caps.permissions")}</h3>
                            <p class="hint">{move || t(locale.get(), "caps.permissions_hint")}</p>
                        </div>
                    })}
                    <div class="row">
                        <button on:click=move |_| show_capabilities.set(false)>{move || t(locale.get(), "caps.close")}</button>
                        {move || bootstrap.get().filter(|b| !b.python_ok || !b.uv_ok).map(|_| view! {
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

fn class_for(item: &ChatItem) -> &'static str {
    match item {
        ChatItem::User(_) => "msg user",
        ChatItem::Assistant(s) if s.starts_with("Error: ") => "tool-wrap",
        ChatItem::Assistant(_) => "msg assistant",
        ChatItem::Reasoning(_) => "msg reasoning",
        ChatItem::Tool { .. } => "tool-wrap",
    }
}

fn render_item(
    ui_index: usize,
    item: &ChatItem,
    artifacts: &[Artifact],
    on_artifact: Callback<usize>,
    on_file: Callback<(String, String)>,
    busy: ReadSignal<bool>,
    on_edit: impl Fn(usize) + Clone + 'static,
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
        ChatItem::Assistant(s) if s.trim().is_empty() => view! {}.into_view(),
        ChatItem::Assistant(s) if s.starts_with("Error: ") => {
            let msg = s.strip_prefix("Error: ").unwrap_or(s).to_string();
            view! {
                <div class="finding err">
                    <div class="finding-head">
                        <span class="finding-tag">{move || format!("● {}", t(locale.get(), "chat.error"))}</span>
                        <span class="finding-title">{msg}</span>
                    </div>
                </div>
            }.into_view()
        }
        ChatItem::Assistant(s) => view! {
            <AssistantMessage
                text=s.clone()
                artifacts=artifacts.to_vec()
                on_artifact=on_artifact
                on_file=on_file
                on_copy=Callback::new(copy_text)
            />
        }.into_view(),
        ChatItem::Tool { name, .. } if name == "attempt_completion" => view! {}.into_view(),
        ChatItem::Reasoning(s) => view! {
            <details class="rz">
                <summary>{move || t(locale.get(), "chat.thinking")}</summary>
                <div class="body">{s.clone()}</div>
            </details>
        }.into_view(),
        ChatItem::Tool { name, ok, input, output } => view! {
            <ToolBlock name=name.clone() ok=*ok input=input.clone() output=output.clone() />
        }.into_view(),
    }
}

pub fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}
