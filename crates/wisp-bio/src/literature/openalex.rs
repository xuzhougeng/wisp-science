//! OpenAlex REST client. Page size follows the documented `per_page` maximum
//! of 100; `per-page=200` is accepted only as deprecated legacy behavior.
use super::{bound, encode_segment, short_id, trimmed, NativeBio, OPENALEX};
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const PER_PAGE: usize = 100;
const LIST_CAP: usize = 200;
const BATCH: usize = 50;
const OPEN_ABSTRACT_LICENSES: &[&str] = &["cc-by", "cc-by-sa", "cc0", "public-domain"];
const WORK_URL: &str = "https://openalex.org/";
const SOURCE_URL: &str = "https://api.openalex.org";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchWorks {
    query: Option<String>,
    year_from: Option<i32>,
    year_to: Option<i32>,
    work_type: Option<String>,
    #[serde(default)]
    open_access_only: bool,
    venue: Option<String>,
    sort: Option<String>,
    #[serde(default = "default_works")]
    max_records: usize,
    #[serde(default)]
    include_abstracts: bool,
    mailto: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkId {
    work_id: String,
    mailto: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Citations {
    work_id: String,
    sort: Option<String>,
    #[serde(default = "default_works")]
    max_records: usize,
    #[serde(default)]
    include_abstracts: bool,
    mailto: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct References {
    work_id: String,
    #[serde(default = "default_refs")]
    max_records: usize,
    mailto: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchAuthors {
    query: String,
    #[serde(default = "default_authors")]
    max_records: usize,
    mailto: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetAuthor {
    author_id: String,
    #[serde(default = "default_sample")]
    works_sample: usize,
    mailto: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VenueInfo {
    venue: String,
    #[serde(default = "default_venues")]
    max_records: usize,
    mailto: Option<String>,
}

fn default_works() -> usize {
    50
}
fn default_refs() -> usize {
    100
}
fn default_authors() -> usize {
    25
}
fn default_sample() -> usize {
    10
}
fn default_venues() -> usize {
    10
}

pub(super) async fn search_works(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchWorks =
        serde_json::from_value(args.clone()).context("invalid OpenAlex search arguments")?;
    let cap = bound("max_records", args.max_records, 1, LIST_CAP)?;
    let query = trimmed(args.query.as_deref());
    let mut filters = Vec::new();
    year_filter(args.year_from, args.year_to, &mut filters)?;
    if let Some(work_type) = trimmed(args.work_type.as_deref()) {
        filters.push(format!("type:{}", work_token(work_type)?));
    }
    if args.open_access_only {
        filters.push("open_access.is_oa:true".into());
    }
    let mut venue_resolved = None;
    if let Some(venue) = trimmed(args.venue.as_deref()) {
        let (source_id, resolved) = resolve_venue(bio, venue, args.mailto.as_deref()).await?;
        venue_resolved = resolved;
        if let Some(issn) = source_id.strip_prefix("issn:") {
            filters.push(format!("primary_location.source.issn:{issn}"));
        } else {
            filters.push(format!("primary_location.source.id:{source_id}"));
        }
    }
    if query.is_none() && filters.is_empty() {
        bail!("provide a query and/or at least one filter");
    }
    let mut params = Vec::new();
    if let Some(query) = query {
        params.push(("search".into(), query.to_string()));
    }
    if !filters.is_empty() {
        params.push(("filter".into(), filters.join(",")));
    }
    if let Some(sort) = sort_param(args.sort.as_deref().unwrap_or("relevance"), query.is_some())? {
        params.push(("sort".into(), sort));
    }
    let listed = list(bio, "/works", params, cap, args.mailto.as_deref()).await?;
    let records: Vec<Value> = listed
        .rows
        .iter()
        .map(|row| lean_work(row, args.include_abstracts))
        .collect();
    let mut result = listing(
        json!({
            "query": query,
            "filters": filters,
            "sort": args.sort.as_deref().unwrap_or("relevance"),
        }),
        &listed,
        records,
    );
    if let Some(resolved) = venue_resolved {
        result["venue_resolved"] = resolved;
    }
    Ok(result)
}

pub(super) async fn get_work(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: WorkId =
        serde_json::from_value(args.clone()).context("invalid OpenAlex work arguments")?;
    let id = normalize_work_id(&args.work_id)?;
    let (raw, claimants) = load_work(bio, &id, args.mailto.as_deref()).await?;
    let mut record = lean_work(&raw, true);
    record["referenced_works"] = json!(referenced_ids(&raw));
    record["counts_by_year"] = raw.get("counts_by_year").cloned().unwrap_or(Value::Null);
    attach_claimants(&mut record, &id, &claimants);
    Ok(record)
}

pub(super) async fn citations(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Citations =
        serde_json::from_value(args.clone()).context("invalid OpenAlex citation arguments")?;
    let cap = bound("max_records", args.max_records, 1, LIST_CAP)?;
    let mut id = normalize_work_id(&args.work_id)?;
    let mut claimants = Vec::new();
    let doi_alias = id.clone();
    if id.starts_with("doi:") {
        let (raw, found) = resolve_doi(bio, &id, args.mailto.as_deref()).await?;
        id = short_id(raw["id"].as_str().unwrap_or(&id)).to_string();
        claimants = found;
    }
    let mut params = vec![("filter".into(), format!("cites:{id}"))];
    if let Some(sort) = sort_param(args.sort.as_deref().unwrap_or("cited_by_count"), false)? {
        params.push(("sort".into(), sort));
    }
    let listed = list(bio, "/works", params, cap, args.mailto.as_deref()).await?;
    let records: Vec<Value> = listed
        .rows
        .iter()
        .map(|row| lean_work(row, args.include_abstracts))
        .collect();
    let mut result = listing(json!({"work_id": id}), &listed, records);
    attach_claimants(&mut result, &doi_alias, &claimants);
    Ok(result)
}

pub(super) async fn references(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: References =
        serde_json::from_value(args.clone()).context("invalid OpenAlex reference arguments")?;
    let cap = bound("max_records", args.max_records, 1, LIST_CAP)?;
    let id = normalize_work_id(&args.work_id)?;
    let (raw, claimants) = load_work(bio, &id, args.mailto.as_deref()).await?;
    let work_id = short_id(raw["id"].as_str().unwrap_or(&id)).to_string();
    let reference_ids = referenced_ids(&raw);
    let hydrate = &reference_ids[..reference_ids.len().min(cap)];
    let mut records = Vec::new();
    for chunk in hydrate.chunks(BATCH) {
        let raw = get(
            bio,
            "/works",
            vec![
                ("filter".into(), format!("openalex:{}", chunk.join("|"))),
                ("per_page".into(), chunk.len().to_string()),
            ],
            args.mailto.as_deref(),
        )
        .await?;
        let rows = raw
            .get("results")
            .and_then(Value::as_array)
            .context("OpenAlex omitted reference records")?;
        records.extend(rows.iter().map(|row| lean_work(row, false)));
    }
    let got: HashSet<String> = records
        .iter()
        .filter_map(|row| row["openalex_id"].as_str().map(str::to_string))
        .collect();
    let not_hydrated: Vec<&String> = hydrate.iter().filter(|id| !got.contains(*id)).collect();
    let order: HashMap<&str, usize> = hydrate
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    records.sort_by_key(|row| {
        order
            .get(row["openalex_id"].as_str().unwrap_or(""))
            .copied()
            .unwrap_or(usize::MAX)
    });
    let mut result = json!({
        "provider": "OpenAlex",
        "source_url": SOURCE_URL,
        "work_id": work_id,
        "n_references": reference_ids.len(),
        "n_records_returned": records.len(),
        "records_truncated": reference_ids.len() > hydrate.len(),
        "references_not_hydrated": not_hydrated,
        "reference_ids": reference_ids,
        "records": records,
    });
    attach_claimants(&mut result, &id, &claimants);
    Ok(result)
}

pub(super) async fn search_authors(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchAuthors =
        serde_json::from_value(args.clone()).context("invalid OpenAlex author search arguments")?;
    let query = trimmed(Some(&args.query)).context("query must contain author-name text")?;
    let cap = bound("max_records", args.max_records, 1, LIST_CAP)?;
    let listed = list(
        bio,
        "/authors",
        vec![("search".into(), query.to_string())],
        cap,
        args.mailto.as_deref(),
    )
    .await?;
    let records: Vec<Value> = listed.rows.iter().map(lean_author).collect();
    Ok(listing(json!({"query": query}), &listed, records))
}

pub(super) async fn get_author(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetAuthor =
        serde_json::from_value(args.clone()).context("invalid OpenAlex author arguments")?;
    let sample = bound("works_sample", args.works_sample, 0, LIST_CAP)?;
    let id = normalize_author_id(&args.author_id)?;
    let raw = get(
        bio,
        &format!("/authors/{}", encode_segment(&id)),
        Vec::new(),
        args.mailto.as_deref(),
    )
    .await?;
    if raw.get("id").is_none() {
        bail!("OpenAlex omitted the author record");
    }
    let mut record = lean_author(&raw);
    record["counts_by_year"] = raw.get("counts_by_year").cloned().unwrap_or(Value::Null);
    if sample > 0 {
        if let Some(author_id) = record["author_id"].as_str() {
            let listed = get(
                bio,
                "/works",
                vec![
                    ("filter".into(), format!("author.id:{author_id}")),
                    ("sort".into(), "cited_by_count:desc".into()),
                    ("per_page".into(), sample.to_string()),
                ],
                args.mailto.as_deref(),
            )
            .await?;
            record["top_works_total"] = json!(meta_count(&listed)?);
            let rows = listed
                .get("results")
                .and_then(Value::as_array)
                .context("OpenAlex omitted the author's works")?;
            record["top_works"] = json!(rows
                .iter()
                .map(|row| lean_work(row, false))
                .collect::<Vec<_>>());
        }
    }
    Ok(record)
}

pub(super) async fn venue_info(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: VenueInfo =
        serde_json::from_value(args.clone()).context("invalid OpenAlex venue arguments")?;
    let venue = trimmed(Some(&args.venue)).context("venue must not be empty")?;
    match normalize_source_id(venue) {
        Ok(id) => {
            let raw = get(
                bio,
                &format!("/sources/{}", encode_segment(&id)),
                Vec::new(),
                args.mailto.as_deref(),
            )
            .await?;
            if raw.get("id").is_none() {
                bail!("OpenAlex omitted the source record");
            }
            let mut record = lean_source(&raw);
            record["counts_by_year"] = raw.get("counts_by_year").cloned().unwrap_or(Value::Null);
            Ok(record)
        }
        Err(error) => {
            let lower = venue.to_ascii_lowercase();
            if lower.starts_with("http") && lower.contains("openalex.org/") {
                bail!("{error}");
            }
            search_sources(bio, venue, args.max_records, args.mailto.as_deref()).await
        }
    }
}

async fn search_sources(
    bio: &NativeBio,
    query: &str,
    max_records: usize,
    mailto: Option<&str>,
) -> Result<Value> {
    let cap = bound("max_records", max_records, 1, LIST_CAP)?;
    let listed = list(
        bio,
        "/sources",
        vec![("search".into(), query.to_string())],
        cap,
        mailto,
    )
    .await?;
    let records: Vec<Value> = listed.rows.iter().map(lean_source).collect();
    Ok(listing(json!({"query": query}), &listed, records))
}

async fn resolve_venue(
    bio: &NativeBio,
    venue: &str,
    mailto: Option<&str>,
) -> Result<(String, Option<Value>)> {
    match normalize_source_id(venue) {
        Ok(id) => Ok((id, None)),
        Err(error) => {
            let lower = venue.to_ascii_lowercase();
            if lower.starts_with("http") && lower.contains("openalex.org/") {
                bail!("{error}");
            }
            let hits = search_sources(bio, venue, 1, mailto).await?;
            let record = hits["records"]
                .as_array()
                .and_then(|rows| rows.first())
                .with_context(|| {
                    format!("no OpenAlex source matches venue {venue:?}; pass an S-id or ISSN")
                })?;
            let source_id = record["source_id"]
                .as_str()
                .context("OpenAlex omitted the resolved source id")?
                .to_string();
            Ok((
                source_id.clone(),
                Some(json!({
                    "input": venue,
                    "source_id": source_id,
                    "display_name": record.get("display_name"),
                    "candidates_total": hits.get("api_total"),
                })),
            ))
        }
    }
}

struct Listed {
    total: u64,
    rows: Vec<Value>,
}

async fn list(
    bio: &NativeBio,
    path: &str,
    extra: Vec<(String, String)>,
    cap: usize,
    mailto: Option<&str>,
) -> Result<Listed> {
    let per_page = cap.min(PER_PAGE);
    let mut rows = Vec::new();
    let mut total = 0;
    let mut page = 1u32;
    while rows.len() < cap {
        let mut params = extra.clone();
        params.push(("per_page".into(), per_page.to_string()));
        params.push(("page".into(), page.to_string()));
        let raw = get(bio, path, params, mailto).await?;
        total = meta_count(&raw)?;
        let page_rows = raw
            .get("results")
            .and_then(Value::as_array)
            .context("OpenAlex omitted the result list")?;
        if page == 1 && page_rows.is_empty() && total > 0 {
            bail!("OpenAlex returned inconsistent pagination");
        }
        if page_rows.len() > per_page {
            bail!("OpenAlex returned more records than requested");
        }
        rows.extend(page_rows.iter().cloned());
        if page_rows.is_empty() || rows.len() as u64 >= total || page_rows.len() < per_page {
            break;
        }
        page += 1;
        if page > 3 {
            break;
        }
    }
    rows.truncate(cap);
    Ok(Listed { total, rows })
}

async fn load_work(bio: &NativeBio, id: &str, mailto: Option<&str>) -> Result<(Value, Vec<Value>)> {
    if id.starts_with("doi:") {
        resolve_doi(bio, id, mailto).await
    } else {
        let raw = get(
            bio,
            &format!("/works/{}", encode_segment(id)),
            Vec::new(),
            mailto,
        )
        .await?;
        if raw.get("id").is_none() {
            bail!("OpenAlex omitted the work record");
        }
        Ok((raw, Vec::new()))
    }
}

async fn resolve_doi(
    bio: &NativeBio,
    doi_alias: &str,
    mailto: Option<&str>,
) -> Result<(Value, Vec<Value>)> {
    let doi = &doi_alias[4..];
    let raw = get(
        bio,
        "/works",
        vec![
            ("filter".into(), format!("doi:{doi}")),
            ("per_page".into(), PER_PAGE.to_string()),
            ("sort".into(), "cited_by_count:desc".into()),
        ],
        mailto,
    )
    .await?;
    let mut results = raw
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .context("OpenAlex omitted DOI lookup results")?;
    if results.is_empty() {
        bail!("no OpenAlex work has {doi_alias}");
    }
    results.sort_by(|left, right| {
        cited_by(right)
            .cmp(&cited_by(left))
            .then_with(|| work_key(left).cmp(&work_key(right)))
    });
    let claimants = results
        .iter()
        .map(|work| {
            json!({
                "openalex_id": work.get("id").and_then(Value::as_str).map(short_id),
                "title": work.get("title").or_else(|| work.get("display_name")),
                "publication_year": work.get("publication_year"),
                "cited_by_count": work.get("cited_by_count"),
            })
        })
        .collect();
    Ok((results.remove(0), claimants))
}

async fn get(
    bio: &NativeBio,
    path: &str,
    mut params: Vec<(String, String)>,
    mailto: Option<&str>,
) -> Result<Value> {
    if let Some(key) = bio
        .credential("OPENALEX_API_KEY")
        .filter(|key| !key.is_empty())
    {
        params.push(("api_key".into(), key.to_string()));
    }
    if let Some(mail) = polite_mail(bio, mailto)? {
        params.push(("mailto".into(), mail));
    }
    let url = format!("{}{path}", super::openalex_base());
    let value = bio
        .http()
        .send(OPENALEX, Method::GET, &url, &params)
        .await?
        .json()?;
    if value.get("error").is_some() {
        bail!("OpenAlex rejected the request");
    }
    Ok(value)
}

fn polite_mail(bio: &NativeBio, mailto: Option<&str>) -> Result<Option<String>> {
    if let Some(raw) = mailto {
        let mail = raw.trim();
        if mail.is_empty() {
            return Ok(None);
        }
        if !mail.contains('@') || mail.contains(char::is_whitespace) || mail.len() > 256 {
            bail!("mailto must be an email address");
        }
        return Ok(Some(mail.to_string()));
    }
    Ok(bio
        .credential("NCBI_EMAIL")
        .map(str::trim)
        .filter(|mail| mail.contains('@') && !mail.is_empty())
        .map(str::to_string))
}

fn listing(extra: Value, listed: &Listed, records: Vec<Value>) -> Value {
    let mut result = extra;
    result["provider"] = json!("OpenAlex");
    result["source_url"] = json!(SOURCE_URL);
    result["api_total"] = json!(listed.total);
    result["n_records_returned"] = json!(records.len());
    result["records_truncated"] = json!(listed.total > records.len() as u64);
    result["records"] = json!(records);
    result
}

fn meta_count(raw: &Value) -> Result<u64> {
    raw.get("meta")
        .and_then(|meta| meta.get("count"))
        .and_then(Value::as_u64)
        .context("OpenAlex omitted the result count")
}

fn year_filter(from: Option<i32>, to: Option<i32>, filters: &mut Vec<String>) -> Result<()> {
    let valid = |year: i32| (1..=9999).contains(&year);
    match (from, to) {
        (None, None) => Ok(()),
        (Some(from), Some(to)) if valid(from) && valid(to) && from <= to => {
            filters.push(format!("publication_year:{from}-{to}"));
            Ok(())
        }
        (Some(from), None) if valid(from) => {
            filters.push(format!("publication_year:>{}", from - 1));
            Ok(())
        }
        (None, Some(to)) if valid(to) => {
            filters.push(format!("publication_year:<{}", to + 1));
            Ok(())
        }
        _ => bail!("years must be 1 to 9999 and year_from must not exceed year_to"),
    }
}

fn work_token(value: &str) -> Result<&str> {
    if value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        bail!("work_type must be an OpenAlex type token such as article or book-chapter");
    }
    Ok(value)
}

fn sort_param(sort: &str, has_search: bool) -> Result<Option<String>> {
    Ok(match sort {
        "relevance" => has_search.then(|| "relevance_score:desc".into()),
        "cited_by_count" => Some("cited_by_count:desc".into()),
        "publication_date" => Some("publication_date:desc".into()),
        _ => bail!("sort must be relevance, cited_by_count or publication_date"),
    })
}

fn referenced_ids(work: &Value) -> Vec<String> {
    work.get("referenced_works")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(short_id)
        .map(str::to_string)
        .collect()
}

fn attach_claimants(record: &mut Value, doi_alias: &str, claimants: &[Value]) {
    if claimants.len() > 1 {
        record["doi_claimants"] = json!(claimants);
        record["doi_resolution_note"] = json!(format!(
            "{} OpenAlex works claim {doi_alias}; selected the most-cited ({}) — see doi_claimants for the alternatives",
            claimants.len(),
            claimants[0]["openalex_id"].as_str().unwrap_or("unknown")
        ));
    }
}

fn cited_by(work: &Value) -> u64 {
    work.get("cited_by_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn work_key(work: &Value) -> String {
    work.get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub(super) fn normalize_work_id(work_id: &str) -> Result<String> {
    let wid = work_id.trim();
    if let Some(id) = openalex_typed_id(wid, 'W')? {
        return Ok(id);
    }
    if let Some(doi) = doi_from(wid) {
        return Ok(format!("doi:{}", checked_doi(&doi, work_id)?));
    }
    if looks_like_work_id(wid) {
        return Ok(wid.to_ascii_uppercase());
    }
    bail!("unrecognized work id {work_id:?} — pass an OpenAlex W-id, an openalex.org URL, or a DOI")
}

pub(super) fn normalize_author_id(author_id: &str) -> Result<String> {
    let aid = author_id.trim();
    if let Some(id) = openalex_typed_id(aid, 'A')? {
        return Ok(id);
    }
    if looks_like_author_id(aid) {
        return Ok(aid.to_ascii_uppercase());
    }
    let mut cand = aid;
    if let Some(rest) = cand
        .strip_prefix("orcid:")
        .or_else(|| cand.strip_prefix("ORCID:"))
    {
        cand = rest.trim();
    }
    for prefix in ["https://orcid.org/", "http://orcid.org/"] {
        if let Some(rest) = cand.strip_prefix(prefix) {
            cand = rest;
            break;
        }
    }
    if is_orcid(cand) {
        return Ok(format!("orcid:{cand}"));
    }
    bail!("unrecognized author id {author_id:?} — pass an OpenAlex A-id, an openalex.org URL, or an ORCID")
}

pub(super) fn normalize_source_id(source_id: &str) -> Result<String> {
    let sid = source_id.trim();
    if let Some(id) = openalex_typed_id(sid, 'S')? {
        return Ok(id);
    }
    if looks_like_source_id(sid) {
        return Ok(sid.to_ascii_uppercase());
    }
    let mut cand = sid;
    if let Some(rest) = cand
        .strip_prefix("issn:")
        .or_else(|| cand.strip_prefix("ISSN:"))
    {
        cand = rest.trim();
    }
    if is_issn(cand) {
        return Ok(format!("issn:{}", cand.to_ascii_uppercase()));
    }
    bail!("unrecognized source id {source_id:?} — pass an OpenAlex S-id, an openalex.org URL, or an ISSN")
}

fn openalex_typed_id(value: &str, want: char) -> Result<Option<String>> {
    for prefix in ["https://openalex.org/", "http://openalex.org/"] {
        if let Some(rest) = value.strip_prefix(prefix) {
            let ident = rest.split(['?', '#']).next().unwrap_or(rest);
            if ident.len() > 1
                && ident.as_bytes()[0].eq_ignore_ascii_case(&(want as u8))
                && ident[1..].bytes().all(|byte| byte.is_ascii_digit())
            {
                return Ok(Some(ident.to_ascii_uppercase()));
            }
            bail!("that openalex.org URL is not a {want} entity");
        }
    }
    Ok(None)
}

fn looks_like_work_id(value: &str) -> bool {
    value.len() > 1
        && value.as_bytes()[0].eq_ignore_ascii_case(&b'W')
        && value[1..].bytes().all(|byte| byte.is_ascii_digit())
}

fn looks_like_author_id(value: &str) -> bool {
    value.len() > 1
        && value.as_bytes()[0].eq_ignore_ascii_case(&b'A')
        && value[1..].bytes().all(|byte| byte.is_ascii_digit())
}

fn looks_like_source_id(value: &str) -> bool {
    value.len() > 1
        && value.as_bytes()[0].eq_ignore_ascii_case(&b'S')
        && value[1..].bytes().all(|byte| byte.is_ascii_digit())
}

fn doi_from(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let rest = if let Some(rest) = lower
        .strip_prefix("https://doi.org/")
        .or_else(|| lower.strip_prefix("http://doi.org/"))
        .or_else(|| lower.strip_prefix("https://dx.doi.org/"))
        .or_else(|| lower.strip_prefix("http://dx.doi.org/"))
    {
        &value[value.len() - rest.len()..]
    } else if let Some(rest) = lower.strip_prefix("doi:") {
        value[value.len() - rest.len()..].trim()
    } else {
        value
    };
    rest.starts_with("10.")
        .then(|| rest.to_string())
        .filter(|doi| doi.contains('/'))
}

fn checked_doi(doi: &str, original: &str) -> Result<String> {
    if doi.contains(',') || doi.contains('|') {
        bail!(
            "unsupported DOI {original:?}: ',' and '|' cannot be expressed in an OpenAlex filter"
        );
    }
    if doi.len() > 256 {
        bail!("DOI is too long");
    }
    Ok(doi.to_string())
}

fn is_orcid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 19
        && bytes[4] == b'-'
        && bytes[9] == b'-'
        && bytes[14] == b'-'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 9 | 14)
                || byte.is_ascii_digit()
                || (index == 18 && (*byte == b'X' || *byte == b'x'))
        })
}

fn is_issn(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 9
        && bytes[4] == b'-'
        && bytes[..4].iter().all(|byte| byte.is_ascii_digit())
        && bytes[5..8].iter().all(|byte| byte.is_ascii_digit())
        && (bytes[8].is_ascii_digit() || bytes[8] == b'X' || bytes[8] == b'x')
}

pub(super) fn reconstruct_abstract(index: &Value) -> Option<String> {
    let object = index.as_object()?;
    if object.is_empty() {
        return None;
    }
    let mut positions = BTreeMap::new();
    for (word, indexes) in object {
        let Some(indexes) = indexes.as_array() else {
            continue;
        };
        for index in indexes {
            if let Some(position) = index.as_u64() {
                positions.insert(position, word.as_str());
            }
        }
    }
    if positions.is_empty() {
        None
    } else {
        Some(positions.into_values().collect::<Vec<_>>().join(" "))
    }
}

fn work_license(work: &Value) -> Option<String> {
    for key in ["primary_location", "best_oa_location"] {
        if let Some(license) = work
            .get(key)
            .and_then(|location| location.get("license"))
            .and_then(Value::as_str)
        {
            let license = short_id(license).trim().to_ascii_lowercase();
            if !license.is_empty() {
                return Some(license);
            }
        }
    }
    None
}

pub(super) fn lean_work(work: &Value, with_abstract: bool) -> Value {
    let ids = work.get("ids").cloned().unwrap_or(Value::Null);
    let primary = work.get("primary_location").cloned().unwrap_or(Value::Null);
    let source = primary.get("source").cloned().unwrap_or(Value::Null);
    let oa = work.get("open_access").cloned().unwrap_or(Value::Null);
    let best_oa = work.get("best_oa_location").cloned().unwrap_or(Value::Null);
    let topic = work.get("primary_topic").cloned().unwrap_or(Value::Null);
    let openalex_id = work.get("id").and_then(Value::as_str).map(short_id);
    let doi = work
        .get("doi")
        .and_then(Value::as_str)
        .map(|doi| {
            doi.trim_start_matches("https://doi.org/")
                .trim_start_matches("http://doi.org/")
        })
        .filter(|doi| !doi.is_empty());
    let pmid = ids
        .get("pmid")
        .and_then(Value::as_str)
        .map(short_id)
        .filter(|id| !id.is_empty());
    let keywords = work
        .get("keywords")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("display_name").and_then(Value::as_str))
                .collect::<Vec<_>>()
        })
        .filter(|rows| !rows.is_empty());
    let mut record = json!({
        "openalex_id": openalex_id,
        "doi": doi,
        "pmid": pmid,
        "title": work.get("title").or_else(|| work.get("display_name")),
        "publication_year": work.get("publication_year"),
        "publication_date": work.get("publication_date"),
        "type": work.get("type"),
        "language": work.get("language"),
        "is_retracted": work.get("is_retracted"),
        "authors": work.get("authorships").and_then(Value::as_array).map(|rows| {
            rows.iter().map(lean_authorship).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "source": if source.is_object() {
            json!({
                "source_id": source.get("id").and_then(Value::as_str).map(short_id),
                "display_name": source.get("display_name"),
                "issn_l": source.get("issn_l"),
                "type": source.get("type"),
            })
        } else {
            Value::Null
        },
        "biblio": work.get("biblio"),
        "cited_by_count": work.get("cited_by_count"),
        "fwci": work.get("fwci"),
        "referenced_works_count": work.get("referenced_works_count"),
        "open_access": {
            "is_oa": oa.get("is_oa"),
            "oa_status": oa.get("oa_status"),
            "oa_url": oa.get("oa_url"),
        },
        "best_oa_pdf_url": best_oa.get("pdf_url"),
        "primary_topic": topic.get("display_name"),
        "keywords": keywords,
        "url": openalex_id.map(|id| format!("{WORK_URL}{id}")),
        "provider": "OpenAlex",
        "source_url": SOURCE_URL,
    });
    if with_abstract {
        let license = work_license(work);
        record["abstract_license"] = json!(license);
        if license
            .as_deref()
            .is_some_and(|license| OPEN_ABSTRACT_LICENSES.contains(&license))
        {
            record["abstract"] = json!(work
                .get("abstract_inverted_index")
                .and_then(reconstruct_abstract));
        } else {
            record["abstract"] = Value::Null;
            record["abstract_policy"] = json!(format!(
                "omitted: work license is {} — not verified-open, so the abstract is not reconstructed; read it at the DOI / landing page",
                license.as_deref().map(|license| format!("{license:?}")).unwrap_or_else(|| "not declared".into())
            ));
        }
    }
    record
}

fn lean_authorship(authorship: &Value) -> Value {
    let author = authorship.get("author").cloned().unwrap_or(Value::Null);
    json!({
        "author_id": author.get("id").and_then(Value::as_str).map(short_id),
        "name": author.get("display_name"),
        "orcid": author.get("orcid"),
        "position": authorship.get("author_position"),
        "is_corresponding": authorship.get("is_corresponding"),
        "institutions": authorship.get("institutions").and_then(Value::as_array).map(|rows| {
            rows.iter().filter_map(|row| row.get("display_name").and_then(Value::as_str)).collect::<Vec<_>>()
        }).unwrap_or_default(),
    })
}

fn lean_author(author: &Value) -> Value {
    let stats = author.get("summary_stats").cloned().unwrap_or(Value::Null);
    let author_id = author.get("id").and_then(Value::as_str).map(short_id);
    json!({
        "author_id": author_id,
        "name": author.get("display_name"),
        "orcid": author.get("orcid"),
        "works_count": author.get("works_count"),
        "cited_by_count": author.get("cited_by_count"),
        "h_index": stats.get("h_index"),
        "i10_index": stats.get("i10_index"),
        "affiliations": author.get("affiliations").and_then(Value::as_array).map(|rows| {
            rows.iter().take(10).map(|row| json!({
                "institution": row.get("institution").and_then(|institution| institution.get("display_name")),
                "years": row.get("years"),
            })).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "last_known_institutions": author.get("last_known_institutions").and_then(Value::as_array).map(|rows| {
            rows.iter().filter_map(|row| row.get("display_name").and_then(Value::as_str)).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "top_topics": author.get("topics").and_then(Value::as_array).map(|rows| {
            rows.iter().take(5).filter_map(|row| row.get("display_name").and_then(Value::as_str)).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "url": author_id.map(|id| format!("{WORK_URL}{id}")),
        "provider": "OpenAlex",
        "source_url": SOURCE_URL,
    })
}

fn lean_source(source: &Value) -> Value {
    let stats = source.get("summary_stats").cloned().unwrap_or(Value::Null);
    let source_id = source.get("id").and_then(Value::as_str).map(short_id);
    json!({
        "source_id": source_id,
        "display_name": source.get("display_name"),
        "type": source.get("type"),
        "issn_l": source.get("issn_l"),
        "issn": source.get("issn"),
        "host_organization": source.get("host_organization_name"),
        "country_code": source.get("country_code"),
        "homepage_url": source.get("homepage_url"),
        "is_oa": source.get("is_oa"),
        "is_in_doaj": source.get("is_in_doaj"),
        "is_core": source.get("is_core"),
        "apc_usd": source.get("apc_usd"),
        "works_count": source.get("works_count"),
        "cited_by_count": source.get("cited_by_count"),
        "h_index": stats.get("h_index"),
        "two_year_mean_citedness": stats.get("2yr_mean_citedness"),
        "first_publication_year": source.get("first_publication_year"),
        "last_publication_year": source.get("last_publication_year"),
        "top_topics": source.get("topics").and_then(Value::as_array).map(|rows| {
            rows.iter().take(5).filter_map(|row| row.get("display_name").and_then(Value::as_str)).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "url": source_id.map(|id| format!("{WORK_URL}{id}")),
        "provider": "OpenAlex",
        "source_url": SOURCE_URL,
    })
}
