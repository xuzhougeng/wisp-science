//! Native bioRxiv/medRxiv clients, independently implemented from the
//! bioRxiv/medRxiv Content API (https://api.biorxiv.org/, reviewed 2026-09-06).
//!
//! Listing, DOI detail, published-article, publisher, funder, content-summary
//! and usage endpoints are documented on that page. Subject categories are the
//! bioRxiv names accepted as the `category` query parameter. Tests use invented
//! records.
use super::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

#[cfg(test)]
mod tests;

const API: &str = "https://api.biorxiv.org";
const SOURCE_URL: &str = "https://api.biorxiv.org/";
const SOURCE: crate::http::Source = crate::http::Source("bioRxiv", Duration::from_millis(500));
const ABSTRACT_PREVIEW: usize = 500;
const MAX_PAGES: usize = 8;
const DETAILS_PAGE: u64 = 30;
const LIST_PAGE: u64 = 100;
const FUNDER_START: &str = "2025-04-10";

/// bioRxiv subject categories documented for the Content API `category` filter.
const BIORXIV_CATEGORIES: &[&str] = &[
    "animal behavior and cognition",
    "biochemistry",
    "bioengineering",
    "bioinformatics",
    "biophysics",
    "cancer biology",
    "cell biology",
    "clinical trials",
    "developmental biology",
    "ecology",
    "epidemiology",
    "evolutionary biology",
    "genetics",
    "genomics",
    "immunology",
    "microbiology",
    "molecular biology",
    "neuroscience",
    "paleontology",
    "pathology",
    "pharmacology and toxicology",
    "physiology",
    "plant biology",
    "scientific communication and education",
    "synthetic biology",
    "systems biology",
    "zoology",
];

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "biorxiv",
            ToolSchema::new(
                "get_categories",
                "List the 27 bioRxiv subject categories accepted as the Content API category query parameter. Each entry includes the spaced name and the underscore form the API accepts. medRxiv uses a separate subject list.",
                json!({"type": "object", "additionalProperties": false, "properties": {}}),
            ),
        ),
        (
            "biorxiv",
            ToolSchema::new(
                "get_content_statistics",
                "Retrieve platform-wide bioRxiv new and revised preprint counts from the Content API summary endpoint. interval is monthly (default) or yearly. Rows include the period, interval counts and cumulative counts. This is not a paper search.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "interval": {"type": "string", "enum": ["monthly", "yearly"], "default": "monthly"}
                    }
                }),
            ),
        ),
        (
            "biorxiv",
            ToolSchema::new(
                "get_preprint",
                "Retrieve every version of one bioRxiv or medRxiv preprint by DOI from the Content API. Accepts a bare DOI or a doi.org URL. Missing identifiers are listed; an empty body is not treated as a record. Preprints are not peer-reviewed. server is biorxiv (default) or medrxiv.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["doi"],
                    "properties": {
                        "doi": {"type": "string", "minLength": 1, "maxLength": 256},
                        "server": {"type": "string", "enum": ["biorxiv", "medrxiv"], "default": "biorxiv"}
                    }
                }),
            ),
        ),
        (
            "biorxiv",
            ToolSchema::new(
                "get_usage_statistics",
                "Retrieve abstract-view, full-text-view and PDF-download counts from the Content API usage endpoint. interval is monthly (default) or yearly. server is biorxiv (default) or medrxiv. Rows include the period, interval counts and cumulative counts.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "interval": {"type": "string", "enum": ["monthly", "yearly"], "default": "monthly"},
                        "server": {"type": "string", "enum": ["biorxiv", "medrxiv"], "default": "biorxiv"}
                    }
                }),
            ),
        ),
        (
            "biorxiv",
            ToolSchema::new(
                "search_by_funder",
                "Page through preprints that declare a funder ROR identifier. Requires YYYY-MM-DD date_from and date_to; funder metadata starts on 2025-04-10. Accepts a 9-character ROR id or a ror.org URL. Optional category uses the Content API underscore form. Results are a bounded page (limit 1-100). server is biorxiv (default) or medrxiv.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["funder_ror_id", "date_from", "date_to"],
                    "properties": {
                        "funder_ror_id": {"type": "string", "minLength": 1, "maxLength": 128},
                        "date_from": {"type": "string", "description": "Start date YYYY-MM-DD; not before 2025-04-10."},
                        "date_to": {"type": "string", "description": "End date YYYY-MM-DD."},
                        "category": {"type": "string", "maxLength": 128},
                        "cursor": {"type": "integer", "minimum": 0, "maximum": 1000000, "default": 0},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 10},
                        "server": {"type": "string", "enum": ["biorxiv", "medrxiv"], "default": "biorxiv"}
                    }
                }),
            ),
        ),
        (
            "biorxiv",
            ToolSchema::new(
                "search_preprints",
                "Page through bioRxiv or medRxiv preprint metadata. The Content API does not offer keyword search; filter by date interval, recent_days, recent_count or subject category. Default interval is the most recent 60 days. server is biorxiv (default) or medrxiv. Results are a bounded page (limit 1-100); use cursor for further pages. Preprints are not peer-reviewed.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "server": {"type": "string", "enum": ["biorxiv", "medrxiv"], "default": "biorxiv"},
                        "category": {"type": "string", "maxLength": 128, "description": "Subject category; spaces become underscores."},
                        "date_from": {"type": "string", "description": "Start date YYYY-MM-DD. Supply both dates."},
                        "date_to": {"type": "string", "description": "End date YYYY-MM-DD. Supply both dates."},
                        "recent_days": {"type": "integer", "minimum": 1, "maximum": 3650},
                        "recent_count": {"type": "integer", "minimum": 1, "maximum": 10000, "description": "Most recent N posts as documented by the Content API interval form."},
                        "cursor": {"type": "integer", "minimum": 0, "maximum": 1000000, "default": 0},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 10}
                    }
                }),
            ),
        ),
        (
            "biorxiv",
            ToolSchema::new(
                "search_published_preprints",
                "Page through preprint-to-journal links from the Content API pubs endpoint, or from the bioRxiv-only publisher prefix endpoint. Date, recent_days and recent_count intervals match the listing API. publisher is a DOI prefix such as 10.1038 and cannot be combined with medrxiv. include_details (default true) controls whether authors and abstracts are returned. Bounded page, limit 1-100.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "server": {"type": "string", "enum": ["biorxiv", "medrxiv"], "default": "biorxiv"},
                        "date_from": {"type": "string"},
                        "date_to": {"type": "string"},
                        "recent_days": {"type": "integer", "minimum": 1, "maximum": 3650},
                        "recent_count": {"type": "integer", "minimum": 1, "maximum": 10000},
                        "publisher": {"type": "string", "description": "Publisher DOI prefix, e.g. 10.1038. bioRxiv only."},
                        "include_details": {"type": "boolean", "default": true},
                        "cursor": {"type": "integer", "minimum": 0, "maximum": 1000000, "default": 0},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 10}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("bioRxiv request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "get_categories" => categories(args),
        "get_content_statistics" => content_statistics(bio, args).await,
        "get_preprint" => preprint(bio, args).await,
        "get_usage_statistics" => usage_statistics(bio, args).await,
        "search_by_funder" => search_funder(bio, args).await,
        "search_preprints" => search_preprints(bio, args).await,
        "search_published_preprints" => search_published(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn categories(args: &Value) -> Result<Value> {
    let _: Empty = parse_args(args, "category")?;
    let categories: Vec<Value> = BIORXIV_CATEGORIES
        .iter()
        .map(|name| {
            json!({
                "name": name,
                "api_format": name.replace(' ', "_")
            })
        })
        .collect();
    Ok(json!({
        "source": "bioRxiv Content API",
        "source_url": SOURCE_URL,
        "server": "biorxiv",
        "returned": categories.len(),
        "categories": categories
    }))
}

async fn content_statistics(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Stats = parse_args(args, "content statistics")?;
    let code = interval_code(&args.interval)?;
    let raw = get_json(bio, &format!("/sum/{code}/json"), &[]).await?;
    require_ok(&raw)?;
    let records = stats_rows(&raw)?
        .iter()
        .map(content_row)
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "source": "bioRxiv Content API",
        "source_url": SOURCE_URL,
        "interval": args.interval,
        "returned": records.len(),
        "records": records
    }))
}

async fn usage_statistics(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Usage = parse_args(args, "usage statistics")?;
    let server = require_server(&args.server)?;
    let code = interval_code(&args.interval)?;
    let raw = get_json(bio, &format!("/usage/{code}/{server}/json"), &[]).await?;
    require_ok(&raw)?;
    let records = stats_rows(&raw)?
        .iter()
        .map(usage_row)
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "source": "bioRxiv Content API",
        "source_url": SOURCE_URL,
        "server": server,
        "interval": args.interval,
        "returned": records.len(),
        "records": records
    }))
}

async fn preprint(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Preprint = parse_args(args, "preprint")?;
    let server = require_server(&args.server)?;
    let doi = normalize_doi(&args.doi)?;
    let response = get(bio, &format!("/details/{server}/{doi}/na/json"), &[]).await?;
    if response.status == reqwest::StatusCode::NOT_FOUND {
        return Ok(missing_preprint(server, &doi));
    }
    let raw = response.json()?;
    match api_status(&raw)?.as_str() {
        "no posts found" => return Ok(missing_preprint(server, &doi)),
        "ok" => {}
        _ => bail!("bioRxiv rejected the request"),
    }
    let versions = collection(&raw)?;
    if versions.is_empty() {
        return Ok(missing_preprint(server, &doi));
    }
    let records = versions
        .iter()
        .map(|row| preprint_record(row, server))
        .collect::<Result<Vec<_>>>()?;
    let latest = latest_index(&versions);
    Ok(json!({
        "source": "bioRxiv Content API",
        "source_url": SOURCE_URL,
        "server": server,
        "requested_doi": doi,
        "found": true,
        "n_versions": records.len(),
        "preprint": records[latest],
        "versions": records,
        "missing_dois": []
    }))
}

async fn search_preprints(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search = parse_args(args, "preprint search")?;
    let listing = listing(&args)?;
    let needed = needed(&listing);
    let server = listing.server.clone();
    let interval = listing.interval.path();
    let params = category_params(&listing.category);
    let (rows, total) = collect(
        bio,
        move |cursor| format!("/details/{server}/{interval}/{cursor}/json"),
        &params,
        listing.cursor,
        needed,
    )
    .await?;
    let records = rows
        .iter()
        .map(|row| preprint_summary(row, &listing.server))
        .collect::<Result<Vec<_>>>()?;
    Ok(search_page(
        &listing,
        records,
        total,
        DETAILS_PAGE,
        json!({ "category": listing.category }),
    ))
}

async fn search_published(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Published = parse_args(args, "published preprint search")?;
    let listing = listing(&Search {
        server: args.server.clone(),
        category: None,
        cursor: args.cursor,
        date_from: args.date_from.clone(),
        date_to: args.date_to.clone(),
        limit: args.limit,
        recent_count: args.recent_count,
        recent_days: args.recent_days,
    })?;
    let publisher = publisher_prefix(args.publisher.as_deref(), &listing.server)?;
    let needed = needed(&listing);
    let server = listing.server.clone();
    let interval = listing.interval.path();
    let (rows, total) = if let Some(prefix) = publisher.clone() {
        collect(
            bio,
            move |cursor| format!("/publisher/{prefix}/{interval}/{cursor}"),
            &[],
            listing.cursor,
            needed,
        )
        .await?
    } else {
        collect(
            bio,
            move |cursor| format!("/pubs/{server}/{interval}/{cursor}/json"),
            &[],
            listing.cursor,
            needed,
        )
        .await?
    };
    let records = rows
        .iter()
        .map(|row| published_record(row, &listing.server, args.include_details))
        .collect::<Result<Vec<_>>>()?;
    Ok(search_page(
        &listing,
        records,
        total,
        LIST_PAGE,
        json!({
            "publisher": publisher,
            "include_details": args.include_details
        }),
    ))
}

async fn search_funder(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Funder = parse_args(args, "funder search")?;
    let server = require_server(&args.server)?.to_string();
    let ror = normalize_ror(&args.funder_ror_id)?;
    let from = parse_date(&args.date_from, "date_from")?;
    let to = parse_date(&args.date_to, "date_to")?;
    if from > to {
        bail!("date_from must be on or before date_to");
    }
    let inception = chrono::NaiveDate::parse_from_str(FUNDER_START, "%Y-%m-%d").unwrap();
    if from < inception {
        bail!("funder metadata starts on {FUNDER_START}");
    }
    if !(1..=100).contains(&args.limit) || args.cursor > 1_000_000 {
        bail!("request 1 to 100 records with cursor 0 to 1000000");
    }
    let category = category_param(args.category.as_deref())?;
    let listing = Listing {
        server: server.clone(),
        interval: Interval::Dates {
            from: args.date_from.clone(),
            to: args.date_to.clone(),
        },
        category: category.clone(),
        cursor: args.cursor,
        limit: args.limit,
    };
    let interval = listing.interval.path();
    let params = category_params(&category);
    let ror_path = ror.clone();
    let (rows, total) = collect(
        bio,
        move |cursor| format!("/funder/{server}/{interval}/{ror_path}/{cursor}/json"),
        &params,
        listing.cursor,
        listing.limit,
    )
    .await?;
    let records = rows
        .iter()
        .map(|row| preprint_summary(row, &listing.server))
        .collect::<Result<Vec<_>>>()?;
    Ok(search_page(
        &listing,
        records,
        total,
        LIST_PAGE,
        json!({
            "funder_ror_id": ror,
            "category": category
        }),
    ))
}

fn search_page(
    listing: &Listing,
    records: Vec<Value>,
    total: Option<u64>,
    page_size: u64,
    extra: Value,
) -> Value {
    let returned = records.len() as u64;
    let next = listing.cursor + returned;
    let total = match total {
        Some(0) if returned > 0 => None,
        other => other,
    };
    let has_more = match total {
        Some(total) => next < total,
        None => returned >= listing.limit as u64 && returned > 0,
    };
    let mut result = json!({
        "source": "bioRxiv Content API",
        "source_url": SOURCE_URL,
        "server": listing.server,
        "interval": listing.interval.path(),
        "cursor": listing.cursor,
        "limit": listing.limit,
        "returned": returned,
        "total": total,
        "has_more": has_more,
        "next_cursor": if has_more { Value::from(next) } else { Value::Null },
        "page_size": page_size,
        "records": records
    });
    if let Some(object) = extra.as_object() {
        for (key, value) in object {
            result[key] = value.clone();
        }
    }
    result
}

fn missing_preprint(server: &str, doi: &str) -> Value {
    json!({
        "source": "bioRxiv Content API",
        "source_url": SOURCE_URL,
        "server": server,
        "requested_doi": doi,
        "found": false,
        "n_versions": 0,
        "preprint": null,
        "versions": [],
        "missing_dois": [doi]
    })
}

fn preprint_record(raw: &Value, server: &str) -> Result<Value> {
    let doi = json_text(&raw["doi"]).context("bioRxiv omitted a preprint DOI")?;
    let version = json_text(&raw["version"]);
    let site = host(json_text(&raw["server"]).as_deref().unwrap_or(server));
    let web = match &version {
        Some(version) => format!("https://{site}/content/{doi}v{version}"),
        None => format!("https://{site}/content/{doi}"),
    };
    Ok(json!({
        "doi": doi,
        "doi_url": format!("https://doi.org/{doi}"),
        "url": web,
        "pdf_url": format!("{web}.full.pdf"),
        "title": json_text(&raw["title"]),
        "authors": json_text(&raw["authors"]),
        "author_corresponding": json_text(&raw["author_corresponding"]),
        "author_corresponding_institution": json_text(&raw["author_corresponding_institution"]),
        "date": json_text(&raw["date"]),
        "version": version,
        "type": json_text(&raw["type"]),
        "category": json_text(&raw["category"]),
        "license": json_text(&raw["license"]),
        "abstract": json_text(&raw["abstract"]),
        "jatsxml": json_text(&raw["jatsxml"]),
        "funding": funding(raw),
        "published_doi": json_text(&raw["published"]),
        "server": json_text(&raw["server"]).unwrap_or_else(|| server.to_string())
    }))
}

fn preprint_summary(raw: &Value, server: &str) -> Result<Value> {
    let record = preprint_record(raw, server)?;
    let (preview, truncated) = match record["abstract"].as_str() {
        Some(text) => abstract_preview(text),
        None => (Value::Null, false),
    };
    Ok(json!({
        "doi": record["doi"],
        "doi_url": record["doi_url"],
        "url": record["url"],
        "title": record["title"],
        "authors": record["authors"],
        "date": record["date"],
        "category": record["category"],
        "version": record["version"],
        "abstract_preview": preview,
        "abstract_truncated": truncated
    }))
}

fn published_record(raw: &Value, server: &str, details: bool) -> Result<Value> {
    let preprint_doi = json_text(&raw["preprint_doi"])
        .or_else(|| json_text(&raw["biorxiv_doi"]))
        .context("bioRxiv omitted a preprint DOI")?;
    let published_doi = json_text(&raw["published_doi"]);
    let platform = json_text(&raw["preprint_platform"]).unwrap_or_else(|| server.to_string());
    let site = host(&platform);
    let mut record = json!({
        "preprint_doi": preprint_doi,
        "published_doi": published_doi,
        "journal": json_text(&raw["published_journal"]),
        "preprint_platform": platform,
        "preprint_title": json_text(&raw["preprint_title"]),
        "preprint_category": json_text(&raw["preprint_category"]),
        "preprint_date": json_text(&raw["preprint_date"]),
        "published_date": json_text(&raw["published_date"]),
        "preprint_url": format!("https://{site}/content/{preprint_doi}"),
        "published_url": published_doi.as_ref().map(|doi| format!("https://doi.org/{doi}"))
    });
    if details {
        record["preprint_authors"] = json!(json_text(&raw["preprint_authors"]));
        record["preprint_abstract"] = json!(json_text(&raw["preprint_abstract"]));
        record["preprint_author_corresponding"] =
            json!(json_text(&raw["preprint_author_corresponding"]));
        record["preprint_author_corresponding_institution"] =
            json!(json_text(&raw["preprint_author_corresponding_institution"]));
    }
    Ok(record)
}

fn funding(raw: &Value) -> Value {
    for key in ["funding", "funder"] {
        match raw.get(key) {
            Some(Value::String(text))
                if text.trim().is_empty() || text.eq_ignore_ascii_case("na") =>
            {
                return Value::Null
            }
            Some(value) if !value.is_null() => return value.clone(),
            _ => {}
        }
    }
    Value::Null
}

fn latest_index(versions: &[Value]) -> usize {
    versions
        .iter()
        .enumerate()
        .max_by_key(|(index, row)| (json_u64(&row["version"]).unwrap_or(0), *index))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn abstract_preview(text: &str) -> (Value, bool) {
    if text.chars().count() <= ABSTRACT_PREVIEW {
        (json!(text), false)
    } else {
        (
            json!(text.chars().take(ABSTRACT_PREVIEW).collect::<String>()),
            true,
        )
    }
}

async fn collect(
    bio: &NativeBio,
    path: impl Fn(u64) -> String,
    params: &[(String, String)],
    start: u64,
    needed: usize,
) -> Result<(Vec<Value>, Option<u64>)> {
    let mut records = Vec::new();
    let mut total = None;
    let mut cursor = start;
    for _ in 0..MAX_PAGES {
        if records.len() >= needed {
            break;
        }
        let raw = get_json(bio, &path(cursor), params).await?;
        match api_status(&raw)?.as_str() {
            "no posts found" => {
                if records.is_empty() {
                    return Ok((records, Some(0)));
                }
                break;
            }
            "ok" => {}
            _ => bail!("bioRxiv rejected the request"),
        }
        if let Some(page_total) = message(&raw).and_then(|msg| json_u64(&msg["total"])) {
            if let Some(previous) = total {
                if previous != page_total {
                    bail!("bioRxiv reported an inconsistent result total");
                }
            }
            total = Some(page_total);
        }
        let page = collection(&raw)?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len() as u64;
        records.extend(page);
        cursor += page_len;
        if total.is_some_and(|total| cursor >= total) {
            break;
        }
    }
    records.truncate(needed);
    Ok((records, total))
}

async fn get_json(bio: &NativeBio, path: &str, params: &[(String, String)]) -> Result<Value> {
    get(bio, path, params).await?.json()
}

async fn get(
    bio: &NativeBio,
    path: &str,
    params: &[(String, String)],
) -> Result<crate::http::Response> {
    let url = format!("{}{path}", api_base(bio));
    bio.http().send(SOURCE, Method::GET, &url, params).await
}

fn api_base(bio: &NativeBio) -> &str {
    bio.credential("BIORXIV_API")
        .map(|base| base.trim_end_matches('/'))
        .filter(|base| !base.is_empty())
        .unwrap_or(API)
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: &Value, what: &str) -> Result<T> {
    serde_json::from_value(args.clone()).with_context(|| format!("invalid {what} arguments"))
}

fn listing(args: &Search) -> Result<Listing> {
    let server = require_server(&args.server)?.to_string();
    if !(1..=100).contains(&args.limit) || args.cursor > 1_000_000 {
        bail!("request 1 to 100 records with cursor 0 to 1000000");
    }
    Ok(Listing {
        server,
        interval: listing_interval(
            args.date_from.as_deref(),
            args.date_to.as_deref(),
            args.recent_days,
            args.recent_count,
        )?,
        category: category_param(args.category.as_deref())?,
        cursor: args.cursor,
        limit: args.limit,
    })
}

fn listing_interval(
    date_from: Option<&str>,
    date_to: Option<&str>,
    recent_days: Option<u64>,
    recent_count: Option<u64>,
) -> Result<Interval> {
    let methods = [
        date_from.is_some() || date_to.is_some(),
        recent_days.is_some(),
        recent_count.is_some(),
    ]
    .iter()
    .filter(|flag| **flag)
    .count();
    if methods > 1 {
        bail!("provide only one of date range, recent_days or recent_count");
    }
    if let Some(days) = recent_days {
        if !(1..=3650).contains(&days) {
            bail!("recent_days must be 1 to 3650");
        }
        return Ok(Interval::RecentDays(days));
    }
    if let Some(count) = recent_count {
        if !(1..=10_000).contains(&count) {
            bail!("recent_count must be 1 to 10000");
        }
        return Ok(Interval::RecentCount(count));
    }
    match (date_from, date_to) {
        (None, None) => Ok(Interval::RecentDays(60)),
        (Some(from), Some(to)) => {
            if parse_date(from, "date_from")? > parse_date(to, "date_to")? {
                bail!("date_from must be on or before date_to");
            }
            Ok(Interval::Dates {
                from: from.to_string(),
                to: to.to_string(),
            })
        }
        _ => bail!("provide both date_from and date_to"),
    }
}

fn needed(listing: &Listing) -> usize {
    match listing.interval {
        Interval::RecentCount(count) => listing.limit.min(count as usize),
        _ => listing.limit,
    }
}

fn category_param(value: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if raw.len() > 128 {
        bail!("category must be at most 128 characters");
    }
    let normalized = raw.to_ascii_lowercase().replace(' ', "_");
    if !normalized
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("category must be a subject name or underscore form");
    }
    Ok(Some(normalized))
}

fn category_params(category: &Option<String>) -> Vec<(String, String)> {
    category
        .iter()
        .map(|value| ("category".into(), value.clone()))
        .collect()
}

fn publisher_prefix(value: Option<&str>, server: &str) -> Result<Option<String>> {
    let Some(raw) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if server != "biorxiv" {
        bail!("publisher prefixes are documented for bioRxiv only");
    }
    if raw.len() > 16
        || !raw.starts_with("10.")
        || raw.contains('/')
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        || raw[3..].len() < 4
    {
        bail!("publisher must be a DOI prefix such as 10.1038");
    }
    Ok(Some(raw.to_string()))
}

fn require_server(value: &str) -> Result<&str> {
    match value {
        "biorxiv" | "medrxiv" => Ok(value),
        _ => bail!("server must be biorxiv or medrxiv"),
    }
}

fn host(server: &str) -> &'static str {
    if server.eq_ignore_ascii_case("medrxiv") {
        "www.medrxiv.org"
    } else {
        "www.biorxiv.org"
    }
}

fn interval_code(interval: &str) -> Result<&'static str> {
    match interval {
        "monthly" => Ok("m"),
        "yearly" => Ok("y"),
        _ => bail!("interval must be monthly or yearly"),
    }
}

fn normalize_doi(raw: &str) -> Result<String> {
    let mut doi = raw.trim();
    for prefix in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi:",
    ] {
        if let Some(rest) = doi.strip_prefix(prefix) {
            doi = rest;
            break;
        }
    }
    let doi = doi.trim().trim_start_matches('/');
    if doi.is_empty()
        || doi.len() > 256
        || !doi.starts_with("10.")
        || !doi.contains('/')
        || doi.chars().any(char::is_whitespace)
    {
        bail!("DOI values must start with 10. and contain a slash");
    }
    Ok(doi.to_string())
}

fn normalize_ror(raw: &str) -> Result<String> {
    let id = raw
        .trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if id.len() != 9
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        bail!("funder_ror_id must be a 9-character ROR identifier");
    }
    Ok(id)
}

fn parse_date(value: &str, name: &str) -> Result<chrono::NaiveDate> {
    let Some((year, month, day)) = value.split_once('-').and_then(|(year, rest)| {
        let (month, day) = rest.split_once('-')?;
        Some((year, month, day))
    }) else {
        bail!("{name} must be YYYY-MM-DD");
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 || value.len() != 10 {
        bail!("{name} must be YYYY-MM-DD");
    }
    let year = year
        .parse()
        .map_err(|_| anyhow!("{name} must be YYYY-MM-DD"))?;
    let month = month
        .parse()
        .map_err(|_| anyhow!("{name} must be YYYY-MM-DD"))?;
    let day = day
        .parse()
        .map_err(|_| anyhow!("{name} must be YYYY-MM-DD"))?;
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| anyhow!("{name} must be a valid calendar date"))
}

fn require_ok(raw: &Value) -> Result<()> {
    match api_status(raw)?.as_str() {
        "ok" => Ok(()),
        _ => bail!("bioRxiv rejected the request"),
    }
}

fn api_status(raw: &Value) -> Result<String> {
    json_text(&message(raw).context("bioRxiv omitted status messages")?["status"])
        .context("bioRxiv omitted a status")
}

fn message(raw: &Value) -> Option<&Value> {
    raw.get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.first())
}

fn collection(raw: &Value) -> Result<Vec<Value>> {
    match raw.get("collection") {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.clone()),
        Some(_) => bail!("bioRxiv returned an invalid result collection"),
    }
}

fn stats_rows(raw: &Value) -> Result<Vec<Value>> {
    raw.as_object()
        .and_then(|object| {
            object
                .iter()
                .find(|(key, value)| *key != "messages" && value.is_array())
                .and_then(|(_, value)| value.as_array().cloned())
        })
        .context("bioRxiv omitted statistics rows")
}

fn content_row(row: &Value) -> Result<Value> {
    let mut out = serde_json::Map::new();
    if let Some(month) = json_text(&row["month"]) {
        out.insert("month".into(), json!(month));
    } else if let Some(year) = json_u64(&row["year"]) {
        out.insert("year".into(), json!(year));
    } else {
        bail!("bioRxiv omitted a statistics period");
    }
    out.insert("new_papers".into(), json!(require_u64(row, "new_papers")?));
    out.insert(
        "new_papers_cumulative".into(),
        json!(require_u64(row, "new_papers_cumulative")?),
    );
    out.insert(
        "revised_papers".into(),
        json!(require_u64(row, "revised_papers")?),
    );
    out.insert(
        "revised_papers_cumulative".into(),
        json!(require_u64(row, "revised_papers_cumulative")?),
    );
    out.insert(
        "preprint_date".into(),
        row.get("preprint_date").cloned().unwrap_or(Value::Null),
    );
    Ok(Value::Object(out))
}

fn usage_row(row: &Value) -> Result<Value> {
    let mut out = serde_json::Map::new();
    if let Some(month) = json_text(&row["month"]) {
        out.insert("month".into(), json!(month));
    } else if let Some(year) = json_u64(&row["year"]) {
        out.insert("year".into(), json!(year));
    } else {
        bail!("bioRxiv omitted a statistics period");
    }
    for key in [
        "abstract_views",
        "full_text_views",
        "pdf_downloads",
        "abstract_cumulative",
        "full_text_cumulative",
        "pdf_cumulative",
    ] {
        out.insert(key.into(), json!(require_u64(row, key)?));
    }
    Ok(Value::Object(out))
}

fn require_u64(row: &Value, key: &str) -> Result<u64> {
    json_u64(&row[key]).with_context(|| format!("bioRxiv omitted {key}"))
}

fn json_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn json_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() || text.eq_ignore_ascii_case("na") {
                None
            } else {
                Some(text.to_string())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn default_server() -> String {
    "biorxiv".into()
}

fn default_limit() -> usize {
    10
}

fn default_monthly() -> String {
    "monthly".into()
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stats {
    #[serde(default = "default_monthly")]
    interval: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Usage {
    #[serde(default = "default_monthly")]
    interval: String,
    #[serde(default = "default_server")]
    server: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preprint {
    doi: String,
    #[serde(default = "default_server")]
    server: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default = "default_server")]
    server: String,
    category: Option<String>,
    #[serde(default)]
    cursor: u64,
    date_from: Option<String>,
    date_to: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    recent_count: Option<u64>,
    recent_days: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Published {
    #[serde(default = "default_server")]
    server: String,
    date_from: Option<String>,
    date_to: Option<String>,
    recent_count: Option<u64>,
    recent_days: Option<u64>,
    publisher: Option<String>,
    #[serde(default = "default_true")]
    include_details: bool,
    #[serde(default)]
    cursor: u64,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Funder {
    funder_ror_id: String,
    date_from: String,
    date_to: String,
    category: Option<String>,
    #[serde(default)]
    cursor: u64,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_server")]
    server: String,
}

struct Listing {
    server: String,
    interval: Interval,
    category: Option<String>,
    cursor: u64,
    limit: usize,
}

enum Interval {
    Dates { from: String, to: String },
    RecentDays(u64),
    RecentCount(u64),
}

impl Interval {
    fn path(&self) -> String {
        match self {
            Self::Dates { from, to } => format!("{from}/{to}"),
            Self::RecentDays(days) => format!("{days}d"),
            Self::RecentCount(count) => count.to_string(),
        }
    }
}
