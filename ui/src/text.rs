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

/// Default DeepSeek chat model for new profiles. Flash is the cheaper tier
/// after the v4-pro price increase; pro stays available as an explicit add.
pub(crate) const DEEPSEEK_FLASH_MODEL: &str = "deepseek-v4-flash";
pub(crate) const DEEPSEEK_PRO_MODEL: &str = "deepseek-v4-pro";

pub(crate) fn provider_defaults(provider: &str) -> (&'static str, &'static str) {
    match provider_value(provider) {
        "anthropic" => ("https://api.anthropic.com", "claude-sonnet-5"),
        "openai_responses" => ("https://api.openai.com/v1", "gpt-5.5"),
        _ => ("https://api.deepseek.com", DEEPSEEK_FLASH_MODEL),
    }
}

/// Same grouping as `models::normalize_endpoint`: one credential per API
/// origin. Keep the suffix list in sync with the Tauri helper.
pub(crate) fn normalize_endpoint(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return String::new();
    }
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https://", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http://", rest)
    } else {
        ("", url)
    };
    let rest = if scheme.is_empty() {
        rest.to_string()
    } else {
        match rest.split_once('/') {
            Some((host, path)) => format!("{}/{}", host.to_ascii_lowercase(), path),
            None => rest.to_ascii_lowercase(),
        }
    };
    let mut endpoint = format!("{scheme}{rest}");
    loop {
        while endpoint.ends_with('/') {
            endpoint.pop();
        }
        let Some(stripped) = [
            "/v1/messages",
            "/v1/chat/completions",
            "/chat/completions",
            "/responses",
            "/v1",
        ]
        .into_iter()
        .find_map(|suffix| endpoint.strip_suffix(suffix).map(str::to_string)) else {
            break;
        };
        endpoint = stripped;
    }
    endpoint
}

pub(crate) fn same_endpoint(left: &str, right: &str) -> bool {
    let left = normalize_endpoint(left);
    !left.is_empty() && left == normalize_endpoint(right)
}

pub(crate) fn join_api_url(base_url: &str, endpoint_suffix: &str) -> String {
    let base_url = base_url.trim().trim_end_matches('/');
    let endpoint_suffix = endpoint_suffix.trim().trim_matches('/');
    if endpoint_suffix.is_empty() {
        base_url.to_string()
    } else {
        format!("{base_url}/{endpoint_suffix}")
    }
}

pub(crate) fn endpoint_host(url: &str) -> String {
    let endpoint = normalize_endpoint(url);
    endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .unwrap_or(endpoint.as_str())
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod endpoint_tests {
    use super::{endpoint_host, join_api_url, normalize_endpoint, same_endpoint};

    #[test]
    fn normalize_endpoint_strips_version_and_api_suffixes() {
        assert_eq!(
            normalize_endpoint("https://api.openai.com/v1"),
            "https://api.openai.com"
        );
        assert_eq!(
            normalize_endpoint("https://API.OpenAI.com/v1/"),
            "https://api.openai.com"
        );
        assert!(same_endpoint(
            "https://api.openai.com",
            "https://api.openai.com/v1"
        ));
        assert!(same_endpoint(
            "https://api.openai.com",
            "https://api.openai.com/v1/responses"
        ));
        assert!(!same_endpoint(
            "https://api.deepseek.com",
            "https://api.openai.com"
        ));
        assert_eq!(
            endpoint_host("https://api.deepseek.com/v1"),
            "api.deepseek.com"
        );
    }

    #[test]
    fn joins_a_per_model_suffix_to_the_shared_base_url() {
        assert_eq!(
            join_api_url("https://api.deepseek.com/", "/anthropic"),
            "https://api.deepseek.com/anthropic"
        );
        assert_eq!(
            join_api_url("https://api.deepseek.com", ""),
            "https://api.deepseek.com"
        );
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
    mark_lead_strong_paragraphs(out)
}

/// Tag paragraphs whose first *content* is `<strong>` so chat CSS can draw the
/// section-lead bar. `:first-child` ignores text nodes, so
/// `该你了，接一个「<strong>点</strong>」` would otherwise match and pick up a
/// mid-sentence green bar.
fn mark_lead_strong_paragraphs(html: String) -> String {
    const OPEN: &str = "<p>";
    const MARKED: &str = r#"<p class="md-lead-strong">"#;
    if !html.contains(OPEN) || !html.contains("<strong") {
        return html;
    }
    let mut out = String::with_capacity(html.len() + MARKED.len());
    let mut rest = html.as_str();
    let mut changed = false;
    while let Some(idx) = rest.find(OPEN) {
        out.push_str(&rest[..idx]);
        let after = &rest[idx + OPEN.len()..];
        if after.trim_start().starts_with("<strong") {
            out.push_str(MARKED);
            changed = true;
        } else {
            out.push_str(OPEN);
        }
        rest = after;
    }
    if !changed {
        return html;
    }
    out.push_str(rest);
    out
}

fn unwrap_single_paragraph(html: &str) -> Option<&str> {
    let rest = html.strip_prefix("<p>").or_else(|| {
        html.strip_prefix("<p ")
            .and_then(|after| after.find('>').map(|i| &after[i + 1..]))
    })?;
    rest.strip_suffix("</p>")
}

/// Render a standalone Markdown document, hiding leading YAML front matter.
/// Chat messages must use `md_to_html` so ordinary thematic breaks are kept.
pub(crate) fn md_document_to_html(src: &str) -> String {
    let src = strip_yaml_front_matter(src);
    md_to_html(src.as_ref())
}

fn preprocess_markdown(src: &str) -> std::borrow::Cow<'_, str> {
    let src = rewrite_image_tags(src);
    let src = match normalize_math_delimiters(src.as_ref()) {
        std::borrow::Cow::Borrowed(_) => src,
        std::borrow::Cow::Owned(s) => std::borrow::Cow::Owned(s),
    };
    match rejoin_empty_list_markers(src.as_ref()) {
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

/// Models sometimes drop the item text onto the line after a bare list marker
/// (`- \nTb1 ...`). CommonMark reads that as an empty item plus a paragraph,
/// which renders as an orphan bullet dot followed by flush-left text. Rejoin
/// the two lines when the next line is plain prose at the marker's indent. A
/// single accidental blank line is tolerated as well; an intentionally empty
/// item followed by two blank lines remains untouched. Indented continuations
/// already attach correctly, and real block starts (fences, headings, quotes,
/// tables, other list items, thematic breaks) are left alone, as is anything
/// inside fenced code.
fn rejoin_empty_list_markers(src: &str) -> std::borrow::Cow<'_, str> {
    let mut out = String::with_capacity(src.len());
    let mut changed = false;
    let mut fence: Option<(char, usize)> = None;
    let mut lines = src.split_inclusive('\n').peekable();
    while let Some(chunk) = lines.next() {
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
                continue;
            }
            None if run >= 3 => {
                out.push_str(chunk);
                fence = Some((mark.unwrap(), run));
                continue;
            }
            None => {}
        }
        let Some(indent) = bare_list_marker_indent(line) else {
            out.push_str(chunk);
            continue;
        };
        let Some(next) = lines.peek() else {
            out.push_str(chunk);
            continue;
        };
        let next_line = next.trim_end_matches(['\r', '\n']);
        let (continuation, skip_blank) = if is_plain_continuation(next_line, indent) {
            (*next, false)
        } else if next_line.trim().is_empty() {
            let mut lookahead = lines.clone();
            lookahead.next();
            match lookahead.next() {
                Some(after_blank)
                    if is_plain_continuation(
                        after_blank.trim_end_matches(['\r', '\n']),
                        indent,
                    ) =>
                {
                    (after_blank, true)
                }
                _ => {
                    out.push_str(chunk);
                    continue;
                }
            }
        } else {
            out.push_str(chunk);
            continue;
        };
        let continuation_line = continuation.trim_end_matches(['\r', '\n']);
        changed = true;
        out.push_str(line.trim_end_matches([' ', '\t']));
        out.push(' ');
        out.push_str(continuation_line.trim_start());
        out.push_str(&continuation[continuation_line.len()..]);
        if skip_blank {
            lines.next();
        }
        lines.next();
    }
    if changed {
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(src)
    }
}

/// Indent of a line holding only a list marker (`-`, `*`, `+`, `1.`, `1)`) plus
/// trailing whitespace, or None. Indents past three spaces are code blocks.
fn bare_list_marker_indent(line: &str) -> Option<usize> {
    let stripped = line.trim_start_matches(' ');
    let indent = line.len() - stripped.len();
    if indent > 3 {
        return None;
    }
    let marker = stripped.trim_end_matches([' ', '\t']);
    let bare_bullet = matches!(marker, "-" | "*" | "+");
    let bare_enum = marker.strip_suffix(['.', ')']).is_some_and(|digits| {
        !digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit())
    });
    (bare_bullet || bare_enum).then_some(indent)
}

/// True when `line` is plain prose that belongs to a preceding bare list
/// marker, not a block start of its own.
fn is_plain_continuation(line: &str, marker_indent: usize) -> bool {
    if line.starts_with('\t') {
        return false;
    }
    let stripped = line.trim_start_matches(' ');
    if stripped.is_empty() || line.len() - stripped.len() > marker_indent {
        return false;
    }
    let t = line.trim_start();
    if t.is_empty() {
        return false; // whitespace-only line
    }
    if t.starts_with("```") || t.starts_with("~~~") {
        return false; // fenced code
    }
    let first = t.chars().next().unwrap();
    if matches!(first, '#' | '>' | '|' | '<') {
        return false; // heading / blockquote / table row / raw HTML
    }
    if matches!(first, '-' | '*' | '+') {
        let rest = &t[1..];
        if rest.is_empty() || rest.starts_with([' ', '\t']) {
            return false; // another list item
        }
    }
    if first.is_ascii_digit() {
        let digits = t.bytes().take_while(|b| b.is_ascii_digit()).count();
        let rest = &t[digits..];
        if rest.starts_with(['.', ')']) && rest[1..].starts_with([' ', '\t']) {
            return false; // ordered list item
        }
    }
    if matches!(first, '-' | '*' | '_') {
        let compact: String = t.chars().filter(|c| !matches!(c, ' ' | '\t')).collect();
        if compact.len() >= 3 && compact.chars().all(|c| c == first) {
            return false; // thematic break
        }
    }
    true
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
    if let Some(inner) = unwrap_single_paragraph(s) {
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

/// Return a workspace-relative spelling when `path` is inside `project_root`.
/// Relative artifact paths are already workspace-relative. Absolute references
/// outside the workspace and URI-backed artifacts keep the old last-directory
/// grouping so the panel does not expose an unrelated full host path.
fn artifact_workspace_path(path: &str, project_root: &str) -> String {
    let normalized = path.trim().replace('\\', "/");
    let normalized = normalized
        .strip_prefix("./")
        .unwrap_or(&normalized)
        .trim_end_matches('/');
    if normalized.contains("://") {
        return normalized.to_string();
    }

    let root = project_root.trim().replace('\\', "/");
    let root = root.trim_end_matches('/');
    let windows_root = root.as_bytes().get(1) == Some(&b':');
    let comparable_path = if windows_root {
        normalized.to_ascii_lowercase()
    } else {
        normalized.to_string()
    };
    let comparable_root = if windows_root {
        root.to_ascii_lowercase()
    } else {
        root.to_string()
    };
    let prefix = format!("{comparable_root}/");
    if let Some(relative) = comparable_path.strip_prefix(&prefix) {
        let offset = normalized.len().saturating_sub(relative.len());
        return normalized[offset..].to_string();
    }
    normalized.to_string()
}

/// Group key for the artifacts panel: complete project-relative parent path for
/// workspace files, `@kind` for inline artifacts.
pub(crate) fn artifact_group_key(a: &crate::dto::Artifact, project_root: &str) -> String {
    use crate::dto::PreviewData;
    match &a.data {
        PreviewData::File { path, .. } => {
            let display_path = a.location.as_deref().unwrap_or(path);
            let relative = artifact_workspace_path(display_path, project_root);
            let outside_workspace_absolute = relative == display_path.trim().replace('\\', "/")
                && (relative.starts_with('/')
                    || relative.as_bytes().get(1) == Some(&b':')
                    || relative.contains("://"));
            let parent = relative.rsplit_once('/').map(|(parent, _)| parent);
            match parent.filter(|parent| !parent.is_empty()) {
                Some(parent) if outside_workspace_absolute => parent
                    .rsplit('/')
                    .next()
                    .map(|name| format!("{name}/"))
                    .unwrap_or_else(|| ".".into()),
                Some(parent) => format!("{parent}/"),
                None => ".".into(),
            }
        }
        _ => format!("@{}", a.kind),
    }
}

/// Sorted artifact groups: directories first (alpha), then inline kinds.
pub(crate) fn group_artifact_indices(
    arts: &[crate::dto::Artifact],
    project_root: &str,
) -> Vec<(String, Vec<usize>)> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<(u8, String), (String, Vec<usize>)> = BTreeMap::new();
    for (i, a) in arts.iter().enumerate() {
        let key = artifact_group_key(a, project_root);
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

#[cfg(test)]
mod artifact_group_tests {
    use super::{artifact_group_key, group_artifact_indices};
    use crate::dto::{Artifact, PreviewData};

    fn file(path: &str) -> Artifact {
        Artifact {
            id: path.into(),
            name: path.rsplit(['/', '\\']).next().unwrap_or(path).into(),
            kind: "image",
            data: PreviewData::File {
                path: path.into(),
                kind: "image".into(),
            },
            location: None,
            source_item: 0,
            superseded: false,
            source_discarded: false,
        }
    }

    #[test]
    fn artifact_groups_keep_nested_workspace_paths() {
        let relative = file("DEG/output/figures/volcano.png");
        assert_eq!(
            artifact_group_key(&relative, r"D:\project"),
            "DEG/output/figures/"
        );

        let absolute = file(r"D:\project\GSEA\output\tables\hallmark.tsv");
        assert_eq!(
            artifact_group_key(&absolute, r"d:\PROJECT"),
            "GSEA/output/tables/"
        );
    }

    #[test]
    fn artifact_groups_do_not_expose_unrelated_absolute_paths() {
        let outside = file(r"C:\external\results\plot.png");
        assert_eq!(artifact_group_key(&outside, r"D:\project"), "results/");
        let remote = file("ssh://gpu/work/results/plot.png");
        assert_eq!(artifact_group_key(&remote, r"D:\project"), "results/");
    }

    #[test]
    fn registered_artifact_groups_by_workspace_location_not_snapshot_path() {
        let snapshot = Artifact {
            id: "artifact-1".into(),
            name: "report.md".into(),
            kind: "markdown",
            data: PreviewData::File {
                path: ".wisp/artifacts/sha256/aa/report.md".into(),
                kind: "markdown".into(),
            },
            location: Some("results/report.md".into()),
            source_item: 0,
            superseded: false,
            source_discarded: false,
        };
        assert_eq!(artifact_group_key(&snapshot, r"D:\project"), "results/");
    }

    #[test]
    fn nested_groups_remain_distinct() {
        let artifacts = vec![
            file("PCA/output/figures/plot.png"),
            file("DEG/output/figures/plot.png"),
        ];
        let groups = group_artifact_indices(&artifacts, r"D:\project");
        assert_eq!(
            groups
                .iter()
                .map(|(key, indices)| (key.as_str(), indices.as_slice()))
                .collect::<Vec<_>>(),
            vec![
                ("DEG/output/figures/", &[1][..]),
                ("PCA/output/figures/", &[0][..]),
            ]
        );
    }
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

/// Highlight language for a preview path. Durable resource tabs use
/// `artifact-version:<id>` (no extension), so fall back to the display filename.
pub(crate) fn preview_code_lang(path: &str, filename: Option<&str>) -> &'static str {
    code_lang(path)
        .or_else(|| filename.and_then(code_lang))
        .unwrap_or("plaintext")
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceSelection {
    pub(crate) before: String,
    pub(crate) selected: String,
    pub(crate) after: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceExecution {
    pub(crate) code: String,
    /// When running a collapsed caret, move it to the following line just like
    /// RStudio. A real selection stays selected, so this is `None` for it.
    pub(crate) next_caret_utf16: Option<u32>,
}

fn utf16_to_byte_index(text: &str, target: u32) -> usize {
    let mut utf16 = 0_u32;
    for (byte, ch) in text.char_indices() {
        if utf16 >= target {
            return byte;
        }
        let next = utf16.saturating_add(ch.len_utf16() as u32);
        if next > target {
            return byte;
        }
        utf16 = next;
    }
    text.len()
}

fn byte_to_utf16_index(text: &str, byte: usize) -> u32 {
    text[..byte.min(text.len())].encode_utf16().count() as u32
}

fn source_line_at(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())]
        .bytes()
        .filter(|ch| *ch == b'\n')
        .count()
        + 1
}

/// Split a textarea selection using its UTF-16 DOM offsets. Keeping this
/// conversion explicit matters for Chinese source comments and astral Unicode:
/// Rust string indexes are bytes, while `selectionStart`/`selectionEnd` are
/// UTF-16 code units.
pub(crate) fn source_selection(text: &str, start_utf16: u32, end_utf16: u32) -> SourceSelection {
    let (start_utf16, end_utf16) = if start_utf16 <= end_utf16 {
        (start_utf16, end_utf16)
    } else {
        (end_utf16, start_utf16)
    };
    let start = utf16_to_byte_index(text, start_utf16);
    let end = utf16_to_byte_index(text, end_utf16).max(start);
    // When the selection ends immediately after a newline, that newline still
    // belongs to the preceding rendered line; do not report the empty next line.
    let end_for_line = if end > start && text.as_bytes().get(end - 1) == Some(&b'\n') {
        end - 1
    } else {
        end
    };
    SourceSelection {
        before: text[..start].to_string(),
        selected: text[start..end].to_string(),
        after: text[end..].to_string(),
        start_line: source_line_at(text, start),
        end_line: source_line_at(text, end_for_line),
    }
}

/// RStudio-style execution unit: run the selection when present, otherwise
/// run the current line and advance the caret to the following line.
pub(crate) fn source_execution(
    text: &str,
    start_utf16: u32,
    end_utf16: u32,
) -> Option<SourceExecution> {
    if end_utf16 > start_utf16 {
        let selection = source_selection(text, start_utf16, end_utf16);
        return (!selection.selected.trim().is_empty()).then_some(SourceExecution {
            code: selection.selected,
            next_caret_utf16: None,
        });
    }

    let caret = utf16_to_byte_index(text, start_utf16);
    let line_start = text[..caret].rfind('\n').map_or(0, |newline| newline + 1);
    let line_end = text[caret..]
        .find('\n')
        .map_or(text.len(), |offset| caret + offset);
    let code = text[line_start..line_end].to_string();
    if code.trim().is_empty() {
        return None;
    }
    let next = if line_end < text.len() {
        line_end + 1
    } else {
        line_end
    };
    Some(SourceExecution {
        code,
        next_caret_utf16: Some(byte_to_utf16_index(text, next)),
    })
}

/// R/Python source selections keep the runtime-centric popup (run, ask AI,
/// quote, explain). Literature research and review notes belong on papers
/// and chat, not on executable source.
pub(crate) fn is_runtime_code_selection(source: Option<&str>) -> bool {
    source.and_then(runtime_language).is_some()
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
        "doc" | "docm" | "odt" | "rtf" | "epub" | "xls" | "xlsm" | "xlsb" | "ods" | "ppt"
        | "pps" | "pot" | "pptm" | "ppsx" | "ppsm" | "odp" => "document",
        // ponytail: LaTeX sources open as plain text, not code. The vendored
        // highlight.js is the common bundle with no `latex` grammar, and asking
        // it for one throws; the `latex` preview kind is KaTeX for inline
        // formulas, which chokes on a whole document. Typesetting a .tex file is
        // a separate project — vendor a latex grammar first if highlighting is
        // wanted.
        "bib" | "tex" | "latex" => "text",
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
    pub(crate) workflows: Vec<String>,
    pub(crate) contexts: Vec<String>,
    pub(crate) runtimes: Vec<String>,
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
        } else if let Some(value) = block.strip_prefix("Selected workflows: ") {
            Some((&mut presentation.workflows, value))
        } else if let Some(value) = block.strip_prefix("Target environments: ") {
            Some((&mut presentation.contexts, value))
        } else if let Some(value) = block.strip_prefix("Target runtimes: ") {
            Some((&mut presentation.runtimes, value))
        } else if block.starts_with("AI source-edit instruction: ") {
            // This persisted, agent-facing hint turns a source selection into
            // an actionable edit target. It is transport metadata, not text
            // the user typed, so keep it out of the rendered chat bubble.
            continue;
        } else if block.starts_with("Feedback context: ") {
            // Diagnostic context is sent to the agent with the first feedback
            // turn, but is not text the user typed.
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
        code_lang, decode_href, fence_identifier_line_runs, file_kind, format_bytes,
        is_runtime_code_selection, md_document_to_html, md_inline_to_html, md_to_html, parent_path,
        parse_notebook, pretty_json, preview_code_lang, push_nb_output, runtime_language,
        source_execution, source_selection, strip_ansi, tool_card_label, user_message_presentation,
        NbOutput, MAX_NB_OUTPUT_BYTES, MAX_NB_TOTAL_OUTPUT_BYTES,
    };

    #[test]
    fn rejoins_text_dropped_after_a_bare_list_marker() {
        // The screenshot case: `- ` alone on a line, item text on the next.
        let html = md_to_html("- 450 = x\n- \nTb1 15,248,784 y\n");
        assert!(html.contains("<li>Tb1 15,248,784 y</li>"), "{html}");
        assert!(!html.contains("<li></li>"), "{html}");
    }

    #[test]
    fn rejoins_bare_ordered_markers_and_keeps_lazy_continuation() {
        let html = md_to_html("1. a\n2.\nb\n");
        assert!(html.contains("<li>b</li>"), "{html}");
        assert!(!html.contains("<li></li>"), "{html}");
        // A wrapped line after the rejoined item stays inside the item.
        let html = md_to_html("- \nlong item\ntail\n");
        assert!(html.contains("<li>long item\ntail</li>"), "{html}");
    }

    #[test]
    fn rejoins_a_bare_marker_across_one_accidental_blank_line() {
        let html = md_to_html("- QC passed\n-\n\nNormalize → PCA\n");
        assert!(html.contains("<li>Normalize → PCA</li>"), "{html}");
        assert!(!html.contains("<li></li>"), "{html}");

        // Two blank lines are enough to preserve an intentionally empty item
        // and a separate paragraph.
        let html = md_to_html("- QC passed\n-\n\n\nSeparate paragraph\n");
        assert!(html.contains("<li></li>"), "{html}");
    }

    #[test]
    fn marks_only_paragraphs_that_begin_with_strong() {
        // Chat CSS draws a lead bar on standalone/section-lead bold. `:first-child`
        // would also match mid-sentence `「**点**」` because text nodes do not count.
        let html = md_to_html(
            "哈哈，这个接得妙！\n\n**可圈可点**\n\n该你了，接一个「**点**」字开头的成语！\n\n你接一个以**上一个成语最后一个字**开头的成语。",
        );
        assert!(
            html.contains(r#"<p class="md-lead-strong"><strong>可圈可点</strong></p>"#),
            "{html}"
        );
        assert!(
            html.contains("<p>该你了，接一个「<strong>点</strong>」字开头的成语！</p>"),
            "{html}"
        );
        assert!(
            html.contains("<p>你接一个以<strong>上一个成语最后一个字</strong>开头的成语。</p>"),
            "{html}"
        );
    }

    #[test]
    fn marks_lead_strong_with_following_prose() {
        let html = md_to_html("**Module results** (Seurat 5):\n");
        assert!(
            html.contains(
                r#"<p class="md-lead-strong"><strong>Module results</strong> (Seurat 5):</p>"#
            ),
            "{html}"
        );
    }

    #[test]
    fn unwraps_lead_strong_paragraphs_for_inline_html() {
        assert_eq!(md_inline_to_html("**bold**"), "<strong>bold</strong>");
        assert_eq!(md_inline_to_html("plain"), "plain");
    }

    #[test]
    fn leaves_block_starts_and_code_fences_untouched() {
        // Indented continuations already attach to the item; do not rejoin.
        let src = "- a\n- \n  b\n";
        assert_eq!(md_to_html(src), "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n");
        // A bare marker followed by another item keeps its (empty) meaning.
        let html = md_to_html("- a\n-\n- b\n");
        assert!(html.contains("<li></li>"), "{html}");
        // One blank line is repaired, but a whitespace-only continuation plus
        // another blank remains an intentional separation.
        for next in ["", "  ", " \t"] {
            let html = md_to_html(&format!("- \n{next}\n\nb\n"));
            assert!(html.contains("<li></li>"), "{next:?}: {html}");
        }
        // Thematic breaks, headings, quotes and tables are not item text.
        for next in ["---", "# h", "> q", "| a | b |"] {
            let html = md_to_html(&format!("- \n{next}\n"));
            assert!(html.contains("<li></li>"), "{next}: {html}");
        }
        // Bare markers inside fenced code are data, not list syntax.
        let src = "```\n-\nfoo\n```\n";
        assert_eq!(md_to_html(src), "<pre><code>-\nfoo\n</code></pre>\n");
    }

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
        let html = md_document_to_html(src);
        assert!(html.contains("<h1>Title</h1>"), "{html}");
        assert!(!html.contains("skill: bear-counter"), "{html}");
        assert!(!html.contains("topic: demo"), "{html}");
    }

    #[test]
    fn keeps_front_matter_like_text_in_chat_messages() {
        let src = "---\nskill: bear-counter\ntopic: demo\n---\n\n# Title\n\nBody\n";
        let html = md_to_html(src);
        assert!(html.contains("skill: bear-counter"), "{html}");
        assert!(html.contains("topic: demo"), "{html}");
        assert!(html.contains("<h1>Title</h1>"), "{html}");
    }

    #[test]
    fn keeps_rule_wrapped_chat_body_after_closing_rule_streams_in() {
        let partial = "---\n\n**Figure 3. Example title.**\n\nColor scale: RdBu_r.";
        let partial_html = md_to_html(partial);
        assert!(
            partial_html.contains("Figure 3. Example title."),
            "{partial_html}"
        );
        assert!(
            partial_html.contains("Color scale: RdBu_r."),
            "{partial_html}"
        );

        let complete = format!("{partial}\n\n---\n");
        let complete_html = md_to_html(&complete);
        assert!(
            complete_html.contains("Figure 3. Example title."),
            "{complete_html}"
        );
        assert!(
            complete_html.contains("Color scale: RdBu_r."),
            "{complete_html}"
        );
    }

    #[test]
    fn keeps_rule_wrapped_chat_body_with_crlf() {
        let src = "---\r\n\r\n**Figure 3.**\r\n\r\nColor scale: RdBu_r.\r\n\r\n---\r\n";
        let html = md_to_html(src);
        assert!(html.contains("Figure 3."), "{html}");
        assert!(html.contains("Color scale: RdBu_r."), "{html}");
    }

    #[test]
    fn strips_crlf_yaml_front_matter_from_markdown_preview() {
        let src = "---\r\nskill: bear-counter\r\ntopic: demo\r\n---\r\n\r\n# Title\r\n";
        let html = md_document_to_html(src);
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
    fn rewrites_windows_clipboard_image_tags_to_clickable_links() {
        let src = r#"<image name="clipboard-preview.png" path="C:\Users\Alice\AppData\Local\Temp\clipboard-preview.png"></image>"#;
        let html = md_to_html(src);
        assert!(html.contains("clipboard-preview.png"), "{html}");
        assert!(html.contains("<a href="), "{html}");
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
        assert_eq!(file_kind("paper.tex"), Some("text"));
        assert_eq!(file_kind("paper.latex"), Some("text"));
        // The text preview reads its language off code_lang, which must keep
        // saying "no grammar" so the highlighter is never asked for `latex`.
        assert_eq!(code_lang("paper.tex"), None);
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
        assert_eq!(
            preview_code_lang(
                "artifact-version:resource-version-python",
                Some("random_walk_demo.py")
            ),
            "python"
        );
        assert_eq!(
            preview_code_lang("artifact-version:resource-version-r", Some("plot.R")),
            "r"
        );
        assert_eq!(preview_code_lang("scripts/regulon2gmt.py", None), "python");
        assert_eq!(
            preview_code_lang("artifact-version:resource-version-txt", Some("notes.txt")),
            "plaintext"
        );
        // Rmd/qmd are Markdown with chunks — the Markdown preview already
        // highlights fenced code, so they must not fall into the code branch.
        assert_eq!(file_kind("analysis.Rmd"), Some("markdown"));
        assert_eq!(file_kind("analysis.qmd"), Some("markdown"));
        assert_eq!(file_kind("analysis.ipynb"), Some("notebook"));
        assert_eq!(file_kind("protocol.rtf"), Some("document"));
        assert_eq!(file_kind("supplement.odt"), Some("document"));
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
            "Inspect this\n\nUploaded files: uploads/plot.png, data.csv\n\nAttached artifacts: counts.csv\n\nProject context: Atlas\n\nSelected skills: bear-review\n\nSelected workflows: Roundtable\n\nTarget environments: CPU, GPU\n\nTarget runtimes: Python · GPU\n\nAI source-edit instruction: hidden\n\nFeedback context: hidden diagnostics",
        );
        assert_eq!(parsed.body, "Inspect this");
        assert_eq!(parsed.attachments, ["uploads/plot.png", "data.csv"]);
        assert_eq!(parsed.artifacts, ["counts.csv"]);
        assert_eq!(parsed.projects, ["Atlas"]);
        assert_eq!(parsed.skills, ["bear-review"]);
        assert_eq!(parsed.workflows, ["Roundtable"]);
        assert_eq!(parsed.contexts, ["CPU", "GPU"]);
        assert_eq!(parsed.runtimes, ["Python · GPU"]);
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
    fn runtime_code_selections_skip_literature_and_review_actions() {
        assert!(is_runtime_code_selection(Some(
            "analysis/scripts/mathys2019_umap.R"
        )));
        assert!(is_runtime_code_selection(Some("qc.py")));
        assert!(!is_runtime_code_selection(Some("notes.md")));
        assert!(!is_runtime_code_selection(Some("paper.pdf")));
        assert!(!is_runtime_code_selection(Some("artifact:art-markdown")));
        assert!(!is_runtime_code_selection(None));
    }

    #[test]
    fn source_selection_maps_utf16_offsets_and_reports_selected_lines() {
        let text = "第一行\nplot(1:3)\n🙂 done\n";
        let start = "第一行\n".encode_utf16().count() as u32;
        let end = "第一行\nplot(1:3)\n".encode_utf16().count() as u32;
        let selection = source_selection(text, start, end);
        assert_eq!(selection.before, "第一行\n");
        assert_eq!(selection.selected, "plot(1:3)\n");
        assert_eq!(selection.after, "🙂 done\n");
        assert_eq!((selection.start_line, selection.end_line), (2, 2));

        // The emoji occupies two UTF-16 code units but remains one Rust char.
        let emoji_start = "第一行\nplot(1:3)\n".encode_utf16().count() as u32;
        let emoji_end = emoji_start + "🙂".encode_utf16().count() as u32;
        assert_eq!(
            source_selection(text, emoji_start, emoji_end).selected,
            "🙂"
        );
    }

    #[test]
    fn source_execution_runs_selection_or_current_line_and_advances() {
        let text = "x <- 1\n绘图 <- x + 1\nprint(绘图)\n";
        let selected_start = "x <- 1\n".encode_utf16().count() as u32;
        let selected_end = selected_start + "绘图 <- x + 1".encode_utf16().count() as u32;
        assert_eq!(
            source_execution(text, selected_start, selected_end),
            Some(super::SourceExecution {
                code: "绘图 <- x + 1".into(),
                next_caret_utf16: None,
            })
        );

        let caret = selected_start + 2;
        assert_eq!(
            source_execution(text, caret, caret),
            Some(super::SourceExecution {
                code: "绘图 <- x + 1".into(),
                next_caret_utf16: Some("x <- 1\n绘图 <- x + 1\n".encode_utf16().count() as u32),
            })
        );
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

#[cfg(test)]
mod provider_defaults_tests {
    use super::{provider_defaults, DEEPSEEK_FLASH_MODEL};

    #[test]
    fn openai_compatible_defaults_to_deepseek_flash() {
        assert_eq!(
            provider_defaults("openai"),
            ("https://api.deepseek.com", DEEPSEEK_FLASH_MODEL)
        );
    }
}
