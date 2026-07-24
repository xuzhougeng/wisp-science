//! Pure helpers: string/path/format transforms, Markdown-to-HTML rendering,
//! CSV/table classification, and small DOM value extractors.
//!
//! Everything here is a plain function with no Leptos signals, no app state,
//! and no `crate::dto` types — just data in, data out. That makes this the one
//! module in the UI that is trivially unit-testable and freely reusable; keep
//! new coupling-free utilities here instead of growing `main.rs`.

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

static NEXT_DOM_ID: AtomicUsize = AtomicUsize::new(0);

/// Process-unique DOM id with the given prefix (for mounting/highlight targets).
pub(crate) fn unique_dom_id(prefix: &str) -> String {
    format!("{prefix}-{}", NEXT_DOM_ID.fetch_add(1, Ordering::Relaxed))
}

thread_local! {
    /// Timestamp of the last `compositionend`, consumed by `ime_composing`.
    static COMPOSITION_ENDED_AT: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
    /// Timestamp of the keydown already judged IME-owned, so every guard
    /// inspecting the same event agrees even after the marker is consumed.
    static IME_SWALLOWED_AT: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
}

/// Record a `compositionend` timestamp (one window-level listener in `App`).
pub(crate) fn note_composition_end(time_stamp_ms: f64) {
    COMPOSITION_ENDED_AT.with(|t| t.set(time_stamp_ms));
}

/// True while an IME is composing. WebKit (macOS WKWebView) fires the Enter
/// keydown that confirms a candidate *after* `compositionend`, so
/// `isComposing` is already false there — but `keyCode` is still 229, the
/// IME-processed sentinel. Only a 229 keydown *near* a compositionend is the
/// confirm key, and it is swallowed once: with a CJK input source active
/// WKWebView keeps tagging later standalone Enters 229 too, and those must
/// send instead of inserting a newline (same 500ms-window + consume-once
/// approach ProseMirror uses for this WebKit quirk).
pub(crate) fn ime_composing(ev: &web_sys::KeyboardEvent) -> bool {
    if ev.is_composing() {
        return true;
    }
    if ev.key_code() != 229 {
        return false;
    }
    let ts = ev.time_stamp();
    if IME_SWALLOWED_AT.with(|t| t.get()) == ts {
        return true;
    }
    let near = COMPOSITION_ENDED_AT.with(|t| (ts - t.get()).abs() < 500.0);
    if near {
        COMPOSITION_ENDED_AT.with(|t| t.set(f64::NEG_INFINITY));
        IME_SWALLOWED_AT.with(|t| t.set(ts));
    }
    near
}

pub(crate) fn dom_value(ev: &web_sys::Event) -> String {
    ev.target()
        .and_then(|target| js_sys::Reflect::get(&target, &JsValue::from_str("value")).ok())
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

pub(crate) fn provider_value(provider: &str) -> &'static str {
    match provider.trim() {
        "anthropic" => "anthropic",
        "openai_responses" | "openai-responses" | "responses" => "openai_responses",
        _ => "openai",
    }
}

pub(crate) fn provider_defaults(provider: &str) -> (&'static str, &'static str) {
    match provider_value(provider) {
        "anthropic" => ("https://api.anthropic.com", "claude-sonnet-5"),
        "openai_responses" => ("https://api.openai.com/v1", "gpt-5.5"),
        _ => ("https://api.deepseek.com", "deepseek-v4-pro"),
    }
}

pub(crate) fn join_path(base: &str, name: &str) -> String {
    if base == "." || base.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches(['/', '\\']), name)
    }
}

pub(crate) fn parent_path(path: &str) -> String {
    if path == "." || path.is_empty() {
        return ".".into();
    }
    let p = path.replace('\\', "/");
    if p == "/" {
        return "/".into();
    }
    match p.rsplit_once('/') {
        Some(("", _)) => "/".into(),
        None => ".".into(),
        Some((a, _)) => a.to_string(),
    }
}

/// Human-readable duration for tool/step timing labels (e.g. `850ms`, `15s`).
pub(crate) fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{}s", ms / 1000)
    } else {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        if secs == 0 {
            format!("{mins}m")
        } else {
            format!("{mins}m {secs}s")
        }
    }
}

pub(crate) fn format_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub(crate) fn event_target_value(ev: &web_sys::Event) -> String {
    // Works for <input>, <textarea>, and <select>. Casting the wrong one used to
    // panic in the event handler (input never registered) — see the project
    // name field.
    let target = ev.target().unwrap();
    if let Some(i) = target.dyn_ref::<web_sys::HtmlInputElement>() {
        return i.value();
    }
    if let Some(a) = target.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        return a.value();
    }
    if let Some(select) = target.dyn_ref::<web_sys::HtmlSelectElement>() {
        return select.value();
    }
    String::new()
}

pub(crate) fn event_target_input(ev: &web_sys::Event) -> web_sys::HtmlInputElement {
    ev.target()
        .unwrap()
        .dyn_into::<web_sys::HtmlInputElement>()
        .unwrap()
}

pub(crate) fn event_target_checked(ev: &web_sys::Event) -> bool {
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|i| i.checked())
        .unwrap_or(false)
}

/// Render agent/assistant Markdown to HTML for `inner_html`. GFM tables,
/// strikethrough, task lists and footnotes are on; the source is trusted
/// (local agent output rendered in the desktop WebView).
pub(crate) fn md_to_html(src: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let src = preprocess_markdown(src);
    let src = fence_identifier_line_runs(src.as_ref());
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_MATH);
    let parser = Parser::new_ext(src.as_ref(), opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

fn preprocess_markdown(src: &str) -> std::borrow::Cow<'_, str> {
    let src = strip_yaml_front_matter(src);
    let src = match rewrite_image_tags(src.as_ref()) {
        std::borrow::Cow::Borrowed(_) => src,
        std::borrow::Cow::Owned(s) => std::borrow::Cow::Owned(s),
    };
    match normalize_math_delimiters(src.as_ref()) {
        std::borrow::Cow::Borrowed(_) => src,
        std::borrow::Cow::Owned(s) => std::borrow::Cow::Owned(s),
    }
}

/// GPT-family models emit LaTeX with `\(...\)` / `\[...\]` delimiters, but
/// pulldown-cmark's math extension only knows `$...$` / `$$...$$`. Rewrite the
/// former into the latter so both styles render (#249). Fenced code blocks and
/// inline code spans are left untouched.
fn normalize_math_delimiters(src: &str) -> std::borrow::Cow<'_, str> {
    if !src.contains("\\(") && !src.contains("\\[") {
        return std::borrow::Cow::Borrowed(src);
    }
    let mut out = String::with_capacity(src.len());
    let mut seg = String::new();
    let mut changed = false;
    let mut fence: Option<(char, usize)> = None;
    for chunk in src.split_inclusive('\n') {
        let line = chunk.trim_end_matches(['\r', '\n']);
        let stripped = line.trim_start();
        let mark = stripped.chars().next().filter(|c| matches!(c, '`' | '~'));
        let run = mark.map_or(0, |m| stripped.chars().take_while(|&c| c == m).count());
        match fence {
            Some((m, n)) => {
                out.push_str(chunk);
                if mark == Some(m) && run >= n && stripped[run..].trim().is_empty() {
                    fence = None;
                }
            }
            None if run >= 3 => {
                convert_math_spans(&seg, &mut out, &mut changed);
                seg.clear();
                out.push_str(chunk);
                fence = Some((mark.unwrap(), run));
            }
            None => seg.push_str(chunk),
        }
    }
    convert_math_spans(&seg, &mut out, &mut changed);
    if changed {
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(src)
    }
}

/// Rewrite `\(...\)` → `$...$` and `\[...\]` → `$$...$$` in one non-fenced
/// segment, skipping inline code spans. Unpaired delimiters pass through.
fn convert_math_spans(seg: &str, out: &mut String, changed: &mut bool) {
    let mut rest = seg;
    loop {
        let bt = rest.find('`');
        let par = rest.find("\\(");
        let brk = rest.find("\\[");
        let Some(pos) = [bt, par, brk].into_iter().flatten().min() else {
            out.push_str(rest);
            return;
        };
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        if Some(pos) == bt {
            // Inline code span: copy verbatim through the matching backtick
            // run; an unmatched opener is literal text, keep scanning after it.
            let n = rest.chars().take_while(|&c| c == '`').count();
            out.push_str(&rest[..n]);
            rest = &rest[n..];
            if let Some(end) = find_backtick_run(rest, n) {
                out.push_str(&rest[..end + n]);
                rest = &rest[end + n..];
            }
            continue;
        }
        let (close, wrap) = if Some(pos) == par {
            ("\\)", "$")
        } else {
            ("\\]", "$$")
        };
        let paired = rest[2..].find(close).filter(|&end| {
            // A blank line between the delimiters means this is not real math.
            let body = &rest[2..2 + end];
            !body.contains("\n\n") && !body.contains("\n\r\n")
        });
        match paired {
            Some(end) if !rest[2..2 + end].trim().is_empty() => {
                *changed = true;
                out.push_str(wrap);
                out.push_str(rest[2..2 + end].trim());
                out.push_str(wrap);
                rest = &rest[2 + end + 2..];
            }
            Some(end) => {
                out.push_str(&rest[..2 + end + 2]);
                rest = &rest[2 + end + 2..];
            }
            None => {
                out.push_str(&rest[..2]);
                rest = &rest[2..];
            }
        }
    }
}

fn find_backtick_run(s: &str, n: usize) -> Option<usize> {
    let mut from = 0;
    while let Some(p) = s[from..].find('`') {
        let at = from + p;
        let run = s[at..].chars().take_while(|&c| c == '`').count();
        if run == n {
            return Some(at);
        }
        from = at + run;
    }
    None
}

/// Treat leading YAML front matter like normal Markdown tooling does: metadata
/// config, not rendered prose. This avoids `report.md`-style headers exploding
/// into one giant paragraph in the preview.
fn strip_yaml_front_matter(src: &str) -> std::borrow::Cow<'_, str> {
    if !src.starts_with("---\n") && !src.starts_with("---\r\n") {
        return std::borrow::Cow::Borrowed(src);
    }
    let mut saw_yaml = false;
    let mut offset = 0usize;
    let mut chunks = src.split_inclusive('\n');
    let Some(first) = chunks.next() else {
        return std::borrow::Cow::Borrowed(src);
    };
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return std::borrow::Cow::Borrowed(src);
    }
    offset += first.len();
    for chunk in chunks {
        let line = chunk.trim_end_matches(['\r', '\n']);
        if line == "---" || line == "..." {
            if !saw_yaml {
                return std::borrow::Cow::Borrowed(src);
            }
            let rest = src[offset + chunk.len()..].trim_start_matches(['\r', '\n']);
            return std::borrow::Cow::Owned(rest.to_string());
        }
        if line.contains(':') {
            saw_yaml = true;
        }
        offset += chunk.len();
    }
    std::borrow::Cow::Borrowed(src)
}

/// Codex-style `<image ... path="...">...</image>` blocks are valid in the
/// transcript, but not in standard Markdown. Rewrite them into local file links
/// so the existing click handler can open the image preview.
fn rewrite_image_tags(src: &str) -> std::borrow::Cow<'_, str> {
    if !src.contains("<image") {
        return std::borrow::Cow::Borrowed(src);
    }
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    let mut changed = false;
    while let Some(start) = rest.find("<image") {
        out.push_str(&rest[..start]);
        let tag_src = &rest[start..];
        let Some(open_end) = tag_src.find('>') else {
            out.push_str(tag_src);
            rest = "";
            break;
        };
        let Some(close_rel) = tag_src[open_end + 1..].find("</image>") else {
            out.push_str(tag_src);
            rest = "";
            break;
        };
        let whole_end = open_end + 1 + close_rel + "</image>".len();
        let open_tag = &tag_src[..=open_end];
        if let Some(replacement) = rewrite_image_tag(open_tag) {
            out.push_str(&replacement);
            changed = true;
        } else {
            out.push_str(&tag_src[..whole_end]);
        }
        rest = &tag_src[whole_end..];
    }
    out.push_str(rest);
    if changed {
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(src)
    }
}

fn rewrite_image_tag(tag: &str) -> Option<String> {
    let path = image_tag_attr(tag, "path")?;
    let label = image_tag_attr(tag, "name")
        .unwrap_or("Image")
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    Some(format!("[{}](<{}>)", label.trim(), path.trim()))
}

fn image_tag_attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let first = rest.chars().next()?;
    if first == '"' || first == '\'' {
        let rest = &rest[1..];
        let end = rest.find(first)?;
        return Some(&rest[..end]);
    }
    if first == '[' {
        let end = rest.find(']')?;
        return Some(&rest[..=end]);
    }
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '>')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Bare runs of snake_case tool/API names collapse into one unreadable `<p>`
/// under `.md { white-space: normal }`. Promote long runs into a fenced
/// `catalog` block so they stay scannable (multi-column CSS).
fn fence_identifier_line_runs(src: &str) -> std::borrow::Cow<'_, str> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len() + 8);
    let mut changed = false;
    let mut i = 0;
    while i < lines.len() {
        let trim = lines[i].trim();
        if trim.starts_with("```") {
            out.push(lines[i]);
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                out.push(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                out.push(lines[i]);
                i += 1;
            }
            continue;
        }
        if is_catalog_ident_line(lines[i]) {
            let start = i;
            while i < lines.len() && is_catalog_ident_line(lines[i]) {
                i += 1;
            }
            if i - start >= 8 {
                changed = true;
                out.push("```catalog");
                out.extend_from_slice(&lines[start..i]);
                out.push("```");
                continue;
            }
            out.extend_from_slice(&lines[start..i]);
            continue;
        }
        out.push(lines[i]);
        i += 1;
    }
    if !changed {
        return std::borrow::Cow::Borrowed(src);
    }
    let mut s = out.join("\n");
    if src.ends_with('\n') {
        s.push('\n');
    }
    std::borrow::Cow::Owned(s)
}

fn is_catalog_ident_line(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 2 || t.len() > 80 {
        return false;
    }
    let mut chars = t.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Inline Markdown for table cells (bold, code, links, etc.).
pub(crate) fn md_inline_to_html(src: &str) -> String {
    if src.is_empty() {
        return String::new();
    }
    let html = md_to_html(src);
    let s = html.trim();
    if let Some(inner) = s
        .strip_prefix("<p>")
        .and_then(|rest| rest.strip_suffix("</p>"))
    {
        if !inner.contains("<p>") {
            return inner.to_string();
        }
    }
    html
}

pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Best-effort JSON pretty-printer for file previews. Invalid JSON falls back
/// to the original text so previews stay usable even for malformed output.
pub(crate) fn pretty_json(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .and_then(|value| serde_json::to_string_pretty(&value))
        .unwrap_or_else(|_| text.to_string())
}

pub(crate) fn next_artifact_id(n: usize) -> String {
    format!("{:08x}", n + 1)
}

/// Group key for the artifacts panel: parent directory for files, `@kind` for inline artifacts.
pub(crate) fn artifact_group_key(a: &crate::dto::Artifact) -> String {
    use crate::dto::PreviewData;
    match &a.data {
        PreviewData::File { path, .. } => path
            .rsplit(['/', '\\'])
            .nth(1)
            .filter(|p| !p.is_empty())
            .map(|p| format!("{p}/"))
            .unwrap_or_else(|| ".".into()),
        _ => format!("@{}", a.kind),
    }
}

/// Sorted artifact groups: directories first (alpha), then inline kinds.
pub(crate) fn group_artifact_indices(arts: &[crate::dto::Artifact]) -> Vec<(String, Vec<usize>)> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<(u8, String), (String, Vec<usize>)> = BTreeMap::new();
    for (i, a) in arts.iter().enumerate() {
        let key = artifact_group_key(a);
        let sort = if let Some(kind) = key.strip_prefix('@') {
            (1, kind.to_string())
        } else {
            (0, key.clone())
        };
        map.entry(sort)
            .or_insert_with(|| (key.clone(), Vec::new()))
            .1
            .push(i);
    }
    map.into_values().collect()
}

pub(crate) fn normalize_path(path: &str) -> String {
    // Only strip redundant `./` prefixes. Do NOT strip a leading `/` — the agent
    // is told to emit absolute paths (system_prompt.rs), and the backend resolves
    // absolute-under-root correctly; stripping the slash turned an absolute path
    // into a bad root-relative one and 404'd on click (#12).
    let path = path
        .trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\");
    strip_image_pdf_shorthand(path).to_string()
}

/// Percent-decode an href read back from rendered HTML. pulldown-cmark
/// percent-encodes link destinations (a Windows path `D:\a\b` becomes
/// `D:%5Ca%5Cb`), so an href taken straight off the DOM never matches a real
/// file path until it is decoded. Decodes byte-wise so multi-byte UTF-8
/// filenames (e.g. Chinese) round-trip; a malformed `%` sequence is left as-is.
pub(crate) fn decode_href(href: &str) -> String {
    let bytes = href.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn strip_image_pdf_shorthand(path: &str) -> &str {
    const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "svg"];
    let lower = path.to_ascii_lowercase();
    for ext in IMAGE_EXTS {
        let slash = format!(".{ext}/.pdf");
        if let Some(start) = lower.find(&slash) {
            return &path[..start + ext.len() + 1];
        }
        let backslash = format!(".{ext}\\.pdf");
        if let Some(start) = lower.find(&backslash) {
            return &path[..start + ext.len() + 1];
        }
    }
    path
}

pub(crate) fn is_external_href(href: &str) -> bool {
    let h = href.trim();
    h.starts_with("http://")
        || h.starts_with("https://")
        || h.starts_with("mailto:")
        || h.starts_with('#')
        || h.starts_with("javascript:")
}

/// Hrefs that should open in the system browser / mail client, not in the webview.
pub(crate) fn opens_in_system_browser(href: &str) -> bool {
    let h = href.trim();
    h.starts_with("http://")
        || h.starts_with("https://")
        || h.starts_with("mailto:")
        || h.starts_with("tel:")
}

pub(crate) fn extract_href_from_tag(tag: &str) -> Option<String> {
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

/// Badge + display title for a tool card. MCP-backed tools arrive with an
/// "mcp:" event-name prefix (see wisp-tools `Registry::event_name`); skills
/// load through the built-in "use_skill" tool whose input is the skill name.
/// The badge is an i18n key; `None` means a plain built-in tool.
pub(crate) fn tool_card_label(name: &str, input: &str) -> (Option<&'static str>, String) {
    if let Some(rest) = name.strip_prefix("mcp:") {
        return (Some("tool.badge.mcp"), rest.to_string());
    }
    if name == "use_skill" {
        let skill = input.lines().next().unwrap_or("").trim();
        let title = if skill.is_empty() { name } else { skill };
        return (Some("tool.badge.skill"), title.to_string());
    }
    (None, name.to_string())
}

pub(crate) fn tool_lang(name: &str) -> &'static str {
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

/// Extract non-empty fenced Markdown blocks as `(language, source)` pairs.
pub(crate) fn fenced_blocks(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let fence = lines[i].trim();
        if !fence.starts_with("```") {
            i += 1;
            continue;
        }
        let language = fence
            .trim_start_matches('`')
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut j = i + 1;
        while j < lines.len() && !lines[j].trim().starts_with("```") {
            j += 1;
        }
        let source = lines[i + 1..j].join("\n");
        if !source.is_empty() {
            blocks.push((language, source));
        }
        i = j.saturating_add(1);
    }
    blocks
}

pub(crate) fn split_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

pub(crate) fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.contains('|')
}

pub(crate) fn is_separator(line: &str) -> bool {
    let cells = split_row(line);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim();
            !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':') && c.contains('-')
        })
}

pub(crate) fn parse_csv_line(line: &str) -> Vec<String> {
    line.split(',')
        .map(|c| c.trim().trim_matches('"').to_string())
        .collect()
}

/// A `.ipynb` cell, flattened to what the preview actually draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NbCell {
    pub(crate) markdown: bool,
    pub(crate) source: String,
    pub(crate) outputs: Vec<NbOutput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NbOutput {
    Text { text: String, error: bool },
    Image { mime: String, b64: String },
    Html(String),
    Svg(String),
    Latex(String),
    Omitted { mime: String, bytes: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notebook {
    /// highlight.js id for the kernel language; code cells are all this language.
    pub(crate) lang: String,
    pub(crate) cells: Vec<NbCell>,
}

/// nbformat spells every text field as either a string or a list of lines
/// (already newline-terminated); both appear in real notebooks.
fn nb_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect(),
        _ => String::new(),
    }
}

/// Keep one pathological saved output from monopolising the WebView. The file
/// reader has its own 32 MiB ceiling; these tighter budgets avoid duplicating
/// most of that payload again while building the notebook projection.
const MAX_NB_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_NB_TOTAL_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

fn push_nb_output(
    out: &mut Vec<NbOutput>,
    used_bytes: &mut usize,
    mime: &str,
    bytes: usize,
    output: NbOutput,
) {
    if bytes > MAX_NB_OUTPUT_BYTES || used_bytes.saturating_add(bytes) > MAX_NB_TOTAL_OUTPUT_BYTES {
        out.push(NbOutput::Omitted {
            mime: mime.to_string(),
            bytes,
        });
        return;
    }
    *used_bytes += bytes;
    out.push(output);
}

/// Tracebacks arrive with the kernel's ANSI colour codes baked in, which would
/// otherwise render as literal `[0;31m` noise.
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // CSI: ESC [ params... final-byte in @-~. Anything else: drop the ESC only.
        if chars.clone().next() == Some('[') {
            chars.next();
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

fn nb_outputs(v: &serde_json::Value, used_bytes: &mut usize) -> Vec<NbOutput> {
    let mut out = Vec::new();
    for o in v.as_array().map(|a| a.as_slice()).unwrap_or_default() {
        match o.get("output_type").and_then(|t| t.as_str()).unwrap_or("") {
            "stream" => {
                let text = nb_text(&o["text"]);
                push_nb_output(
                    &mut out,
                    used_bytes,
                    "text/plain",
                    text.len(),
                    NbOutput::Text {
                        text,
                        error: o.get("name").and_then(|n| n.as_str()) == Some("stderr"),
                    },
                );
            }
            "error" => {
                let tb = nb_text(&o["traceback"]);
                let text = if tb.trim().is_empty() {
                    format!(
                        "{}: {}",
                        o.get("ename").and_then(|e| e.as_str()).unwrap_or("Error"),
                        o.get("evalue").and_then(|e| e.as_str()).unwrap_or(""),
                    )
                } else {
                    strip_ansi(&tb)
                };
                push_nb_output(
                    &mut out,
                    used_bytes,
                    "text/plain",
                    text.len(),
                    NbOutput::Text { text, error: true },
                );
            }
            // Match Jupyter's display priority without executing notebook code:
            // raster image, isolated SVG, sandboxed HTML, KaTeX, then plain text.
            "execute_result" | "display_data" => {
                let data = &o["data"];
                let img = ["image/png", "image/jpeg", "image/gif"]
                    .iter()
                    .find_map(|m| {
                        let value = nb_text(&data[*m]);
                        (!value.trim().is_empty()).then_some((*m, value))
                    });
                if let Some((mime, b64)) = img {
                    // Line-wrapped base64 is legal in nbformat but not in a data: URL.
                    let b64: String = b64.split_whitespace().collect();
                    push_nb_output(
                        &mut out,
                        used_bytes,
                        mime,
                        b64.len(),
                        NbOutput::Image {
                            mime: mime.to_string(),
                            b64,
                        },
                    );
                    continue;
                }

                let rich = [
                    ("image/svg+xml", "svg"),
                    ("text/html", "html"),
                    ("text/latex", "latex"),
                ]
                .iter()
                .find_map(|(mime, kind)| {
                    let value = nb_text(&data[*mime]);
                    (!value.trim().is_empty()).then_some((*mime, *kind, value))
                });
                if let Some((mime, kind, value)) = rich {
                    let bytes = value.len();
                    let output = match kind {
                        "svg" => NbOutput::Svg(value),
                        "html" => NbOutput::Html(value),
                        _ => NbOutput::Latex(value),
                    };
                    push_nb_output(&mut out, used_bytes, mime, bytes, output);
                    continue;
                }

                let text = nb_text(&data["text/plain"]);
                if !text.trim().is_empty() {
                    push_nb_output(
                        &mut out,
                        used_bytes,
                        "text/plain",
                        text.len(),
                        NbOutput::Text { text, error: false },
                    );
                }
            }
            _ => {}
        }
    }
    out
}

/// Parse a Jupyter notebook into cells. `None` when the text isn't a notebook,
/// which lets the caller fall back to showing the raw JSON.
pub(crate) fn parse_notebook(text: &str) -> Option<Notebook> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let cells = v.get("cells")?.as_array()?;
    let meta = &v["metadata"];
    let lang = meta["language_info"]["name"]
        .as_str()
        .or_else(|| meta["kernelspec"]["language"].as_str())
        .unwrap_or("python");
    // The kernel names its language ("R", "python3"); hljs wants its own ids.
    let lang = tool_lang(lang);
    let mut output_bytes = 0;
    Some(Notebook {
        lang: lang.to_string(),
        cells: cells
            .iter()
            .filter_map(|c| {
                let kind = c.get("cell_type").and_then(|t| t.as_str()).unwrap_or("");
                let source = nb_text(&c["source"]);
                // Raw cells carry no rendering semantics worth guessing at.
                if kind == "raw" || source.trim().is_empty() {
                    return None;
                }
                Some(NbCell {
                    markdown: kind == "markdown",
                    source,
                    outputs: nb_outputs(&c["outputs"], &mut output_bytes),
                })
            })
            .collect(),
    })
}

/// Source-file extension → highlight.js language id, for the languages the
/// vendored highlight.min.js build actually registers. `None` means "not code
/// we can colour", not "not text" — plain text still previews, just unstyled.
pub(crate) fn code_lang(path: &str) -> Option<&'static str> {
    let (_, ext) = path.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "r" => "r",
        "py" | "pyw" => "python",
        "sh" | "bash" | "zsh" => "bash",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "pl" | "pm" => "perl",
        "lua" => "lua",
        "php" => "php",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "m" => "objectivec",
        "sql" => "sql",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "xml" | "xsl" | "rss" => "xml",
        "yaml" | "yml" => "yaml",
        "toml" | "ini" | "cfg" | "conf" => "ini",
        "diff" | "patch" => "diff",
        "json" | "jsonl" | "ipynb" => "json",
        _ => return None,
    })
}

/// The persistent-runtime language a source file can be bound to, or `None` for
/// files with no runtime. The returned ids are the `RuntimeLanguage` wire
/// spelling, so they pass straight to the runtime commands.
pub(crate) fn runtime_language(path: &str) -> Option<&'static str> {
    match code_lang(path)? {
        language @ ("r" | "python") => Some(language),
        _ => None,
    }
}

pub(crate) fn file_kind(path: &str) -> Option<&'static str> {
    let (_, ext) = path.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
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
        // Rmd/qmd are Markdown with code chunks: the Markdown preview already
        // renders + highlights fenced blocks, so they need nothing of their own.
        "md" | "rmd" | "qmd" | "markdown" => "markdown",
        "docx" => "docx",
        "xlsx" => "xlsx",
        "pptx" => "pptx",
        "bib" => "text",
        "html" | "htm" => "html",
        "nwk" | "newick" | "treefile" | "tre" => "text",
        "ipynb" => "notebook",
        "json" => "json",
        "txt" | "log" => "text",
        _ if code_lang(path).is_some() => "code",
        _ => return None,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UserMessagePresentation {
    pub(crate) body: String,
    pub(crate) attachments: Vec<String>,
    pub(crate) artifacts: Vec<String>,
    pub(crate) sessions: Vec<String>,
    pub(crate) projects: Vec<String>,
    pub(crate) skills: Vec<String>,
}

/// Split the stable transcript suffixes from the text the user actually
/// typed. Keeping this parser pure makes old sessions and optimistic messages
/// render identically without changing the persisted chat schema.
pub(crate) fn user_message_presentation(text: &str) -> UserMessagePresentation {
    let mut presentation = UserMessagePresentation::default();
    let mut body = Vec::new();
    for block in text.split("\n\n") {
        let target = if let Some(value) = block.strip_prefix("Uploaded files: ") {
            Some((&mut presentation.attachments, value))
        } else if let Some(value) = block.strip_prefix("Attached artifacts: ") {
            Some((&mut presentation.artifacts, value))
        } else if let Some(value) = block.strip_prefix("Attached sessions: ") {
            Some((&mut presentation.sessions, value))
        } else if let Some(value) = block.strip_prefix("Project context: ") {
            Some((&mut presentation.projects, value))
        } else if let Some(value) = block.strip_prefix("Selected skills: ") {
            Some((&mut presentation.skills, value))
        } else if block.starts_with("AI source-edit instruction: ") {
            // This persisted, agent-facing hint turns a source selection into
            // an actionable edit target. It is transport metadata, not text
            // the user typed, so keep it out of the rendered chat bubble.
            continue;
        } else {
            None
        };
        if let Some((items, value)) = target {
            items.extend(
                value
                    .split(", ")
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string),
            );
        } else {
            body.push(block);
        }
    }
    presentation.body = body.join("\n\n").trim().to_string();
    presentation
}

pub(crate) fn fasta_seq_count(text: &str) -> usize {
    text.lines()
        .filter(|l| l.trim_start().starts_with('>'))
        .count()
}

#[cfg(test)]
mod md_catalog_tests {
    use super::{
        code_lang, decode_href, fence_identifier_line_runs, file_kind, format_bytes, md_to_html,
        parent_path, parse_notebook, pretty_json, push_nb_output, runtime_language, strip_ansi,
        tool_card_label, user_message_presentation, NbOutput, MAX_NB_OUTPUT_BYTES,
        MAX_NB_TOTAL_OUTPUT_BYTES,
    };

    #[test]
    fn decodes_percent_encoded_windows_href() {
        // pulldown-cmark percent-encodes the backslashes of an absolute Windows
        // path in the rendered <a href>; clicking it must round-trip back to the
        // real path, not hit the filesystem as `D:%5C...` (#outside-project-root).
        assert_eq!(
            decode_href("D:%5CPHD_project%5CAI4drug%5CPeptide%5Cfig.png"),
            "D:\\PHD_project\\AI4drug\\Peptide\\fig.png"
        );
    }

    #[test]
    fn decodes_multibyte_and_leaves_plain_and_malformed_untouched() {
        // Chinese filename: pulldown-cmark encodes each UTF-8 byte.
        assert_eq!(decode_href("out/%E5%9B%BE1.png"), "out/图1.png");
        // No encoding: unchanged.
        assert_eq!(decode_href("results/fig.png"), "results/fig.png");
        // A lone/percent-with-non-hex stays literal.
        assert_eq!(decode_href("100%done/%zz"), "100%done/%zz");
    }

    #[test]
    fn formats_large_runtime_memory_in_gigabytes() {
        assert_eq!(format_bytes(10 * 1024 * 1024 * 1024), "10.0 GB");
    }

    #[test]
    fn finds_parents_for_relative_and_absolute_paths() {
        assert_eq!(parent_path("data/results"), "data");
        assert_eq!(parent_path("/home/research"), "/home");
        assert_eq!(parent_path("/home"), "/");
        assert_eq!(parent_path("/"), "/");
    }

    #[test]
    fn fences_long_identifier_runs() {
        let src = (0..12)
            .map(|i| format!("tool_name_{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = fence_identifier_line_runs(&src);
        assert!(out.starts_with("```catalog\n"));
        assert!(out.contains("tool_name_0"));
        assert!(out.trim_end().ends_with("```"));
        let html = md_to_html(&src);
        assert!(html.contains("language-catalog"), "{html}");
        assert!(!html.contains("<p>tool_name_0"), "{html}");
    }

    #[test]
    fn leaves_short_runs_and_prose_alone() {
        let src = "Here are a few:\nread\nwrite\nedit\n\nDone.";
        assert!(matches!(
            fence_identifier_line_runs(src),
            std::borrow::Cow::Borrowed(_)
        ));
        let html = md_to_html(src);
        assert!(html.contains("<p>"), "{html}");
    }

    #[test]
    fn skips_existing_fences() {
        let src = "```\nread\nwrite\nedit\nsearch\ngrep\nshell\npython\ncodex\n```\n";
        assert!(matches!(
            fence_identifier_line_runs(src),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn strips_yaml_front_matter_from_markdown_preview() {
        let src = "---\nskill: bear-counter\ntopic: demo\n---\n\n# Title\n\nBody\n";
        let html = md_to_html(src);
        assert!(html.contains("<h1>Title</h1>"), "{html}");
        assert!(!html.contains("skill: bear-counter"), "{html}");
        assert!(!html.contains("topic: demo"), "{html}");
    }

    #[test]
    fn rewrites_codex_image_tags_to_clickable_links() {
        let src = r#"<image name=[Image #1] path="/tmp/example.png">ignored</image>"#;
        let html = md_to_html(src);
        assert!(
            html.contains(r#"<a href="/tmp/example.png">Image #1</a>"#),
            "{html}"
        );
        assert!(!html.contains("<image"), "{html}");
    }

    #[test]
    fn renders_dollar_math_as_math_spans() {
        let html = md_to_html("质能方程 $E = mc^2$ 成立。\n\n$$\\int_0^1 x^2 dx$$\n");
        assert!(
            html.contains(r#"<span class="math math-inline">"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<span class="math math-display">"#),
            "{html}"
        );
    }

    #[test]
    fn converts_gpt_style_math_delimiters() {
        let html = md_to_html("Inline \\(a_i^2\\) and display:\n\n\\[\nE = mc^2\n\\]\n");
        assert!(
            html.contains(r#"<span class="math math-inline">a_i^2</span>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<span class="math math-display">E = mc^2</span>"#),
            "{html}"
        );
    }

    #[test]
    fn leaves_math_delimiters_in_code_alone() {
        let src = "Use `\\(x\\)` here.\n\n```tex\n\\[ y \\]\n```\n";
        let html = md_to_html(src);
        assert!(!html.contains("math-inline"), "{html}");
        assert!(!html.contains("math-display"), "{html}");
        assert!(html.contains("\\(x\\)"), "{html}");
    }

    #[test]
    fn leaves_unpaired_math_delimiters_alone() {
        let src = "A stray \\( paren and \\[ bracket.\n";
        assert!(matches!(
            super::normalize_math_delimiters(src),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn detects_html_files_for_preview() {
        assert_eq!(file_kind("report.html"), Some("html"));
        assert_eq!(file_kind("report.htm"), Some("html"));
    }

    #[test]
    fn detects_json_files_for_preview() {
        assert_eq!(file_kind("report.json"), Some("json"));
    }

    #[test]
    fn detects_manuscripts_and_bibliographies_for_preview() {
        assert_eq!(file_kind("manuscript.docx"), Some("docx"));
        assert_eq!(file_kind("results.xlsx"), Some("xlsx"));
        assert_eq!(file_kind("talk.pptx"), Some("pptx"));
        assert_eq!(file_kind("references.bib"), Some("text"));
    }

    #[test]
    fn detects_source_files_as_highlightable_code() {
        // #307: these previewed as one unhighlighted paragraph because nothing
        // claimed the extension.
        assert_eq!(file_kind("01-metacell.R"), Some("code"));
        assert_eq!(file_kind("02-run_pyscenic.sh"), Some("code"));
        assert_eq!(file_kind("scripts/regulon2gmt.py"), Some("code"));
        assert_eq!(file_kind("pixi.toml"), Some("code"));
        assert_eq!(code_lang("01-metacell.R"), Some("r"));
        assert_eq!(code_lang("a.py"), Some("python"));
        assert_eq!(code_lang("pixi.toml"), Some("ini"));
        assert_eq!(code_lang("notes.txt"), None);
        // Rmd/qmd are Markdown with chunks — the Markdown preview already
        // highlights fenced code, so they must not fall into the code branch.
        assert_eq!(file_kind("analysis.Rmd"), Some("markdown"));
        assert_eq!(file_kind("analysis.qmd"), Some("markdown"));
        assert_eq!(file_kind("analysis.ipynb"), Some("notebook"));
    }

    #[test]
    fn strips_ansi_colour_codes_from_kernel_output() {
        assert_eq!(
            strip_ansi("\u{1b}[0;31mNameError\u{1b}[0m: x"),
            "NameError: x"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn parses_notebook_cells_sources_and_outputs() {
        // r##"..."## because the Markdown heading below contains `"#`.
        let nb = parse_notebook(
            r##"{
              "metadata": {"kernelspec": {"language": "R"}},
              "cells": [
                {"cell_type": "markdown", "source": ["# Title\n", "text"]},
                {"cell_type": "raw", "source": "dropped"},
                {"cell_type": "code", "source": "plot(1)", "outputs": [
                  {"output_type": "stream", "name": "stdout", "text": ["hi\n"]},
                  {"output_type": "display_data", "data": {"image/png": "AAA\nBBB", "text/plain": "<fig>"}},
                  {"output_type": "error", "ename": "E", "evalue": "v", "traceback": ["\u001b[0;31mboom\u001b[0m"]},
                  {"output_type": "display_data", "data": {"image/svg+xml": ["<svg>", "<circle/></svg>"], "text/plain": "svg"}},
                  {"output_type": "display_data", "data": {"text/html": "<table><tr><td>1</td></tr></table>", "text/plain": "table"}},
                  {"output_type": "execute_result", "data": {"text/latex": "\\frac{a}{b}", "text/plain": "a/b"}}
                ]}
              ]
            }"##,
        )
        .expect("valid notebook");
        assert_eq!(nb.lang, "r");
        // Raw + empty cells are dropped; markdown and code survive.
        assert_eq!(nb.cells.len(), 2);
        assert!(nb.cells[0].markdown);
        assert_eq!(nb.cells[0].source, "# Title\ntext");
        assert_eq!(nb.cells[1].source, "plot(1)");
        assert_eq!(
            nb.cells[1].outputs,
            vec![
                NbOutput::Text {
                    text: "hi\n".into(),
                    error: false
                },
                // Image wins over text/plain, and its wrapped base64 is joined
                // so the data: URL stays valid.
                NbOutput::Image {
                    mime: "image/png".into(),
                    b64: "AAABBB".into()
                },
                NbOutput::Text {
                    text: "boom".into(),
                    error: true
                },
                NbOutput::Svg("<svg><circle/></svg>".into()),
                NbOutput::Html("<table><tr><td>1</td></tr></table>".into()),
                NbOutput::Latex("\\frac{a}{b}".into()),
            ]
        );
        assert!(parse_notebook("not json").is_none());
        assert!(parse_notebook(r#"{"no":"cells"}"#).is_none());
    }

    #[test]
    fn notebook_output_budget_replaces_excess_payloads_with_a_marker() {
        let mut out = Vec::new();
        let mut used = 0;
        push_nb_output(
            &mut out,
            &mut used,
            "image/png",
            MAX_NB_OUTPUT_BYTES + 1,
            NbOutput::Image {
                mime: "image/png".into(),
                b64: "oversized".into(),
            },
        );
        assert_eq!(used, 0);
        assert_eq!(
            out,
            vec![NbOutput::Omitted {
                mime: "image/png".into(),
                bytes: MAX_NB_OUTPUT_BYTES + 1,
            }]
        );

        out.clear();
        used = MAX_NB_TOTAL_OUTPUT_BYTES - 1;
        push_nb_output(
            &mut out,
            &mut used,
            "text/html",
            2,
            NbOutput::Html("ok".into()),
        );
        assert_eq!(used, MAX_NB_TOTAL_OUTPUT_BYTES - 1);
        assert_eq!(
            out,
            vec![NbOutput::Omitted {
                mime: "text/html".into(),
                bytes: 2,
            }]
        );
    }

    #[test]
    fn presents_persisted_user_context_as_structured_sections() {
        let parsed = user_message_presentation(
            "Inspect this\n\nUploaded files: uploads/plot.png, data.csv\n\nAttached artifacts: counts.csv\n\nProject context: Atlas\n\nSelected skills: bear-review\n\nAI source-edit instruction: hidden",
        );
        assert_eq!(parsed.body, "Inspect this");
        assert_eq!(parsed.attachments, ["uploads/plot.png", "data.csv"]);
        assert_eq!(parsed.artifacts, ["counts.csv"]);
        assert_eq!(parsed.projects, ["Atlas"]);
        assert_eq!(parsed.skills, ["bear-review"]);
        assert!(parsed.sessions.is_empty());
    }

    #[test]
    fn pretty_prints_json_for_preview() {
        let pretty = pretty_json(r#"{"b":1,"a":[true,false]}"#);
        assert!(pretty.contains("\n  \"a\": [\n"), "{pretty}");
        assert!(pretty.contains("\n  \"b\": 1\n"), "{pretty}");
    }

    #[test]
    fn leaves_invalid_json_as_is() {
        let raw = "{\"a\":";
        assert_eq!(pretty_json(raw), raw);
    }

    #[test]
    fn only_r_and_python_sources_bind_to_a_runtime() {
        assert_eq!(runtime_language("pipeline.R"), Some("r"));
        assert_eq!(runtime_language("qc.r"), Some("r"));
        assert_eq!(runtime_language("scan.py"), Some("python"));
        assert_eq!(runtime_language("scan.pyw"), Some("python"));
        // Highlighted, but no persistent runtime exists for them.
        assert_eq!(runtime_language("build.sh"), None);
        assert_eq!(runtime_language("main.rs"), None);
        assert_eq!(runtime_language("notes.md"), None);
        assert_eq!(runtime_language("Makefile"), None);
    }

    #[test]
    fn tool_card_label_badges_mcp_and_skills() {
        assert_eq!(
            tool_card_label("mcp:pubmed_search", "{}"),
            (Some("tool.badge.mcp"), "pubmed_search".to_string())
        );
        assert_eq!(
            tool_card_label("use_skill", "bear-support"),
            (Some("tool.badge.skill"), "bear-support".to_string())
        );
        assert_eq!(
            tool_card_label("use_skill", ""),
            (Some("tool.badge.skill"), "use_skill".to_string())
        );
        assert_eq!(tool_card_label("shell", "ls"), (None, "shell".to_string()));
    }
}
