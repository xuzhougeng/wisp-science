//! Native `omics-archives` domain against GEO, ArrayExpress (BioStudies),
//! MetaboLights, MGnify and PRIDE Archive. Independently implemented from:
//!
//! - [NCBI GEO programmatic access](https://www.ncbi.nlm.nih.gov/geo/info/geo_paccess.html)
//! - [NCBI E-utilities](https://www.ncbi.nlm.nih.gov/books/NBK25497/)
//! - [GEO SOFT](https://www.ncbi.nlm.nih.gov/geo/info/soft.html)
//! - [BioStudies API](https://www.ebi.ac.uk/biostudies/help)
//! - [ArrayExpress in BioStudies](https://www.ebi.ac.uk/biostudies/arrayexpress/help)
//! - [MetaboLights REST](https://www.ebi.ac.uk/metabolights/ws/api/spec.html)
//! - [MGnify API v2](https://docs.mgnify.org/src/docs/api.html)
//! - [PRIDE Archive API](https://www.ebi.ac.uk/pride/markdownpage/prideapi)
//!
//! References reviewed 2026-09-06. Tests use invented records.

mod arrayexpress;
mod geo;
mod metabolights;
mod mgnify;
mod pride;
#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const DOMAIN: &str = "omics-archives";
const NCBI_EUTILS: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils";
const GEO_ACC: &str = "https://www.ncbi.nlm.nih.gov/geo/query/acc.cgi";
const BIOSTUDIES: &str = "https://www.ebi.ac.uk/biostudies/api/v1";
const BIOSTUDIES_FILES: &str = "https://www.ebi.ac.uk/biostudies/files";
const METABOLIGHTS: &str = "https://www.ebi.ac.uk/metabolights/ws";
const MGNIFY: &str = "https://www.ebi.ac.uk/metagenomics/api/v2";
const PRIDE: &str = "https://www.ebi.ac.uk/pride/ws/archive/v3";

const GEO_SOFT: Source = Source("NCBI GEO", Duration::from_millis(350));
const ARRAYEXPRESS: Source = Source("ArrayExpress", Duration::from_millis(500));
const MTBLS: Source = Source("MetaboLights", Duration::from_millis(500));
const MGNIFY_SRC: Source = Source("MGnify", Duration::from_millis(500));
const PRIDE_SRC: Source = Source("PRIDE", Duration::from_millis(500));

const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 200;
const DEFAULT_ROWS: u32 = 200;
const MAX_ROWS: u32 = 500;
const MAX_IDS: usize = 20;
const MAX_GEO_SERIES: usize = 10;
const MAX_QUERY: usize = 8192;
const PAGE_SIZE: u32 = 100;

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    (DOMAIN, ToolSchema::new(name, description, parameters))
}

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "arrayexpress_get_experiment",
            "Retrieve one ArrayExpress functional-genomics experiment from the EMBL-EBI BioStudies JSON API (GET /studies/{accession}). Returns a flattened record: title, organisms, study type, sample and assay counts, experimental factors, authors, publications, protocol types, file summary and a BioStudies source URL. Accessions look like E-MTAB-5061. A missing accession is an error, not empty evidence.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accession"],
                "properties": {
                    "accession": {"type": "string", "minLength": 3, "maxLength": 32,
                        "pattern": "^[Ee]-[A-Za-z]{4}-[0-9]+$"}
                }
            }),
        ),
        tool(
            "arrayexpress_get_experiment_files",
            "List files declared on an ArrayExpress BioStudies submission (GET /studies/{accession}) together with the /info FTP and HTTP base links when present. Each file includes path, size, type/format attributes and a BioStudies files URL. Does not download file contents. File inventories can be large; the response is metadata only.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accession"],
                "properties": {
                    "accession": {"type": "string", "minLength": 3, "maxLength": 32,
                        "pattern": "^[Ee]-[A-Za-z]{4}-[0-9]+$"}
                }
            }),
        ),
        tool(
            "arrayexpress_get_experiment_samples",
            "Parse the MAGE-TAB SDRF of an ArrayExpress experiment into per-sample rows (characteristics, factor values, assay/data-file columns). Experiments without an SDRF (some sequencing submissions keep samples in ENA only) return no_sdrf rather than an empty sample list. n_samples is the parsed total; the row list is capped by max_rows_returned.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accession"],
                "properties": {
                    "accession": {"type": "string", "minLength": 3, "maxLength": 32,
                        "pattern": "^[Ee]-[A-Za-z]{4}-[0-9]+$"},
                    "max_rows_returned": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "arrayexpress_search_experiments",
            "Search the ArrayExpress collection through BioStudies GET /arrayexpress/search. Filters combine: Lucene query, organism, study_type, technology, inclusive release-date range (YYYY-MM-DD) and extra facet.<name> values. Provide a query or at least one filter. Results are a bounded page ordered by release date; total_hits is the API total when supplied. A capped page is not the complete hit list.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 8192},
                    "organism": {"type": "string", "minLength": 1, "maxLength": 256},
                    "study_type": {"type": "string", "minLength": 1, "maxLength": 256},
                    "technology": {"type": "string", "minLength": 1, "maxLength": 256},
                    "released_after": {"type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"},
                    "released_before": {"type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$"},
                    "extra_facets": {"type": "object", "additionalProperties": {
                        "type": "string", "minLength": 1, "maxLength": 256}},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "geo_get_series",
            "Fetch NCBI GEO series (GSE) metadata. Resolves accessions with E-utilities db=gds (ESearch+ESummary) and reads SOFT brief headers from acc.cgi (targ=self/gsm, view=brief) so data tables and platform listings are not downloaded. Returns design, platforms, sample characteristics and GEO source URLs. Missing GSE accessions are listed individually. At most 10 series per call.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accessions"],
                "properties": {
                    "accessions": {
                        "type": "array", "minItems": 1, "maxItems": 10,
                        "items": {"type": "string", "minLength": 4, "maxLength": 16,
                            "pattern": "^[Gg][Ss][Ee][0-9]+$"}
                    }
                }
            }),
        ),
        tool(
            "geo_search_series",
            "Search NCBI GEO DataSets (ESearch db=gds) and return a bounded ESummary page. term is Entrez syntax; add gse[ETYP] to restrict to series. total is the ESearch count and may exceed the returned page (NCBI retrieves only the first 10,000 matches). Does not download SOFT tables. Use geo_get_series for per-sample characteristics.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["term"],
                "properties": {
                    "term": {"type": "string", "minLength": 1, "maxLength": 8192},
                    "retmax": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "metabolights_get_studies",
            "Fetch public MetaboLights studies (MTBLSnnn) from GET /studies/public/study/{accession} and project the ISA payload into title, description, organisms, assays (measurement/technology/platform), factors, descriptors, protocols and sample count. Unknown or private accessions are listed in not_found. Optional sample-table rows are capped; n_rows_total is the parsed size. At most 20 accessions per call.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accessions"],
                "properties": {
                    "accessions": {
                        "type": "array", "minItems": 1, "maxItems": 20,
                        "items": {"type": "string", "minLength": 6, "maxLength": 16,
                            "pattern": "^[Mm][Tt][Bb][Ll][Ss][0-9]+$"}
                    },
                    "include_samples": {"type": "boolean", "default": false},
                    "max_sample_rows_returned": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "metabolights_get_study_files",
            "List files for a public MetaboLights study. GET /studies/{id}/files returns the study folder (ISA-Tab i_/s_/a_*.txt, MAF tables, directory entries). When include_data_files is true, GET /studies/{id}/public-data-files lists the FILES tree. Volatile timestamps are dropped. Does not download raw instrument files.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accession"],
                "properties": {
                    "accession": {"type": "string", "minLength": 6, "maxLength": 16,
                        "pattern": "^[Mm][Tt][Bb][Ll][Ss][0-9]+$"},
                    "include_data_files": {"type": "boolean", "default": true}
                }
            }),
        ),
        tool(
            "metabolights_list_studies",
            "List public MetaboLights study accessions from GET /studies. Returns a bounded accession page plus the API's reported study count when present. There is no server-side topic search on this endpoint (a query parameter is not interpreted as a filter); inspect titles with metabolights_get_studies.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "max_returned": {"type": "integer", "minimum": 1, "maximum": 2000, "default": 200}
                }
            }),
        ),
        tool(
            "metabolights_search_data_files",
            "Search a public MetaboLights study's FILES tree through GET /studies/{id}/public-data-files. pattern is a filename glob (for example *.mzML or *.raw); omit it to list data files. Results are relative paths under the study folder, sorted, and capped. Does not download file contents.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accession"],
                "properties": {
                    "accession": {"type": "string", "minLength": 6, "maxLength": 16,
                        "pattern": "^[Mm][Tt][Bb][Ll][Ss][0-9]+$"},
                    "pattern": {"type": "string", "minLength": 1, "maxLength": 256}
                }
            }),
        ),
        tool(
            "mgnify_get_studies",
            "Retrieve MGnify metagenomics studies (MGYSnnnnnnnn) from GET /metagenomics/api/v2/studies/{accession}. Returns name, abstract, biome lineages, sample count, centre, ENA/BioProject accessions and a MGnify source URL. include_analyses adds a bounded analyses page per study with pipeline/experiment-type counts. Missing accessions are listed. Uses API v2 (v1 is deprecated).",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accessions"],
                "properties": {
                    "accessions": {
                        "type": "array", "minItems": 1, "maxItems": 20,
                        "items": {"type": "string", "minLength": 8, "maxLength": 24,
                            "pattern": "^[Mm][Gg][Yy][Ss][0-9]+$"}
                    },
                    "include_analyses": {"type": "boolean", "default": false}
                }
            }),
        ),
        tool(
            "mgnify_get_study_analyses",
            "List analyses for one MGnify study from GET /metagenomics/api/v2/studies/{accession}/analyses. Returns a bounded page of MGYA records (pipeline version, experiment type, status, run/assembly/sample accessions) plus the API count when supplied. A capped page is not the complete analysis set.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accession"],
                "properties": {
                    "accession": {"type": "string", "minLength": 8, "maxLength": 24,
                        "pattern": "^[Mm][Gg][Yy][Ss][0-9]+$"},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "mgnify_search_studies",
            "Search MGnify studies through API v2. Provide exactly one of: query (GET /studies?search=) or biome_lineage (GET /biomes/{lineage}/studies, GOLD-style lineage such as root:Host-associated:Human). Results are a bounded page of MGYS records; count is the API total when supplied. A capped page is not the complete listing.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 512},
                    "biome_lineage": {"type": "string", "minLength": 1, "maxLength": 512},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "pride_find_projects_for_protein",
            "Find PRIDE Archive projects whose identifications include a protein (typically a UniProt accession) via GET /proteins/search. Returns project accession lists and PRIDE source URLs. This is the protein-to-project direction for mass-spec (PXD) submissions; per-project protein tables are served mainly for affinity-proteomics (PAD) projects.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["protein_accession"],
                "properties": {
                    "protein_accession": {"type": "string", "minLength": 3, "maxLength": 32,
                        "pattern": "^[A-Za-z0-9_\\.-]+$"}
                }
            }),
        ),
        tool(
            "pride_get_projects",
            "Fetch PRIDE Archive project metadata (PXD, PAD or PRD accessions) from GET /projects/{accession}: title, description, organisms, instruments, experiment types, quantification methods, dates, submitters, lab heads and literature references (PubMed/DOI). Missing accessions are listed in not_found. At most 20 accessions per call.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accessions"],
                "properties": {
                    "accessions": {
                        "type": "array", "minItems": 1, "maxItems": 20,
                        "items": {"type": "string", "minLength": 4, "maxLength": 32,
                            "pattern": "^[Pp]([Xx][Dd]|[Aa][Dd]|[Rr][Dd])[0-9]+$"}
                    }
                }
            }),
        ),
        tool(
            "pride_search_project_proteins",
            "List protein evidence rows for one PRIDE affinity-proteomics project from GET /pride-ap/search/proteins. Optional keyword filters by protein accession, gene symbol or name. Classic PXD mass-spec projects typically return an empty table (identifications live in submitted mzIdentML/mzTab files); use pride_find_projects_for_protein for that direction. The row list is capped.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["project_accession"],
                "properties": {
                    "project_accession": {"type": "string", "minLength": 4, "maxLength": 32,
                        "pattern": "^[Pp]([Xx][Dd]|[Aa][Dd]|[Rr][Dd])[0-9]+$"},
                    "keyword": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "pride_search_projects",
            "Search PRIDE Archive proteomics projects via GET /search/projects. Filters combine (AND): keyword plus exact facet strings for organism, instrument and disease, and extra field==value pairs (for example experimentTypes). Provide a keyword or at least one filter. Results are a bounded page; total is reported when the JSON payload includes it. A capped page is not the complete index.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "keyword": {"type": "string", "minLength": 1, "maxLength": 512},
                    "organism": {"type": "string", "minLength": 1, "maxLength": 256},
                    "instrument": {"type": "string", "minLength": 1, "maxLength": 256},
                    "disease": {"type": "string", "minLength": 1, "maxLength": 256},
                    "extra_filters": {"type": "object", "additionalProperties": {
                        "type": "string", "minLength": 1, "maxLength": 256}},
                    "max_records_returned": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "arrayexpress_get_experiment" => arrayexpress::get_experiment(bio, args).await,
        "arrayexpress_get_experiment_files" => arrayexpress::get_experiment_files(bio, args).await,
        "arrayexpress_get_experiment_samples" => {
            arrayexpress::get_experiment_samples(bio, args).await
        }
        "arrayexpress_search_experiments" => arrayexpress::search_experiments(bio, args).await,
        "geo_get_series" => geo::get_series(bio, args).await,
        "geo_search_series" => geo::search_series(bio, args).await,
        "metabolights_get_studies" => metabolights::get_studies(bio, args).await,
        "metabolights_get_study_files" => metabolights::get_study_files(bio, args).await,
        "metabolights_list_studies" => metabolights::list_studies(bio, args).await,
        "metabolights_search_data_files" => metabolights::search_data_files(bio, args).await,
        "mgnify_get_studies" => mgnify::get_studies(bio, args).await,
        "mgnify_get_study_analyses" => mgnify::get_study_analyses(bio, args).await,
        "mgnify_search_studies" => mgnify::search_studies(bio, args).await,
        "pride_find_projects_for_protein" => pride::find_projects_for_protein(bio, args).await,
        "pride_get_projects" => pride::get_projects(bio, args).await,
        "pride_search_project_proteins" => pride::search_project_proteins(bio, args).await,
        "pride_search_projects" => pride::search_projects(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn default_page() -> u32 {
    DEFAULT_PAGE
}

fn default_rows() -> u32 {
    DEFAULT_ROWS
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_list() -> u32 {
    200
}

fn bound_page(n: u32) -> Result<usize> {
    if !(1..=MAX_PAGE).contains(&n) {
        bail!("max_records must be between 1 and {MAX_PAGE}");
    }
    Ok(n as usize)
}

fn bound_rows(n: u32) -> Result<usize> {
    if !(1..=MAX_ROWS).contains(&n) {
        bail!("row limit must be between 1 and {MAX_ROWS}");
    }
    Ok(n as usize)
}

fn require_text<'a>(value: &'a str, what: &str, max: usize) -> Result<&'a str> {
    let text = value.trim();
    if text.is_empty() || text.len() > max {
        bail!("{what} must contain 1 to {max} characters");
    }
    Ok(text)
}

fn iso_date(value: &str) -> Result<&str> {
    let parts: Vec<_> = value.split('-').collect();
    if parts.len() != 3
        || parts[0].len() != 4
        || parts[1].len() != 2
        || parts[2].len() != 2
        || parts
            .iter()
            .any(|part| !part.bytes().all(|b| b.is_ascii_digit()))
    {
        bail!("dates must be YYYY-MM-DD");
    }
    let year = parts[0].parse().unwrap_or(0);
    let month = parts[1].parse().unwrap_or(0);
    let day = parts[2].parse().unwrap_or(0);
    chrono::NaiveDate::from_ymd_opt(year, month, day).context("dates must be YYYY-MM-DD")?;
    Ok(value)
}

fn unique_ids(ids: &[String], bound: usize, what: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        let entry = id.trim();
        if entry.is_empty() {
            continue;
        }
        if entry.chars().any(|c| c == ',' || c.is_whitespace()) {
            bail!(
                "{what} {entry:?} contains a comma or whitespace; pass each identifier as its own list item (at most {bound} per call)"
            );
        }
        if entry.len() > 64 {
            bail!("{what} exceeds 64 characters");
        }
        let key = entry.to_ascii_uppercase();
        if seen.insert(key.clone()) {
            out.push(key);
        }
    }
    if out.is_empty() {
        bail!("provide at least one {what}");
    }
    if out.len() > bound {
        bail!(
            "{} {what}s exceeds the per-call bound of {bound}",
            out.len()
        );
    }
    Ok(out)
}

fn matches_prefix_digits(value: &str, prefix: &str) -> bool {
    let rest = match value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .and_then(|_| value.get(prefix.len()..))
    {
        Some(rest) => rest,
        None => return false,
    };
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
}

fn path_seg(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn api_base(bio: &NativeBio, credential: &str, default: &str) -> String {
    bio.credential(credential)
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.trim_end_matches('/').to_string())
}

fn ncbi_identity(bio: &NativeBio) -> Vec<(String, String)> {
    let mut params = vec![("tool".into(), "wisp-science".into())];
    if let Some(key) = bio.credential("NCBI_API_KEY") {
        params.push(("api_key".into(), key.to_string()));
    }
    if let Some(email) = bio.credential("NCBI_EMAIL") {
        params.push(("email".into(), email.to_string()));
    }
    params
}

async fn send(
    bio: &NativeBio,
    source: Source,
    method: Method,
    url: &str,
    params: &[(String, String)],
) -> Result<crate::http::Response> {
    bio.http().send(source, method, url, params).await
}

async fn get_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    let value = send(bio, source, Method::GET, url, params).await?.json()?;
    reject_error_payload(source.0, &value)?;
    Ok(value)
}

async fn get_text(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<String> {
    send(bio, source, Method::GET, url, params).await?.text()
}

async fn post_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    let value = send(bio, source, Method::POST, url, params).await?.json()?;
    reject_error_payload(source.0, &value)?;
    Ok(value)
}

fn reject_error_payload(source: &str, value: &Value) -> Result<()> {
    if value.get("error").is_some() || value.get("ERROR").is_some() {
        bail!("{source} rejected the request");
    }
    Ok(())
}

fn missing_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403 | 404)
}

fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(|n| n as u64)
        }),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "true" | "yes" | "y" | "1" => Some(true),
            "false" | "no" | "n" | "0" => Some(false),
            _ => None,
        },
        Value::Number(number) if number.as_u64() == Some(1) => Some(true),
        Value::Number(number) if number.as_u64() == Some(0) => Some(false),
        _ => None,
    }
}

fn field<'a>(record: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| record.get(*key))
}

fn field_text(record: &Value, keys: &[&str]) -> Option<String> {
    field(record, keys).and_then(as_text)
}

fn field_u64(record: &Value, keys: &[&str]) -> Option<u64> {
    field(record, keys).and_then(as_u64)
}

fn names(value: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(name) = as_text(item).or_else(|| field_text(item, &["name", "value"])) {
                    let name = name.trim();
                    if !name.is_empty() {
                        out.push(name.to_string());
                    }
                }
            }
        }
        Some(Value::String(text)) => {
            let text = text.trim();
            if !text.is_empty() {
                out.push(text.to_string());
            }
        }
        _ => {}
    }
    out.sort();
    out.dedup();
    out
}

fn looks_like_html(text: &str) -> bool {
    let prefix: String = text
        .trim_start()
        .chars()
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    prefix.starts_with("<!doctype") || prefix.starts_with("<html")
}

fn glob_match(pattern: &str, name: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), name.as_bytes())
}

fn glob_match_bytes(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|i| glob_match_bytes(rest, &name[i..])),
        Some((b'?', rest)) => !name.is_empty() && glob_match_bytes(rest, &name[1..]),
        Some((byte, rest)) => name.first() == Some(byte) && glob_match_bytes(rest, &name[1..]),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
