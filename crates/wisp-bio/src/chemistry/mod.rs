//! Native `chemistry` domain (PubChem, ChEBI, Rhea, BindingDB).
//!
//! Independently implemented from operator APIs reviewed 2026-09-06:
//! - PubChem PUG REST <https://pubchem.ncbi.nlm.nih.gov/docs/pug-rest>
//! - PubChem PUG-View <https://pubchem.ncbi.nlm.nih.gov/docs/pug-view>
//! - ChEBI public backend <https://www.ebi.ac.uk/chebi/backend/api/docs/>
//! - Rhea SPARQL <https://www.rhea-db.org/help/sparql>,
//!   <https://sparql.rhea-db.org/>
//! - BindingDB REST <https://www.bindingdb.org/rwd/bind/BindingDBRESTfulAPI.jsp>
//!
//! Tests use invented records.

mod bindingdb;
mod chebi;
mod pubchem;
mod rhea;
#[cfg(test)]
mod tests;

use super::http::Source;
use super::NativeBio;
use anyhow::{anyhow, bail, Result};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

pub(super) const PUBCHEM: Source = Source("PubChem", Duration::from_millis(200));
pub(super) const EBI: Source = Source("EBI", Duration::from_millis(200));
pub(super) const RHEA: Source = Source("Rhea", Duration::from_millis(200));
pub(super) const BINDINGDB: Source = Source("BindingDB", Duration::from_millis(500));

const PUBCHEM_PUG: &str = "https://pubchem.ncbi.nlm.nih.gov/rest/pug";
const PUBCHEM_VIEW: &str = "https://pubchem.ncbi.nlm.nih.gov/rest/pug_view";
const CHEBI_PUBLIC: &str = "https://www.ebi.ac.uk/chebi/backend/api/public";
const RHEA_SPARQL: &str = "https://sparql.rhea-db.org/sparql";
const BINDINGDB_REST: &str = "https://bindingdb.org/rest";

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "pubchem_search_compounds",
            "Resolve a chemical name, SMILES, InChIKey or CID to PubChem compound IDs with PUG REST. Returns a bounded CID page, the full match count, and optional computed properties. No match is an empty list, not an error.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 8192},
                    "namespace": {"type": "string", "enum": ["name", "smiles", "inchikey", "cid"], "default": "name"},
                    "max_cids": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25},
                    "with_properties": {"type": "boolean", "default": true}
                }
            }),
        ),
        tool(
            "pubchem_get_compounds",
            "Retrieve computed PubChem properties for up to 50 CIDs. Duplicate CIDs collapse to one record in first-occurrence order. Missing CIDs are listed. Optional synonym lists are capped per CID with the true synonym count retained.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["cids"],
                "properties": {
                    "cids": {"type": "array", "minItems": 1, "maxItems": 50,
                        "items": {"type": "integer", "minimum": 1}},
                    "include_synonyms": {"type": "boolean", "default": false},
                    "max_synonyms": {"type": "integer", "minimum": 1, "maximum": 200, "default": 30}
                }
            }),
        ),
        tool(
            "pubchem_similarity_search",
            "Synchronous 2D Tanimoto similarity search over PubChem (`fastsimilarity_2d`). Threshold is percent identity (1–100). The API does not return an uncapped total; `may_be_truncated` is true when the requested cap is filled. Optional properties cover at most the first 10 hits.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["smiles"],
                "properties": {
                    "smiles": {"type": "string", "minLength": 1, "maxLength": 8192},
                    "threshold": {"type": "integer", "minimum": 1, "maximum": 100, "default": 90},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "with_properties": {"type": "boolean", "default": false}
                }
            }),
        ),
        tool(
            "pubchem_get_bioassay_summary",
            "Bioassay activity summary for one PubChem CID: assay IDs, outcomes, targets and potency when PubChem supplies them. Optional Active-only filtering happens before the row cap. Compounds with no assays return an empty page.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["cid"],
                "properties": {
                    "cid": {"type": "integer", "minimum": 1},
                    "active_only": {"type": "boolean", "default": false},
                    "max_rows": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }
            }),
        ),
        tool(
            "pubchem_get_safety",
            "GHS classification for one PubChem CID from PUG-View, aggregated across SDS sources (signals, pictograms, hazard statements, precautionary codes). `found` is false when PubChem has no GHS heading for the compound.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["cid"],
                "properties": {
                    "cid": {"type": "integer", "minimum": 1}
                }
            }),
        ),
        tool(
            "chebi_search",
            "Full-text search of ChEBI entities (names, synonyms, formulae, InChIKeys) via the public backend. `api_total` is ChEBI's own hit count. Page is 1-based.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["term"],
                "properties": {
                    "term": {"type": "string", "minLength": 1, "maxLength": 512},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20},
                    "page": {"type": "integer", "minimum": 1, "maximum": 10000, "default": 1}
                }
            }),
        ),
        tool(
            "chebi_get_entity",
            "Full ChEBI entity: structure, chemical data, roles and cross-references. Accepts `CHEBI:27732` or a bare integer. Secondary (merged) IDs resolve to the primary record. Synonyms and xrefs are capped with true totals. Unknown IDs fail. Ontology parents/children are returned by chebi_get_ontology.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["chebi_id"],
                "properties": {
                    "chebi_id": {"type": "string", "minLength": 1, "maxLength": 32},
                    "max_synonyms": {"type": "integer", "minimum": 1, "maximum": 200, "default": 30},
                    "max_xrefs": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "chebi_get_ontology",
            "Outgoing and incoming ChEBI ontology relations for one entity (is a, has role, conjugate acid/base, functional parent, tautomer, enantiomer, and others). Optional exact `relation_type` filter. Each direction is capped independently with true totals.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["chebi_id"],
                "properties": {
                    "chebi_id": {"type": "string", "minLength": 1, "maxLength": 32},
                    "relation_type": {"type": "string", "minLength": 1, "maxLength": 128},
                    "max_relations": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }
            }),
        ),
        tool(
            "rhea_search_reactions",
            "Search Rhea master reactions through the public SPARQL endpoint. A ChEBI ID (CHEBI:n or n) matches participants; a complete EC number matches enzyme-linked reactions; anything else is a case-insensitive substring of the chemical equation. Partial EC classes such as 2.1.1.- are rejected. `api_total` comes from a companion COUNT query.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 512},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50}
                }
            }),
        ),
        tool(
            "rhea_get_reaction",
            "One Rhea reaction: equation, status, transport/balance flags, EC numbers, PubMed citations, directional family IDs, and left/right participants with ChEBI accessions and stoichiometry. Accepts `RHEA:10280` or a bare integer. Unknown IDs fail.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["rhea_id"],
                "properties": {
                    "rhea_id": {"type": "string", "minLength": 1, "maxLength": 32}
                }
            }),
        ),
        tool(
            "bindingdb_ligands_by_target",
            "Measured BindingDB affinities (Ki/Kd/IC50/EC50) for ligands of one UniProt target. `affinity_cutoff_nm` keeps measurements at or below that potency (nM). The full matching set is counted; the returned page is capped and sorted most-potent-first within each affinity type. No hits is an empty page.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["uniprot"],
                "properties": {
                    "uniprot": {"type": "string", "minLength": 6, "maxLength": 15},
                    "affinity_cutoff_nm": {"type": "integer", "minimum": 1, "maximum": 10000000, "default": 10000},
                    "max_rows": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }
            }),
        ),
        tool(
            "bindingdb_targets_by_compound",
            "Protein targets with measured BindingDB affinities for compounds 2D-similar to a query SMILES. Similarity is Tanimoto 0.5–1.0. `api_hit_count` is BindingDB's matching-compound count and is not row-for-row comparable with `n_rows_total` (several measurements per compound). No hits is an empty page.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["smiles"],
                "properties": {
                    "smiles": {"type": "string", "minLength": 1, "maxLength": 8192},
                    "similarity": {"type": "number", "minimum": 0.5, "maximum": 1.0, "default": 0.85},
                    "max_rows": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                }
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, parameters: Value) -> (&'static str, ToolSchema) {
    ("chemistry", ToolSchema::new(name, description, parameters))
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("chemistry request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "pubchem_search_compounds" => pubchem::search_compounds(bio, args).await,
        "pubchem_get_compounds" => pubchem::get_compounds(bio, args).await,
        "pubchem_similarity_search" => pubchem::similarity_search(bio, args).await,
        "pubchem_get_bioassay_summary" => pubchem::get_bioassay_summary(bio, args).await,
        "pubchem_get_safety" => pubchem::get_safety(bio, args).await,
        "chebi_search" => chebi::search(bio, args).await,
        "chebi_get_entity" => chebi::get_entity(bio, args).await,
        "chebi_get_ontology" => chebi::get_ontology(bio, args).await,
        "rhea_search_reactions" => rhea::search_reactions(bio, args).await,
        "rhea_get_reaction" => rhea::get_reaction(bio, args).await,
        "bindingdb_ligands_by_target" => bindingdb::ligands_by_target(bio, args).await,
        "bindingdb_targets_by_compound" => bindingdb::targets_by_compound(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

pub(super) fn pubchem_pug(bio: &NativeBio) -> String {
    endpoint(bio, "WISP_BIO_TEST_PUBCHEM", PUBCHEM_PUG)
}

pub(super) fn pubchem_view(bio: &NativeBio) -> String {
    endpoint(bio, "WISP_BIO_TEST_PUBCHEM_VIEW", PUBCHEM_VIEW)
}

pub(super) fn chebi_base(bio: &NativeBio) -> String {
    endpoint(bio, "WISP_BIO_TEST_CHEBI", CHEBI_PUBLIC)
}

pub(super) fn rhea_endpoint(bio: &NativeBio) -> String {
    endpoint(bio, "WISP_BIO_TEST_RHEA", RHEA_SPARQL)
}

pub(super) fn bindingdb_base(bio: &NativeBio) -> String {
    endpoint(bio, "WISP_BIO_TEST_BINDINGDB", BINDINGDB_REST)
}

fn endpoint(bio: &NativeBio, test_key: &str, production: &str) -> String {
    #[cfg(test)]
    if let Some(base) = bio.credential(test_key) {
        if !base.is_empty() {
            return trim_slash(base);
        }
    }
    #[cfg(not(test))]
    {
        let _ = (bio, test_key);
    }
    trim_slash(production)
}

pub(super) fn trim_slash(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

pub(super) fn join_url(base: &str, path: &str) -> String {
    format!("{}/{}", trim_slash(base), path.trim_start_matches('/'))
}

pub(super) fn ncbi_identity(bio: &NativeBio) -> Vec<(String, String)> {
    let mut params = vec![("tool".into(), "wisp-science".into())];
    if let Some(email) = bio.credential("NCBI_EMAIL") {
        params.push(("email".into(), email.to_string()));
    }
    params
}

pub(super) async fn send(
    bio: &NativeBio,
    source: Source,
    method: Method,
    url: &str,
    params: &[(String, String)],
) -> Result<crate::http::Response> {
    bio.http().send(source, method, url, params).await
}

pub(super) async fn send_json(
    bio: &NativeBio,
    source: Source,
    method: Method,
    url: &str,
    params: &[(String, String)],
) -> Result<Option<Value>> {
    let response = send(bio, source, method, url, params).await?;
    if response.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(response.json()?))
}

pub(super) fn require_text<'a>(value: &'a str, field: &str, max: usize) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() || value.len() > max {
        bail!("{field} must contain 1 to {max} bytes of text");
    }
    Ok(value)
}

pub(super) fn require_range(value: usize, min: usize, max: usize, field: &str) -> Result<usize> {
    if !(min..=max).contains(&value) {
        bail!("{field} must be between {min} and {max}");
    }
    Ok(value)
}

pub(super) fn require_positive_id(id: u64, field: &str) -> Result<u64> {
    if id == 0 {
        bail!("{field} must be a positive integer");
    }
    Ok(id)
}

pub(super) fn cap<T: Clone>(items: &[T], max: usize) -> (Vec<T>, bool) {
    if items.len() > max {
        (items[..max].to_vec(), true)
    } else {
        (items.to_vec(), false)
    }
}

pub(super) fn json_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

pub(super) fn json_plain(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

pub(super) fn json_opt(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
}

pub(super) fn as_object_array(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item) if item.is_object() => vec![item],
        _ => Vec::new(),
    }
}
