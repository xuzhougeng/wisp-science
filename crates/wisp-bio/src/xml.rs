//! XML helpers for NCBI metadata and Europe PMC JATS. No external entity resolver
//! is installed; DTD declarations cannot read files or access the network.
use anyhow::{bail, Context, Result};
use roxmltree::{Document, Node, ParsingOptions};

pub(crate) fn parse(text: &str) -> Result<Document<'_>> {
    let doc = Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: true,
            nodes_limit: 100_000,
            entity_resolver: None,
        },
    )
    .context("upstream returned invalid XML")?;
    // roxmltree's node/expansion limits bound memory; cap depth as well before
    // processing arbitrary nested JATS with our formatting helpers.
    for node in doc.descendants().filter(|node| node.is_element()) {
        if node.ancestors().take(129).count() > 128 {
            bail!("XML nesting exceeds 128 levels");
        }
    }
    Ok(doc)
}

pub(crate) fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(name))
}

pub(crate) fn path<'a, 'i>(node: Node<'a, 'i>, names: &[&str]) -> Option<Node<'a, 'i>> {
    names.iter().try_fold(node, |node, name| child(node, name))
}

pub(crate) fn text(node: Node<'_, '_>) -> Option<String> {
    let value: String = node
        .descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn field(node: Node<'_, '_>, names: &[&str]) -> Option<String> {
    path(node, names).and_then(text)
}

/// Preserve inline word boundaries and separate block text, excluding selected
/// subtrees (captions, references or keyword metadata).
pub(crate) fn prose(node: Node<'_, '_>, excluded: &[&str]) -> String {
    fn visit(node: Node<'_, '_>, excluded: &[&str], out: &mut String) {
        if node.is_text() {
            out.push_str(node.text().unwrap_or_default());
            return;
        }
        if node.has_tag_name("title")
            && node.parent().is_some_and(|p| p.has_tag_name("abstract"))
            && text(node).is_some_and(|text| text.eq_ignore_ascii_case("abstract"))
        {
            return;
        }
        if excluded.iter().any(|name| node.has_tag_name(*name))
            || node.attribute("sec-type") == Some("kwd-group")
        {
            return;
        }
        let block = ["p", "sec", "title", "abstract", "list-item"]
            .iter()
            .any(|name| node.has_tag_name(*name));
        if block {
            out.push('\n');
        }
        for child in node.children() {
            visit(child, excluded, out);
        }
        if block {
            out.push('\n');
        }
    }
    let mut out = String::new();
    visit(node, excluded, &mut out);
    out.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
