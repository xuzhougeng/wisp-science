use super::*;

pub(crate) fn js_error_text(err: JsValue) -> String {
    err.as_string()
        .or_else(|| {
            js_sys::Reflect::get(&err, &JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_else(|| t(Locale::En, "err.unknown").into())
}

pub(crate) fn show_copy_toast() {
    let is_zh = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .and_then(|element| element.get_attribute("lang"))
        .as_deref()
        == Some("zh");
    show_toast(if is_zh { "已复制" } else { "Copied" });
}

pub(crate) fn show_toast(message: &str) {
    show_toast_for(message, "copy-toast", 1_600);
}

pub(crate) fn show_warning_toast(message: &str) {
    show_toast_for(message, "copy-toast copy-toast-warning", 1_600);
}

/// Longer-lived variant for toasts the user must read and act on (setup
/// instructions), instead of the blink-and-gone copy confirmation.
pub(crate) fn show_actionable_toast(message: &str) {
    show_toast_for(message, "copy-toast", 8_000);
}

pub(crate) fn show_actionable_warning_toast(message: &str) {
    show_toast_for(message, "copy-toast copy-toast-warning", 8_000);
}

fn show_toast_for(message: &str, class_name: &str, duration_ms: i32) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    if let Some(old) = document.get_element_by_id("copy-toast") {
        old.remove();
    }
    let Ok(toast) = document.create_element("div") else {
        return;
    };
    toast.set_id("copy-toast");
    toast.set_class_name(class_name);
    toast.set_text_content(Some(message));
    let Some(body) = document.body() else {
        return;
    };
    if body.append_child(&toast).is_err() {
        return;
    }
    let remove = Closure::once(move || toast.remove());
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        remove.as_ref().unchecked_ref(),
        duration_ms,
    );
    remove.forget();
}

pub(crate) fn copy_text(text: String) {
    if text.is_empty() {
        return;
    }
    spawn_local(async move {
        let Some(window) = web_sys::window() else {
            return;
        };
        let promise = window.navigator().clipboard().write_text(&text);
        if wasm_bindgen_futures::JsFuture::from(promise).await.is_ok() {
            show_copy_toast();
        }
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

pub(crate) fn table_data_to_tsv(t: &TableData) -> String {
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
        let Ok(row) = row.dyn_into::<web_sys::HtmlTableRowElement>() else {
            continue;
        };
        let cells = row.cells();
        let mut vals = Vec::with_capacity(cells.length() as usize);
        for j in 0..cells.length() {
            let Some(cell) = cells.item(j) else { continue };
            vals.push(normalize_table_copy_cell(
                &cell.text_content().unwrap_or_default(),
            ));
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

fn wrap_markdown_code_blocks_with_copy_controls(html: String, locale: Locale) -> String {
    let copy_label = html_escape(&t(locale, "tool.copy_code"));
    let mut out = String::with_capacity(html.len());
    let mut rest = html.as_str();
    while let Some(start) = rest.find(r#"<pre class="md-code">"#) {
        out.push_str(&rest[..start]);
        let pre_rest = &rest[start..];
        let Some(end) = pre_rest.find("</pre>") else {
            out.push_str(pre_rest);
            return out;
        };
        let pre_html = &pre_rest[..end + "</pre>".len()];
        out.push_str(&format!(
            r#"<div class="md-code-card"><button type="button" class="md-code-copy" title="{copy_label}" aria-label="{copy_label}"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg></button>{pre_html}</div>"#
        ));
        rest = &pre_rest[end + "</pre>".len()..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn art_label(a: &Artifact) -> String {
    if a.name.len() <= 28 {
        a.name.clone()
    } else {
        format!("artifact-{}", &a.id[..8.min(a.id.len())])
    }
}

pub(crate) fn art_chip(idx: usize, a: &Artifact) -> String {
    let label = html_escape(&art_label(a));
    let title = html_escape(&a.name);
    let path_attr = artifact_workspace_reference(a)
        .map(|path| format!(r#" data-workspace-path="{}""#, html_escape(path)))
        .unwrap_or_default();
    format!(
        r#"<button type="button" class="art-ref" data-art-idx="{idx}"{path_attr} title="{title}">{label}</button>"#
    )
}

fn artifact_workspace_reference(a: &Artifact) -> Option<&str> {
    a.location.as_deref().or_else(|| match &a.data {
        PreviewData::File { path, .. } => Some(path.as_str()),
        _ => None,
    })
}

pub(crate) fn artifact_file_paths(a: &Artifact) -> Vec<String> {
    match &a.data {
        PreviewData::File { path, .. } => {
            let mut out = vec![normalize_path(path)];
            if let Some(location) = a.location.as_deref() {
                let location = normalize_path(location);
                if !out.contains(&location) {
                    out.push(location);
                }
            }
            if let Some(name) = path.rsplit(['/', '\\']).next() {
                let name = normalize_path(name);
                if !out.contains(&name) {
                    out.push(name);
                }
            }
            let name = normalize_path(&a.name);
            if !out.contains(&name) {
                out.push(name);
            }
            out
        }
        _ => vec![normalize_path(&a.name)],
    }
}

pub(crate) fn href_matches_artifact(href: &str, a: &Artifact) -> bool {
    let h = normalize_path(href);
    artifact_file_paths(a).iter().any(|p| *p == h)
}

pub(crate) fn artifact_index_for_href(arts: &[Artifact], href: &str) -> Option<usize> {
    arts.iter().position(|a| href_matches_artifact(href, a))
}

pub(crate) fn replace_file_links(html: String, arts: &[Artifact]) -> String {
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

        if let Some(href) = extract_href_from_tag(tag).map(|h| decode_href(&h)) {
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

pub(crate) fn artifact_matches_token(token: &str, id: &str) -> bool {
    let t = token.trim();
    t == id
        || t.starts_with(id)
        || id.starts_with(&t[..t.len().min(8)])
        || t.starts_with(&id[..id.len().min(8)])
}

pub(crate) fn replace_artifact_tokens(mut html: String, arts: &[Artifact]) -> String {
    while let Some(start) = html.find("{{artifact:") {
        let (head, rest) = html.split_at(start);
        let rest = &rest["{{artifact:".len()..];
        let Some(end) = rest.find("}}") else {
            break;
        };
        let token = rest[..end].trim();
        let tail = &rest[end + 2..];
        let chip = arts
            .iter()
            .enumerate()
            .find_map(|(i, a)| {
                if artifact_matches_token(token, &a.id) {
                    Some(art_chip(i, a))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                let short = &token[..token.len().min(8)];
                format!(r#"<span class="art-ref dead" title="{token}">artifact-{short}</span>"#)
            });
        html = format!("{head}{chip}{tail}");
    }
    html
}

/// Promote bare `<code>filename</code>` to artifact chips, without nesting
/// inside an existing `.art-ref` (browsers auto-split nested `<button>`s into
/// an empty outer chip + a filled sibling — the dashed pills in lists).
pub(crate) fn wrap_code_filenames_as_art_refs(html: String, arts: &[Artifact]) -> String {
    let mut html = html;
    for (i, a) in arts.iter().enumerate() {
        let fname = html_escape(&a.name);
        if fname.is_empty() {
            continue;
        }
        let needle = format!("<code>{fname}</code>");
        let path_attr = artifact_workspace_reference(a)
            .map(|path| format!(r#" data-workspace-path="{}""#, html_escape(path)))
            .unwrap_or_default();
        let replacement = format!(
            r#"<button type="button" class="art-ref" data-art-idx="{i}"{path_attr} title="{fname}"><code>{fname}</code></button>"#
        );
        html = replace_code_outside_art_refs(&html, &needle, &replacement);
    }
    html
}

fn replace_code_outside_art_refs(html: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(idx) = rest.find(needle) {
        let before = &rest[..idx];
        out.push_str(before);
        if code_is_inside_art_ref(before) {
            out.push_str(needle);
        } else {
            out.push_str(replacement);
        }
        rest = &rest[idx + needle.len()..];
    }
    out.push_str(rest);
    out
}

fn code_is_inside_art_ref(before: &str) -> bool {
    let open_btn = before.rfind(r#"class="art-ref""#);
    let close_btn = before.rfind("</button>");
    let close_span = before.rfind("</span>");
    let last_close = match (close_btn, close_span) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    match (open_btn, last_close) {
        (Some(o), Some(c)) => o > c,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Drop stray list markers left in front of artifact chips.
/// Models often write `- \`file\`` inside table cells; after chip promotion the
/// leading `- ` remains as a dash beside the pill.
pub(crate) fn strip_list_markers_before_art_refs(html: &str) -> String {
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

/// Malformed model Markdown can place the separator between two resource
/// links in its own paragraph (`link\n\n、\n\nlink`). Once the links become
/// compact resource chips that paragraph creates a conspicuous empty row (or
/// a lone backtick). Fold only punctuation-only paragraphs back into the
/// surrounding paragraph; ordinary prose and intentional paragraph breaks
/// remain untouched.
fn collapse_orphan_separator_paragraphs(mut html: String) -> String {
    for (paragraph, replacement) in [
        ("<p>、</p>", " 、 "),
        ("<p>，</p>", "， "),
        ("<p>,</p>", ", "),
        ("<p>`</p>", " "),
    ] {
        let needle = format!("</p>\n{paragraph}\n<p>");
        while html.contains(&needle) {
            html = html.replacen(&needle, replacement, 1);
        }
    }
    html
}

fn html_attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!(r#"{name}=""#);
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

fn decode_html_attribute(value: &str) -> String {
    value
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn resource_reference_matches(rendered: &str, original: &str) -> bool {
    normalize_path(&decode_href(&decode_html_attribute(rendered)))
        == normalize_path(&decode_href(original))
}

/// Replace path-bearing Markdown tags with durable resource identities. Only
/// bindings persisted with this exact message are considered; old unbound
/// messages intentionally retain their original behavior.
fn replace_bound_resource_tags(html: String, resources: &[MessageResource]) -> String {
    if resources.is_empty() {
        return html;
    }
    let mut out = String::with_capacity(html.len() + resources.len() * 48);
    let mut rest = html.as_str();
    let mut consumed = vec![false; resources.len()];
    loop {
        let next = [(rest.find("<a "), "href"), (rest.find("<img "), "src")]
            .into_iter()
            .filter_map(|(position, attribute)| position.map(|position| (position, attribute)))
            .min_by_key(|(position, _)| *position);
        let Some((position, attribute)) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..position]);
        rest = &rest[position..];
        let Some(end) = rest.find('>') else {
            out.push_str(rest);
            break;
        };
        let tag = &rest[..=end];
        rest = &rest[end + 1..];
        let Some(reference) = html_attr(tag, attribute) else {
            out.push_str(tag);
            continue;
        };
        let Some(resource_index) = resources.iter().enumerate().position(|(index, resource)| {
            !consumed[index] && resource_reference_matches(&reference, &resource.original_reference)
        }) else {
            out.push_str(tag);
            continue;
        };
        consumed[resource_index] = true;
        let resource = &resources[resource_index];
        let old = format!(r#"{attribute}="{}""#, reference);
        let title = html_escape(
            resource
                .error
                .as_deref()
                .unwrap_or(resource.display_name.as_str()),
        );
        if attribute == "src" && resource.status != "ready" {
            out.push_str(&format!(
                r#"<span class="resource-unresolved" data-resource-id="{}" data-resource-status="unresolved" title="{title}">{}</span>"#,
                html_escape(&resource.id),
                html_escape(&format!("{} — {title}", resource.display_name)),
            ));
            continue;
        }
        let replacement = if resource.status == "ready" && resource.artifact_version_id.is_some() {
            let value = if attribute == "src" { "" } else { "#" };
            format!(
                r#"{attribute}="{value}" data-resource-id="{}" data-resource-kind="{}" data-resource-status="ready" title="{title}""#,
                html_escape(&resource.id),
                html_escape(&resource.kind),
            )
        } else {
            let value = if attribute == "src" { "" } else { "#" };
            format!(
                r#"{attribute}="{value}" data-resource-id="{}" data-resource-kind="{}" data-resource-status="unresolved" title="{title}""#,
                html_escape(&resource.id),
                html_escape(&resource.kind),
            )
        };
        out.push_str(&tag.replacen(&old, &replacement, 1));
    }
    out
}

/// Post-process rendered Markdown: durable resources, artifact chips, code
/// wrappers, and filename links.
pub(crate) fn enrich_md_html(
    mut html: String,
    arts: &[Artifact],
    resources: &[MessageResource],
    locale: Locale,
    project_root: Option<&str>,
) -> String {
    html = replace_bound_resource_tags(html, resources);
    html = replace_artifact_tokens(html, arts);
    html = replace_file_links(html, arts);
    for (i, a) in arts.iter().enumerate() {
        let chip = art_chip(i, a);
        let marker = format!("{{{{artifact:{}}}}}", a.id);
        html = html.replace(&marker, &chip);
    }
    html = wrap_code_filenames_as_art_refs(html, arts);
    html = linkify_bare_urls(html);
    html = wrap_inline_workspace_paths(html, project_root);
    html = strip_list_markers_before_art_refs(&html);
    html = collapse_orphan_separator_paragraphs(html);
    html = html.replace("<pre><code", "<pre class=\"md-code\"><code");
    html = wrap_markdown_code_blocks_with_copy_controls(html, locale);
    html = wrap_markdown_tables_with_copy_controls(html, locale);
    html
}

/// Turn inline-code project paths into ordinary Markdown-style file links.
/// The href remains the portable relative path that `handle_md_click` resolves,
/// while an absolute in-project path is shortened for display. Code blocks,
/// artifact chips, URLs, commands, and paths outside the project are untouched.
fn wrap_inline_workspace_paths(html: String, project_root: Option<&str>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html.as_str();
    while let Some(start) = rest.find("<code>") {
        let before = &rest[..start];
        out.push_str(before);
        let code_rest = &rest[start + "<code>".len()..];
        let Some(end) = code_rest.find("</code>") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let encoded = &code_rest[..end];
        let candidate = decode_html_attribute(encoded);
        let relative = workspace_relative_path(project_root.unwrap_or_default(), &candidate);
        let linked = relative
            .filter(|path| !path.is_empty() && !path.chars().any(char::is_whitespace))
            .filter(|path| file_kind(path).is_some())
            .filter(|_| !code_is_inside_art_ref(before) && !code_is_inside_link(before));
        if let Some(path) = linked {
            let escaped = html_escape(&path);
            out.push_str(&format!(
                r#"<a class="workspace-path-link" href="{escaped}" data-workspace-path="{escaped}"><code>{escaped}</code></a>"#
            ));
        } else {
            out.push_str("<code>");
            out.push_str(encoded);
            out.push_str("</code>");
        }
        rest = &code_rest[end + "</code>".len()..];
    }
    out.push_str(rest);
    out
}

/// Turn bare `http(s)://…` runs into links. CommonMark has no GFM autolink
/// literals and pulldown-cmark implements none, so a URL the agent simply
/// types stays plain text and cannot be opened — only `[text](url)` and
/// `<url>` produced anchors. Text inside a tag, an existing anchor, or code is
/// left alone; the result routes through the same external-link confirmation
/// as any other link.
fn linkify_bare_urls(html: String) -> String {
    if !html.contains("http://") && !html.contains("https://") {
        return html;
    }
    let src = html.as_str();
    let mut out = String::with_capacity(src.len());
    let mut copied = 0usize;
    let mut i = 0usize;
    let mut skipping: Option<&'static str> = None;
    while i < src.len() {
        if src.as_bytes()[i] == b'<' {
            let end = src[i..]
                .find('>')
                .map_or(src.len(), |offset| i + offset + 1);
            let tag = &src[i..end];
            let closing = tag.starts_with("</");
            match (skipping, container_tag_name(tag)) {
                (Some(open), Some(name)) if closing && name == open => skipping = None,
                (None, Some(name)) if !closing && !tag.ends_with("/>") => skipping = Some(name),
                _ => {}
            }
            i = end;
            continue;
        }
        let candidate = skipping.is_none()
            && (src[i..].starts_with("http://") || src[i..].starts_with("https://"))
            && !src[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric());
        if candidate {
            let end = src[i..]
                .find(|c: char| !url_char(c))
                .map_or(src.len(), |offset| i + offset);
            let url = trim_url_tail(&src[i..end]);
            if url
                .split_once("//")
                .is_some_and(|(_, host)| !host.is_empty())
            {
                out.push_str(&src[copied..i]);
                out.push_str(&format!(r#"<a href="{url}">{url}</a>"#));
                i += url.len();
                copied = i;
                continue;
            }
        }
        i += src[i..].chars().next().map_or(1, char::len_utf8);
    }
    out.push_str(&src[copied..]);
    out
}

/// The element name of `<a>`/`<code>`/`<pre>` open or close tags, whose text
/// content must never be autolinked.
fn container_tag_name(tag: &str) -> Option<&'static str> {
    let name: String = tag
        .trim_start_matches('<')
        .trim_start_matches('/')
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    ["a", "code", "pre"]
        .into_iter()
        .find(|known| name.eq_ignore_ascii_case(known))
}

/// A URL is ASCII, so any other character ends it — including CJK punctuation
/// and prose, which often follows a link with no space in between.
fn url_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c)
}

/// Drop sentence punctuation that follows a bare URL rather than belonging to
/// it, plus unbalanced closing brackets and a trailing HTML entity.
fn trim_url_tail(url: &str) -> &str {
    let mut url = url;
    loop {
        let before = url;
        for entity in ["&quot;", "&amp;", "&#39;", "&gt;", "&lt;"] {
            url = url.strip_suffix(entity).unwrap_or(url);
        }
        url = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']);
        for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
            if url.ends_with(close) && url.matches(close).count() > url.matches(open).count() {
                url = &url[..url.len() - close.len_utf8()];
            }
        }
        if url == before {
            return url;
        }
    }
}

fn code_is_inside_link(before: &str) -> bool {
    matches!((before.rfind("<a "), before.rfind("</a>")), (Some(open), Some(close)) if open > close)
        || (before.rfind("<a ").is_some() && before.rfind("</a>").is_none())
}

#[cfg(test)]
mod art_ref_marker_tests {
    use super::*;

    fn message_resource(reference: &str, kind: &str, ready: bool) -> MessageResource {
        MessageResource {
            id: "resource-link".into(),
            ordinal: 0,
            original_reference: reference.into(),
            artifact_id: ready.then(|| "artifact-id".into()),
            artifact_version_id: ready.then(|| "version-id".into()),
            display_name: "plot.png".into(),
            kind: kind.into(),
            mime_type: "image/png".into(),
            status: if ready { "ready" } else { "unresolved" }.into(),
            error: (!ready).then(|| "not found".into()),
        }
    }

    #[test]
    fn replaces_bound_links_and_images_with_resource_identity() {
        let html = r#"<p><a href="D:/work/report.md">report</a><img src="figures/plot.png" alt="plot" /></p>"#;
        let resources = vec![
            message_resource("D:/work/report.md", "markdown", true),
            message_resource("figures/plot.png", "image", true),
        ];
        let out = replace_bound_resource_tags(html.into(), &resources);
        assert_eq!(out.matches(r#"data-resource-status="ready""#).count(), 2);
        assert!(out.contains(r##"href="#" data-resource-id="resource-link""##));
        assert!(out.contains(r#"src="" data-resource-id="resource-link""#));
        assert!(!out.contains("D:/work/report.md"));
    }

    #[test]
    fn unresolved_binding_is_visible_and_never_keeps_the_raw_path() {
        let html = r#"<p><a href="figures/missing.md">missing</a></p>"#;
        let out = replace_bound_resource_tags(
            html.into(),
            &[message_resource("figures/missing.md", "markdown", false)],
        );
        assert!(out.contains(r#"data-resource-status="unresolved""#));
        assert!(out.contains(r#"title="not found""#));
        assert!(!out.contains("figures/missing.md"));
    }

    #[test]
    fn unresolved_image_becomes_an_error_placeholder() {
        let html = r#"<p><img src="figures/missing.png" alt="plot" /></p>"#;
        let out = replace_bound_resource_tags(
            html.into(),
            &[message_resource("figures/missing.png", "image", false)],
        );
        assert!(out.contains(r#"class="resource-unresolved""#));
        assert!(out.contains("plot.png — not found"));
        assert!(!out.contains("<img"));
        assert!(!out.contains("figures/missing.png"));
    }

    #[test]
    fn matches_html_escaped_quotes_in_rendered_destinations() {
        let markdown = "[report](D:/work/report.md')";
        let out = enrich_md_html(
            md_to_html(markdown),
            &[],
            &[message_resource("D:/work/report.md'", "markdown", true)],
            Locale::En,
            None,
        );
        assert!(out.contains(r#"data-resource-status="ready""#));
        assert!(!out.contains("D:/work/report.md"));
    }

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
    fn folds_orphan_resource_separators_without_touching_prose() {
        let html = "<ul>\n<li><p><a href=\"a.png\">A</a></p>\n<p>、</p>\n<p><a href=\"b.png\">B</a></p></li>\n</ul>\n<p>`code` stays</p>";
        let out = collapse_orphan_separator_paragraphs(html.into());
        assert!(out.contains(r#"<p><a href="a.png">A</a> 、 <a href="b.png">B</a></p>"#));
        assert!(out.contains("<p>`code` stays</p>"));
        assert!(!out.contains("<p>、</p>"));

        let html = "<p><a href=\"a.png\">A</a></p>\n<p>`</p>\n<p><a href=\"b.png\">B</a></p>";
        let out = collapse_orphan_separator_paragraphs(html.into());
        assert_eq!(out, r#"<p><a href="a.png">A</a> <a href="b.png">B</a></p>"#);
    }

    #[test]
    fn does_not_nest_art_refs_for_duplicate_filenames() {
        let arts = vec![
            Artifact {
                id: "aaa".into(),
                name: "denovo_design_worklist.csv".into(),
                kind: "csv",
                data: PreviewData::File {
                    path: "a/denovo_design_worklist.csv".into(),
                    kind: "csv".into(),
                },
                location: None,
                source_item: 0,
                superseded: false,
                source_discarded: false,
            },
            Artifact {
                id: "bbb".into(),
                name: "denovo_design_worklist.csv".into(),
                kind: "csv",
                data: PreviewData::File {
                    path: "b/denovo_design_worklist.csv".into(),
                    kind: "csv".into(),
                },
                location: None,
                source_item: 0,
                superseded: false,
                source_discarded: false,
            },
        ];
        let html = r#"<ul><li><code>denovo_design_worklist.csv</code></li></ul>"#;
        let out = wrap_code_filenames_as_art_refs(html.into(), &arts);
        assert_eq!(out.matches(r#"class="art-ref""#).count(), 1);
        assert!(out.contains(r#"data-art-idx="0""#));
        assert!(!out.contains(r#"data-art-idx="1""#));
        assert!(!out.contains("</button></button>"));
    }

    #[test]
    fn filename_link_matches_registered_artifact_location() {
        let artifact = Artifact {
            id: "artifact-patho-double".into(),
            name: "patho_double.png".into(),
            kind: "image",
            data: PreviewData::File {
                path: "artifact:artifact-patho-double".into(),
                kind: "image".into(),
            },
            location: Some("figures/patho_double.png".into()),
            source_item: usize::MAX,
            superseded: false,
            source_discarded: false,
        };

        let html = replace_file_links(
            r#"<p><a href="patho_double.png">patho_double.png</a></p>"#.into(),
            &[artifact],
        );
        assert!(html.contains(r#"class="art-ref" data-art-idx="0""#));
        assert!(!html.contains(r#"href="patho_double.png""#));
    }

    #[test]
    fn skips_code_already_inside_art_ref_chip() {
        let html = r#"<button type="button" class="art-ref" data-art-idx="0" title="x.csv"><code>x.csv</code></button>"#;
        let out = replace_code_outside_art_refs(
            html,
            "<code>x.csv</code>",
            r#"<button type="button" class="art-ref" data-art-idx="1" title="x.csv"><code>x.csv</code></button>"#,
        );
        assert_eq!(out, html);
    }

    #[test]
    fn inline_project_paths_are_relative_clickable_links() {
        let html = r#"<p>Saved <code>D:\Wisp-Science\合作项目\analysis\figures\FIGURE_LEGEND.md</code> and <code>figures/plot.png</code>.</p>"#;
        let out = wrap_inline_workspace_paths(html.into(), Some(r"D:\Wisp-Science\合作项目"));
        assert!(out.contains(
            r#"href="analysis/figures/FIGURE_LEGEND.md" data-workspace-path="analysis/figures/FIGURE_LEGEND.md"><code>analysis/figures/FIGURE_LEGEND.md</code>"#
        ));
        assert!(out.contains(
            r#"href="figures/plot.png" data-workspace-path="figures/plot.png"><code>figures/plot.png</code>"#
        ));
        assert!(!out.contains(r"D:\Wisp-Science"));
    }

    #[test]
    fn bare_urls_in_prose_become_links() {
        let out = enrich_md_html(
            md_to_html("这是百度的网址：https://www.baidu.com，点开看看"),
            &[],
            &[],
            Locale::En,
            None,
        );
        assert!(out.contains(r#"<a href="https://www.baidu.com">https://www.baidu.com</a>"#));
        assert!(out.contains("，点开看看"));
    }

    #[test]
    fn autolinking_keeps_query_strings_and_drops_sentence_punctuation() {
        let out = linkify_bare_urls(
            "<p>see http://a.test/x?a=1&amp;b=2. and (https://b.test/docs), done</p>".into(),
        );
        assert!(out.contains(r#"<a href="http://a.test/x?a=1&amp;b=2">"#));
        assert!(out.contains(r#"<a href="https://b.test/docs">https://b.test/docs</a>)"#));
        assert!(out.ends_with(", done</p>"));
    }

    #[test]
    fn autolinking_never_touches_existing_links_code_or_attributes() {
        let html = concat!(
            r#"<p><a href="https://x.test">https://x.test</a> "#,
            r#"<code>https://y.test</code></p>"#,
            "<pre class=\"md-code\"><code>curl https://z.test</code></pre>",
            r#"<p><img src="https://img.test/a.png" alt="https://img.test/a.png" /></p>"#
        );
        assert_eq!(linkify_bare_urls(html.into()), html);
    }

    #[test]
    fn inline_commands_and_foreign_absolute_paths_stay_plain_code() {
        let html = r#"<p><code>monitor_run</code> <code>/usr/bin/python3</code> <code>D:\Other\secret.md</code></p>"#;
        let out = wrap_inline_workspace_paths(html.into(), Some(r"D:\Project"));
        assert_eq!(out, html);
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
    fn wraps_markdown_code_blocks_with_copy_controls() {
        let html = r#"<p>Example</p><pre class="md-code"><code class="language-rust">fn main() {}
</code></pre>"#;
        let out = wrap_markdown_code_blocks_with_copy_controls(html.into(), Locale::En);
        assert!(out.contains("md-code-card"));
        assert!(out.contains("md-code-copy"));
        assert!(out.contains(r#"aria-label="Copy code""#));
        assert!(out.contains(
            r#"<code class="language-rust">fn main() {}
</code></pre>"#
        ));
    }

    #[test]
    fn table_data_to_tsv_uses_tabs_and_newlines() {
        let t = TableData {
            headers: vec!["Gene".into(), "TPM".into()],
            rows: vec![
                vec!["A".into(), "2.62".into()],
                vec!["B".into(), "1.81".into()],
            ],
        };
        assert_eq!(table_data_to_tsv(&t), "Gene\tTPM\nA\t2.62\nB\t1.81");
    }
}

pub(crate) fn handle_md_click(
    ev: &web_sys::MouseEvent,
    arts: &[Artifact],
    resources: &[MessageResource],
    on_artifact: &Callback<usize>,
    on_file: &Callback<ModalArtifact>,
) {
    use wasm_bindgen::JsCast;
    let mut el = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    while let Some(n) = el {
        if n.class_list().contains("md-code-copy") {
            ev.prevent_default();
            ev.stop_propagation();
            if let Ok(Some(card)) = n.closest(".md-code-card") {
                if let Ok(Some(code)) = card.query_selector("pre.md-code code") {
                    copy_text(code.text_content().unwrap_or_default());
                }
            }
            return;
        }
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
            if let Ok(idx) = n
                .get_attribute("data-art-idx")
                .unwrap_or_default()
                .parse::<usize>()
            {
                ev.prevent_default();
                ev.stop_propagation();
                on_artifact.call(idx);
            }
            return;
        }
        if n.tag_name().eq_ignore_ascii_case("a") {
            if let Some(resource_id) = n.get_attribute("data-resource-id") {
                ev.prevent_default();
                ev.stop_propagation();
                if let Some(resource) = resources.iter().find(|resource| resource.id == resource_id)
                {
                    if let Some(version_id) = resource
                        .artifact_version_id
                        .as_ref()
                        .filter(|_| resource.status == "ready")
                    {
                        on_file.call((
                            format!("artifact-version:{version_id}"),
                            resource.display_name.clone(),
                            resource.kind.clone(),
                        ));
                    }
                }
                return;
            }
            if let Some(href) = n.get_attribute("href") {
                if opens_in_system_browser(&href) {
                    // Left untouched on purpose: the window-level handler in
                    // `App` owns every external link so one confirmation
                    // prompt covers all render paths.
                    return;
                }
                if !is_external_href(&href) {
                    ev.prevent_default();
                    ev.stop_propagation();
                    let path = normalize_path(&decode_href(&href));
                    if let Some(idx) = artifact_index_for_href(arts, &path) {
                        on_artifact.call(idx);
                    } else {
                        let kind = file_kind(&path).unwrap_or("text").to_string();
                        let name = attachment_name(&path);
                        on_file.call((path, name, kind));
                    }
                    return;
                }
                // "#"/"javascript:" and friends: no handler owns them, but the
                // webview's default navigation must still be suppressed.
                ev.prevent_default();
                ev.stop_propagation();
                return;
            }
        }
        el = n.parent_element();
    }
}
