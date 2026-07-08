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
        "codex_app" | "codex-app" | "codex_app_server" | "codex-app-server" => "codex_app",
        "codex_cli" | "codex-cli" | "codex" => "codex_cli",
        "anthropic" => "anthropic",
        "openai_responses" | "openai-responses" | "responses" => "openai_responses",
        _ => "openai",
    }
}

pub(crate) fn provider_defaults(provider: &str) -> (&'static str, &'static str) {
    match provider_value(provider) {
        "codex_cli" | "codex_app" => ("", "gpt-5.5"),
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

pub(crate) fn next_artifact_id(n: usize) -> String {
    format!("{:08x}", n + 1)
}

pub(crate) fn normalize_path(path: &str) -> String {
    // Only strip redundant `./` prefixes. Do NOT strip a leading `/` — the agent
    // is told to emit absolute paths (system_prompt.rs), and the backend resolves
    // absolute-under-root correctly; stripping the slash turned an absolute path
    // into a bad root-relative one and 404'd on click (#12).
    path.trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .to_string()
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
        "nwk" | "newick" | "treefile" | "tre" => "text",
        "txt" | "log" | "json" => "text",
        _ => return None,
    })
}

pub(crate) fn fasta_seq_count(text: &str) -> usize {
    text.lines()
        .filter(|l| l.trim_start().starts_with('>'))
        .count()
}
