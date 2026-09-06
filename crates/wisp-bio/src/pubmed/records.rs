//! JATS helpers for Europe PMC full-text XML. References: JATS
//! (https://jats.nlm.nih.gov/). Examples in tests are synthetic.
use crate::xml::{self, child, field, path, text};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

pub(super) fn pmc_id(value: &str) -> Option<String> {
    let value = value.trim();
    let number = value
        .strip_prefix("PMC")
        .or_else(|| value.strip_prefix("pmc"))
        .unwrap_or(value);
    (!number.is_empty()
        && !number.starts_with('0')
        && number.len() <= 12
        && number.bytes().all(|b| b.is_ascii_digit()))
    .then(|| format!("PMC{number}"))
}

pub(super) fn full_text(xml_text: &str, expected_id: &str) -> Result<Value> {
    let doc = xml::parse(xml_text)?;
    let root = doc.root_element();
    if !root.has_tag_name("article") {
        bail!("Europe PMC returned an unexpected article document");
    }
    let meta =
        path(root, &["front", "article-meta"]).context("JATS article omitted its metadata")?;
    for id in meta.children().filter(|n| n.has_tag_name("article-id")) {
        if matches!(id.attribute("pub-id-type"), Some("pmc" | "pmcid")) {
            if text(id).and_then(|s| pmc_id(&s)).as_deref() != Some(expected_id) {
                bail!("Europe PMC full text does not match the requested PMCID");
            }
        }
    }
    let abstracts = meta
        .children()
        .filter(|n| n.has_tag_name("abstract"))
        .collect::<Vec<_>>();
    let abstract_node = abstracts
        .iter()
        .find(|n| n.attribute("abstract-type").is_none())
        .or(abstracts.first());
    let abstract_text = abstract_node.map(|n| xml::prose(*n, &["kwd-group"]));
    let sections: Vec<_> = child(root, "body")
        .into_iter()
        .flat_map(|n| n.children())
        .filter(|n| n.has_tag_name("sec") || n.has_tag_name("p"))
        .map(|n| xml::prose(n, &["fig", "table-wrap", "ref-list"]))
        .filter(|text| !text.is_empty())
        .collect();
    Ok(json!({
        "title": field(meta, &["title-group", "article-title"]),
        "abstract": abstract_text,
        "full_text": sections.join("\n\n")
    }))
}
