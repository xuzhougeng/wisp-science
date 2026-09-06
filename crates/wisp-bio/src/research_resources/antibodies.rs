use super::{
    antibody_get, antibody_origin, bound_u32, get_json, json_text, json_u64, require_text,
    ANON_ROW_LIMIT, ANTIBODY, ANTIBODY_API, ANTIBODY_SITE, DEFAULT_ANTIBODY_RECORDS,
    DEFAULT_PAGE_SIZE, MAX_ANTIBODY_RECORDS, MAX_PAGE_SIZE, MAX_QUERY,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    query: String,
    page: Option<u32>,
    #[serde(default = "default_page_size")]
    page_size: u32,
    #[serde(default = "default_max_records")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Get {
    antibody_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByCatalog {
    catalog_number: String,
    vendor: Option<String>,
    #[serde(default = "default_page_size")]
    page_size: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stats {}

fn default_page_size() -> u32 {
    DEFAULT_PAGE_SIZE
}

fn default_max_records() -> u32 {
    DEFAULT_ANTIBODY_RECORDS
}

pub async fn search(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search = serde_json::from_value(args.clone())
        .context("invalid Antibody Registry search arguments")?;
    let query = require_text(&args.query, "query", MAX_QUERY)?;
    let page_size = bound_u32(args.page_size, 1, MAX_PAGE_SIZE, "page_size")?;
    match args.page {
        Some(page) => {
            let page = bound_u32(page, 1, 50, "page")?;
            ensure_anonymous_page(page, page_size)?;
            let (total, items) = fetch_page(bio, &query, page, page_size).await?;
            let records: Vec<Value> = items.iter().filter_map(project_antibody).collect();
            Ok(json!({
                "source": "Antibody Registry",
                "source_url": ANTIBODY_SITE,
                "query": query,
                "page": page,
                "page_size": page_size,
                "total_elements": total,
                "returned": records.len(),
                "unique_ab_ids": unique_ab_ids(&records),
                "has_more": (page as u64) * (page_size as u64) < total,
                "anonymous_limit_hit": false,
                "records": records
            }))
        }
        None => {
            let cap = bound_u32(args.max_records, 1, MAX_ANTIBODY_RECORDS, "max_records")? as usize;
            walk(bio, &query, page_size, cap).await
        }
    }
}

pub async fn get(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Get = serde_json::from_value(args.clone())
        .context("invalid Antibody Registry lookup arguments")?;
    let ab_id = parse_ab_id(&args.antibody_id)?;
    let url = format!("{}/api/antibodies/{ab_id}", antibody_origin(bio));
    let response = antibody_get(bio, &url, &[]).await?;
    if response.status == StatusCode::NOT_FOUND {
        return Ok(missing_antibody(ab_id));
    }
    let raw = response.json()?;
    let rows = match raw {
        Value::Array(rows) => rows,
        Value::Object(_) => vec![raw],
        Value::Null => Vec::new(),
        _ => bail!("Antibody Registry returned an unexpected antibody document"),
    };
    let records: Vec<Value> = rows.iter().filter_map(project_antibody).collect();
    Ok(json!({
        "source": "Antibody Registry",
        "source_url": ANTIBODY_SITE,
        "antibody_id": ab_id,
        "rrid": rrid(ab_id),
        "registry_url": registry_url(ab_id),
        "found": !records.is_empty(),
        "returned": records.len(),
        "records": records
    }))
}

pub async fn by_catalog(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ByCatalog = serde_json::from_value(args.clone())
        .context("invalid Antibody Registry catalog arguments")?;
    let catalog = require_text(&args.catalog_number, "catalog_number", 128)?;
    let vendor = match args.vendor.as_deref() {
        Some(value) => Some(require_text(value, "vendor", 256)?),
        None => None,
    };
    let page_size = bound_u32(args.page_size, 1, MAX_PAGE_SIZE, "page_size")?;
    let searched = walk(bio, &catalog, page_size, MAX_ANTIBODY_RECORDS as usize).await?;
    let want = catalog.to_ascii_lowercase();
    let vendor_want = vendor.as_deref().map(|value| value.to_ascii_lowercase());
    let mut matches = Vec::new();
    if let Some(Value::Array(records)) = searched.get("records") {
        for record in records {
            if catalog_matches(record, &want) && vendor_matches(record, vendor_want.as_deref()) {
                matches.push(record.clone());
            }
        }
    }
    Ok(json!({
        "source": "Antibody Registry",
        "source_url": ANTIBODY_SITE,
        "catalog_number": catalog,
        "vendor": vendor,
        "search_total_elements": searched.get("total_elements"),
        "anonymous_limit_hit": searched.get("anonymous_limit_hit"),
        "returned": matches.len(),
        "records": matches
    }))
}

pub async fn stats(bio: &NativeBio, args: &Value) -> Result<Value> {
    let _: Stats = serde_json::from_value(args.clone())
        .context("invalid Antibody Registry stats arguments")?;
    let url = format!("{}/api/datainfo", antibody_origin(bio));
    let raw = get_json(bio, ANTIBODY, &url, &[]).await?;
    let total = raw
        .get("total")
        .and_then(json_u64)
        .context("Antibody Registry omitted registry size")?;
    let last_update = raw
        .get("lastupdate")
        .and_then(json_text)
        .context("Antibody Registry omitted last-update date")?;
    Ok(json!({
        "source": "Antibody Registry",
        "source_url": ANTIBODY_SITE,
        "api_url": ANTIBODY_API,
        "total_antibodies": total,
        "last_update": last_update
    }))
}

async fn walk(bio: &NativeBio, query: &str, page_size: u32, cap: usize) -> Result<Value> {
    let mut records = Vec::new();
    let mut total = None;
    let mut anonymous_limit_hit = false;
    let mut page = 1u32;
    loop {
        if records.len() >= cap {
            break;
        }
        if page.saturating_mul(page_size) > ANON_ROW_LIMIT {
            anonymous_limit_hit = true;
            break;
        }
        match fetch_page_allowing_anon_cap(bio, query, page, page_size, !records.is_empty()).await?
        {
            Page::Cap => {
                anonymous_limit_hit = true;
                break;
            }
            Page::Body {
                total: page_total,
                items,
            } => {
                total = Some(page_total);
                if items.is_empty() {
                    if page_total > 0 && records.is_empty() {
                        bail!("Antibody Registry reported hits but returned no rows");
                    }
                    break;
                }
                for item in &items {
                    if let Some(record) = project_antibody(item) {
                        records.push(record);
                        if records.len() >= cap {
                            break;
                        }
                    }
                }
                if records.len() >= cap || (records.len() as u64) >= page_total {
                    break;
                }
                page += 1;
            }
        }
    }
    let total = total.unwrap_or(records.len() as u64);
    let truncated = (records.len() as u64) < total;
    Ok(json!({
        "source": "Antibody Registry",
        "source_url": ANTIBODY_SITE,
        "query": query,
        "page_size": page_size,
        "total_elements": total,
        "returned": records.len(),
        "unique_ab_ids": unique_ab_ids(&records),
        "complete": !truncated,
        "truncated": truncated,
        "has_more": truncated,
        "anonymous_limit_hit": anonymous_limit_hit,
        "records": records
    }))
}

enum Page {
    Cap,
    Body { total: u64, items: Vec<Value> },
}

async fn fetch_page(
    bio: &NativeBio,
    query: &str,
    page: u32,
    page_size: u32,
) -> Result<(u64, Vec<Value>)> {
    match fetch_page_allowing_anon_cap(bio, query, page, page_size, false).await? {
        Page::Cap => bail!("Antibody Registry returned HTTP 401"),
        Page::Body { total, items } => Ok((total, items)),
    }
}

async fn fetch_page_allowing_anon_cap(
    bio: &NativeBio,
    query: &str,
    page: u32,
    page_size: u32,
    have_rows: bool,
) -> Result<Page> {
    let url = format!("{}/api/fts-antibodies", antibody_origin(bio));
    let params = [
        ("q".into(), query.to_string()),
        ("page".into(), page.to_string()),
        ("size".into(), page_size.to_string()),
    ];
    let response = antibody_get(bio, &url, &params).await?;
    if response.status == StatusCode::UNAUTHORIZED {
        if have_rows {
            return Ok(Page::Cap);
        }
        bail!("Antibody Registry returned HTTP 401");
    }
    let raw = response.json()?;
    let total = raw
        .get("totalElements")
        .and_then(json_u64)
        .context("Antibody Registry omitted totalElements")?;
    let items = match raw.get("items") {
        Some(Value::Array(items)) => items.clone(),
        _ => bail!("Antibody Registry omitted search items"),
    };
    Ok(Page::Body { total, items })
}

fn ensure_anonymous_page(page: u32, page_size: u32) -> Result<()> {
    let offset = page
        .checked_mul(page_size)
        .context("page*page_size exceeds the Antibody Registry anonymous window")?;
    if offset > ANON_ROW_LIMIT {
        bail!(
            "page*page_size={offset} exceeds the unauthenticated Antibody Registry offset cap ({ANON_ROW_LIMIT}); deeper pages return HTTP 401"
        );
    }
    Ok(())
}

pub(super) fn parse_ab_id(value: &str) -> Result<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 32 {
        bail!("antibody_id must be an Antibody Registry accession such as 3643095, AB_3643095, or RRID:AB_3643095");
    }
    let rest = trimmed
        .strip_prefix("RRID:")
        .or_else(|| trimmed.strip_prefix("rrid:"))
        .unwrap_or(trimmed);
    let rest = rest
        .strip_prefix("AB_")
        .or_else(|| rest.strip_prefix("ab_"))
        .unwrap_or(rest);
    if rest.is_empty()
        || rest.len() > 12
        || !rest.bytes().all(|b| b.is_ascii_digit())
        || (rest.starts_with('0') && rest != "0")
        || rest == "0"
    {
        bail!("antibody_id must be an Antibody Registry accession such as 3643095, AB_3643095, or RRID:AB_3643095");
    }
    rest.parse()
        .context("antibody_id must be an Antibody Registry accession")
}

fn rrid(ab_id: u64) -> String {
    format!("AB_{ab_id}")
}

fn registry_url(ab_id: u64) -> String {
    format!("{ANTIBODY_SITE}/AB_{ab_id}")
}

fn missing_antibody(ab_id: u64) -> Value {
    json!({
        "source": "Antibody Registry",
        "source_url": ANTIBODY_SITE,
        "antibody_id": ab_id,
        "rrid": rrid(ab_id),
        "registry_url": registry_url(ab_id),
        "found": false,
        "returned": 0,
        "records": []
    })
}

fn project_antibody(raw: &Value) -> Option<Value> {
    let ab_id = raw
        .get("abId")
        .and_then(json_u64)
        .or_else(|| raw.get("accession").and_then(json_u64))?;
    Some(json!({
        "ab_id": ab_id,
        "rrid": rrid(ab_id),
        "name": raw.get("abName"),
        "target": raw.get("abTarget"),
        "catalog_number": raw.get("catalogNum"),
        "catalog_alternates": raw.get("catAlt"),
        "vendor": raw.get("vendorName"),
        "clone_id": raw.get("cloneId"),
        "clonality": raw.get("clonality"),
        "source_organism": raw.get("sourceOrganism"),
        "target_species": raw.get("targetSpecies"),
        "applications": raw.get("applications"),
        "uniprot_id": raw.get("uniprotId"),
        "entrez_id": raw.get("abTargetEntrezId"),
        "product_form": raw.get("productForm"),
        "product_conjugate": raw.get("productConjugate"),
        "status": raw.get("status"),
        "url": raw.get("url"),
        "registry_url": registry_url(ab_id)
    }))
}

fn unique_ab_ids(records: &[Value]) -> usize {
    records
        .iter()
        .filter_map(|record| record.get("ab_id").and_then(json_u64))
        .collect::<BTreeSet<_>>()
        .len()
}

fn catalog_matches(record: &Value, want: &str) -> bool {
    if json_text(&record["catalog_number"])
        .or_else(|| json_text(&record["catalogNum"]))
        .is_some_and(|value| value.eq_ignore_ascii_case(want))
    {
        return true;
    }
    let alternates = json_text(&record["catalog_alternates"])
        .or_else(|| json_text(&record["catAlt"]))
        .unwrap_or_default();
    alternates
        .split([',', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .any(|part| part.eq_ignore_ascii_case(want))
}

fn vendor_matches(record: &Value, vendor: Option<&str>) -> bool {
    let Some(want) = vendor else {
        return true;
    };
    json_text(&record["vendor"])
        .or_else(|| json_text(&record["vendorName"]))
        .is_some_and(|value| value.eq_ignore_ascii_case(want))
}
