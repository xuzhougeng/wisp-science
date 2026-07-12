//! Pure helpers: string/path/format transforms, Markdown-to-HTML rendering,
//! CSV/table classification, and small DOM value extractors.
//!
//! Everything here is a plain function with no Leptos signals, no app state,
//! and no `crate::dto` types — just data in, data out. That makes this the one
//! module in the UI that is trivially unit-testable and freely reusable; keep
//! new coupling-free utilities here instead of growing `main.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

static NEXT_DOM_ID: AtomicUsize = AtomicUsize::new(0);

/// Process-unique DOM id with the given prefix (for mounting/highlight targets).
pub(crate) fn unique_dom_id(prefix: &str) -> String {
    format!("{prefix}-{}", NEXT_DOM_ID.fetch_add(1, Ordering::Relaxed))
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
    match p.rsplit_once('/') {
        None | Some(("", _)) => ".".into(),
        Some((a, _)) if a.is_empty() => ".".into(),
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
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

pub(crate) fn event_target_value(ev: &web_sys::Event) -> String {
    // Works for both <input> and <textarea>. Casting the wrong one used to
    // panic in the event handler (input never registered) — see the project
    // name field.
    let target = ev.target().unwrap();
    if let Some(i) = target.dyn_ref::<web_sys::HtmlInputElement>() {
        return i.value();
    }
    if let Some(a) = target.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        return a.value();
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
    let parser = Parser::new_ext(src.as_ref(), opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

fn preprocess_markdown(src: &str) -> std::borrow::Cow<'_, str> {
    let src = strip_yaml_front_matter(src);
    match rewrite_image_tags(src.as_ref()) {
        std::borrow::Cow::Borrowed(_) => src,
        std::borrow::Cow::Owned(s) => std::borrow::Cow::Owned(s),
    }
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
        "md" => "markdown",
        "html" | "htm" => "html",
        "nwk" | "newick" | "treefile" | "tre" => "text",
        "json" => "json",
        "txt" | "log" => "text",
        _ => return None,
    })
}

pub(crate) fn fasta_seq_count(text: &str) -> usize {
    text.lines()
        .filter(|l| l.trim_start().starts_with('>'))
        .count()
}

#[cfg(test)]
mod md_catalog_tests {
    use super::{fence_identifier_line_runs, file_kind, md_to_html, pretty_json};

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
    fn detects_html_files_for_preview() {
        assert_eq!(file_kind("report.html"), Some("html"));
        assert_eq!(file_kind("report.htm"), Some("html"));
    }

    #[test]
    fn detects_json_files_for_preview() {
        assert_eq!(file_kind("report.json"), Some("json"));
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
}
