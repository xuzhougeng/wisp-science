//! Native `regulation` domain against ENCODE, JASPAR and UniBind.
//! Independently implemented from operator APIs (reviewed 2026-09-06):
//!
//! - [ENCODE REST API](https://www.encodeproject.org/help/rest-api/)
//! - [JASPAR REST API](https://jaspar.elixir.no/api/)
//! - [JASPAR API overview](https://jaspar.elixir.no/api/overview)
//! - [UniBind REST API](https://unibind.uio.no/api/)
//! - [UniBind API overview](https://unibind.uio.no/api/overview)
//! - [UCSC Genome Browser REST API](https://genome.ucsc.edu/goldenPath/help/api.html)
//!
//! Search and list tools return a bounded page plus the upstream total when
//! supplied. A capped page is not the complete hit list. No API keys are
//! published for these hosts. Tests use invented records.

#[cfg(test)]
mod tests;

mod encode;
mod jaspar;
mod unibind;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use wisp_llm::ToolSchema;

const ENCODE_PORTAL: &str = "https://www.encodeproject.org";
const JASPAR_API: &str = "https://jaspar.elixir.no/api/v1";
const UNIBIND_API: &str = "https://unibind.uio.no/api/v1";
const UCSC_API: &str = "https://api.genome.ucsc.edu";

const ENCODE: Source = Source("ENCODE", Duration::from_millis(200));
const JASPAR: Source = Source("JASPAR", Duration::from_millis(200));
const UNIBIND: Source = Source("UniBind", Duration::from_millis(200));
const UCSC: Source = Source("UCSC Genome Browser", Duration::from_millis(1000));

const ENCODE_MAX_ROWS: u32 = 100;
const LIST_MAX_ROWS: u32 = 200;
const DEFAULT_ROWS: u32 = 25;
const MAX_PAGE: u32 = 1000;
const DRF_PAGE_SIZE: u32 = 1000;
const MAX_FILTERS: usize = 8;
const MAX_REGION_SPAN: u64 = 1_000_000;
const MAX_SITES: u32 = 2000;
const DEFAULT_SITES: u32 = 200;
const REGION_SCAN_CAP: u32 = 5000;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "regulation",
            ToolSchema::new(
                "encode_search_experiments",
                "Search ENCODE functional-genomics experiments through the portal JSON search API. Filter by assay_title (for example TF ChIP-seq), target protein label, organism scientific name and release status. Returns a bounded page of experiment summaries plus the portal total; a capped page is not the complete match set. Use encode_get_experiment for one accession.",
                json_object(&[
                    ("assay_title", json_string("ENCODE assay_title, e.g. TF ChIP-seq or ATAC-seq.")),
                    ("target", json_string("Target protein label, e.g. CTCF.")),
                    ("organism", json_string("Scientific name, e.g. Homo sapiens.")),
                    ("status", json!({
                        "type": "string", "minLength": 1, "maxLength": 64, "default": "released"
                    })),
                    ("date_released_before", json_string("ISO date YYYY-MM-DD sent as date_released=lte:DATE.")),
                    ("extra_filters", extra_filters_schema()),
                    ("max_rows", json_max_rows(ENCODE_MAX_ROWS)),
                ]),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "encode_search_biosamples",
                "Search ENCODE biosamples (cell lines, tissues, primary cells) through the portal JSON search API. Filter by ontology term_name, classification, organism and status. Returns a bounded page plus the portal total. Use encode_get_biosample for one accession.",
                json_object(&[
                    ("term_name", json_string("Biosample ontology term, e.g. K562 or liver.")),
                    ("classification", json_string("Biosample classification, e.g. cell line or tissue.")),
                    ("organism", json_string("Scientific name, e.g. Homo sapiens.")),
                    ("status", json!({
                        "type": "string", "minLength": 1, "maxLength": 64, "default": "released"
                    })),
                    ("date_created_before", json_string("ISO date YYYY-MM-DD sent as date_created=lte:DATE.")),
                    ("extra_filters", extra_filters_schema()),
                    ("max_rows", json_max_rows(ENCODE_MAX_ROWS)),
                ]),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "encode_list_files",
                "List ENCODE data files through the portal JSON search API. file_format is the file type (fastq, bam, bigWig, bed). assay_term_name is the ontology term (ChIP-seq, ATAC-seq), not the experiment assay_title. Unfiltered file searches match millions of rows; combine several filters. Returns a bounded page plus the portal total. Use encode_get_file for one accession.",
                json_object(&[
                    ("file_format", json_string("File format, e.g. fastq, bam, bigWig, bed.")),
                    ("assay_term_name", json_string("Assay ontology term, e.g. ChIP-seq. Not assay_title.")),
                    ("biosample_term_name", json_string("Biosample ontology term, e.g. K562.")),
                    ("status", json!({
                        "type": "string", "minLength": 1, "maxLength": 64, "default": "released"
                    })),
                    ("date_created_before", json_string("ISO date YYYY-MM-DD sent as date_created=lte:DATE.")),
                    ("extra_filters", extra_filters_schema()),
                    ("max_rows", json_max_rows(ENCODE_MAX_ROWS)),
                ]),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "encode_get_experiment",
                "Retrieve one ENCODE experiment by accession (ENCSR…). Returns assay, target, biosample, lab, dates, assemblies, replicate counts and the portal URL. Does not download data files.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["accession"],
                    "properties": {
                        "accession": encode_accession_schema("ENCSR")
                    }
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "encode_get_file",
                "Retrieve one ENCODE file record by accession (ENCFF…). Returns format, output type, assembly, dataset, size, checksums and the portal/download URLs. Does not download the file bytes.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["accession"],
                    "properties": {
                        "accession": encode_accession_schema("ENCFF")
                    }
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "encode_get_biosample",
                "Retrieve one ENCODE biosample by accession (ENCBS…). Returns ontology term, classification, organism, donor, lab, summary and the portal URL.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["accession"],
                    "properties": {
                        "accession": encode_accession_schema("ENCBS")
                    }
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "jaspar_get_matrix",
                "Retrieve one JASPAR transcription-factor profile by versioned matrix id (for example MA0002.2), including the position frequency matrix, species, collection and sequence-logo URL. Unversioned base ids are rejected; use jaspar_matrix_versions first.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["matrix_id"],
                    "properties": {
                        "matrix_id": {
                            "type": "string", "minLength": 6, "maxLength": 32,
                            "pattern": "^[A-Za-z]{2}[0-9]{4}\\.[0-9]{1,4}$"
                        }
                    }
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "jaspar_matrix_versions",
                "List released versions of a JASPAR base matrix id (for example MA0002). A versioned id is reduced to its base. Returns each version's matrix_id, name and API URL.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["base_id"],
                    "properties": {
                        "base_id": {
                            "type": "string", "minLength": 6, "maxLength": 32,
                            "pattern": "^[A-Za-z]{2}[0-9]{4}(\\.[0-9]{1,4})?$"
                        }
                    }
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "jaspar_list_matrices",
                "List JASPAR TF binding profiles through GET /api/v1/matrix/. Filter by collection (CORE, UNVALIDATED), tax_group, NCBI tax_id (species= is not a working JASPAR filter), exact name, free-text search, or version=latest. Returns one DRF page (page_size at most 1000) plus the API count; a capped page is not the full catalog.",
                json_object(&[
                    ("collection", json_string("JASPAR collection, e.g. CORE.")),
                    ("tax_group", json_string("Taxonomic group, e.g. vertebrates or plants.")),
                    ("tax_id", json!({
                        "type": "integer", "minimum": 1, "maximum": 99999999,
                        "description": "NCBI taxonomy id, e.g. 9606. Maps to tax_id=."
                    })),
                    ("name", json_string("Exact TF name, e.g. FOXA1.")),
                    ("search", json_string("Free-text search term.")),
                    ("version", json!({
                        "type": "string", "enum": ["latest"],
                        "description": "If latest, restrict to current versions."
                    })),
                    ("page", json_page()),
                    ("max_rows", json_max_rows(LIST_MAX_ROWS)),
                ]),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "jaspar_list_species",
                "List species that have JASPAR profiles (NCBI tax_id and scientific name) from GET /api/v1/species/. Use tax_id values with jaspar_list_matrices.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {}
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "jaspar_list_taxa",
                "List JASPAR taxonomic groups from GET /api/v1/taxon/ (vertebrates, plants, insects, nematodes, fungi, urochordates). Use the names as tax_group on jaspar_list_matrices.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {}
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "jaspar_list_collections",
                "List JASPAR collections from GET /api/v1/collections/ (CORE is the curated non-redundant set). Use the names as collection on jaspar_list_matrices.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {}
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "jaspar_list_releases",
                "List JASPAR database releases from GET /api/v1/releases/ (year, release number, active flag). Record the active release when selecting motifs for reproducibility.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {}
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "unibind_search_tfbs",
                "Search UniBind ChIP-seq datasets with direct TF–DNA predictions through GET /api/v1/datasets/. Filters (AND): tf_name, cell_line, species, collection (Robust or Permissive), jaspar_id, free-text search. Returns one DRF page plus the API count; total_peaks is the ChIP-seq peak count, not the TFBS count. Use unibind_get_dataset for per-model TFBS files.",
                json_object(&[
                    ("tf_name", json_string("TF gene symbol as used by UniBind, e.g. CTCF.")),
                    ("cell_line", json_string("UniBind cell or tissue title.")),
                    ("species", json_string("Scientific name, e.g. Homo sapiens.")),
                    ("collection", json!({
                        "type": "string", "enum": ["Robust", "Permissive"]
                    })),
                    ("jaspar_id", json_string("Versioned JASPAR matrix id, e.g. MA0139.1.")),
                    ("search", json_string("Free-text search across dataset fields.")),
                    ("page", json_page()),
                    ("max_rows", json_max_rows(LIST_MAX_ROWS)),
                ]),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "unibind_get_dataset",
                "Retrieve one UniBind dataset by tf_id (identifier.cell_line.TF, as returned by unibind_search_tfbs). Returns TF name, source identifiers, cell types, JASPAR matrix ids, peak count and per-model TFBS counts with BED/FASTA URLs. Does not download site lists.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["tf_id"],
                    "properties": {
                        "tf_id": {
                            "type": "string", "minLength": 3, "maxLength": 256
                        }
                    }
                }),
            ),
        ),
        (
            "regulation",
            ToolSchema::new(
                "unibind_tfbs_in_region",
                "Return UniBind 2021 TF binding sites overlapping a genomic interval via the UCSC Genome Browser hubApi (GET /getData/track) against UniBind's public Robust or Permissive track hubs. UniBind's own REST API has no region endpoint. Coordinates are UCSC 0-based start / half-open end. The scan is capped; region_scan_complete is false when UCSC set maxItemsLimit. No hg19 hub — lift first.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["genome", "chrom", "start", "end"],
                    "properties": {
                        "genome": {
                            "type": "string",
                            "enum": ["hg38", "mm10", "ce11", "dm6", "danRer11", "sacCer3", "rn6", "araTha1", "spo2"]
                        },
                        "chrom": {
                            "type": "string", "minLength": 1, "maxLength": 64,
                            "pattern": "^[A-Za-z0-9_.-]+$"
                        },
                        "start": {"type": "integer", "minimum": 0, "maximum": 3000000000i64},
                        "end": {"type": "integer", "minimum": 1, "maximum": 3000000000i64},
                        "tf_name": json_string("Optional TF symbol filter applied after the region scan."),
                        "collection": {
                            "type": "string", "enum": ["Robust", "Permissive"], "default": "Robust"
                        },
                        "max_sites": {
                            "type": "integer", "minimum": 1, "maximum": 2000, "default": 200
                        }
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "encode_search_experiments" => encode::search_experiments(bio, args).await,
        "encode_search_biosamples" => encode::search_biosamples(bio, args).await,
        "encode_list_files" => encode::list_files(bio, args).await,
        "encode_get_experiment" => encode::get_experiment(bio, args).await,
        "encode_get_file" => encode::get_file(bio, args).await,
        "encode_get_biosample" => encode::get_biosample(bio, args).await,
        "jaspar_get_matrix" => jaspar::get_matrix(bio, args).await,
        "jaspar_matrix_versions" => jaspar::matrix_versions(bio, args).await,
        "jaspar_list_matrices" => jaspar::list_matrices(bio, args).await,
        "jaspar_list_species" => jaspar::list_species(bio, args).await,
        "jaspar_list_taxa" => jaspar::list_taxa(bio, args).await,
        "jaspar_list_collections" => jaspar::list_collections(bio, args).await,
        "jaspar_list_releases" => jaspar::list_releases(bio, args).await,
        "unibind_search_tfbs" => unibind::search_tfbs(bio, args).await,
        "unibind_get_dataset" => unibind::get_dataset(bio, args).await,
        "unibind_tfbs_in_region" => unibind::tfbs_in_region(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn json_object(properties: &[(&str, Value)]) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in properties {
        map.insert((*key).into(), value.clone());
    }
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": map
    })
}

fn json_string(description: &str) -> Value {
    serde_json::json!({
        "type": "string", "minLength": 1, "maxLength": 256,
        "description": description
    })
}

fn json_max_rows(max: u32) -> Value {
    serde_json::json!({
        "type": "integer", "minimum": 1, "maximum": max, "default": DEFAULT_ROWS
    })
}

fn json_page() -> Value {
    serde_json::json!({
        "type": "integer", "minimum": 1, "maximum": MAX_PAGE, "default": 1
    })
}

fn extra_filters_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": {"type": "string", "minLength": 1, "maxLength": 256},
        "description": "Additional ENCODE portal field filters (property=value). At most 8 keys. Cannot set type, format, limit, from, frame or field."
    })
}

fn encode_accession_schema(prefix: &str) -> Value {
    serde_json::json!({
        "type": "string", "minLength": 8, "maxLength": 32,
        "pattern": format!("^{prefix}[A-Z0-9]+$"),
        "description": format!("ENCODE accession starting with {prefix}.")
    })
}

fn default_rows() -> u32 {
    DEFAULT_ROWS
}

fn default_page() -> u32 {
    1
}

fn default_status() -> String {
    "released".into()
}

fn default_sites() -> u32 {
    DEFAULT_SITES
}

fn default_robust() -> String {
    "Robust".into()
}

fn encode_base(bio: &NativeBio) -> String {
    credential_base(bio, "ENCODE_BASE_URL", ENCODE_PORTAL)
}

fn jaspar_base(bio: &NativeBio) -> String {
    credential_base(bio, "JASPAR_BASE_URL", JASPAR_API)
}

fn unibind_base(bio: &NativeBio) -> String {
    credential_base(bio, "UNIBIND_BASE_URL", UNIBIND_API)
}

fn ucsc_base(bio: &NativeBio) -> String {
    credential_base(bio, "UCSC_BASE_URL", UCSC_API)
}

fn credential_base(bio: &NativeBio, name: &str, fallback: &str) -> String {
    bio.credential(name)
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.trim_end_matches('/').to_string())
}

fn join_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

async fn get_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<(StatusCode, Option<Value>)> {
    let response = bio.http().send(source, Method::GET, url, params).await?;
    if !response.status.is_success() {
        return Ok((response.status, None));
    }
    if looks_like_html(&response.body) {
        bail!("{} returned HTML instead of JSON", source.0);
    }
    let value = serde_json::from_slice(&response.body)
        .with_context(|| format!("{} returned invalid JSON", source.0))?;
    Ok((response.status, Some(value)))
}

async fn get_json_ok(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    let (status, body) = get_json(bio, source, url, params).await?;
    if status == StatusCode::NOT_FOUND {
        bail!("{} returned HTTP 404", source.0);
    }
    match body {
        Some(value) => Ok(value),
        None => bail!("{} returned HTTP {}", source.0, status.as_u16()),
    }
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

fn bound_rows(n: u32, max: u32) -> Result<u32> {
    if !(1..=max).contains(&n) {
        bail!("max_rows must be between 1 and {max}");
    }
    Ok(n)
}

fn bound_page(n: u32) -> Result<u32> {
    if !(1..=MAX_PAGE).contains(&n) {
        bail!("page must be between 1 and {MAX_PAGE}");
    }
    Ok(n)
}

fn query_text(value: &str, max: usize, name: &str) -> Result<String> {
    let text = value.trim();
    if text.is_empty() || text.len() > max {
        bail!("{name} must contain 1 to {max} characters");
    }
    if text.chars().any(|c| c.is_control()) {
        bail!("{name} contains control characters");
    }
    Ok(text.to_string())
}

fn optional_query(value: &Option<String>, max: usize, name: &str) -> Result<Option<String>> {
    match value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        Some(text) => Ok(Some(query_text(text, max, name)?)),
        None => Ok(None),
    }
}

fn iso_date(value: &str, name: &str) -> Result<String> {
    let text = value.trim();
    let date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .filter(|_| text.len() == 10)
        .with_context(|| format!("{name} must be an ISO date YYYY-MM-DD"))?;
    Ok(date.format("%Y-%m-%d").to_string())
}

fn extra_filters(extra: Option<BTreeMap<String, String>>) -> Result<BTreeMap<String, String>> {
    let Some(extra) = extra else {
        return Ok(BTreeMap::new());
    };
    if extra.len() > MAX_FILTERS {
        bail!("extra_filters supports at most {MAX_FILTERS} portal field filters");
    }
    let reserved = [
        "type",
        "format",
        "limit",
        "from",
        "frame",
        "field",
        "mode",
        "datastore",
    ];
    let mut out = BTreeMap::new();
    for (key, value) in extra {
        let key = key.trim();
        if key.is_empty() || key.len() > 80 {
            bail!("extra_filters keys must be 1–80 characters");
        }
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
        {
            bail!("extra_filters key {key:?} is not an ENCODE field path");
        }
        if reserved.iter().any(|item| key.eq_ignore_ascii_case(item)) {
            bail!("extra_filters cannot set {key}");
        }
        out.insert(
            key.to_string(),
            query_text(&value, 256, "extra_filters value")?,
        );
    }
    Ok(out)
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

fn nested_name(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Object(map) => [
            "title",
            "label",
            "term_name",
            "name",
            "scientific_name",
            "@id",
        ]
        .iter()
        .find_map(|key| map.get(*key).and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .map(str::to_string),
        _ => None,
    }
}

fn as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn drf_page(payload: &Value) -> Result<(u64, Vec<Value>, Option<String>)> {
    let count = payload
        .get("count")
        .and_then(Value::as_u64)
        .context("upstream list response omitted count")?;
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .context("upstream list response omitted results")?;
    let next = match payload.get("next") {
        None | Some(Value::Null) => None,
        Some(Value::String(url)) if !url.is_empty() => Some(url.clone()),
        Some(_) => bail!("upstream next page URL is not a string"),
    };
    Ok((count, results, next))
}

fn next_on_base(next: Option<String>, base: &str) -> Result<Option<String>> {
    let Some(next) = next else {
        return Ok(None);
    };
    let prefix = format!("{}/", base.trim_end_matches('/'));
    if next == base || next.starts_with(&prefix) || next.starts_with(&format!("{base}?")) {
        Ok(Some(next))
    } else {
        bail!("upstream next page URL is not on the same API host")
    }
}

async fn drf_catalog(
    bio: &NativeBio,
    source: Source,
    base: &str,
    path: &str,
) -> Result<(u64, Vec<Value>, bool)> {
    let url = join_url(base, path);
    let params = vec![
        ("format".into(), "json".into()),
        ("page".into(), "1".into()),
        ("page_size".into(), DRF_PAGE_SIZE.to_string()),
    ];
    let payload = get_json_ok(bio, source, &url, &params).await?;
    let (count, results, next) = drf_page(&payload)?;
    let next = next_on_base(next, base)?;
    if next.is_none() && results.len() as u64 != count {
        bail!(
            "{} catalog {path} returned {} rows but count={count}",
            source.0,
            results.len()
        );
    }
    Ok((count, results, next.is_some()))
}
