//! Native `protein-annotation` domain against InterPro/Pfam, the Human Protein
//! Atlas, and STRING. Independently implemented from:
//!
//! - [InterPro 7 REST API](https://www.ebi.ac.uk/interpro/api/)
//! - [InterPro API URL design](https://github.com/ProteinsWebTeam/interpro7-api)
//! - [Human Protein Atlas programmatic access](https://www.proteinatlas.org/about/help/dataaccess)
//! - [STRING REST API](https://string-db.org/help/api/)
//!
//! References reviewed 2026-09-06. InterPro lists paginate with `count`/`next`
//! and `page_size` at most 200; HTTP 204 is an empty page, HTTP 404 is an
//! unknown accession. HPA gene JSON is `/{ENSG}.json`; bulk search is
//! `/api/search_download.php`. STRING is pinned to version 12.0; identifiers
//! are POSTed as form fields separated by CR. Tests use invented records.

mod interpro;
mod protein_atlas;
mod string;

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const DOMAIN: &str = "protein-annotation";
const INTERPRO_API: &str = "https://www.ebi.ac.uk/interpro/api";
const INTERPRO_SITE: &str = "https://www.ebi.ac.uk/interpro";
const HPA_SITE: &str = "https://www.proteinatlas.org";
const STRING_API: &str = "https://version-12-0.string-db.org/api";
const STRING_SITE: &str = "https://version-12-0.string-db.org";

const INTERPRO: Source = Source("InterPro", Duration::from_millis(500));
const HPA: Source = Source("Human Protein Atlas", Duration::from_millis(500));
const STRING: Source = Source("STRING", Duration::from_secs(1));

const MAX_RESULTS: u32 = 200;
const DEFAULT_MAX: u32 = 25;
const MAX_PAGES: usize = 5;
const MAX_PAGE_SIZE: u32 = 200;
const MAX_IDS: usize = 50;
const MAX_PROTEINS: usize = 20;
const MAX_QUERY: usize = 512;
const MAX_COLUMNS: usize = 40;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            DOMAIN,
            ToolSchema::new(
                "get_domain_architecture",
                "Retrieve InterPro domain architecture for UniProt proteins. GET /entry/interpro/protein/uniprot/{accession}/ with cursor pagination (page_size ≤ 200). HTTP 204 is a protein with no InterPro matches; HTTP 404 is listed in missing_ids. Each protein reports total_entries versus the bounded returned page and has_more when InterPro's count exceeds the page. Member-database signatures and fragment coordinates are included. At most 20 accessions per call.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["accessions"],
                    "properties": {
                        "accessions": {
                            "type": "array", "minItems": 1, "maxItems": 20,
                            "items": {"type": "string", "minLength": 6, "maxLength": 16}
                        },
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_interpro_entry",
                "Fetch one InterPro entry (IPRxxxxxx) or Pfam family (PFxxxxx) from GET /entry/{interpro|pfam}/{accession}/. The database path is chosen from the accession prefix. Returns name, type, integration, GO terms, member-database signatures, clan/set membership when present, and the InterPro website URL. HTTP 404 means the accession is unknown.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["accession"],
                    "properties": {
                        "accession": {"type": "string", "minLength": 5, "maxLength": 16}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_pfam_clan",
                "Fetch a Pfam clan (InterPro set CLxxxx) from GET /set/pfam/{accession}/. Members come from metadata.relationships.nodes. member_count is the upstream node count; the members array is a bounded page and has_more is set when it is truncated. HTTP 404 means the clan is unknown.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["clan_accession"],
                    "properties": {
                        "clan_accession": {"type": "string", "minLength": 5, "maxLength": 16, "pattern": "^CL[0-9]{4}$"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_pfam_family_proteins",
                "List UniProt proteins matching a Pfam family via GET /protein/{uniprot|reviewed}/entry/pfam/{accession}/. tax_id restricts to an NCBI taxon. count_only issues page_size=1 and returns InterPro's count without walking members — required for huge families. Otherwise the response is a bounded page (not the complete member set); has_more is true when count exceeds returned.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["pfam_accession"],
                    "properties": {
                        "pfam_accession": {"type": "string", "minLength": 7, "maxLength": 16, "pattern": "^PF[0-9]{5,6}$"},
                        "reviewed_only": {"type": "boolean", "default": false},
                        "tax_id": {"type": "integer", "minimum": 1, "maximum": 999999999},
                        "count_only": {"type": "boolean", "default": false},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_pfam_family_proteomes",
                "Report UniProt proteomes that contain a Pfam family via GET /proteome/uniprot/entry/pfam/{accession}/. count_only (default true) returns InterPro's count from a single page_size=1 request. A full cursor walk is not performed: InterPro's proteome cursor has been observed to drop the final page, and proteome lists are large. Set count_only false only for a bounded first page.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["pfam_accession"],
                    "properties": {
                        "pfam_accession": {"type": "string", "minLength": 7, "maxLength": 16, "pattern": "^PF[0-9]{5,6}$"},
                        "count_only": {"type": "boolean", "default": true},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_protein_atlas_gene",
                "Retrieve a Human Protein Atlas gene record. Ensembl gene IDs (ENSG + 11 digits) are fetched from /{ENSG}.json. Other values are resolved with search_download (columns g,gs,eg) by exact Gene symbol, then Gene synonym; ambiguous symbols fail with the candidate ENSG list. full=false returns a compact identity/expression/localization/antibody/pathology summary; full=true returns HPA's flat record. One gene per call.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["gene"],
                    "properties": {
                        "gene": {"type": "string", "minLength": 1, "maxLength": 64},
                        "full": {"type": "boolean", "default": false}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_string_best_similarity_hits",
                "Map gene symbols to STRING IDs then GET/POST /api/json/homology_best for the best Smith–Waterman hit of each protein in a target species. target_species is an NCBI taxon (species_b); omit it only with a tight max_results cap — STRING returns one best hit per organism and the unfiltered set is large. Bitscore cutoff 50. Unmapped symbols are listed. At most 50 symbols.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["symbols"],
                    "properties": {
                        "symbols": {
                            "type": "array", "minItems": 1, "maxItems": 50,
                            "items": {"type": "string", "minLength": 1, "maxLength": 128}
                        },
                        "species": {"type": "integer", "minimum": 1, "maximum": 999999999, "default": 9606},
                        "target_species": {"type": "integer", "minimum": 1, "maximum": 999999999},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_string_network",
                "Map gene symbols to STRING v12.0 identifiers then retrieve the interaction network via POST /api/json/network. species is always sent (STRING rejects >10 proteins without it). add_nodes=0 so a single protein is not auto-expanded by 10 neighbors. required_score is 0–1000 (400 medium, 700 high). Isolated mapped nodes are kept; unmapped symbols are listed. Edges are a bounded page of unordered pairs with combined score and non-zero evidence channels. At most 50 symbols.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["symbols"],
                    "properties": {
                        "symbols": {
                            "type": "array", "minItems": 1, "maxItems": 50,
                            "items": {"type": "string", "minLength": 1, "maxLength": 128}
                        },
                        "species": {"type": "integer", "minimum": 1, "maximum": 999999999, "default": 9606},
                        "required_score": {"type": "integer", "minimum": 0, "maximum": 1000, "default": 400},
                        "network_type": {"type": "string", "enum": ["functional", "physical"], "default": "functional"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "get_string_similarity_scores",
                "Map gene symbols then retrieve Smith–Waterman bitscores among them via POST /api/json/homology. STRING stores pairs at bitscore ≥ 50 and returns half of the matrix plus self-hits; missing pairs are unreported, not zero. Output pairs are unordered (id_a ≤ id_b). Unmapped symbols are listed. At most 50 symbols.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["symbols"],
                    "properties": {
                        "symbols": {
                            "type": "array", "minItems": 1, "maxItems": 50,
                            "items": {"type": "string", "minLength": 1, "maxLength": 128}
                        },
                        "species": {"type": "integer", "minimum": 1, "maximum": 999999999, "default": 9606},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "map_string_ids",
                "Map gene symbols, synonyms or UniProt accessions to STRING v12.0 protein identifiers via POST /api/json/get_string_ids (limit=1, echo_query=1). mapped and unmapped partition the input: every symbol appears in exactly one list. HTTP 404 (no identifier resolved) is reported as all unmapped, not as empty scientific evidence being omitted. At most 50 symbols. species is an NCBI taxonomy ID (9606 = human).",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["symbols"],
                    "properties": {
                        "symbols": {
                            "type": "array", "minItems": 1, "maxItems": 50,
                            "items": {"type": "string", "minLength": 1, "maxLength": 128}
                        },
                        "species": {"type": "integer", "minimum": 1, "maximum": 999999999, "default": 9606}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "search_interpro_entries",
                "Keyword search of InterPro or a member database via GET /entry/{source_db}/?search=&type=&go_term=. source_db defaults to interpro; pfam searches Pfam families. type is an InterPro entry type. go_term is a GO:####### filter. Supply search and/or go_term. The response is a bounded page: total is InterPro's count, has_more means further cursor pages exist. HTTP 204 is an empty hit list.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 512},
                        "entry_type": {
                            "type": "string",
                            "enum": ["family", "domain", "repeat", "homologous_superfamily", "conserved_site", "active_site", "binding_site", "ptm"]
                        },
                        "source_db": {"type": "string", "minLength": 1, "maxLength": 32, "default": "interpro"},
                        "go_term": {"type": "string", "pattern": "^GO:[0-9]{7}$"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "search_pfam_clans",
                "Keyword search of Pfam clans (InterPro sets) via GET /set/pfam/?search=. HTTP 204 is an empty hit list. The response is a bounded page of clan accessions and names with InterPro's total count and has_more. Without query, the first page of the clan catalogue is returned (not the complete set).",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 512},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                    }
                }),
            ),
        ),
        (
            DOMAIN,
            ToolSchema::new(
                "search_protein_atlas",
                "Column-selected Human Protein Atlas search via GET /api/search_download.php (format=json, compress=no). columns are HPA specifier codes (g=Gene, gs=synonym, eg=Ensembl, gd=description, up=Uniprot, chr, chrp, scl, …). HTTP 400 means a bad query or a result set HPA refuses to materialise. The JSON array is truncated to max_results; a capped page is not the complete hit list.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 512},
                        "columns": {"type": "string", "minLength": 1, "maxLength": 512, "default": "g,gs,eg,gd,up,chr,chrp,scl"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 25}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "get_domain_architecture" => interpro::get_domain_architecture(bio, args).await,
        "get_interpro_entry" => interpro::get_interpro_entry(bio, args).await,
        "get_pfam_clan" => interpro::get_pfam_clan(bio, args).await,
        "get_pfam_family_proteins" => interpro::get_pfam_family_proteins(bio, args).await,
        "get_pfam_family_proteomes" => interpro::get_pfam_family_proteomes(bio, args).await,
        "search_interpro_entries" => interpro::search_interpro_entries(bio, args).await,
        "search_pfam_clans" => interpro::search_pfam_clans(bio, args).await,
        "get_protein_atlas_gene" => protein_atlas::get_protein_atlas_gene(bio, args).await,
        "search_protein_atlas" => protein_atlas::search_protein_atlas(bio, args).await,
        "map_string_ids" => string::map_string_ids(bio, args).await,
        "get_string_network" => string::get_string_network(bio, args).await,
        "get_string_similarity_scores" => string::get_string_similarity_scores(bio, args).await,
        "get_string_best_similarity_hits" => {
            string::get_string_best_similarity_hits(bio, args).await
        }
        _ => bail!("unknown native biological tool: {name}"),
    }
}

enum Fetch {
    Json(Value),
    Empty,
    NotFound,
}

async fn fetch_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
    method: Method,
) -> Result<Fetch> {
    let response = bio.http().send(source, method, url, params).await?;
    match response.status {
        StatusCode::NO_CONTENT => Ok(Fetch::Empty),
        StatusCode::NOT_FOUND => Ok(Fetch::NotFound),
        status if status.is_success() => {
            if looks_like_html(&response.body) {
                bail!("{} returned HTML instead of JSON", source.0);
            }
            let value: Value = serde_json::from_slice(&response.body)
                .with_context(|| format!("{} returned invalid JSON", source.0))?;
            reject_error_payload(source.0, &value)?;
            Ok(Fetch::Json(value))
        }
        status => bail!("{} returned HTTP {}", source.0, status.as_u16()),
    }
}

async fn get_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Fetch> {
    fetch_json(bio, source, url, params, Method::GET).await
}

async fn post_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Fetch> {
    fetch_json(bio, source, url, params, Method::POST).await
}

fn reject_error_payload(source: &str, value: &Value) -> Result<()> {
    if value.get("Error").is_some() || value.get("error").is_some() || value.get("detail").is_some()
    {
        bail!("{source} rejected the request");
    }
    Ok(())
}

fn looks_like_html(body: &[u8]) -> bool {
    let text = std::str::from_utf8(body).unwrap_or("").trim_start();
    let prefix: String = text
        .chars()
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    prefix.starts_with("<!doctype") || prefix.starts_with("<html")
}

fn api_base(bio: &NativeBio, credential: &str, default: &str) -> String {
    bio.credential(credential)
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn interpro_base(bio: &NativeBio) -> String {
    api_base(bio, "INTERPRO_BASE_URL", INTERPRO_API)
}

fn hpa_base(bio: &NativeBio) -> String {
    api_base(bio, "PROTEIN_ATLAS_BASE_URL", HPA_SITE)
}

fn string_base(bio: &NativeBio) -> String {
    api_base(bio, "STRING_BASE_URL", STRING_API)
}

fn bound_page(n: u32) -> Result<usize> {
    if !(1..=MAX_RESULTS).contains(&n) {
        bail!("max_results must be between 1 and {MAX_RESULTS}");
    }
    Ok(n as usize)
}

fn default_max() -> u32 {
    DEFAULT_MAX
}

fn default_true() -> bool {
    true
}

fn default_species() -> i64 {
    9606
}

fn default_score() -> i64 {
    400
}

fn default_architecture_max() -> u32 {
    50
}

fn default_clan_max() -> u32 {
    100
}

fn default_network_max() -> u32 {
    100
}

fn default_columns() -> String {
    "g,gs,eg,gd,up,chr,chrp,scl".into()
}

fn default_network_type() -> String {
    "functional".into()
}

fn require_text(value: &str, what: &str, max: usize) -> Result<String> {
    let text = value.trim();
    if text.is_empty() || text.len() > max {
        bail!("{what} must contain 1 to {max} characters");
    }
    if text.chars().any(|c| c == '\0' || c == '\n' || c == '\r') {
        bail!("{what} must not contain line breaks");
    }
    Ok(text.to_string())
}

fn require_ids(ids: &[String], bound: usize, what: &str) -> Result<Vec<String>> {
    if ids.len() > bound {
        bail!(
            "{} {what}s exceeds the per-call bound of {bound}",
            ids.len()
        );
    }
    let mut cleaned = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        let entry = require_text(id, what, 128)?;
        if entry.contains(',') || entry.contains('/') || entry.contains("..") {
            bail!("{what} {entry:?} must be a single identifier, not a list or path");
        }
        if !seen.insert(entry.clone()) {
            continue;
        }
        cleaned.push(entry);
    }
    if cleaned.is_empty() {
        bail!("provide at least one {what}");
    }
    if cleaned.len() > bound {
        bail!(
            "{} {what}s exceeds the per-call bound of {bound}",
            cleaned.len()
        );
    }
    Ok(cleaned)
}

fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn origin(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http", rest)
    } else {
        return None;
    };
    let hostport = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|host| !host.is_empty() && !host.contains('@'))?;
    Some((scheme.into(), hostport.to_ascii_lowercase()))
}

fn resolve_next(base: &str, next: &Value) -> Result<Option<String>> {
    let Some(url) = next
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if url.starts_with('/') && !url.starts_with("//") {
        let (scheme, host) = origin(base).context("InterPro API base URL is not absolute")?;
        return Ok(Some(format!("{scheme}://{host}{url}")));
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("InterPro next URL is not http(s)");
    }
    if origin(base).zip(origin(url)).is_none_or(|(a, b)| a != b) {
        bail!("InterPro next URL left the API origin; pagination stopped");
    }
    Ok(Some(url.to_string()))
}

fn json_u64(value: Option<&Value>) -> Result<u64> {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|n| u64::try_from(n).ok()))
            .or_else(|| number.as_f64().and_then(|n| (n >= 0.0).then_some(n as u64)))
            .context("count is not a non-negative number"),
        Some(Value::String(text)) => text.parse().context("count is not a non-negative number"),
        Some(Value::Null) | None => Ok(0),
        _ => bail!("count is not a number"),
    }
}

fn json_f64(value: &Value) -> Result<f64> {
    match value {
        Value::Number(number) => number.as_f64().context("score is not numeric"),
        Value::String(text) => text.parse().context("score is not numeric"),
        _ => bail!("score is not numeric"),
    }
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn display_name(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(Value::Object(map)) => map
            .get("name")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

fn metadata<'a>(row: &'a Value) -> &'a Value {
    row.get("metadata").unwrap_or(row)
}

fn page_size_for(max_results: usize) -> u32 {
    u32::try_from(max_results)
        .unwrap_or(MAX_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE)
}

fn taxon_id(value: i64, what: &str) -> Result<i64> {
    if !(1..=999_999_999).contains(&value) {
        bail!("{what} must be an NCBI taxonomy id between 1 and 999999999");
    }
    Ok(value)
}
