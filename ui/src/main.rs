use leptos::*;
use serde::{Deserialize, Serialize};
use serde_wasm_bindgen::to_value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen(module = "/src/api.js")]
extern "C" {
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
    #[wasm_bindgen(catch, js_name = invoke)]
    async fn invoke_checked(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    async fn listen(event: &str, cb: &js_sys::Function) -> JsValue;
    async fn mount_preview(kind: &str, el_id: &str, payload: &str) -> JsValue;
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
    Tool { name: String, ok: Option<bool>, content: String },
}

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    provider: String,
    api_url: String,
    model: String,
    has_api_key: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            api_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
            has_api_key: false,
        }
    }
}

fn js_error_text(err: JsValue) -> String {
    err.as_string()
        .or_else(|| js_sys::Reflect::get(&err, &JsValue::from_str("message")).ok().and_then(|v| v.as_string()))
        .unwrap_or_else(|| "Unknown error".into())
}

fn provider_value(provider: &str) -> &'static str {
    match provider.trim() {
        "anthropic" => "anthropic",
        _ => "openai",
    }
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

fn settings_required_error(cfg: &Settings, key: &str) -> Option<&'static str> {
    if cfg.api_url.trim().is_empty() {
        return Some("API URL is required.");
    }
    if cfg.model.trim().is_empty() {
        return Some("Model is required.");
    }
    let has_new_key = !key.trim().is_empty() && !key.starts_with("(stored");
    if !cfg.has_api_key && !has_new_key {
        return Some("API key is required.");
    }
    None
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
            "tool" => ChatItem::Tool { name: self.tool_name.unwrap_or_else(|| "tool".into()), ok: self.ok, content: self.text },
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
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "csv" | "tsv" => "csv",
        "pdf" => "pdf",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "image",
        "pdb" | "mol2" | "cif" => "structure",
        "sdf" | "mol" => "molecule",
        "smi" | "smiles" => "smiles",
        "fasta" | "fa" => "msa",
        "md" => "markdown",
        _ => return None,
    })
}

fn push_file_artifact(out: &mut Vec<Artifact>, seen: &mut std::collections::HashSet<String>, path: &str) {
    let p = path.trim().trim_matches('`').trim_matches('"').trim_matches('\'');
    if p.is_empty() || seen.contains(p) { return; }
    let Some(kind) = file_kind(p) else { return; };
    seen.insert(p.to_string());
    let name = p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
    out.push(Artifact { name, kind, data: PreviewData::File { path: p.to_string(), kind: kind.to_string() } });
}

/// Collect tables, code, latex, and file-path artifacts from the transcript.
fn collect_artifacts(items: &[ChatItem]) -> Vec<Artifact> {
    let mut out: Vec<Artifact> = vec![];
    let mut seen = std::collections::HashSet::<String>::new();
    let mut tbl_n = 0;
    let mut csv_n = 0;
    let mut code_n = 0;
    let mut tex_n = 0;

    for it in items {
        match it {
            ChatItem::Assistant(s) => {
                for seg in split_segments(s) {
                    if let Seg::Table(t) = seg {
                        tbl_n += 1;
                        out.push(Artifact { name: format!("Table {tbl_n}"), kind: "table", data: PreviewData::Table(t) });
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
                                csv_n += 1;
                                out.push(Artifact { name: format!("data-{csv_n}.csv"), kind: "csv", data: PreviewData::Table(TableData { headers, rows }) });
                            } else if lang == "fasta" || lang == "fa" {
                                out.push(Artifact { name: format!("alignment-{csv_n}.fasta"), kind: "msa", data: PreviewData::Fasta(body.join("\n")) });
                            } else {
                                code_n += 1;
                                out.push(Artifact { name: format!("Code {code_n}"), kind: "code", data: PreviewData::Code { lang, body: body.join("\n") } });
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
                        tex_n += 1;
                        out.push(Artifact { name: format!("Equation {tex_n}"), kind: "latex", data: PreviewData::Latex { tex: body.join("\n"), display: true } });
                        i = j + 1;
                        continue;
                    }
                    i += 1;
                }
                for word in s.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']') {
                    push_file_artifact(&mut out, &mut seen, word);
                }
            }
            ChatItem::Tool { content, .. } => {
                for word in content.split(|c: char| c.is_whitespace() || c == '\n' || c == '"' || c == '\'') {
                    push_file_artifact(&mut out, &mut seen, word);
                }
            }
            _ => {}
        }
    }
    out
}

fn table_view(t: &TableData) -> impl IntoView {
    let headers = t.headers.clone();
    let rows: Vec<Vec<String>> = t.rows.iter().take(500).cloned().collect();
    view! {
        <div class="tbl-wrap">
            <table class="tbl">
                <thead><tr>{headers.into_iter().map(|h| view! { <th>{h}</th> }).collect_view()}</tr></thead>
                <tbody>
                    {rows.into_iter().map(|r| view! {
                        <tr>{r.into_iter().map(|c| view! { <td>{c}</td> }).collect_view()}</tr>
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

fn artifact_meta(a: &Artifact) -> String {
    match &a.data {
        PreviewData::Table(t) => format!("{} rows × {} cols", t.rows.len(), t.headers.len()),
        PreviewData::Code { lang, body } => format!("{lang} · {} lines", body.lines().count()),
        PreviewData::File { path, .. } => path.clone(),
        PreviewData::Latex { .. } => "LaTeX".into(),
        PreviewData::Fasta(s) => format!("{} lines", s.lines().count()),
        PreviewData::Smiles(s) => s.chars().take(28).collect(),
        PreviewData::Text(s) | PreviewData::Markdown(s) => format!("{} chars", s.len()),
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

#[component]
fn FilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
    let id_for_effect = dom_id.clone();
    create_effect(move |_| {
        let path = path.clone();
        let kind = kind.clone();
        let dom_id = id_for_effect.clone();
        spawn_local(async move {
            let v = invoke("read_file", to_value(&serde_json::json!({ "path": path })).unwrap()).await;
            let Ok(fc) = serde_wasm_bindgen::from_value::<FileContent>(v) else { return; };
            if kind == "markdown" {
                if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(&dom_id)) {
                    el.set_inner_html(&md_to_html(fc.text.as_deref().unwrap_or("")));
                }
                return;
            }
            let (mount_kind, payload) = match kind.as_str() {
                "pdf" => ("pdf", serde_json::json!({ "b64": fc.base64 }).to_string()),
                "image" => ("image", serde_json::json!({ "b64": fc.base64, "mime": fc.mime }).to_string()),
                "structure" => ("structure", serde_json::json!({ "text": fc.text, "format": "pdb" }).to_string()),
                "molecule" | "smiles" => ("molecule", serde_json::json!({ "text": fc.text, "smiles": fc.text }).to_string()),
                _ => ("text", serde_json::json!({ "text": fc.text }).to_string()),
            };
            let _ = mount_preview(mount_kind, &dom_id, &payload).await;
        });
    });
    view! { <div class="rp-heavy" id=dom_id>"Loading…"</div> }
}

fn artifact_preview(a: &Artifact, dom_id: String) -> impl IntoView {
    match &a.data {
        PreviewData::Table(t) => table_view(t).into_view(),
        PreviewData::Text(s) => view! { <pre class="rp-pre">{s.clone()}</pre> }.into_view(),
        PreviewData::Markdown(s) => view! { <div class="md rp-md" inner_html=md_to_html(s)></div> }.into_view(),
        PreviewData::Code { lang, body } => view! {
            <div class="rp-code-head">{lang.clone()}</div>
            <pre class="rp-pre"><code>{body.clone()}</code></pre>
        }.into_view(),
        PreviewData::Latex { tex, display } => {
            let payload = serde_json::json!({ "tex": tex, "display": display }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="latex".to_string() payload=payload /> }.into_view()
        }
        PreviewData::Fasta(text) => {
            let payload = serde_json::json!({ "text": text, "length": text.lines().count() }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="msa".to_string() payload=payload /> }.into_view()
        }
        PreviewData::Smiles(s) => {
            let payload = serde_json::json!({ "smiles": s }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="molecule".to_string() payload=payload /> }.into_view()
        }
        PreviewData::File { path, kind } => view! {
            <FilePreview dom_id=dom_id path=path.clone() kind=kind.clone() />
        }.into_view(),
    }
}

#[component]
fn App() -> impl IntoView {
    let items = create_rw_signal::<Vec<ChatItem>>(vec![]);
    let input = create_rw_signal(String::new());
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
    let artifacts = create_memo(move |_| collect_artifacts(&items.get()));
    let sel_artifact = create_rw_signal(0usize);
    let right_tab = create_rw_signal(RightTab::Artifacts);
    let show_files = create_rw_signal(false);
    let file_cwd = create_rw_signal(".".to_string());
    let file_entries = create_rw_signal::<Vec<DirEntry>>(vec![]);
    let open_file = create_rw_signal::<Option<(String, String)>>(None);
    let project_info = create_rw_signal::<Option<ProjectInfo>>(None);
    let show_capabilities = create_rw_signal(false);
    let caps = create_rw_signal::<Option<Capabilities>>(None);
    let show_onboarding = create_rw_signal(false);
    let onboard_step = create_rw_signal(0usize);

    spawn_local(async move {
        let v = invoke("get_project_info", JsValue::UNDEFINED).await;
        if let Ok(p) = serde_wasm_bindgen::from_value::<ProjectInfo>(v) {
            project_info.set(Some(p));
        }
        let v = invoke("get_onboarding_state", JsValue::UNDEFINED).await;
        if let Ok(s) = serde_wasm_bindgen::from_value::<OnboardingState>(v) {
            if s.show { show_onboarding.set(true); }
        }
    });

    // Wire the agent event stream once.
    let items_cb = items;
    let busy_cb = busy;
    let status_cb = status;
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
            AgentEvent::Reasoning { delta, .. } => items_cb.update(|v| v.push(ChatItem::Reasoning(delta))),
            AgentEvent::ToolCall { name, preview, .. } => items_cb.update(|v| v.push(ChatItem::Tool { name, ok: None, content: preview })),
            AgentEvent::ToolResult { name, ok, content, .. } => items_cb.update(|v| {
                let idx = v.iter().rposition(|c| matches!(c, ChatItem::Tool { name: n, ok: None, .. } if n == &name));
                if let Some(i) = idx {
                    if let ChatItem::Tool { ok: o, content: c, .. } = &mut v[i] { *o = Some(ok); *c = content; }
                } else {
                    v.push(ChatItem::Tool { name, ok: Some(ok), content });
                }
            }),
            AgentEvent::Usage { input, output, ctx_tokens, max_context, .. } => {
                let pct = if max_context > 0 { ctx_tokens * 100 / max_context } else { 0 };
                status_cb.set(format!("{:.1}k in / {:.1}k out | ctx {}%", input as f64 / 1000.0, output as f64 / 1000.0, pct));
            }
            AgentEvent::Compaction { before, after, .. } => status_cb.set(format!("compact {} → {}", before, after)),
            AgentEvent::Stdout { chunk, .. } => items_cb.update(|v| match v.last_mut() { Some(ChatItem::Tool { content, .. }) => content.push_str(&chunk), _ => v.push(ChatItem::Tool { name: "stdout".into(), ok: None, content: chunk }) }),
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

    let send = move || {
        let text = input.get();
        if text.trim().is_empty() || busy.get() { return; }
        items.update(|v| { v.push(ChatItem::User(text.clone())); v.push(ChatItem::Assistant(String::new())); });
        input.set(String::new());
        busy.set(true);
        let args = to_value(&SendArgs { message: "" }).unwrap();
        // Re-serialize with the real text (SendArgs borrows; build a fresh value).
        let arg = to_value(&serde_json::json!({ "message": text })).unwrap();
        let _ = args;
        spawn_local(async move {
            let _ = invoke("send_message", arg).await;
        });
    };

    let on_send = move |_ev: web_sys::KeyboardEvent| {
        if _ev.key() == "Enter" && !_ev.shift_key() { _ev.prevent_default(); send(); }
    };

    let open_settings = move |_| {
        show_settings.set(true);
        settings_message.set(None);
        let s = settings;
        let api_key_input = api_key_input;
        let msg = settings_message;
        spawn_local(async move {
            let v = invoke("get_settings", JsValue::UNDEFINED).await;
            if let Ok(cfg) = serde_wasm_bindgen::from_value::<Settings>(v) {
                let cfg = normalized_settings(cfg);
                api_key_input.set(if cfg.has_api_key { "(stored — leave blank to keep)".into() } else { String::new() });
                s.set(cfg);
            } else {
                msg.set(Some((false, "Failed to load settings".into())));
            }
        });
    };

    let save_settings = move |_| {
        if settings_busy.get() { return; }
        let cfg = normalized_settings(settings.get());
        let key = api_key_input.get();
        let s = settings;
        let show = show_settings;
        let busy = settings_busy;
        let msg = settings_message;
        let status_msg = status;
        if let Some(err) = settings_required_error(&cfg, &key) {
            let text = format!("Save failed: {err}");
            msg.set(Some((false, text.clone())));
            status_msg.set(text);
            return;
        }
        busy.set(true);
        msg.set(Some((true, "Saving settings...".into())));
        status_msg.set("Saving settings...".into());
        spawn_local(async move {
            let settings_result = invoke_checked(
                "set_settings",
                to_value(&serde_json::json!({ "settings": cfg.clone() })).unwrap(),
            ).await;
            if let Err(err) = settings_result {
                let text = format!("Save failed: {}", js_error_text(err));
                msg.set(Some((false, text.clone())));
                status_msg.set(text);
                busy.set(false);
                return;
            }
            if !key.is_empty() && !key.starts_with("(stored") {
                if let Err(err) = invoke_checked("set_api_key", to_value(&serde_json::json!({ "key": key })).unwrap()).await {
                    let text = format!("API key save failed: {}", js_error_text(err));
                    msg.set(Some((false, text.clone())));
                    status_msg.set(text);
                    busy.set(false);
                    return;
                }
            }
            busy.set(false);
            show.set(false);
            status_msg.set("Settings saved".into());
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
        if let Some(err) = settings_required_error(&cfg, &key) {
            let text = format!("Validation failed: {err}");
            msg.set(Some((false, text.clone())));
            status_msg.set(text);
            return;
        }
        busy.set(true);
        msg.set(Some((true, "Validating current settings...".into())));
        status_msg.set("Validating current settings...".into());
        spawn_local(async move {
            let res = invoke_checked(
                "validate_settings",
                to_value(&serde_json::json!({ "settings": cfg, "key": key })).unwrap(),
            ).await;
            match res {
                Ok(v) => {
                    let text = v.as_string().unwrap_or_else(|| "Validation succeeded".into());
                    msg.set(Some((true, text.clone())));
                    status_msg.set(text);
                }
                Err(err) => {
                    let text = format!("Validation failed: {}", js_error_text(err));
                    msg.set(Some((false, text.clone())));
                    status_msg.set(text);
                }
            }
            busy.set(false);
        });
    };

    let new_session = move |_| {
        items.set(vec![]);
        active_session.set(None);
        sel_artifact.set(0);
        spawn_local(async move {
            let _ = invoke("new_session", JsValue::UNDEFINED).await;
            refresh_sessions(sessions);
        });
    };

    let load_session = move |id: String| {
        active_session.set(Some(id.clone()));
        sel_artifact.set(0);
        busy.set(true);
        spawn_local(async move {
            let v = invoke("load_session", to_value(&serde_json::json!({ "id": id })).unwrap()).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<LoadedItem>>(v) {
                items.set(list.into_iter().map(LoadedItem::into_chat).collect());
            }
            busy.set(false);
        });
    };

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
                status_cb.set(format!("demo: {}", demo.title));
            }
            busy.set(false);
        });
    };

    let approve = move |v: bool| move |_| {
        confirm_state.set(None);
        let arg = to_value(&serde_json::json!({ "approved": v })).unwrap();
        spawn_local(async move { let _ = invoke("confirm_response", arg).await; });
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

    let dismiss_onboard = move |_| {
        show_onboarding.set(false);
        spawn_local(async move { let _ = invoke("dismiss_onboarding", JsValue::UNDEFINED).await; });
    };

    view! {
        <div class="app">
        <aside class="sidebar" class:collapsed=move || !show_sidebar.get()>
            <div class="proj">
                <span class="logo"></span>
                <span class="proj-name">{move || project_info.get().map(|p| p.name.clone()).unwrap_or_else(|| "wisp-science".into())}</span>
                <button class="icon-btn" title="Collapse" on:click=move |_| show_sidebar.set(false)>"‹"</button>
            </div>
            <nav class="nav">
                <button class="side-btn primary" on:click=new_session><span class="gi plus"></span>"New session"</button>
                <button class="side-btn" on:click=open_demos><span class="gi grid"></span>"Open demo"</button>
                <button class="side-btn" on:click=open_files><span class="gi doc"></span>"Files"</button>
            </nav>
            <div class="side-section">"Sessions"</div>
            <div class="side-list">
                {move || {
                    let list = sessions.get();
                    if list.is_empty() {
                        view! { <div class="side-hint">"No saved sessions yet."</div> }.into_view()
                    } else {
                        list.into_iter().map(|s| {
                            let id = s.id.clone();
                            let id_active = id.clone();
                            let title = if s.title.trim().is_empty() { "Untitled session".to_string() } else { s.title.clone() };
                            view! {
                                <button class="side-item ses"
                                    class:active=move || active_session.get().as_deref() == Some(id_active.as_str())
                                    on:click=move |_| load_session(id.clone())>
                                    <span class="dot"></span>
                                    <span class="ses-title">{title}</span>
                                </button>
                            }.into_view()
                        }).collect_view()
                    }
                }}
            </div>
            <div class="side-foot">
                {move || project_info.get().map(|p| view! {
                    <div class="proj-meta">
                        <span>{format!("{} skills · {} MCP · {} mem", p.skill_count, p.mcp_server_count, p.memory_file_count)}</span>
                    </div>
                })}
                <button class="side-btn" on:click=open_capabilities><span class="gi grid"></span>"Capabilities"</button>
                <button class="side-btn" on:click=open_settings><span class="gi gear"></span>"Settings"</button>
            </div>
        </aside>

        <main class="center">
            <div class="topbar">
                {move || (!show_sidebar.get()).then(|| view! {
                    <button class="icon-btn" title="Show sidebar" on:click=move |_| show_sidebar.set(true)>"›"</button>
                })}
                <span class="center-title">"New session"</span>
                <span class="hint">{move || status.get()}</span>
                <div class="spacer"></div>
                <button class="icon-btn" title="Toggle panel"
                    class:active=move || show_right.get()
                    on:click=move |_| show_right.update(|v| *v = !*v)><span class="gi panel"></span></button>
            </div>

            <div class="chat">
                <div class="thread">
                    {move || items.get().is_empty().then(|| view! {
                        <div class="empty">
                            <span class="empty-logo"></span>
                            <h1>"How can I help with your science today?"</h1>
                            <p>"Design experiments, analyze data, or explore ~80 biological databases — all running locally."</p>
                        </div>
                    })}
                    {move || items.get().into_iter().enumerate().map(|(i, item)| view! {
                        <div class=format!("{}", class_for(&item)) key=i>
                            {render_item(&item)}
                        </div>
                    }.into_view()).collect_view()}
                </div>
            </div>

            <div class="composer">
                <div class="composer-inner">
                    <textarea
                        prop:value={move || input.get()}
                        on:input=move|ev| input.set(event_target_value(&ev))
                        on:keydown=on_send
                        placeholder="Ask wisp-science to design, analyze, or build something…"
                    ></textarea>
                    <div class="composer-actions">
                        <span class="composer-hint">"Enter to send · Shift+Enter for newline"</span>
                        <button class="send" disabled={move || busy.get()} on:click=move |_| send()>"Send"</button>
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
                            if n == 0 { "Artifacts".to_string() } else { format!("Artifacts ({n})") }
                        }}
                    </button>
                    <button class="rp-tab" class:active=move || right_tab.get() == RightTab::File
                        on:click=move |_| right_tab.set(RightTab::File)>"File"</button>
                    <button class="rp-tab" class:active=move || right_tab.get() == RightTab::Provenance
                        on:click=move |_| right_tab.set(RightTab::Provenance)>
                        {move || {
                            let n = items.get().iter().filter(|i| matches!(i, ChatItem::Tool { .. })).count();
                            if n == 0 { "Provenance".to_string() } else { format!("Provenance ({n})") }
                        }}
                    </button>
                    <div class="spacer"></div>
                    <button class="icon-btn" title="Close panel" on:click=move |_| show_right.set(false)>"×"</button>
                </div>
                <div class="rp-doc">
                    {move || match right_tab.get() {
                        RightTab::Artifacts => {
                            let arts = artifacts.get();
                            if arts.is_empty() {
                                view! {
                                    <div class="rp-empty">
                                        <span class="rp-empty-icon"></span>
                                        <div class="rp-empty-title">"No artifacts yet"</div>
                                        <p>"Markdown tables and CSV blocks wisp-science produces will render here as sortable tables."</p>
                                    </div>
                                }.into_view()
                            } else {
                                let sel = sel_artifact.get().min(arts.len() - 1);
                                let tiles = arts.iter().enumerate().map(|(i, a)| {
                                    let meta = artifact_meta(a);
                                    view! {
                                        <button class="rp-tile" class:active=move || sel_artifact.get() == i
                                            on:click=move |_| sel_artifact.set(i)>
                                            <span class="rp-tile-name">{a.name.clone()}</span>
                                            <span class="rp-tile-meta">{meta}</span>
                                        </button>
                                    }.into_view()
                                }).collect_view();
                                let cur = arts[sel].clone();
                                let dom_id = format!("rp-{sel}");
                                view! {
                                    <div class="rp-tiles">{tiles}</div>
                                    <div class="rp-view">
                                        <div class="rp-view-head">
                                            <span class=format!("rp-badge {}", cur.kind)>{cur.kind}</span>
                                            <span class="rp-view-name">{cur.name.clone()}</span>
                                        </div>
                                        {artifact_preview(&cur, dom_id)}
                                    </div>
                                }.into_view()
                            }
                        }
                        RightTab::File => {
                            match open_file.get() {
                                None => view! {
                                    <div class="rp-empty">
                                        <span class="rp-empty-icon"></span>
                                        <div class="rp-empty-title">"No file open"</div>
                                        <p>"Browse project files from the sidebar Files button, or click a path in chat."</p>
                                        <button class="side-btn" on:click=open_files>"Browse files"</button>
                                    </div>
                                }.into_view(),
                                Some((path, kind)) => {
                                    let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
                                    let dom_id = "rp-file".to_string();
                                    view! {
                                        <div class="rp-view">
                                            <div class="rp-view-head">
                                                <span class="rp-badge">{kind.clone()}</span>
                                                <span class="rp-view-name">{name.clone()}</span>
                                            </div>
                                            <div class="hint">{path.clone()}</div>
                                            <FilePreview dom_id=dom_id path=path kind=kind />
                                        </div>
                                    }.into_view()
                                }
                            }
                        }
                        RightTab::Provenance => {
                            let tools: Vec<_> = items.get().iter().filter_map(|it| match it {
                                ChatItem::Tool { name, ok, content } => Some((name.clone(), *ok, content.clone())),
                                _ => None,
                            }).collect();
                            if tools.is_empty() {
                                view! {
                                    <div class="rp-empty">
                                        <span class="rp-empty-icon"></span>
                                        <div class="rp-empty-title">"No tool calls yet"</div>
                                        <p>"Shell, Python, and MCP tool invocations appear here with inputs and outputs."</p>
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                                    <div class="prov-list">
                                        {tools.into_iter().map(|(name, ok, content)| view! {
                                            <details class="prov-item" open=ok != Some(true)>
                                                <summary class="prov-head">
                                                    <span class="prov-name">{name.clone()}</span>
                                                    {match ok {
                                                        Some(true) => view! { <span class="ok">"✓"</span> }.into_view(),
                                                        Some(false) => view! { <span class="fail">"✗"</span> }.into_view(),
                                                        None => view! { <span class="run">"…"</span> }.into_view(),
                                                    }}
                                                </summary>
                                                <pre class="prov-body">{content.clone()}</pre>
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
                    <h2>"Confirm action"</h2>
                    <div class="hint">{msg}</div>
                    <div class="row">
                        <button on:click=approve(false)>"Deny"</button>
                        <button class="primary" on:click=approve(true)>"Approve"</button>
                    </div>
                </div>
            </div>
        }.into_view())}

        {move || show_demos.get().then(|| view! {
            <div class="overlay">
                <div class="modal">
                    <h2>"Open a demo session"</h2>
                    <span class="hint">"Pre-baked example runs from the bundled seed catalog (read-only)."</span>
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
                        <button on:click=move |_| show_demos.set(false)>"Close"</button>
                    </div>
                </div>
            </div>
        }.into_view())}

        {move || show_settings.get().then(|| view! {
            <div class="overlay">
                <div class="modal">
                    <h2>"Settings"</h2>
                    <label>"Provider"
                        <select on:change=move|ev| settings.update(|s| s.provider = provider_value(&event_target_input(&ev).value()).into())
                            prop:value={move || provider_value(&settings.get().provider).to_string()}>
                            <option value="openai">"OpenAI-compatible"</option>
                            <option value="anthropic">"Anthropic"</option>
                        </select>
                    </label>
                    <label>"API URL"
                        <input on:input=move|ev| settings.update(|s| {
                                normalize_settings_mut(s);
                                s.api_url = event_target_input(&ev).value();
                            })
                            prop:value={move || settings.get().api_url} />
                    </label>
                    <label>"Model"
                        <input on:input=move|ev| settings.update(|s| {
                                normalize_settings_mut(s);
                                s.model = event_target_input(&ev).value();
                            })
                            prop:value={move || settings.get().model} />
                    </label>
                    <label>"API key (stored in OS keyring)"
                        <input on:input=move|ev| api_key_input.set(event_target_input(&ev).value())
                            prop:value={move || api_key_input.get()} type="password" />
                    </label>
                    <span class="hint">"Tip: DeepSeek/OpenAI-compatible uses /chat/completions; Anthropic uses /v1/messages."</span>
                    {move || settings_message.get().map(|(ok, text)| view! {
                        <div class="settings-status"
                            class:ok=move || ok
                            class:fail=move || !ok>{text}</div>
                    })}
                    <div class="row">
                        <button type="button" disabled=move || settings_busy.get() on:click=validate_settings>"Valid"</button>
                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>"Cancel"</button>
                        <button type="button" class="primary" disabled=move || settings_busy.get() on:click=save_settings>"Save"</button>
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
                            <h2>"Project files"</h2>
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
                        {move || project_info.get().map(|p| view! {
                            <div class="hint fb-root">{format!("Root: {}", p.root)}</div>
                        })}
                    </div>
                </div>
            }.into_view()
        })}

        {move || show_capabilities.get().then(|| view! {
            <div class="overlay">
                <div class="modal modal-wide">
                    <div class="fb-head">
                        <h2>"Capabilities"</h2>
                        <button class="icon-btn" on:click=move |_| show_capabilities.set(false)>"×"</button>
                    </div>
                    {move || caps.get().map(|c| view! {
                        <div class="cap-grid">
                            <div class="cap-stat"><span class="cap-num">{c.project.skill_count}</span><span class="cap-label">"Skills"</span></div>
                            <div class="cap-stat"><span class="cap-num">{c.mcp_servers.len()}</span><span class="cap-label">"MCP servers"</span></div>
                            <div class="cap-stat"><span class="cap-num">{c.memory_files.len()}</span><span class="cap-label">"Memory files"</span></div>
                        </div>
                        <div class="cap-section">
                            <h3>"MCP bio-tools"</h3>
                            <div class="cap-tags">
                                {c.mcp_servers.iter().map(|s| view! { <span class="cap-tag">{s.clone()}</span> }).collect_view()}
                            </div>
                        </div>
                        <div class="cap-section">
                            <h3>"Skills"</h3>
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
                            <h3>"Permissions"</h3>
                            <p class="hint">"Shell and destructive file operations require your approval in a confirm dialog before running."</p>
                        </div>
                    })}
                    <div class="row">
                        <button on:click=move |_| show_capabilities.set(false)>"Close"</button>
                    </div>
                </div>
            </div>
        }.into_view())}

        {move || show_onboarding.get().then(|| {
            let step = onboard_step.get();
            view! {
                <div class="overlay onboard-overlay">
                    <div class="modal onboard">
                        {match step {
                            0 => view! {
                                <h2>"Welcome to wisp-science"</h2>
                                <p class="hint">"Your local science assistant — design experiments, analyze data, and query ~80 biological databases without leaving your machine."</p>
                            }.into_view(),
                            1 => view! {
                                <h2>"Connect your model"</h2>
                                <p class="hint">"Add an API key in Settings (OpenAI-compatible or Anthropic). Keys are stored in your OS keyring, not in the project folder."</p>
                            }.into_view(),
                            _ => view! {
                                <h2>"What wisp-science can do"</h2>
                                <p class="hint">"Run Python in a sandboxed REPL, call bio-tools MCP servers, preview PDFs/molecules/structures in the right panel, and browse project files from the sidebar."</p>
                            }.into_view(),
                        }}
                        <div class="onboard-dots">
                            {(0..3).map(|i| view! {
                                <span class="onboard-dot" class:active=move || onboard_step.get() == i></span>
                            }).collect_view()}
                        </div>
                        <div class="row">
                            {if step > 0 {
                                view! { <button on:click=move |_| onboard_step.update(|s| *s = s.saturating_sub(1))>"Back"</button> }.into_view()
                            } else { view! { <span></span> }.into_view() }}
                            {if step < 2 {
                                view! { <button class="primary" on:click=move |_| onboard_step.update(|s| *s += 1)>"Next"</button> }.into_view()
                            } else {
                                view! {
                                    <button class="primary" on:click=dismiss_onboard>"Get started"</button>
                                }.into_view()
                            }}
                        </div>
                    </div>
                </div>
            }.into_view()
        })}
        </div>
    }
}

fn class_for(item: &ChatItem) -> &'static str {
    match item { ChatItem::User(_) => "msg user", ChatItem::Assistant(_) => "msg assistant", ChatItem::Reasoning(_) => "msg reasoning", ChatItem::Tool { .. } => "tool-wrap" }
}

fn render_item(item: &ChatItem) -> impl IntoView {
    match item {
        ChatItem::User(s) => view! { <div class="role">"You"</div><div class="body">{s.clone()}</div> }.into_view(),
        ChatItem::Assistant(s) => view! {
            <div class="role">"wisp-science"</div>
            <div class="body md" inner_html=md_to_html(s)></div>
        }.into_view(),
        ChatItem::Reasoning(s) => view! {
            <details class="rz">
                <summary>"thinking"</summary>
                <div class="body">{s.clone()}</div>
            </details>
        }.into_view(),
        ChatItem::Tool { name, ok, content } => {
            // Collapse successful calls; keep running/failed ones expanded.
            let open = *ok != Some(true);
            view! {
                <details class="tool" open=open>
                    <summary class="head">
                        <span>{name.clone()}</span>
                        {match ok {
                            Some(true) => view!{ <span class="ok">"✓"</span> }.into_view(),
                            Some(false) => view!{ <span class="fail">"✗"</span> }.into_view(),
                            None => view!{ <span class="run">"…"</span> }.into_view(),
                        }}
                    </summary>
                    <div class="content">{content.clone()}</div>
                </details>
            }.into_view()
        }
    }
}

pub fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}
