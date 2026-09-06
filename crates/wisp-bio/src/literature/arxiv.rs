//! arXiv Atom API. Official references (reviewed 2026-09-06):
//! https://info.arxiv.org/help/api/user-manual.html
//! https://info.arxiv.org/help/api/tou.html
use super::{bound, trimmed, NativeBio, ARXIV};
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

const PAGE_CAP: usize = 100;
const SOURCE_URL: &str = "https://export.arxiv.org/api/query";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: Option<String>,
    category: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    #[serde(default)]
    start: usize,
    #[serde(default = "default_page")]
    max_results: usize,
    sort_by: Option<String>,
    sort_order: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetPapers {
    arxiv_ids: Vec<String>,
}

fn default_page() -> usize {
    25
}

pub(super) async fn search(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid arXiv search arguments")?;
    let max_results = bound("max_results", args.max_results, 1, PAGE_CAP)?;
    let start = bound("start", args.start, 0, 30_000)?;
    let sort_by = args.sort_by.as_deref().unwrap_or("relevance");
    let sort_order = args.sort_order.as_deref().unwrap_or("descending");
    if !matches!(sort_by, "relevance" | "submittedDate" | "lastUpdatedDate") {
        bail!("sort_by must be relevance, submittedDate or lastUpdatedDate");
    }
    if !matches!(sort_order, "descending" | "ascending") {
        bail!("sort_order must be descending or ascending");
    }
    let mut clauses = Vec::new();
    if let Some(query) = trimmed(args.query.as_deref()) {
        if query.contains(" AND ") || query.contains(" OR ") || query.contains(" ANDNOT ") {
            clauses.push(format!("({query})"));
        } else {
            clauses.push(query.to_string());
        }
    }
    if let Some(category) = trimmed(args.category.as_deref()) {
        clauses.push(format!("cat:{}", arxiv_category(category)?));
    }
    if args.date_from.is_some() || args.date_to.is_some() {
        let lo = match trimmed(args.date_from.as_deref()) {
            Some(date) => date_stamp(date, "0000")?,
            None => "199101010000".into(),
        };
        let hi = match trimmed(args.date_to.as_deref()) {
            Some(date) => date_stamp(date, "2359")?,
            None => "299912312359".into(),
        };
        clauses.push(format!("submittedDate:[{lo} TO {hi}]"));
    }
    if clauses.is_empty() {
        bail!("provide a query, a category, and/or a date range");
    }
    let search_query = clauses.join(" AND ");
    let feed = query(
        bio,
        vec![
            ("search_query".into(), search_query.clone()),
            ("start".into(), start.to_string()),
            ("max_results".into(), max_results.to_string()),
            ("sortBy".into(), sort_by.into()),
            ("sortOrder".into(), sort_order.into()),
        ],
    )
    .await?;
    let returned = feed.records.len();
    Ok(json!({
        "provider": "arXiv",
        "source_url": SOURCE_URL,
        "search_query": search_query,
        "api_total": feed.total,
        "start_index": feed.start_index,
        "n_records_returned": returned,
        "records_truncated": feed.start_index + returned < feed.total,
        "sort_by": sort_by,
        "sort_order": sort_order,
        "records": feed.records,
    }))
}

pub(super) async fn get_papers(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetPapers =
        serde_json::from_value(args.clone()).context("invalid arXiv paper arguments")?;
    let requested: Vec<&str> = args
        .arxiv_ids
        .iter()
        .filter_map(|id| trimmed(Some(id)))
        .collect();
    if requested.is_empty() || requested.len() > PAGE_CAP {
        bail!("provide 1 to 100 arXiv identifiers");
    }
    let mut ids = Vec::new();
    let mut not_found = Vec::new();
    for text in &requested {
        match normalize_arxiv_id(text) {
            Ok(id) if is_arxiv_id(&id) => ids.push(id),
            _ => not_found.push((*text).to_string()),
        }
    }
    let mut records = Vec::new();
    let mut duplicates = Vec::new();
    if !ids.is_empty() {
        let feed = query(
            bio,
            vec![
                ("id_list".into(), ids.join(",")),
                ("max_results".into(), ids.len().to_string()),
            ],
        )
        .await?;
        let mut stored = Vec::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        for record in feed.records {
            let index = stored.len();
            if let Some(id) = record["arxiv_id"].as_str() {
                by_id.entry(id.to_string()).or_insert(index);
            }
            if let Some(versioned) = record["id_versioned"].as_str() {
                by_id.entry(versioned.to_string()).or_insert(index);
            }
            stored.push(record);
        }
        let mut seen: HashMap<usize, String> = HashMap::new();
        for requested in &ids {
            let stripped = strip_version(requested);
            let Some(&index) = by_id.get(requested).or_else(|| by_id.get(stripped)) else {
                not_found.push(requested.clone());
                continue;
            };
            if let Some(first) = seen.get(&index) {
                duplicates.push(json!({"requested": requested, "resolved_as": first}));
            } else {
                seen.insert(index, requested.clone());
                records.push(stored[index].clone());
            }
        }
    }
    Ok(json!({
        "provider": "arXiv",
        "source_url": SOURCE_URL,
        "n_requested": requested.len(),
        "n_found": records.len(),
        "duplicates": duplicates,
        "not_found": not_found,
        "records": records,
    }))
}

#[derive(Debug)]
pub(super) struct Feed {
    pub total: usize,
    pub start_index: usize,
    pub records: Vec<Value>,
}

async fn query(bio: &NativeBio, params: Vec<(String, String)>) -> Result<Feed> {
    let body = bio
        .http()
        .send(ARXIV, Method::GET, &super::arxiv_url(), &params)
        .await?
        .text()?;
    parse_feed(&body)
}

pub(super) fn parse_feed(body: &str) -> Result<Feed> {
    if body.to_ascii_uppercase().contains("<!DOCTYPE") {
        bail!("arXiv returned an unexpected DTD");
    }
    let doc = crate::xml::parse(body)?;
    let root = doc.root_element();
    if !root.has_tag_name("feed") {
        bail!("arXiv returned an unexpected Atom document");
    }
    let entries: Vec<_> = root
        .children()
        .filter(|node| node.has_tag_name("entry"))
        .collect();
    if entries.len() == 1 {
        if child_text(entries[0], "id").is_some_and(|id| id.contains("/api/errors")) {
            let message = child_text(entries[0], "summary")
                .or_else(|| child_text(entries[0], "title"))
                .unwrap_or_else(|| "error".into());
            bail!("arXiv API error: {message}");
        }
    }
    let total = child_int(root, "totalResults").unwrap_or(0);
    let start_index = child_int(root, "startIndex").unwrap_or(0);
    Ok(Feed {
        total,
        start_index,
        records: entries.into_iter().map(parse_entry).collect(),
    })
}

fn parse_entry(entry: roxmltree::Node<'_, '_>) -> Value {
    let abs_url = child_text(entry, "id").unwrap_or_default();
    let versioned = abs_url.rsplit("/abs/").next().unwrap_or("").to_string();
    let (arxiv_id, version) = split_version(&versioned);
    let mut pdf_url = None;
    for link in entry.children().filter(|node| node.has_tag_name("link")) {
        if link.attribute("title") == Some("pdf")
            || link.attribute("type") == Some("application/pdf")
        {
            pdf_url = link.attribute("href").map(str::to_string);
        }
    }
    let primary = entry
        .children()
        .find(|node| node.has_tag_name("primary_category"))
        .and_then(|node| node.attribute("term"))
        .map(str::to_string);
    let authors: Vec<String> = entry
        .children()
        .filter(|node| node.has_tag_name("author"))
        .filter_map(|author| child_text(author, "name"))
        .collect();
    let categories: Vec<String> = entry
        .children()
        .filter(|node| node.has_tag_name("category"))
        .filter_map(|node| node.attribute("term").map(str::to_string))
        .collect();
    json!({
        "arxiv_id": if arxiv_id.is_empty() { Value::Null } else { json!(arxiv_id) },
        "version": version,
        "id_versioned": if versioned.is_empty() { Value::Null } else { json!(versioned) },
        "title": child_text(entry, "title"),
        "abstract": child_text(entry, "summary"),
        "authors": authors,
        "published": child_text(entry, "published"),
        "updated": child_text(entry, "updated"),
        "primary_category": primary,
        "categories": categories,
        "doi": child_text(entry, "doi"),
        "journal_ref": child_text(entry, "journal_ref"),
        "comment": child_text(entry, "comment"),
        "abs_url": if abs_url.is_empty() { Value::Null } else { json!(abs_url) },
        "pdf_url": pdf_url,
        "url": if abs_url.is_empty() { Value::Null } else { json!(abs_url) },
        "provider": "arXiv",
    })
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    crate::xml::child(node, name)
        .and_then(crate::xml::text)
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty())
}

fn child_int(node: roxmltree::Node<'_, '_>, name: &str) -> Option<usize> {
    child_text(node, name)?.parse().ok()
}

pub(super) fn normalize_arxiv_id(raw: &str) -> Result<String> {
    let mut value = raw.trim();
    for prefix in [
        "https://export.arxiv.org/abs/",
        "http://export.arxiv.org/abs/",
        "https://export.arxiv.org/pdf/",
        "http://export.arxiv.org/pdf/",
        "https://arxiv.org/abs/",
        "http://arxiv.org/abs/",
        "https://arxiv.org/pdf/",
        "http://arxiv.org/pdf/",
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest;
            break;
        }
    }
    if let Some(rest) = value
        .strip_prefix("arxiv:")
        .or_else(|| value.strip_prefix("arXiv:"))
        .or_else(|| value.strip_prefix("ARXIV:"))
    {
        value = rest.trim();
    }
    if let Some(stripped) = value.strip_suffix(".pdf") {
        value = stripped;
    }
    if value.is_empty() {
        bail!("empty arXiv id from {raw:?}");
    }
    Ok(value.to_string())
}

fn is_arxiv_id(value: &str) -> bool {
    let (id, _) = split_version(value);
    if let Some((year_month, number)) = id.split_once('.') {
        return year_month.len() == 4
            && year_month.bytes().all(|byte| byte.is_ascii_digit())
            && matches!(number.len(), 4 | 5)
            && number.bytes().all(|byte| byte.is_ascii_digit());
    }
    if let Some((archive, number)) = id.split_once('/') {
        return !archive.is_empty()
            && archive
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-' || byte == b'.')
            && archive.as_bytes()[0].is_ascii_lowercase()
            && number.len() == 7
            && number.bytes().all(|byte| byte.is_ascii_digit());
    }
    false
}

fn split_version(value: &str) -> (&str, Option<u32>) {
    if let Some(index) = value.rfind('v') {
        if index > 0 && value[index + 1..].bytes().all(|byte| byte.is_ascii_digit()) {
            if let Ok(version) = value[index + 1..].parse::<u32>() {
                if version > 0 {
                    return (&value[..index], Some(version));
                }
            }
        }
    }
    (value, None)
}

fn strip_version(value: &str) -> &str {
    split_version(value).0
}

fn arxiv_category(value: &str) -> Result<&str> {
    if value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
    {
        bail!("category must be an arXiv subject class such as q-bio.GN or cs.LG");
    }
    Ok(value)
}

fn date_stamp(date: &str, hhmm: &str) -> Result<String> {
    let digits: String = date.chars().filter(|ch| *ch != '-').collect();
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("dates must be YYYY-MM-DD");
    }
    let year: i32 = digits[..4].parse().unwrap_or(0);
    let month: u32 = digits[4..6].parse().unwrap_or(0);
    let day: u32 = digits[6..8].parse().unwrap_or(0);
    if chrono::NaiveDate::from_ymd_opt(year, month, day).is_none() {
        bail!("dates must be YYYY-MM-DD");
    }
    Ok(format!("{digits}{hhmm}"))
}
