//! Native literature-domain clients, independently implemented from:
//! - OpenAlex API (https://docs.openalex.org/, https://help.openalex.org/api/,
//!   https://help.openalex.org/api/paging/,
//!   https://help.openalex.org/data/works/attributes/)
//! - arXiv Atom API (https://info.arxiv.org/help/api/user-manual.html)
//! - arXiv API terms of use (https://info.arxiv.org/help/api/tou.html)
//!
//! References reviewed 2026-09-06. Tests use invented records.

mod arxiv;
mod openalex;
#[cfg(test)]
mod tests;

use super::NativeBio;
use crate::http::Source;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const OPENALEX_API: &str = "https://api.openalex.org";
const ARXIV_API: &str = "https://export.arxiv.org/api/query";
const OPENALEX: Source = Source("OpenAlex", Duration::from_millis(200));
const ARXIV: Source = Source("arXiv", Duration::from_secs(3));

#[cfg(test)]
tokio::task_local! {
    static TEST_ENDPOINTS: (String, String);
}

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "openalex_search_works",
            "Search OpenAlex scholarly works by text, year, type, venue, or open-access status. Returns a bounded page of work records with OpenAlex IDs, DOIs, and source links. OpenAlex's own match count is reported separately from the returned page.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 2048, "description": "Full-text search over title, abstract, and indexed full text."},
                    "year_from": {"type": "integer", "minimum": 1, "maximum": 9999},
                    "year_to": {"type": "integer", "minimum": 1, "maximum": 9999},
                    "work_type": {"type": "string", "minLength": 1, "maxLength": 64, "description": "OpenAlex work type such as article, review, preprint, book-chapter, or dataset."},
                    "open_access_only": {"type": "boolean", "default": false},
                    "venue": {"type": "string", "minLength": 1, "maxLength": 256, "description": "OpenAlex source ID (S…), ISSN, source URL, or journal name."},
                    "sort": {"type": "string", "enum": ["relevance", "cited_by_count", "publication_date"], "default": "relevance"},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "include_abstracts": {"type": "boolean", "default": false},
                    "mailto": {"type": "string", "description": "Optional contact address sent as OpenAlex's mailto parameter."}
                }
            }),
        ),
        tool(
            "openalex_get_work",
            "Fetch one OpenAlex work by OpenAlex W-id, openalex.org URL, or DOI. Returns metadata, license-gated abstract reconstruction, outgoing reference IDs, and citation counts by year. DOI lookups that match several works select the most-cited record and list the other claimants.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["work_id"],
                "properties": {
                    "work_id": {"type": "string", "minLength": 1, "maxLength": 512},
                    "mailto": {"type": "string"}
                }
            }),
        ),
        tool(
            "openalex_get_author",
            "Fetch one OpenAlex author profile by A-id, openalex.org URL, or ORCID, optionally with a bounded sample of that author's most-cited works.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["author_id"],
                "properties": {
                    "author_id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "works_sample": {"type": "integer", "minimum": 0, "maximum": 200, "default": 10},
                    "mailto": {"type": "string"}
                }
            }),
        ),
        tool(
            "openalex_search_authors",
            "Search OpenAlex author profiles by name. Homonyms are common; check ORCID, affiliations, and topics before treating a match as the intended person.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25},
                    "mailto": {"type": "string"}
                }
            }),
        ),
        tool(
            "openalex_citations",
            "List works that cite a given OpenAlex work (incoming citation graph). Heavily cited papers may have far more citers than one page; check the upstream total.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["work_id"],
                "properties": {
                    "work_id": {"type": "string", "minLength": 1, "maxLength": 512},
                    "sort": {"type": "string", "enum": ["cited_by_count", "publication_date", "relevance"], "default": "cited_by_count"},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "include_abstracts": {"type": "boolean", "default": false},
                    "mailto": {"type": "string"}
                }
            }),
        ),
        tool(
            "openalex_references",
            "List the works a given OpenAlex work cites. Returns every outgoing OpenAlex ID and hydrates a bounded prefix to full records in reference-list order. IDs that cannot be hydrated are listed rather than dropped.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["work_id"],
                "properties": {
                    "work_id": {"type": "string", "minLength": 1, "maxLength": 512},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100},
                    "mailto": {"type": "string"}
                }
            }),
        ),
        tool(
            "openalex_venue_info",
            "Look up OpenAlex sources (journals, repositories, conferences) by S-id, ISSN, source URL, or name. Exact identifiers return one source with yearly counts; a name search returns a bounded list.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["venue"],
                "properties": {
                    "venue": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 10},
                    "mailto": {"type": "string"}
                }
            }),
        ),
        tool(
            "arxiv_search",
            "Search arXiv preprints through the official Atom API. Plain terms search all fields; field prefixes ti:, au:, abs:, and cat: and Boolean AND/OR/ANDNOT are accepted. Returns a bounded page with arXiv's own match count.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 2048},
                    "category": {"type": "string", "minLength": 1, "maxLength": 64, "description": "arXiv subject class such as q-bio.GN or cs.LG."},
                    "date_from": {"type": "string", "description": "Inclusive submitted date YYYY-MM-DD."},
                    "date_to": {"type": "string", "description": "Inclusive submitted date YYYY-MM-DD."},
                    "start": {"type": "integer", "minimum": 0, "maximum": 30000, "default": 0},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25},
                    "sort_by": {"type": "string", "enum": ["relevance", "submittedDate", "lastUpdatedDate"], "default": "relevance"},
                    "sort_order": {"type": "string", "enum": ["descending", "ascending"], "default": "descending"}
                }
            }),
        ),
        tool(
            "arxiv_get_papers",
            "Fetch arXiv paper metadata, including abstracts, for a batch of identifiers. Accepts current and legacy IDs, optional versions, arXiv: prefixes, and abs/pdf URLs. Unknown and malformed IDs are listed; they do not fail the rest of the batch.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["arxiv_ids"],
                "properties": {
                    "arxiv_ids": {
                        "type": "array", "minItems": 1, "maxItems": 100,
                        "items": {"type": "string", "minLength": 1, "maxLength": 256}
                    }
                }
            }),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "openalex_search_works" => openalex::search_works(bio, args).await,
        "openalex_get_work" => openalex::get_work(bio, args).await,
        "openalex_get_author" => openalex::get_author(bio, args).await,
        "openalex_search_authors" => openalex::search_authors(bio, args).await,
        "openalex_citations" => openalex::citations(bio, args).await,
        "openalex_references" => openalex::references(bio, args).await,
        "openalex_venue_info" => openalex::venue_info(bio, args).await,
        "arxiv_search" => arxiv::search(bio, args).await,
        "arxiv_get_papers" => arxiv::get_papers(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    ("literature", ToolSchema::new(name, description, parameters))
}

fn openalex_base() -> String {
    #[cfg(test)]
    if let Ok(base) = TEST_ENDPOINTS.try_with(|ends| ends.0.clone()) {
        return base;
    }
    OPENALEX_API.to_string()
}

fn arxiv_url() -> String {
    #[cfg(test)]
    if let Ok(url) = TEST_ENDPOINTS.try_with(|ends| ends.1.clone()) {
        return url;
    }
    ARXIV_API.to_string()
}

fn bound(name: &str, value: usize, min: usize, max: usize) -> Result<usize> {
    if !(min..=max).contains(&value) {
        bail!("{name} must be between {min} and {max}");
    }
    Ok(value)
}

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

fn short_id(value: &str) -> &str {
    value
        .rsplit('/')
        .next()
        .filter(|text| !text.is_empty())
        .unwrap_or(value)
}

fn encode_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b':' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
