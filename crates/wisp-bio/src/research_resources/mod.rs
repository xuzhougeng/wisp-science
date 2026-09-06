//! Native `research-resources` domain against the Antibody Registry and
//! Grants.gov search2 APIs. Independently implemented from:
//!
//! - [SciCrunch Antibody Registry OpenAPI](https://www.antibodyregistry.org/api/openapi.json)
//! - [Grants.gov search2](https://www.grants.gov/api/common/search2)
//! - [Grants.gov API guide](https://www.grants.gov/api/api-guide)
//!
//! References reviewed 2026-09-06. search2 accepts JSON POST only; GET is not
//! the search contract and is not used. Antibody Registry list pages are
//! 1-based. Unauthenticated full-text pages whose offset exceeds 500 return
//! HTTP 401; the client stops and sets `anonymous_limit_hit` rather than
//! dropping rows. Tests use invented records. No API keys are published for
//! these endpoints.

mod antibodies;
mod grants;

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{anyhow, bail, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const DOMAIN: &str = "research-resources";
const ANTIBODY_SITE: &str = "https://www.antibodyregistry.org";
const ANTIBODY_API: &str = "https://www.antibodyregistry.org/api";
const GRANTS_SITE: &str = "https://www.grants.gov";
const GRANTS_API: &str = "https://api.grants.gov";
const ANTIBODY: Source = Source("Antibody Registry", Duration::from_millis(500));
const GRANTS: Source = Source("Grants.gov", Duration::from_millis(500));
const ANON_ROW_LIMIT: u32 = 500;
const MAX_QUERY: usize = 512;
const MAX_PAGE_SIZE: u32 = 100;
const MAX_ANTIBODY_RECORDS: u32 = 500;
const MAX_GRANT_RECORDS: u32 = 200;
const DEFAULT_PAGE_SIZE: u32 = 50;
const DEFAULT_ANTIBODY_RECORDS: u32 = 100;
const DEFAULT_GRANT_RECORDS: u32 = 25;
const MAX_FILTERS: usize = 20;
const MAX_FILTER_ITEM: usize = 64;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            DOMAIN,
            ToolSchema::new(
                "search_antibodies",
                "Search the Antibody Registry (RRID:SCR_006397) by antibody name, target, catalog number or clone via GET /api/fts-antibodies. Pages are 1-based. total_elements counts index rows, not unique antibodies. Unauthenticated retrieval stops at offset 500 (deeper pages return HTTP 401); anonymous_limit_hit is set instead of omitting rows. A capped page is not the complete hit list.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 512},
                        "page": {"type": "integer", "minimum": 1, "maximum": 50,
                            "description": "1-based page. Omit to walk pages up to max_records or the anonymous offset cap."},
                        "page_size": {"type": "integer", "minimum": 1, "maximum": 100, "default": 50},
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_antibody",
                "Retrieve Antibody Registry records for one accession or RRID (3643095, AB_3643095, or RRID:AB_3643095) via GET /api/antibodies/{id}. The upstream route is list-valued: one accession can map to several curated rows. A missing identifier returns found=false with an empty records list, not an error.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["antibody_id"],
                    "properties": {
                        "antibody_id": {"type": "string", "minLength": 1, "maxLength": 32}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "find_antibodies_by_catalog",
                "Find Antibody Registry records whose catalogNum or listed catAlt alternatives equal a vendor catalog number (case-insensitive). Uses full-text search plus exact client-side matching because that is the reliable public read path. An optional vendor filter is an exact, case-insensitive vendorName match. Matching is limited to the bounded anonymous search window.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["catalog_number"],
                    "properties": {
                        "catalog_number": {"type": "string", "minLength": 1, "maxLength": 128},
                        "vendor": {"type": "string", "minLength": 1, "maxLength": 256},
                        "page_size": {"type": "integer", "minimum": 1, "maximum": 100, "default": 50}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_antibody_registry_stats",
                "Return Antibody Registry size and last-update date from GET /api/datainfo. This is registry-level metadata, not a search and not a reuse grant.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {}
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "search_grants",
                "Search Grants.gov funding opportunities through POST https://api.grants.gov/v1/api/search2 (JSON body; GET is not the API). At least one of keyword, opportunity_number, aln, agencies, eligibilities, funding_categories or funding_instruments is required. Opportunity statuses default to forecasted and posted as documented by Grants.gov. The response is a bounded page: total is hitCount, and truncated/has_more mean more hits exist. Facet blocks are independent aggregations of the same query. No API key is required.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "keyword": {"type": "string", "minLength": 1, "maxLength": 512},
                        "opportunity_number": {"type": "string", "minLength": 1, "maxLength": 64},
                        "aln": {"type": "string", "minLength": 1, "maxLength": 16,
                            "description": "Assistance Listing Number (formerly CFDA), for example 93.866."},
                        "agencies": {"type": "array", "minItems": 1, "maxItems": 20,
                            "items": {"type": "string", "minLength": 1, "maxLength": 64}},
                        "opportunity_statuses": {"type": "array", "minItems": 1, "maxItems": 4,
                            "items": {"type": "string", "enum": ["forecasted", "posted", "closed", "archived"]},
                            "default": ["forecasted", "posted"]},
                        "eligibilities": {"type": "array", "minItems": 1, "maxItems": 20,
                            "items": {"type": "string", "minLength": 1, "maxLength": 64}},
                        "funding_categories": {"type": "array", "minItems": 1, "maxItems": 20,
                            "items": {"type": "string", "minLength": 1, "maxLength": 16}},
                        "funding_instruments": {"type": "array", "minItems": 1, "maxItems": 20,
                            "items": {"type": "string", "minLength": 1, "maxLength": 16}},
                        "count_only": {"type": "boolean", "default": false},
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25},
                        "start_record": {"type": "integer", "minimum": 0, "maximum": 100000, "default": 0},
                        "include_facets": {"type": "boolean", "default": true}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("research-resources request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "search_antibodies" => antibodies::search(bio, args).await,
        "get_antibody" => antibodies::get(bio, args).await,
        "find_antibodies_by_catalog" => antibodies::by_catalog(bio, args).await,
        "get_antibody_registry_stats" => antibodies::stats(bio, args).await,
        "search_grants" => grants::search(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn antibody_origin(bio: &NativeBio) -> String {
    override_origin(bio, "ANTIBODY_REGISTRY_BASE_URL", ANTIBODY_SITE)
}

fn grants_origin(bio: &NativeBio) -> String {
    override_origin(bio, "GRANTS_GOV_BASE_URL", GRANTS_API)
}

fn override_origin(bio: &NativeBio, key: &str, default: &str) -> String {
    bio.credential(key)
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.trim_end_matches('/').to_string())
}

async fn get_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    bio.http()
        .send(source, Method::GET, url, params)
        .await?
        .json()
}

async fn antibody_get(
    bio: &NativeBio,
    url: &str,
    params: &[(String, String)],
) -> Result<crate::http::Response> {
    bio.http().send(ANTIBODY, Method::GET, url, params).await
}

fn require_text(value: &str, field: &str, max: usize) -> Result<String> {
    let text = value.trim();
    if text.is_empty() || text.len() > max {
        bail!("{field} must contain 1 to {max} characters");
    }
    Ok(text.to_string())
}

fn bound_u32(value: u32, min: u32, max: u32, field: &str) -> Result<u32> {
    if !(min..=max).contains(&value) {
        bail!("{field} must be between {min} and {max}");
    }
    Ok(value)
}

fn json_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn json_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn pipe_join(values: &[String], field: &str) -> Result<String> {
    if values.len() > MAX_FILTERS {
        bail!("{field} accepts at most {MAX_FILTERS} values");
    }
    let mut parts = Vec::new();
    for value in values {
        let text = require_text(value, field, MAX_FILTER_ITEM)?;
        if text.contains('|') {
            bail!("{field} values must not contain '|'");
        }
        parts.push(text);
    }
    if parts.is_empty() {
        bail!("{field} must not be empty");
    }
    Ok(parts.join("|"))
}
