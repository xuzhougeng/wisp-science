//! Native `cellguide` domain against CZ CELLxGENE CellGuide snapshot JSON.
//! Independently implemented from:
//!
//! - [CELLxGENE CellGuide](https://cellxgene.cziscience.com/cellguide)
//! - [CellGuide OpenAPI](https://github.com/chanzuckerberg/single-cell-data-portal/blob/main/backend/cellguide/api/cellguide-api.yml)
//! - [CellGuide pipeline constants](https://github.com/chanzuckerberg/single-cell-data-portal/blob/main/backend/cellguide/common/constants.py)
//! - [Find Marker Genes](https://cellxgene.cziscience.com/docs/04__Analyze%20Public%20Data/4_2__Gene%20Expression%20Documentation/4_2_5__Find%20Marker%20Genes)
//!
//! References reviewed 2026-09-06. CellGuide is a versioned static JSON tree
//! at `https://cellguide.cellxgene.cziscience.com`, not the curator REST API
//! (`POST /v1/upload` on `api.cellxgene.cziscience.com`). `GET
//! /latest_snapshot_identifier` returns a plaintext snapshot id. Catalogs live
//! at `{snapshot}/celltype_metadata.json` and `{snapshot}/tissue_metadata.json`.
//! Per-cell files use filesystem ids (`CL_0000000`, colon → underscore).
//! Computational markers are the Welch t-test scores documented for Gene
//! Expression; canonical markers are ASCTB literature tables. Descriptions are
//! unversioned: curator `validated_descriptions/` first, then
//! `gpt_descriptions/`. Human tissue mapping is
//! `{snapshot}/ontology_tree/NCBITaxon_9606/celltype_to_tissue_mapping.json`.
//! No API key is published. Tests use invented records.

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const CELLGUIDE_CDN: &str = "https://cellguide.cellxgene.cziscience.com";
const CELLGUIDE_UI: &str = "https://cellxgene.cziscience.com/cellguide";
const HUMAN_TAXONOMY: &str = "NCBITaxon_9606";
const CELLGUIDE: Source = Source("CELLxGENE CellGuide", Duration::from_millis(200));
const MAX_QUERY: usize = 256;
const DEFAULT_SEARCH: u32 = 10;
const MAX_SEARCH: u32 = 50;
const DEFAULT_MARKERS: u32 = 20;
const MAX_MARKERS: u32 = 100;
const MAX_TISSUES: usize = 200;
const MAX_COLLECTIONS: usize = 100;
const MAX_SYNONYMS: usize = 50;
const MAX_REFERENCES: usize = 25;
const MAX_DESCRIPTION: usize = 32 * 1024;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "cellguide",
            ToolSchema::new(
                "search_cell_types",
                "Search CZ CELLxGENE CellGuide cell types in the current snapshot's celltype_metadata.json. Matching is a case-insensitive substring rank over names, then synonyms (exact, prefix, contains). A Cell Ontology identifier (CL:0000000 / CL_0000000) matches on id. Returns a bounded page; total_available is the untruncated match count. A capped page is not the complete catalog.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 256,
                            "description": "Cell type name, synonym, or Cell Ontology identifier."},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}
                    }
                }),
            ),
        ),
        (
            "cellguide",
            ToolSchema::new(
                "get_cell_type_info",
                "Retrieve Cell Ontology metadata and the CellGuide narrative for one cell type. Accepts a Cell Ontology ID (CL:0000000 or CL_0000000) or a name/synonym resolved against celltype_metadata.json. Prefers curator-validated descriptions over GPT drafts. A missing description is reported as description_source=none; an unknown cell type is an error. Includes the public CellGuide card URL.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["cell_type"],
                    "properties": {
                        "cell_type": {"type": "string", "minLength": 1, "maxLength": 256,
                            "description": "Cell Ontology ID or cell type name/synonym."}
                    }
                }),
            ),
        ),
        (
            "cellguide",
            ToolSchema::new(
                "get_marker_genes",
                "List marker genes for a CellGuide cell type from the current snapshot. computational uses Welch t-test marker scores under computational_marker_genes/; canonical uses ASCTB literature markers under canonical_marker_genes/. Computational rows are sorted by marker_score descending. The response is a bounded page (at most 100). A missing marker file means no markers in this snapshot, not an unknown cell type.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["cell_type"],
                    "properties": {
                        "cell_type": {"type": "string", "minLength": 1, "maxLength": 256},
                        "marker_type": {"type": "string", "enum": ["computational", "canonical"], "default": "computational"},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
                    }
                }),
            ),
        ),
        (
            "cellguide",
            ToolSchema::new(
                "get_source_data",
                "List CZ CELLxGENE Discover collections that contribute cells of this type in the CellGuide snapshot (source_collections/). Each collection includes its Discover URL, publication citation when present, and tissues/diseases/organisms annotated on those datasets. At most 100 collections are returned; truncated is set when more exist.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["cell_type"],
                    "properties": {
                        "cell_type": {"type": "string", "minLength": 1, "maxLength": 256}
                    }
                }),
            ),
        ),
        (
            "cellguide",
            ToolSchema::new(
                "get_cell_tissues",
                "List UBERON tissues in which a cell type appears in the human (NCBITaxon:9606) CellGuide ontology tree: ontology_tree/NCBITaxon_9606/celltype_to_tissue_mapping.json joined with tissue_metadata.json. Names come from tissue metadata when present. At most 200 tissues are returned; truncated is set when the mapping is longer. Human mapping only.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["cell_type"],
                    "properties": {
                        "cell_type": {"type": "string", "minLength": 1, "maxLength": 256}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("CELLxGENE CellGuide request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "search_cell_types" => search_cell_types(bio, args).await,
        "get_cell_type_info" => get_cell_type_info(bio, args).await,
        "get_marker_genes" => get_marker_genes(bio, args).await,
        "get_source_data" => get_source_data(bio, args).await,
        "get_cell_tissues" => get_cell_tissues(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    #[serde(default = "default_search")]
    limit: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CellTypeArgs {
    cell_type: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerArgs {
    cell_type: String,
    #[serde(default = "default_marker_type")]
    marker_type: String,
    #[serde(default = "default_markers")]
    limit: u32,
}

fn default_search() -> u32 {
    DEFAULT_SEARCH
}

fn default_markers() -> u32 {
    DEFAULT_MARKERS
}

fn default_marker_type() -> String {
    "computational".into()
}

#[derive(Clone)]
struct CellEntry {
    id: String,
    name: String,
    synonyms: Vec<String>,
    ontology_description: Option<String>,
}

#[derive(Clone)]
struct TissueEntry {
    id: String,
    name: String,
    description: Option<String>,
}

async fn search_cell_types(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchArgs = serde_json::from_value(args.clone())
        .context("invalid CellGuide search_cell_types arguments")?;
    let query = require_text(&args.query, "query")?;
    let cap = bound_limit(args.limit, MAX_SEARCH, "limit")?;
    let snapshot = snapshot_id(bio).await?;
    let catalog = cell_catalog(bio, &snapshot).await?;
    let ranked = rank_cells(&catalog, query);
    let total = ranked.len();
    let page: Vec<Value> = ranked.into_iter().take(cap).map(cell_hit).collect();
    Ok(json!({
        "source": "CELLxGENE CellGuide",
        "source_url": CELLGUIDE_UI,
        "snapshot": snapshot,
        "artifact_url": catalog_url(&snapshot, "celltype_metadata.json"),
        "query": query,
        "total_available": total,
        "returned": page.len(),
        "truncated": total > page.len(),
        "records": page,
    }))
}

async fn get_cell_type_info(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: CellTypeArgs = serde_json::from_value(args.clone())
        .context("invalid CellGuide get_cell_type_info arguments")?;
    let query = require_text(&args.cell_type, "cell_type")?;
    let snapshot = snapshot_id(bio).await?;
    let catalog = cell_catalog(bio, &snapshot).await?;
    let cell = resolve_cell(&catalog, query, &snapshot)?;
    let (description, description_source, references, description_truncated) =
        load_description(bio, &cell.id).await?;
    Ok(json!({
        "source": "CELLxGENE CellGuide",
        "source_url": CELLGUIDE_UI,
        "snapshot": snapshot,
        "url": card_url(&cell.id),
        "query": query,
        "id": cell.id,
        "name": cell.name,
        "synonyms": cell.synonyms,
        "ontology_description": cell.ontology_description,
        "description": description,
        "description_source": description_source,
        "description_truncated": description_truncated,
        "references": references,
    }))
}

async fn get_marker_genes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MarkerArgs = serde_json::from_value(args.clone())
        .context("invalid CellGuide get_marker_genes arguments")?;
    let query = require_text(&args.cell_type, "cell_type")?;
    let marker_type = match args.marker_type.trim() {
        "computational" | "canonical" => args.marker_type.trim(),
        other => bail!("marker_type must be computational or canonical, not {other:?}"),
    };
    let cap = bound_limit(args.limit, MAX_MARKERS, "limit")?;
    let snapshot = snapshot_id(bio).await?;
    let catalog = cell_catalog(bio, &snapshot).await?;
    let cell = resolve_cell(&catalog, query, &snapshot)?;
    let folder = if marker_type == "canonical" {
        "canonical_marker_genes"
    } else {
        "computational_marker_genes"
    };
    let artifact = format!(
        "{}/{snapshot}/{folder}/{}.json",
        CELLGUIDE_CDN,
        filesystem_id(&cell.id)
    );
    let raw = optional_json(bio, &cell_artifact(bio, &snapshot, folder, &cell.id)).await?;
    let mut genes = match raw {
        None => Vec::new(),
        Some(Value::Array(rows)) => rows
            .iter()
            .filter_map(|row| project_marker(row, marker_type))
            .collect(),
        Some(_) => bail!("CELLxGENE CellGuide {folder} payload was not a JSON array"),
    };
    if marker_type == "computational" {
        genes.sort_by(|a, b| {
            let sa = marker_score(a);
            let sb = marker_score(b);
            sb.partial_cmp(&sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| marker_symbol(a).cmp(&marker_symbol(b)))
        });
    }
    let total = genes.len();
    let page: Vec<Value> = genes.into_iter().take(cap).collect();
    Ok(json!({
        "source": "CELLxGENE CellGuide",
        "source_url": CELLGUIDE_UI,
        "snapshot": snapshot,
        "url": card_url(&cell.id),
        "artifact_url": artifact,
        "query": query,
        "cell_type_id": cell.id,
        "cell_type_name": cell.name,
        "marker_type": marker_type,
        "total_available": total,
        "returned": page.len(),
        "truncated": total > page.len(),
        "marker_genes": page,
    }))
}

async fn get_source_data(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: CellTypeArgs = serde_json::from_value(args.clone())
        .context("invalid CellGuide get_source_data arguments")?;
    let query = require_text(&args.cell_type, "cell_type")?;
    let snapshot = snapshot_id(bio).await?;
    let catalog = cell_catalog(bio, &snapshot).await?;
    let cell = resolve_cell(&catalog, query, &snapshot)?;
    let artifact = format!(
        "{}/{snapshot}/source_collections/{}.json",
        CELLGUIDE_CDN,
        filesystem_id(&cell.id)
    );
    let raw = optional_json(
        bio,
        &cell_artifact(bio, &snapshot, "source_collections", &cell.id),
    )
    .await?;
    let collections = match raw {
        None => Vec::new(),
        Some(Value::Array(rows)) => rows.iter().filter_map(project_collection).collect(),
        Some(_) => bail!("CELLxGENE CellGuide source_collections payload was not a JSON array"),
    };
    let total = collections.len();
    let page: Vec<Value> = collections.into_iter().take(MAX_COLLECTIONS).collect();
    Ok(json!({
        "source": "CELLxGENE CellGuide",
        "source_url": CELLGUIDE_UI,
        "snapshot": snapshot,
        "url": card_url(&cell.id),
        "artifact_url": artifact,
        "query": query,
        "cell_type_id": cell.id,
        "cell_type_name": cell.name,
        "total_available": total,
        "returned": page.len(),
        "truncated": total > page.len(),
        "collections": page,
    }))
}

async fn get_cell_tissues(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: CellTypeArgs = serde_json::from_value(args.clone())
        .context("invalid CellGuide get_cell_tissues arguments")?;
    let query = require_text(&args.cell_type, "cell_type")?;
    let snapshot = snapshot_id(bio).await?;
    let catalog = cell_catalog(bio, &snapshot).await?;
    let cell = resolve_cell(&catalog, query, &snapshot)?;
    let mapping = tissue_mapping(bio, &snapshot).await?;
    let tissues_meta = tissue_catalog(bio, &snapshot).await?;
    let ids = mapping.get(&cell.id).cloned().unwrap_or_default();
    let total = ids.len();
    let page: Vec<Value> = ids
        .iter()
        .take(MAX_TISSUES)
        .map(|id| tissue_hit(id, tissues_meta.get(id)))
        .collect();
    Ok(json!({
        "source": "CELLxGENE CellGuide",
        "source_url": CELLGUIDE_UI,
        "snapshot": snapshot,
        "url": card_url(&cell.id),
        "artifact_url": format!(
            "{}/{snapshot}/ontology_tree/{HUMAN_TAXONOMY}/celltype_to_tissue_mapping.json",
            CELLGUIDE_CDN
        ),
        "organism": "NCBITaxon:9606",
        "query": query,
        "cell_type_id": cell.id,
        "cell_type_name": cell.name,
        "total_available": total,
        "returned": page.len(),
        "truncated": total > page.len(),
        "tissues": page,
    }))
}

async fn snapshot_id(bio: &NativeBio) -> Result<String> {
    let url = format!("{}/latest_snapshot_identifier", api_base(bio));
    let response = bio.http().send(CELLGUIDE, Method::GET, &url, &[]).await?;
    response.check()?;
    if looks_like_html(&response.body) {
        bail!("CELLxGENE CellGuide returned an HTML page instead of a snapshot identifier");
    }
    let text = String::from_utf8(response.body)
        .context("CELLxGENE CellGuide snapshot identifier was not UTF-8")?;
    let id = text.trim();
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        bail!("CELLxGENE CellGuide snapshot identifier was missing or invalid");
    }
    Ok(id.to_string())
}

async fn cell_catalog(bio: &NativeBio, snapshot: &str) -> Result<Vec<CellEntry>> {
    let raw = required_json(bio, &snapshot_file(bio, snapshot, "celltype_metadata.json")).await?;
    parse_cell_catalog(&raw)
}

async fn tissue_catalog(
    bio: &NativeBio,
    snapshot: &str,
) -> Result<std::collections::BTreeMap<String, TissueEntry>> {
    let raw = required_json(bio, &snapshot_file(bio, snapshot, "tissue_metadata.json")).await?;
    parse_tissue_catalog(&raw)
}

async fn tissue_mapping(
    bio: &NativeBio,
    snapshot: &str,
) -> Result<std::collections::BTreeMap<String, Vec<String>>> {
    let path = format!("ontology_tree/{HUMAN_TAXONOMY}/celltype_to_tissue_mapping.json");
    let raw = required_json(bio, &snapshot_file(bio, snapshot, &path)).await?;
    parse_tissue_mapping(&raw)
}

async fn load_description(
    bio: &NativeBio,
    cell_id: &str,
) -> Result<(Option<String>, &'static str, Vec<String>, bool)> {
    let file = format!("{}.json", filesystem_id(cell_id));
    if let Some(raw) = optional_json(
        bio,
        &cdn_file(bio, &format!("validated_descriptions/{file}")),
    )
    .await?
    {
        let (text, references, truncated) = parse_validated_description(&raw)?;
        return Ok((text, "validated", references, truncated));
    }
    if let Some(raw) =
        optional_json(bio, &cdn_file(bio, &format!("gpt_descriptions/{file}"))).await?
    {
        let (text, truncated) = parse_gpt_description(&raw)?;
        return Ok((text, "gpt", Vec::new(), truncated));
    }
    Ok((None, "none", Vec::new(), false))
}

async fn required_json(bio: &NativeBio, url: &str) -> Result<Value> {
    let response = cellguide_get(bio, url).await?;
    response.check()?;
    if looks_like_html(&response.body) {
        bail!("CELLxGENE CellGuide returned an HTML page instead of JSON");
    }
    if response.body.iter().all(|b| b.is_ascii_whitespace()) {
        bail!("CELLxGENE CellGuide returned an empty JSON body");
    }
    serde_json::from_slice(&response.body).context("CELLxGENE CellGuide returned invalid JSON")
}

async fn optional_json(bio: &NativeBio, url: &str) -> Result<Option<Value>> {
    let response = cellguide_get(bio, url).await?;
    if response.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    response.check()?;
    if looks_like_html(&response.body) {
        bail!("CELLxGENE CellGuide returned an HTML page instead of JSON");
    }
    if response.body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(None);
    }
    let value = serde_json::from_slice(&response.body)
        .context("CELLxGENE CellGuide returned invalid JSON")?;
    Ok(Some(value))
}

async fn cellguide_get(bio: &NativeBio, url: &str) -> Result<crate::http::Response> {
    bio.http().send(CELLGUIDE, Method::GET, url, &[]).await
}

fn api_base(bio: &NativeBio) -> String {
    bio.credential("CELLGUIDE_BASE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| CELLGUIDE_CDN.to_string())
}

fn snapshot_file(bio: &NativeBio, snapshot: &str, path: &str) -> String {
    format!("{}/{snapshot}/{path}", api_base(bio))
}

fn cdn_file(bio: &NativeBio, path: &str) -> String {
    format!("{}/{path}", api_base(bio))
}

fn cell_artifact(bio: &NativeBio, snapshot: &str, folder: &str, cell_id: &str) -> String {
    format!(
        "{}/{snapshot}/{folder}/{}.json",
        api_base(bio),
        filesystem_id(cell_id)
    )
}

fn catalog_url(snapshot: &str, file: &str) -> String {
    format!("{CELLGUIDE_CDN}/{snapshot}/{file}")
}

fn card_url(cell_id: &str) -> String {
    format!("{CELLGUIDE_UI}/{}", filesystem_id(cell_id))
}

fn parse_cell_catalog(raw: &Value) -> Result<Vec<CellEntry>> {
    let object = raw
        .as_object()
        .context("CELLxGENE CellGuide celltype_metadata.json was not a JSON object")?;
    let mut cells = Vec::new();
    for (key, value) in object {
        if let Some(cell) = cell_from_entry(key, value) {
            cells.push(cell);
        }
    }
    Ok(cells)
}

fn cell_from_entry(key: &str, value: &Value) -> Option<CellEntry> {
    let object = value.as_object()?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(key)
        .trim();
    if id.is_empty() {
        return None;
    }
    let id = parse_cl_id(id).unwrap_or_else(|| id.to_string());
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    let synonyms = string_list(object.get("synonyms"))
        .into_iter()
        .take(MAX_SYNONYMS)
        .collect();
    let ontology_description = object
        .get("clDescription")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    Some(CellEntry {
        id,
        name,
        synonyms,
        ontology_description,
    })
}

fn parse_tissue_catalog(raw: &Value) -> Result<std::collections::BTreeMap<String, TissueEntry>> {
    let object = raw
        .as_object()
        .context("CELLxGENE CellGuide tissue_metadata.json was not a JSON object")?;
    let mut tissues = std::collections::BTreeMap::new();
    for (key, value) in object {
        let Some(object) = value.as_object() else {
            continue;
        };
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .trim();
        if id.is_empty() {
            continue;
        }
        let id = normalize_uberon(id).unwrap_or_else(|| id.to_string());
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or(id.as_str())
            .to_string();
        let description = object
            .get("uberonDescription")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string);
        tissues.insert(
            id.clone(),
            TissueEntry {
                id,
                name,
                description,
            },
        );
    }
    Ok(tissues)
}

fn parse_tissue_mapping(raw: &Value) -> Result<std::collections::BTreeMap<String, Vec<String>>> {
    let object = raw
        .as_object()
        .context("CELLxGENE CellGuide celltype_to_tissue_mapping.json was not a JSON object")?;
    let mut mapping = std::collections::BTreeMap::new();
    for (key, value) in object {
        let cell_id = parse_cl_id(key).unwrap_or_else(|| key.clone());
        let Some(ids) = value.as_array() else {
            continue;
        };
        let tissues: Vec<String> = ids
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| normalize_uberon(id).unwrap_or_else(|| id.to_string()))
            .collect();
        mapping.insert(cell_id, tissues);
    }
    Ok(mapping)
}

fn parse_validated_description(raw: &Value) -> Result<(Option<String>, Vec<String>, bool)> {
    match raw {
        Value::String(text) => {
            let (text, truncated) = bound_description(text);
            Ok((text, Vec::new(), truncated))
        }
        Value::Object(object) => {
            let (text, truncated) = object
                .get("description")
                .and_then(Value::as_str)
                .map(bound_description)
                .unwrap_or((None, false));
            let references = string_list(object.get("references"))
                .into_iter()
                .take(MAX_REFERENCES)
                .collect();
            Ok((text, references, truncated))
        }
        _ => bail!("CELLxGENE CellGuide validated description was not a JSON object or string"),
    }
}

fn parse_gpt_description(raw: &Value) -> Result<(Option<String>, bool)> {
    match raw {
        Value::String(text) => Ok(bound_description(text)),
        Value::Object(object) => Ok(object
            .get("description")
            .and_then(Value::as_str)
            .map(bound_description)
            .unwrap_or((None, false))),
        _ => bail!("CELLxGENE CellGuide GPT description was not a JSON string or object"),
    }
}

fn bound_description(text: &str) -> (Option<String>, bool) {
    let text = text.trim();
    if text.is_empty() {
        return (None, false);
    }
    if text.len() > MAX_DESCRIPTION {
        let mut end = MAX_DESCRIPTION;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        return (Some(text[..end].to_string()), true);
    }
    (Some(text.to_string()), false)
}

fn project_marker(row: &Value, marker_type: &str) -> Option<Value> {
    let object = row.as_object()?;
    let symbol = object
        .get("symbol")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if marker_type == "canonical" {
        let publication = object
            .get("publication")
            .and_then(Value::as_str)
            .map(split_joined)
            .unwrap_or_default();
        let publication_titles = object
            .get("publication_titles")
            .and_then(Value::as_str)
            .map(split_joined)
            .unwrap_or_default();
        return Some(json!({
            "symbol": symbol,
            "name": name,
            "tissue": object.get("tissue").and_then(Value::as_str),
            "publication": publication,
            "publication_titles": publication_titles,
        }));
    }
    let dims = object.get("groupby_dims").and_then(Value::as_object);
    Some(json!({
        "symbol": symbol,
        "name": name,
        "gene_ontology_term_id": object.get("gene_ontology_term_id").and_then(Value::as_str),
        "marker_score": object.get("marker_score").and_then(Value::as_f64),
        "specificity": object.get("specificity").and_then(Value::as_f64),
        "mean_expression": object.get("me").and_then(Value::as_f64),
        "fraction_expressed": object.get("pc").and_then(Value::as_f64),
        "organism": dim_label(dims, "organism_ontology_term_label", "organism_ontology_term_id"),
        "tissue": dim_label(dims, "tissue_ontology_term_label", "tissue_ontology_term_id"),
    }))
}

fn project_collection(row: &Value) -> Option<Value> {
    let object = row.as_object()?;
    let name = object
        .get("collection_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let url = object
        .get("collection_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if name.is_none() && url.is_none() {
        return None;
    }
    Some(json!({
        "collection_name": name,
        "collection_url": url,
        "publication_title": object.get("publication_title").and_then(Value::as_str),
        "publication_url": object.get("publication_url").and_then(Value::as_str),
        "tissues": labeled_terms(object.get("tissue")),
        "diseases": labeled_terms(object.get("disease")),
        "organisms": labeled_terms(object.get("organism")),
    }))
}

fn labeled_terms(value: Option<&Value>) -> Vec<Value> {
    let Some(Value::Array(rows)) = value else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let object = row.as_object()?;
            let id = object
                .get("ontology_term_id")
                .or_else(|| object.get("id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty());
            let name = object
                .get("label")
                .or_else(|| object.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty());
            if id.is_none() && name.is_none() {
                return None;
            }
            Some(json!({"id": id, "name": name}))
        })
        .collect()
}

fn dim_label(dims: Option<&Map<String, Value>>, label_key: &str, id_key: &str) -> Option<String> {
    let dims = dims?;
    dims.get(label_key)
        .or_else(|| dims.get(id_key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn tissue_hit(id: &str, meta: Option<&TissueEntry>) -> Value {
    match meta {
        Some(tissue) => json!({
            "id": tissue.id,
            "name": tissue.name,
            "description": tissue.description,
            "url": card_url(&tissue.id),
        }),
        None => json!({
            "id": id,
            "name": id,
            "description": Value::Null,
        }),
    }
}

fn cell_hit(cell: CellEntry) -> Value {
    json!({
        "id": cell.id,
        "name": cell.name,
        "synonyms": cell.synonyms,
        "ontology_description": cell.ontology_description,
        "url": card_url(&cell.id),
    })
}

fn resolve_cell<'a>(catalog: &'a [CellEntry], query: &str, snapshot: &str) -> Result<CellEntry> {
    if let Some(id) = parse_cl_id(query) {
        return catalog
            .iter()
            .find(|cell| cell.id == id)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "cell type {query:?} was not found in CELLxGENE CellGuide snapshot {snapshot}"
                )
            });
    }
    rank_cells(catalog, query)
        .into_iter()
        .next()
        .ok_or_else(|| {
            anyhow!("cell type {query:?} was not found in CELLxGENE CellGuide snapshot {snapshot}")
        })
}

fn rank_cells(catalog: &[CellEntry], query: &str) -> Vec<CellEntry> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let id_query = parse_cl_id(query);
    let mut scored: Vec<(i32, CellEntry)> = catalog
        .iter()
        .filter_map(|cell| {
            if id_query.as_ref().is_some_and(|id| id == &cell.id) {
                return Some((200, cell.clone()));
            }
            let name = cell.name.to_ascii_lowercase();
            let mut score = match_score(&needle, &name, 100, 80, 50);
            for synonym in &cell.synonyms {
                score = score.max(match_score(
                    &needle,
                    &synonym.to_ascii_lowercase(),
                    70,
                    60,
                    30,
                ));
            }
            (score > 0).then(|| (score, cell.clone()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().map(|(_, cell)| cell).collect()
}

fn match_score(needle: &str, haystack: &str, exact: i32, prefix: i32, contains: i32) -> i32 {
    if haystack == needle {
        exact
    } else if haystack.starts_with(needle) {
        prefix
    } else if haystack.contains(needle) {
        contains
    } else {
        0
    }
}

fn parse_cl_id(raw: &str) -> Option<String> {
    parse_curie(raw, "CL", 7, 12)
}

fn normalize_uberon(raw: &str) -> Option<String> {
    parse_curie(raw, "UBERON", 7, 12)
}

fn parse_curie(raw: &str, prefix: &str, min_digits: usize, max_digits: usize) -> Option<String> {
    let raw = raw.trim();
    let upper = raw.to_ascii_uppercase();
    let prefix_upper = prefix.to_ascii_uppercase();
    let digits = if let Some(rest) = upper.strip_prefix(&format!("{prefix_upper}:")) {
        rest
    } else if let Some(rest) = upper.strip_prefix(&format!("{prefix_upper}_")) {
        rest
    } else if prefix_upper == "CL" && upper.bytes().all(|b| b.is_ascii_digit()) {
        upper.as_str()
    } else {
        return None;
    };
    if digits.len() < min_digits
        || digits.len() > max_digits
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{prefix_upper}:{digits}"))
}

fn filesystem_id(curie: &str) -> String {
    path_segment(&curie.replace(':', "_"))
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

fn require_text<'a>(value: &'a str, what: &str) -> Result<&'a str> {
    let text = value.trim();
    if text.is_empty() || text.len() > MAX_QUERY {
        bail!("{what} must contain 1 to {MAX_QUERY} characters");
    }
    if text.chars().any(|c| c == '\0' || c == '/' || c == '\\') {
        bail!("{what} contains unsupported path characters");
    }
    Ok(text)
}

fn bound_limit(n: u32, max: u32, name: &str) -> Result<usize> {
    if !(1..=max).contains(&n) {
        bail!("{name} must be between 1 and {max}");
    }
    Ok(n as usize)
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(text)) => {
            let text = text.trim();
            if text.is_empty() {
                Vec::new()
            } else {
                vec![text.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

fn split_joined(value: &str) -> Vec<String> {
    value
        .split(";;")
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn marker_score(row: &Value) -> f64 {
    row.get("marker_score")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NEG_INFINITY)
}

fn marker_symbol(row: &Value) -> &str {
    row.get("symbol").and_then(Value::as_str).unwrap_or("")
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
