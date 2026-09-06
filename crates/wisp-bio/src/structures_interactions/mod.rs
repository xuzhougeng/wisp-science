//! Native `structures-interactions` domain against RCSB PDB, AlphaFold DB,
//! EMDB, Complex Portal and IntAct. Independently implemented from:
//!
//! - [RCSB PDB Search API](https://search.rcsb.org/)
//! - [RCSB PDB Data API](https://data.rcsb.org/index.html)
//! - [AlphaFold DB API](https://alphafold.ebi.ac.uk/api-docs)
//! - [EMDB REST API](https://www.ebi.ac.uk/emdb/api/)
//! - [Complex Portal web service](https://www.ebi.ac.uk/intact/complex-ws/search/)
//! - [IntAct technical corner](https://www.ebi.ac.uk/intact/documentation/technical_corner)
//!
//! References reviewed 2026-09-06. No API keys are published. Tests use
//! invented records. Coordinate files, map volumes and PAE payloads are never
//! downloaded.

mod alphafold;
mod complexportal;
mod emdb;
mod intact;
mod pdb;

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde_json::{json, Map, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const DOMAIN: &str = "structures-interactions";

pub(crate) const PDB_SEARCH: Source = Source("RCSB PDB Search", Duration::from_millis(500));
pub(crate) const PDB_DATA: Source = Source("RCSB PDB Data", Duration::from_millis(500));
pub(crate) const ALPHAFOLD: Source = Source("AlphaFold DB", Duration::from_millis(500));
pub(crate) const EMDB: Source = Source("EMDB", Duration::from_millis(500));
pub(crate) const COMPLEX_PORTAL: Source = Source("Complex Portal", Duration::from_millis(500));
pub(crate) const INTACT: Source = Source("IntAct", Duration::from_millis(500));

pub(crate) const PDB_SEARCH_DEFAULT: &str = "https://search.rcsb.org/rcsbsearch/v2/query";
pub(crate) const PDB_DATA_DEFAULT: &str = "https://data.rcsb.org/rest/v1/core";
pub(crate) const ALPHAFOLD_DEFAULT: &str = "https://alphafold.ebi.ac.uk/api";
pub(crate) const EMDB_DEFAULT: &str = "https://www.ebi.ac.uk/emdb/api";
pub(crate) const COMPLEXPORTAL_DEFAULT: &str = "https://www.ebi.ac.uk/intact/complex-ws";
pub(crate) const INTACT_DEFAULT: &str = "https://www.ebi.ac.uk/intact/ws";

pub(crate) const PDB_SITE: &str = "https://www.rcsb.org";
pub(crate) const ALPHAFOLD_SITE: &str = "https://alphafold.ebi.ac.uk";
pub(crate) const EMDB_SITE: &str = "https://www.ebi.ac.uk/emdb";
pub(crate) const COMPLEXPORTAL_SITE: &str = "https://www.ebi.ac.uk/complexportal";
pub(crate) const INTACT_SITE: &str = "https://www.ebi.ac.uk/intact";

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "alphafold_check_coverage",
            "Check AlphaFold DB coverage for up to 40 unique UniProt accessions. Returns whether each accession has a predicted model, how many models exist, and the primary model's identifier, version, global pLDDT and sequence length. Accessions with no model are listed with has_model=false; malformed identifiers carry an error. Blank and duplicate inputs are skipped and counted. Metadata and download URLs only; coordinate files are never fetched.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["uniprot_accessions"],
                "properties": {
                    "uniprot_accessions": {
                        "type": "array", "minItems": 1, "maxItems": 80,
                        "items": {"type": "string", "maxLength": 32}
                    }
                }
            }),
        ),
        tool(
            "alphafold_get_prediction",
            "Retrieve AlphaFold DB predicted-structure metadata for one UniProt accession. Returns per-model identifiers, UniProt annotation, residue span, global pLDDT and pLDDT-bin fractions, version, and download URLs for coordinates/PAE/pLDDT (URLs only). An accession with no archived prediction returns has_model=false rather than an error. include_sequence adds the model sequence when present.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["uniprot_accession"],
                "properties": {
                    "uniprot_accession": {"type": "string", "minLength": 1, "maxLength": 32},
                    "include_sequence": {"type": "boolean", "default": false}
                }
            }),
        ),
        tool(
            "complexportal_get_complexes",
            "Fetch curated Complex Portal records by CPX accession from the EBI Complex Portal web service. Each record includes recommended and systematic names, species, participant stoichiometry and roles, evidence ECO code, GO annotations and cross-references. Unknown accessions are listed in not_found. At most 25 unique accessions per call. This is curated complex membership, not binary interaction evidence.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["complex_acs"],
                "properties": {
                    "complex_acs": {
                        "type": "array", "minItems": 1, "maxItems": 25,
                        "items": {"type": "string", "minLength": 3, "maxLength": 32}
                    }
                }
            }),
        ),
        tool(
            "complexportal_search_by_participant",
            "Search Complex Portal for complexes that mention a participant accession (UniProt, ChEBI or RNAcentral). participants_only=true (default) uses the pxref field so only complexes that list the molecule as a participant are returned. The response is a bounded page of compact hits; total_reported is the service count and truncated is true when more pages exist. Fetch full records with complexportal_get_complexes.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accession"],
                "properties": {
                    "accession": {"type": "string", "minLength": 1, "maxLength": 64},
                    "participants_only": {"type": "boolean", "default": true},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "emdb_get_entries",
            "Fetch EMDB cryo-EM entry metadata for up to 25 accessions (EMD-1234, emd-1234 or 1234). Returns title, structure-determination method, resolution, dates, sample names, fitted PDB IDs, primary citation and map geometry. Map volumes are never downloaded. Unknown accessions are reported with error=not_found. Obsolete entries set is_obsolete and list superseding accessions.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["emdb_ids"],
                "properties": {
                    "emdb_ids": {
                        "type": "array", "minItems": 1, "maxItems": 25,
                        "items": {"type": "string", "minLength": 1, "maxLength": 24}
                    }
                }
            }),
        ),
        tool(
            "emdb_get_entry_section",
            "Fetch one detailed metadata section for EMDB entries: publications (primary and auxiliary citations), map (dimensions, voxel size, statistics, contour, symmetry), sample (macromolecules and supramolecules) or imaging (microscope sessions and specimen preparation). Unknown accessions are reported with error=not_found. At most 25 accessions per call.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["emdb_ids", "section"],
                "properties": {
                    "emdb_ids": {
                        "type": "array", "minItems": 1, "maxItems": 25,
                        "items": {"type": "string", "minLength": 1, "maxLength": 24}
                    },
                    "section": {"type": "string", "enum": ["publications", "map", "sample", "imaging"]}
                }
            }),
        ),
        tool(
            "emdb_get_validation",
            "Fetch EMDB validation-analysis metrics (Q-score, atom inclusion, contour recommendations, FSC-derived resolution, volume estimates) from GET /analysis/{id}. Entries with no analysis report has_validation_analysis=false. At most 25 accessions per call. Numeric metrics only; map volumes are not downloaded.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["emdb_ids"],
                "properties": {
                    "emdb_ids": {
                        "type": "array", "minItems": 1, "maxItems": 25,
                        "items": {"type": "string", "minLength": 1, "maxLength": 24}
                    }
                }
            }),
        ),
        tool(
            "emdb_search_entries",
            "Search EMDB with a Solr/Lucene query against GET /search/{query}. Returns a bounded page of compact rows (accession, title, resolution, method, status, fitted PDBs). num_found_released is the facet-route released-entry count when available; truncated is true when more matching rows exist than were retrieved. max_rows is 1–1000 (default 100). A capped page is not the complete hit list.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 512},
                    "max_rows": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }
            }),
        ),
        tool(
            "intact_build_network",
            "Build a depth-1 IntAct binary-interaction network around UniProt seed accessions. Each seed is queried with an MI-score floor (default 0.45). Partners of seed edges form the node set; up to max_interactors_expanded partners (most-connected first) are queried for partner–partner edges already inside that set. expansion.complete is false when more partners were not expanded. At most 5 seeds. Interaction sweeps are bounded pages, not complete interactomes.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["seed_accessions"],
                "properties": {
                    "seed_accessions": {
                        "type": "array", "minItems": 1, "maxItems": 5,
                        "items": {"type": "string", "minLength": 1, "maxLength": 32}
                    },
                    "min_mi_score": {"type": "number", "minimum": 0, "maximum": 1, "default": 0.45},
                    "max_interactors_expanded": {"type": "integer", "minimum": 0, "maximum": 25, "default": 10},
                    "interactor_species": {
                        "type": "array", "maxItems": 8,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    }
                }
            }),
        ),
        tool(
            "intact_fetch_interactions",
            "Search IntAct binary interactions with a UniProt accession, gene symbol or MIQL query via the Interaction Search service. min_mi_score/max_mi_score filter the IntAct MI confidence score; interactor_species filters by species name or taxid. Returns a bounded page of slim records (interactor pair, type, detection method, host, MI score, PubMed). total_elements is the service count; truncated is true when more matches exist than were returned. max_records_returned is 1–500 (default 200).",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 512},
                    "min_mi_score": {"type": "number", "minimum": 0, "maximum": 1, "default": 0.0},
                    "max_mi_score": {"type": "number", "minimum": 0, "maximum": 1, "default": 1.0},
                    "interactor_species": {
                        "type": "array", "maxItems": 8,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "max_records_returned": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "intact_get_interaction_details",
            "Retrieve curated detail for one IntAct interaction accession (EBI-…) from the graph data service: type, host, detection method, publication, cross-references, annotations, parameters and confidences. include_participants (default true) adds a bounded participant page. Unknown accessions return error=not_found (the graph route answers them with an empty body).",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["interaction_ac"],
                "properties": {
                    "interaction_ac": {"type": "string", "minLength": 3, "maxLength": 32},
                    "include_participants": {"type": "boolean", "default": true}
                }
            }),
        ),
        tool(
            "intact_get_interactor",
            "Resolve a UniProt accession, gene symbol or IntAct interactor AC to IntAct interactor records. A UniProt accession can match the canonical protein plus chain or isoform interactors; every match is returned with n_matches rather than silently picking one. Each record includes preferred identifier, name, species, molecule type and interaction_count.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 64}
                }
            }),
        ),
        tool(
            "pdb_get_entities",
            "Retrieve polymer-entity metadata for one PDB entry from the RCSB Data API, including polymer type, chain IDs, source organisms, UniProt mappings and SIFTS-aligned regions. Omit entity_ids to fetch the entry's polymer entities (capped at 25; truncated reports the true count). An explicit entity_ids list larger than 25 is rejected. include_sequences adds one-letter sequences unless their combined size exceeds max_bytes. Unknown entity IDs are listed in not_found; an unknown entry is an error. Coordinates are never downloaded.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["pdb_id"],
                "properties": {
                    "pdb_id": {"type": "string", "minLength": 4, "maxLength": 32},
                    "entity_ids": {
                        "type": "array", "maxItems": 25,
                        "items": {"type": "string", "minLength": 1, "maxLength": 8}
                    },
                    "include_sequences": {"type": "boolean", "default": false},
                    "max_bytes": {"type": "integer", "minimum": 1, "maximum": 400000, "default": 400000}
                }
            }),
        ),
        tool(
            "pdb_get_ligands",
            "List bound nonpolymer ligands for one PDB entry from the RCSB Data API, with chemical-component formula, InChIKey and stereo SMILES when served. Waters are not nonpolymer entities and never appear. n_nonpolymer_entities is the entry total; truncated is true when it exceeds max_ligands (1–25). Partial missing components are reported inline with error=not_found. An unknown entry is an error.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["pdb_id"],
                "properties": {
                    "pdb_id": {"type": "string", "minLength": 4, "maxLength": 32},
                    "max_ligands": {"type": "integer", "minimum": 1, "maximum": 25, "default": 25}
                }
            }),
        ),
        tool(
            "pdb_get_structures",
            "Fetch entry-level RCSB PDB summaries for up to 25 unique PDB IDs: title, experimental methods, resolution, determination methodology, dates, assembly/entity counts, bound ligand comp IDs, polymer/nonpolymer entity ID lists and the primary citation. Unknown IDs return error=not_found. Blank and duplicate inputs are skipped and counted. Metadata only; mmCIF/PDB coordinate files are never downloaded.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["pdb_ids"],
                "properties": {
                    "pdb_ids": {
                        "type": "array", "minItems": 1, "maxItems": 50,
                        "items": {"type": "string", "minLength": 1, "maxLength": 32}
                    }
                }
            }),
        ),
        tool(
            "pdb_search_structures",
            "Search RCSB PDB entries with the Search API v2. Filters AND together; at least one of text, organism, taxonomy_id, uniprot_accession, experimental_method, max_resolution_angstrom or ligand_comp_id is required. include_computed_models adds computational models to the default experimental-only results. Returns identifiers and relevance scores, not coordinates. total_count is the API match total; truncated is true when more hits exist than max_rows (1–1000, default 100). Chain to pdb_get_structures for metadata.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "text": {"type": "string", "minLength": 1, "maxLength": 256},
                    "organism": {"type": "string", "minLength": 1, "maxLength": 128},
                    "taxonomy_id": {"type": "integer", "minimum": 1, "maximum": 999999999},
                    "uniprot_accession": {"type": "string", "minLength": 1, "maxLength": 32},
                    "experimental_method": {"type": "string", "minLength": 1, "maxLength": 64},
                    "max_resolution_angstrom": {"type": "number", "minimum": 0.1, "maximum": 100},
                    "ligand_comp_id": {"type": "string", "minLength": 1, "maxLength": 8},
                    "include_computed_models": {"type": "boolean", "default": false},
                    "max_rows": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, parameters: Value) -> (&'static str, ToolSchema) {
    (DOMAIN, ToolSchema::new(name, description, parameters))
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "alphafold_check_coverage" => alphafold::check_coverage(bio, args).await,
        "alphafold_get_prediction" => alphafold::get_prediction(bio, args).await,
        "complexportal_get_complexes" => complexportal::get_complexes(bio, args).await,
        "complexportal_search_by_participant" => {
            complexportal::search_by_participant(bio, args).await
        }
        "emdb_get_entries" => emdb::get_entries(bio, args).await,
        "emdb_get_entry_section" => emdb::get_entry_section(bio, args).await,
        "emdb_get_validation" => emdb::get_validation(bio, args).await,
        "emdb_search_entries" => emdb::search_entries(bio, args).await,
        "intact_build_network" => intact::build_network(bio, args).await,
        "intact_fetch_interactions" => intact::fetch_interactions(bio, args).await,
        "intact_get_interaction_details" => intact::get_interaction_details(bio, args).await,
        "intact_get_interactor" => intact::get_interactor(bio, args).await,
        "pdb_get_entities" => pdb::get_entities(bio, args).await,
        "pdb_get_ligands" => pdb::get_ligands(bio, args).await,
        "pdb_get_structures" => pdb::get_structures(bio, args).await,
        "pdb_search_structures" => pdb::search_structures(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

pub(crate) fn api_base(bio: &NativeBio, credential: &str, default: &str) -> String {
    bio.credential(credential)
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.trim_end_matches('/').to_string())
}

pub(crate) async fn send_json(
    bio: &NativeBio,
    source: Source,
    method: Method,
    url: &str,
    params: &[(String, String)],
) -> Result<(StatusCode, Option<Value>)> {
    let response = bio.http().send(source, method, url, params).await?;
    let status = response.status;
    if !status.is_success() {
        return Ok((status, None));
    }
    if status == StatusCode::NO_CONTENT || response.body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok((status, None));
    }
    let value = serde_json::from_slice(&response.body)
        .with_context(|| format!("{} returned invalid JSON", source.0))?;
    Ok((status, Some(value)))
}

pub(crate) fn require_ok(source: Source, status: StatusCode) -> Result<()> {
    if status.is_success() {
        Ok(())
    } else {
        bail!("{} returned HTTP {}", source.0, status.as_u16())
    }
}

pub(crate) fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub(crate) struct UniqueIds {
    pub requested: usize,
    pub unique: Vec<String>,
    pub n_blank: usize,
    pub n_duplicate: usize,
}

pub(crate) fn unique_ids(
    raw: &[String],
    bound: usize,
    what: &str,
    fold: impl Fn(&str) -> Result<Option<String>>,
) -> Result<UniqueIds> {
    let mut unique = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut n_blank = 0;
    let mut n_duplicate = 0;
    for item in raw {
        match fold(item.trim())? {
            None => n_blank += 1,
            Some(id) => {
                if !seen.insert(id.clone()) {
                    n_duplicate += 1;
                } else {
                    unique.push(id);
                }
            }
        }
    }
    if unique.is_empty() {
        bail!("provide at least one {what}");
    }
    if unique.len() > bound {
        bail!(
            "{} unique {what}s exceeds the per-call bound of {bound}",
            unique.len()
        );
    }
    Ok(UniqueIds {
        requested: raw.len(),
        unique,
        n_blank,
        n_duplicate,
    })
}

pub(crate) fn require_text(value: &str, what: &str, max: usize) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max {
        bail!("{what} must contain 1 to {max} characters");
    }
    if trimmed.chars().any(|c| c == '\0' || c == '\n' || c == '\r') {
        bail!("{what} must be a single line");
    }
    Ok(trimmed.to_string())
}

pub(crate) fn bound_int(value: u32, min: u32, max: u32, what: &str) -> Result<usize> {
    if !(min..=max).contains(&value) {
        bail!("{what} must be between {min} and {max}");
    }
    Ok(value as usize)
}

pub(crate) fn bound_score(value: f64, what: &str) -> Result<f64> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        bail!("{what} must be a finite number between 0 and 1");
    }
    Ok(value)
}

pub(crate) fn text_field(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        match value.get(*key) {
            Some(Value::String(text)) if !text.is_empty() => return Some(text.clone()),
            Some(Value::Number(number)) => return Some(number.to_string()),
            _ => {}
        }
    }
    None
}

pub(crate) fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) if !text.is_empty() => text.parse().ok(),
        _ => None,
    }
}

pub(crate) fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|n| n as i64)),
        Value::String(text) if !text.is_empty() => text.parse().ok(),
        _ => None,
    }
}

pub(crate) fn as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|n| u64::try_from(n).ok())),
        Value::String(text) if !text.is_empty() => text.parse().ok(),
        _ => None,
    }
}

pub(crate) fn as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) if text.eq_ignore_ascii_case("true") => Some(true),
        Value::String(text) if text.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

pub(crate) fn unwrap_value(node: &Value) -> &Value {
    node.get("valueOf_").unwrap_or(node)
}

pub(crate) fn listify(node: Option<&Value>) -> Vec<&Value> {
    match node {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item) => vec![item],
    }
}

pub(crate) fn string_list(value: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(node) = value.get(*key) {
            return listify(Some(node))
                .into_iter()
                .filter_map(|item| match item {
                    Value::String(text) if !text.is_empty() => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                })
                .collect();
        }
    }
    Vec::new()
}

pub(crate) fn object_field<'a>(value: &'a Value, key: &str) -> Option<&'a Map<String, Value>> {
    value.get(key).and_then(Value::as_object)
}

pub(crate) fn json_string(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => json!(text),
        Some(Value::Number(number)) => json!(number.to_string()),
        Some(Value::Null) | None => Value::Null,
        Some(other) => other.clone(),
    }
}
