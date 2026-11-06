//! Review only flat inline revisions. Group adjacent delete/insert pairs from
//! the same author and timestamp so a rewrite is never half-accepted in a batch.
use super::*;
use wisp_dto::{
    DocxChange, DocxChangeKind, DocxReviewAction, DocxReviewSelection, DocxReviewSnapshot,
};

struct Group<'a, 'input> {
    change: DocxChange,
    nodes: Vec<Node<'a, 'input>>,
}

pub fn read_document(path: &Path) -> Result<Vec<u8>> {
    read_source(path)
}

fn revision(n: Node<'_, '_>) -> bool {
    n.tag_name().namespace() == Some(W)
        && (matches!(n.tag_name().name(), "ins" | "del" | "moveFrom" | "moveTo")
            || n.tag_name().name().ends_with("Change"))
}

fn supported(n: Node<'_, '_>, protected: &[Range<usize>]) -> bool {
    if !is_word(n, "ins") && !is_word(n, "del") {
        return false;
    }
    let Some(p) = n.parent().filter(|p| is_word(*p, "p")) else {
        return false;
    };
    if !p.parent().is_some_and(|p| is_word(p, "body"))
        || protected
            .iter()
            .any(|r| r.start < p.range().end && p.range().start < r.end)
    {
        return false;
    }
    if n.attribute((W, "id")).is_none() {
        return false;
    }
    let mut has_text = false;
    for run in n.children().filter(|c| c.is_element()) {
        if !is_word(run, "r") {
            return false;
        }
        for child in run.children().filter(|c| c.is_element()) {
            if is_word(child, "rPr") {
                if child.descendants().any(revision) {
                    return false;
                }
            } else if is_word(child, if is_word(n, "del") { "delText" } else { "t" }) {
                if child.children().any(|c| c.is_element()) {
                    return false;
                }
                has_text = true;
            } else {
                return false;
            }
        }
    }
    has_text
}

fn content(n: Node<'_, '_>) -> String {
    n.descendants()
        .filter(|c| is_word(*c, "t") || is_word(*c, "delText"))
        .filter_map(|c| c.text())
        .collect()
}

fn groups<'a, 'input>(doc: &'a Document<'input>) -> (Vec<Group<'a, 'input>>, usize) {
    let protected = protected_ranges(doc);
    let mut used = HashSet::new();
    let mut groups = Vec::new();
    for (index, p) in doc.descendants().filter(|p| is_word(*p, "p")).enumerate() {
        for n in p.children().filter(|n| revision(*n)) {
            if used.contains(&n.id()) || !supported(n, &protected) {
                continue;
            }
            let mut nodes = vec![n];
            if let Some(next) = n.next_sibling_element().filter(|next| {
                supported(*next, &protected)
                    && is_word(*next, "ins") != is_word(n, "ins")
                    && n.attribute((W, "author")).is_some_and(|s| !s.is_empty())
                    && n.attribute((W, "date")).is_some_and(|s| !s.is_empty())
                    && n.attribute((W, "author")) == next.attribute((W, "author"))
                    && n.attribute((W, "date")) == next.attribute((W, "date"))
            }) {
                nodes.push(next);
            }
            let mut old = String::new();
            let mut new = String::new();
            for node in &nodes {
                used.insert(node.id());
                if is_word(*node, "del") {
                    old.push_str(&content(*node));
                } else {
                    new.push_str(&content(*node));
                }
            }
            let kind = if nodes.len() == 2 {
                DocxChangeKind::Rewrite
            } else if is_word(n, "del") {
                DocxChangeKind::Deletion
            } else {
                DocxChangeKind::Insertion
            };
            groups.push(Group {
                change: DocxChange {
                    // Byte offsets are unique even in malformed inputs reusing w:id.
                    // The caller must bind this ID to source_sha256 before mutation.
                    id: format!("change-{}", n.range().start),
                    paragraph_id: format!("p{}", index + 1),
                    kind,
                    author: n.attribute((W, "author")).unwrap_or("").into(),
                    date: n.attribute((W, "date")).unwrap_or("").into(),
                    old,
                    new,
                },
                nodes,
            });
        }
    }
    let unsupported = doc
        .descendants()
        .filter(|n| revision(*n) && !used.contains(&n.id()))
        .count();
    (groups, unsupported)
}

pub fn inspect_revisions(bytes: &[u8]) -> Result<DocxReviewSnapshot> {
    let package = Package::open(bytes)?;
    let doc = parse(&package.xml)?;
    let (groups, unsupported_count) = groups(&doc);
    let changes: Vec<_> = groups.into_iter().map(|g| g.change).collect();
    if changes.len() > 1000
        || changes
            .iter()
            .map(|c| c.old.len() + c.new.len())
            .sum::<usize>()
            > MAX_RESULT
    {
        return Err("Too many revisions to review safely; split the document in Word".into());
    }
    Ok(DocxReviewSnapshot {
        source_sha256: sha256(bytes),
        changes,
        unsupported_count,
        writable: false,
    })
}

fn retained_content(n: Node<'_, '_>, xml: &str) -> Result<String> {
    let raw = &xml[n.range()];
    let start = opening_tag(raw)?.len();
    let end = raw.rfind("</").ok_or("Revision has no closing tag")?;
    let mut inside = raw[start..end].to_string();
    if is_word(n, "del") {
        // Rejecting a deletion restores w:t. Preserve the element prefix and
        // all attributes; a text replacement alone would leave delText behind.
        let mut patches = Vec::new();
        for text in n.descendants().filter(|t| is_word(*t, "delText")) {
            let raw = &xml[text.range()];
            let open = opening_tag(raw)?;
            let qualified_end = open
                .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
                .ok_or("Invalid delText")?;
            let qualified = &open[1..qualified_end];
            let name = qualified
                .rsplit_once(':')
                .map(|(p, _)| format!("{p}:t"))
                .unwrap_or_else(|| "t".into());
            let start_tag = format!("<{name}{}", &open[qualified_end..]);
            let replacement = if open.ends_with("/>") {
                start_tag
            } else {
                let close = raw.rfind("</").ok_or("Invalid delText end")?;
                format!("{start_tag}{}</{name}>", &raw[open.len()..close])
            };
            patches.push((
                text.range().start - n.range().start - start
                    ..text.range().end - n.range().start - start,
                replacement,
            ));
        }
        for (range, value) in patches.into_iter().rev() {
            inside.replace_range(range, &value);
        }
    }
    // Revision wrappers may declare namespaces used by the retained runs. Move
    // all in-scope declarations onto each child rather than dropping bindings.
    let declarations: String = n
        .namespaces()
        .filter(|ns| ns.name() != Some("xml"))
        .map(|ns| {
            format!(
                " xmlns{}=\"{}\"",
                ns.name().map(|p| format!(":{p}")).unwrap_or_default(),
                escape(ns.uri()).replace('"', "&quot;")
            )
        })
        .collect();
    // Reparse inside a namespace-bearing wrapper to locate immediate children
    // after the delText renames (their byte offsets may have changed).
    let wrapper = format!("<wrapper{declarations}>{inside}</wrapper>");
    let parsed = parse(&wrapper)?;
    let mut result = String::new();
    for child in parsed.root_element().children() {
        let raw = &wrapper[child.range()];
        if !child.is_element() {
            result.push_str(raw);
            continue;
        }
        let open = opening_tag(raw)?;
        let name_end = open
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .ok_or("Invalid run start")?;
        result.push_str(&open[..name_end]);
        for ns in child.namespaces().filter(|ns| ns.name() != Some("xml")) {
            let attr = ns
                .name()
                .map(|p| format!("xmlns:{p}"))
                .unwrap_or_else(|| "xmlns".into());
            result.push_str(&format!(
                " {attr}=\"{}\"",
                escape(ns.uri()).replace('"', "&quot;")
            ));
        }
        for attr in child.attributes() {
            result.push(' ');
            result.push_str(&wrapper[attr.range()]);
        }
        result.push_str(if open.ends_with("/>") { "/>" } else { ">" });
        result.push_str(&raw[open.len()..]);
    }
    Ok(result)
}

pub fn resolve_revisions(
    bytes: &[u8],
    expected_sha256: &str,
    selection: &DocxReviewSelection,
    action: DocxReviewAction,
) -> Result<Vec<u8>> {
    if sha256(bytes) != expected_sha256 {
        return Err("DOCX changed; refresh the preview before reviewing".into());
    }
    // Apply the same bounds as the review listing; batches never include unseen
    // or unsupported structural revisions.
    inspect_revisions(bytes)?;
    let package = Package::open(bytes)?;
    let doc = parse(&package.xml)?;
    let (groups, _) = groups(&doc);
    let mut patches = Vec::new();
    for group in groups.into_iter().filter(|g| match selection {
        DocxReviewSelection::Change(id) => &g.change.id == id,
        DocxReviewSelection::Kind(kind) => &g.change.kind == kind,
    }) {
        for node in group.nodes {
            let keep = is_word(node, "ins") == (action == DocxReviewAction::Accept);
            patches.push((
                node.range(),
                if keep {
                    retained_content(node, &package.xml)?
                } else {
                    String::new()
                },
            ));
        }
    }
    if patches.is_empty() {
        return Err("No matching reviewable revisions; refresh the preview".into());
    }
    patches.sort_by_key(|(r, _)| r.start);
    let mut xml = package.xml.clone();
    for (range, replacement) in patches.into_iter().rev() {
        xml.replace_range(range, &replacement);
    }
    package.with_xml(&xml)
}
