//! Native `genes-ontologies` domain against MyGene.info, UniProt, OLS4,
//! QuickGO, the Reactome Analysis Service and KEGG REST. Independently
//! implemented from:
//!
//! - [MyGene.info v3 query](https://docs.mygene.info/en/latest/doc/query_service.html)
//! - [UniProt REST entry and search](https://www.uniprot.org/help/api_retrieve_entries)
//! - [UniProtKB return fields](https://www.uniprot.org/help/return_fields)
//! - [OLS4 REST API](https://www.ebi.ac.uk/ols4/ols3help)
//! - [QuickGO REST API](https://www.ebi.ac.uk/QuickGO/api/index.html)
//! - [Reactome Analysis Service](https://reactome.org/AnalysisService)
//! - [KEGG API Manual](https://www.kegg.jp/kegg/rest/keggapi.html)
//! - [KEGG copyright and disclaimer](https://www.kegg.jp/kegg/legal.html)
//!
//! References reviewed 2026-09-06. Tests use invented records.

mod kegg;
#[cfg(test)]
mod tests;

use crate::http::{Source, MAX_RESPONSE};
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Duration;
use wisp_llm::ToolSchema;

const MYGENE: Source = Source("MyGene.info", Duration::from_millis(300));
const UNIPROT: Source = Source("UniProt", Duration::from_millis(300));
const OLS: Source = Source("OLS", Duration::from_millis(300));
const QUICKGO: Source = Source("QuickGO", Duration::from_millis(300));
const REACTOME: Source = Source("Reactome", Duration::from_millis(300));

const MYGENE_HOST: &str = "https://mygene.info";
const UNIPROT_HOST: &str = "https://rest.uniprot.org";
const UNIPROT_PAGE: &str = "https://www.uniprot.org";
const OLS_HOST: &str = "https://www.ebi.ac.uk/ols4";
const QUICKGO_HOST: &str = "https://www.ebi.ac.uk/QuickGO";
const REACTOME_HOST: &str = "https://reactome.org";

const MAX_GENE_TERMS: usize = 200;
const MAX_UNIPROT: usize = 50;
const MAX_OLS_IDS: usize = 40;
const MAX_RELATED: usize = 200;
const MAX_REACTOME: usize = 25;
const MAX_SEARCH: u32 = 100;
const DEFAULT_SEARCH: u32 = 25;
const MAX_GO_RECORDS: u32 = 500;
const DEFAULT_GO_RECORDS: u32 = 100;
const QUICKGO_PAGE: u32 = 100;
const OLS_PAGE: u32 = 50;
const OLS_MAX_PAGES: u32 = 20;
const REACTOME_PATHWAYS: u32 = 50;

const RELATIONS: &[&str] = &[
    "parents",
    "children",
    "ancestors",
    "descendants",
    "hierarchicalParents",
    "hierarchicalChildren",
    "hierarchicalAncestors",
    "hierarchicalDescendants",
];

const REACTOME_RESOURCES: &[&str] = &[
    "TOTAL",
    "UNIPROT",
    "ENSEMBL",
    "CHEBI",
    "IUPHAR",
    "MIRBASE",
    "NCBI_PROTEIN",
    "EMBL",
    "COMPOUND",
    "PUBCHEM_COMPOUND",
];

const DOMAIN: &str = "genes-ontologies";

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "get_go_annotations",
            "Retrieve Gene Ontology annotations for one UniProt accession from EMBL-EBI QuickGO (GET /services/annotation/search). Optional filters: aspect (biological_process, molecular_function, cellular_component), evidence (experimental_manual, automatic_iea, or an ECO identifier such as ECO:0000314), and NCBI taxon_id. Three-letter GO evidence codes are rejected because QuickGO filters on ECO identifiers. The response is a bounded page (default 100, at most 500) and reports the upstream hit total; a capped page is not the complete annotation set. Each record includes a QuickGO term URL.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["uniprot_accession"],
                "properties": {
                    "uniprot_accession": {"type": "string", "minLength": 6, "maxLength": 32},
                    "aspect": {"type": "string", "enum": ["biological_process", "molecular_function", "cellular_component"]},
                    "evidence": {"type": "string", "minLength": 3, "maxLength": 32},
                    "taxon_id": {"type": "integer", "minimum": 1, "maximum": 99999999},
                    "include_term_names": {"type": "boolean", "default": true},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100}
                }
            }),
        ),
        tool(
            "get_kegg_entries",
            concat!(
                "Retrieve KEGG entries (genes, pathways, compounds and other databases) via GET /get/{id1}+{id2}+... as official flat-file records (https://www.kegg.jp/kegg/rest/keggapi.html). At most 10 identifiers per HTTP request, batched sequentially; at most 50 unique ids per call. Parses ENTRY, NAME, SYMBOL, DEFINITION/DESCRIPTION, ORGANISM, FORMULA, PATHWAY and ORTHOLOGY. An identifier with no returned record is an error. include_raw adds the flat-file chunk including the /// terminator. Each record includes a KEGG entry URL.",
                " KEGG REST is for academic use of the KEGG website/API; non-academic use requires a commercial license from Pathway Solutions (https://www.kegg.jp/kegg/legal.html). Academic users who provide KEGG as a service need the academic service-provider license. This client queries rest.kegg.jp; it does not redistribute the KEGG database."
            ),
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["ids"],
                "properties": {
                    "ids": {
                        "type": "array", "minItems": 1, "maxItems": 50,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "include_raw": {"type": "boolean", "default": false}
                }
            }),
        ),
        tool(
            "get_ontology_term",
            "Look up one OLS4 term by ontology id plus CURIE, short form or IRI (GET /api/ontologies/{id}/terms). Without relation: label, synonyms, description, obsolete flag and optional direct parents. With relation: a bounded page of related terms (parents, children, ancestors, descendants, or the hierarchical* variants). Related-term retrieval reports the OLS page total; a capped page is not the complete neighbourhood. Unknown terms are errors, not empty evidence.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["ontology", "term_id"],
                "properties": {
                    "ontology": {"type": "string", "minLength": 1, "maxLength": 64},
                    "term_id": {"type": "string", "minLength": 1, "maxLength": 512},
                    "relation": {"type": "string", "enum": [
                        "parents", "children", "ancestors", "descendants",
                        "hierarchicalParents", "hierarchicalChildren",
                        "hierarchicalAncestors", "hierarchicalDescendants"
                    ]},
                    "include_parents": {"type": "boolean", "default": true}
                }
            }),
        ),
        tool(
            "get_uniprot_entries",
            "Retrieve UniProtKB records for a batch of accessions through rest.uniprot.org search (one OR-query, at most 50 accessions). fields selects TSV columns (accession is always included). Without fields, format is fasta (sequences) or txt (UniProt flat file). Accessions with no returned record are listed in missing; merged or deleted accessions land there. Each record includes a UniProt entry URL. A search page is not a proteome download.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accessions"],
                "properties": {
                    "accessions": {
                        "type": "array", "minItems": 1, "maxItems": 50,
                        "items": {"type": "string", "minLength": 6, "maxLength": 32}
                    },
                    "format": {"type": "string", "enum": ["fasta", "txt"], "default": "fasta"},
                    "fields": {
                        "type": "array", "minItems": 1, "maxItems": 20,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    }
                }
            }),
        ),
        tool(
            "link_kegg_ids",
            concat!(
                "Cross-reference KEGG identifiers with GET /link/{target_db}/{ids} or convert outside identifiers with GET /conv/{target_db}/{ids} (https://www.kegg.jp/kegg/rest/keggapi.html). Requires 1 to 50 explicit ids; database-wide dumps are not supported. Batches 10 ids per request. Two-column tab text is returned as source_id/target_id using the caller's id spelling. Inputs with zero hits appear in missing_ids; an empty mapping is success.",
                " KEGG REST is for academic use of the KEGG website/API; non-academic use requires a commercial license from Pathway Solutions (https://www.kegg.jp/kegg/legal.html). Academic users who provide KEGG as a service need the academic service-provider license. This client queries rest.kegg.jp; it does not redistribute the KEGG database."
            ),
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["ids", "target_db"],
                "properties": {
                    "ids": {
                        "type": "array", "minItems": 1, "maxItems": 50,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "target_db": {"type": "string", "minLength": 1, "maxLength": 32},
                    "operation": {"type": "string", "enum": ["link", "conv"], "default": "link"}
                }
            }),
        ),
        tool(
            "list_ontologies",
            "List ontologies in the EMBL-EBI Ontology Lookup Service (OLS4 GET /api/ontologies). With ontology_ids: fetch those catalogue records and list unknown ids in not_found. Without: a bounded catalogue page that reports OLS totalElements and whether the returned set is complete. Each record includes the OLS ontology URL.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "ontology_ids": {
                        "type": "array", "minItems": 1, "maxItems": 40,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    }
                }
            }),
        ),
        tool(
            "map_reactome_pathways",
            "Map gene symbols or UniProt accessions to Reactome pathways via AnalysisService (POST /identifiers/ with a text/plain identifier list). resource selects the molecule view (TOTAL default; UNIPROT for protein-level mappings). include_disease follows the service default (true). compact=true returns per-identifier low-level pathways with Reactome URLs; compact=false adds per-pathway entity/reaction statistics, the analysis token and identifiers that did not map. The pathway list is a bounded page (50) and is not the complete hit set when pathwaysFound is larger. At most 25 identifiers per call.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["identifiers"],
                "properties": {
                    "identifiers": {
                        "type": "array", "minItems": 1, "maxItems": 25,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "id_type": {"type": "string", "enum": ["symbol", "uniprot"], "default": "symbol"},
                    "species": {"type": "string", "minLength": 1, "maxLength": 64, "default": "Homo sapiens"},
                    "resource": {"type": "string", "enum": [
                        "TOTAL", "UNIPROT", "ENSEMBL", "CHEBI", "IUPHAR",
                        "MIRBASE", "NCBI_PROTEIN", "EMBL", "COMPOUND", "PUBCHEM_COMPOUND"
                    ], "default": "TOTAL"},
                    "include_disease": {"type": "boolean", "default": true},
                    "compact": {"type": "boolean", "default": true}
                }
            }),
        ),
        tool(
            "query_genes",
            "Resolve gene symbols or identifiers through MyGene.info v3 batch query (POST /v3/query). Terms are sent in one request (at most 200; commas inside a term are not supported). scopes selects identifier namespaces (default symbol). fields limits returned annotation fields. species accepts a common name or NCBI taxid (default human). Terms with no match are listed in missing_terms; a term that matches several genes yields several records. Each hit includes a MyGene gene URL. This does not download sequences.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["terms"],
                "properties": {
                    "terms": {
                        "type": "array", "minItems": 1, "maxItems": 200,
                        "items": {"type": "string", "minLength": 1, "maxLength": 128}
                    },
                    "scopes": {"type": "string", "minLength": 1, "maxLength": 256, "default": "symbol"},
                    "fields": {"type": "string", "minLength": 1, "maxLength": 512, "default": "symbol,name,taxid,entrezgene,ensembl.gene"},
                    "species": {"type": "string", "minLength": 1, "maxLength": 64, "default": "human"}
                }
            }),
        ),
        tool(
            "search_kegg",
            concat!(
                "Search KEGG identifier and name fields via GET /find/{database}/{query} (substring match; https://www.kegg.jp/kegg/rest/keggapi.html). database defaults to hsa; use an organism code or a find-database from the KEGG API manual. option formula, exact_mass or mol_weight is allowed only for compound or drug. exact_gene_symbol keeps rows whose comma-separated symbol list (tokens before the first semicolon in the description) contains the query as a whole token, case-insensitive. No hits is success. The page is bounded (default 50, at most 200); truncated means further hits exist.",
                " KEGG REST is for academic use of the KEGG website/API; non-academic use requires a commercial license from Pathway Solutions (https://www.kegg.jp/kegg/legal.html). Academic users who provide KEGG as a service need the academic service-provider license. This client queries rest.kegg.jp; it does not redistribute the KEGG database."
            ),
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 256},
                    "database": {"type": "string", "minLength": 1, "maxLength": 32, "default": "hsa"},
                    "option": {"type": "string", "enum": ["formula", "exact_mass", "mol_weight"]},
                    "exact_gene_symbol": {"type": "boolean", "default": false},
                    "max_hits": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "search_ontology_terms",
            "Search OLS4 term labels and synonyms (GET /api/search). Restrict with ontologies (lowercase OLS ids such as go, efo, cl, chebi). exact requests a whole-string match; include_obsolete adds obsolete terms. The response is a bounded page (default 25, at most 100) and reports numFound versus returned; a capped page is not the complete hit list. Each term includes CURIE, IRI, ontology and an OLS class URL.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 256},
                    "ontologies": {
                        "type": "array", "minItems": 1, "maxItems": 20,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "exact": {"type": "boolean", "default": false},
                    "include_obsolete": {"type": "boolean", "default": false},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
    ]
}

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    (DOMAIN, ToolSchema::new(name, description, parameters))
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("genes-ontologies request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "query_genes" => query_genes(bio, args).await,
        "get_uniprot_entries" => get_uniprot_entries(bio, args).await,
        "get_go_annotations" => get_go_annotations(bio, args).await,
        "get_kegg_entries" => kegg::get_kegg_entries(bio, args).await,
        "get_ontology_term" => get_ontology_term(bio, args).await,
        "link_kegg_ids" => kegg::link_kegg_ids(bio, args).await,
        "list_ontologies" => list_ontologies(bio, args).await,
        "map_reactome_pathways" => map_reactome_pathways(bio, args).await,
        "search_kegg" => kegg::search_kegg(bio, args).await,
        "search_ontology_terms" => search_ontology_terms(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryGenes {
    terms: Vec<String>,
    #[serde(default = "default_scopes")]
    scopes: String,
    #[serde(default = "default_gene_fields")]
    fields: String,
    #[serde(default = "default_gene_species")]
    species: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetUniprot {
    accessions: Vec<String>,
    #[serde(default = "default_format")]
    format: String,
    fields: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetGo {
    uniprot_accession: String,
    aspect: Option<String>,
    evidence: Option<String>,
    taxon_id: Option<i64>,
    #[serde(default = "default_true")]
    include_term_names: bool,
    #[serde(default = "default_go_max")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetTerm {
    ontology: String,
    term_id: String,
    relation: Option<String>,
    #[serde(default = "default_true")]
    include_parents: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListOntologies {
    ontology_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapReactome {
    identifiers: Vec<String>,
    #[serde(default = "default_id_type")]
    id_type: String,
    #[serde(default = "default_reactome_species")]
    species: String,
    #[serde(default = "default_resource")]
    resource: String,
    #[serde(default = "default_true")]
    include_disease: bool,
    #[serde(default = "default_true")]
    compact: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchTerms {
    query: String,
    ontologies: Option<Vec<String>>,
    #[serde(default)]
    exact: bool,
    #[serde(default)]
    include_obsolete: bool,
    #[serde(default = "default_search_max")]
    max_results: u32,
}

fn default_scopes() -> String {
    "symbol".into()
}
fn default_gene_fields() -> String {
    "symbol,name,taxid,entrezgene,ensembl.gene".into()
}
fn default_gene_species() -> String {
    "human".into()
}
fn default_format() -> String {
    "fasta".into()
}
fn default_true() -> bool {
    true
}
fn default_go_max() -> u32 {
    DEFAULT_GO_RECORDS
}
fn default_search_max() -> u32 {
    DEFAULT_SEARCH
}
fn default_id_type() -> String {
    "symbol".into()
}
fn default_reactome_species() -> String {
    "Homo sapiens".into()
}
fn default_resource() -> String {
    "TOTAL".into()
}

async fn query_genes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: QueryGenes =
        serde_json::from_value(args.clone()).context("invalid query_genes arguments")?;
    let terms = require_terms(&args.terms, MAX_GENE_TERMS, "gene term")?;
    let scopes = csv_param(&args.scopes, "scopes")?;
    let fields = csv_param(&args.fields, "fields")?;
    let species = token_param(&args.species, 64, "species")?;
    let raw = json_request(
        bio,
        MYGENE,
        Method::POST,
        &format!("{}/v3/query", mygene_base(bio)),
        &[
            ("q".into(), terms.join(",")),
            ("scopes".into(), scopes.clone()),
            ("fields".into(), fields.clone()),
            ("species".into(), species.clone()),
        ],
    )
    .await?;
    let hits = match raw {
        Value::Array(rows) => rows,
        Value::Object(map) if map.contains_key("error") => {
            bail!(
                "MyGene.info rejected the query ({})",
                map.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("upstream error")
            )
        }
        _ => bail!("MyGene.info returned an unrecognized batch result (expected a JSON array)"),
    };
    let mut records = Vec::new();
    let mut missing = BTreeSet::new();
    let mut seen_missing = HashSet::new();
    for hit in hits {
        let query = hit
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if hit.get("notfound").and_then(Value::as_bool) == Some(true) {
            if seen_missing.insert(query.clone()) {
                missing.insert(query);
            }
            continue;
        }
        let id = hit
            .get("_id")
            .and_then(|v| match v {
                Value::String(text) => Some(text.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_default();
        let mut record = hit;
        if !id.is_empty() {
            record["url"] = json!(format!("{MYGENE_HOST}/v3/gene/{}", path_segment(&id)));
        }
        records.push(record);
    }
    Ok(json!({
        "source": "MyGene.info",
        "source_url": format!("{MYGENE_HOST}/v3/query"),
        "query": {"terms": terms, "scopes": scopes, "fields": fields, "species": species},
        "n_input": terms.len(),
        "returned": records.len(),
        "missing_terms": missing.into_iter().collect::<Vec<_>>(),
        "records": records
    }))
}

async fn get_uniprot_entries(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetUniprot =
        serde_json::from_value(args.clone()).context("invalid get_uniprot_entries arguments")?;
    let accessions = require_uniprot(&args.accessions)?;
    if let Some(fields) = args.fields.as_ref() {
        let fields = require_uniprot_fields(fields)?;
        return uniprot_tsv(bio, &accessions, &fields).await;
    }
    match args.format.as_str() {
        "fasta" => uniprot_fasta(bio, &accessions).await,
        "txt" => uniprot_txt(bio, &accessions).await,
        other => bail!("format must be fasta or txt (got {other})"),
    }
}

async fn uniprot_tsv(bio: &NativeBio, accessions: &[String], fields: &[String]) -> Result<Value> {
    let mut request_fields = fields.to_vec();
    if !request_fields.iter().any(|f| f == "accession") {
        request_fields.insert(0, "accession".into());
    }
    let text = uniprot_search(bio, accessions, "tsv", Some(&request_fields.join(","))).await?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| anyhow!("UniProt TSV response had no header row"))?;
    let columns: Vec<String> = header.split('\t').map(|c| c.to_string()).collect();
    let keys: Vec<String> = columns.iter().map(|c| tsv_field_name(c)).collect();
    let mut records = Vec::new();
    let mut found = HashSet::new();
    for line in lines {
        if line == header {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let mut row = serde_json::Map::new();
        for (i, key) in keys.iter().enumerate() {
            row.insert(key.clone(), json!(cols.get(i).copied().unwrap_or("")));
        }
        let acc = row
            .get("accession")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !acc.is_empty() {
            row.insert(
                "url".into(),
                json!(format!("{UNIPROT_PAGE}/uniprotkb/{acc}")),
            );
            found.insert(acc);
        }
        records.push(Value::Object(row));
    }
    let missing: Vec<String> = accessions
        .iter()
        .filter(|acc| !found.contains(*acc))
        .cloned()
        .collect();
    Ok(json!({
        "source": "UniProt",
        "source_url": format!("{UNIPROT_HOST}/uniprotkb/search"),
        "accessions": accessions,
        "fields": fields,
        "returned": records.len(),
        "missing": missing,
        "records": records
    }))
}

async fn uniprot_fasta(bio: &NativeBio, accessions: &[String]) -> Result<Value> {
    let text = uniprot_search(bio, accessions, "fasta", None).await?;
    let parsed = parse_fasta(&text)?;
    uniprot_text_result(accessions, "fasta", parsed)
}

async fn uniprot_txt(bio: &NativeBio, accessions: &[String]) -> Result<Value> {
    let text = uniprot_search(bio, accessions, "txt", None).await?;
    let parsed = parse_uniprot_txt(&text)?;
    uniprot_text_result(accessions, "txt", parsed)
}

fn uniprot_text_result(
    accessions: &[String],
    format: &str,
    parsed: BTreeMap<String, String>,
) -> Result<Value> {
    let mut records = serde_json::Map::new();
    let mut urls = serde_json::Map::new();
    for acc in accessions {
        if let Some(text) = parsed.get(acc) {
            records.insert(acc.clone(), json!(text));
            urls.insert(
                acc.clone(),
                json!(format!("{UNIPROT_PAGE}/uniprotkb/{acc}")),
            );
        }
    }
    let missing: Vec<String> = accessions
        .iter()
        .filter(|acc| !parsed.contains_key(*acc))
        .cloned()
        .collect();
    Ok(json!({
        "source": "UniProt",
        "source_url": format!("{UNIPROT_HOST}/uniprotkb/search"),
        "accessions": accessions,
        "format": format,
        "n_found": records.len(),
        "missing": missing,
        "urls": urls,
        "records": records
    }))
}

async fn uniprot_search(
    bio: &NativeBio,
    accessions: &[String],
    format: &str,
    fields: Option<&str>,
) -> Result<String> {
    let query = accessions
        .iter()
        .map(|acc| format!("(accession_id:{acc})"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let mut params = vec![
        ("query".into(), query),
        ("format".into(), format.into()),
        ("size".into(), accessions.len().to_string()),
    ];
    if let Some(fields) = fields {
        params.push(("fields".into(), fields.into()));
    }
    text_request(
        bio,
        UNIPROT,
        Method::GET,
        &format!("{}/uniprotkb/search", uniprot_base(bio)),
        &params,
    )
    .await
}

async fn get_go_annotations(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetGo =
        serde_json::from_value(args.clone()).context("invalid get_go_annotations arguments")?;
    let accession = parse_uniprot_one(&args.uniprot_accession)?;
    let cap = bound_u32(args.max_records, 1, MAX_GO_RECORDS, "max_records")?;
    if let Some(aspect) = args.aspect.as_deref() {
        if !matches!(
            aspect,
            "biological_process" | "molecular_function" | "cellular_component"
        ) {
            bail!("aspect must be biological_process, molecular_function or cellular_component");
        }
    }
    if let Some(taxon) = args.taxon_id {
        if !(1..=99_999_999).contains(&taxon) {
            bail!("taxon_id must be a positive NCBI taxonomy identifier");
        }
    }
    let evidence = args
        .evidence
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(evidence_codes)
        .transpose()?;
    let mut records = Vec::new();
    let mut total = 0u64;
    let mut page = 1u32;
    loop {
        let remaining = cap.saturating_sub(records.len() as u32);
        if remaining == 0 {
            break;
        }
        let limit = remaining.min(QUICKGO_PAGE);
        let mut params = vec![
            ("geneProductId".into(), accession.clone()),
            ("limit".into(), limit.to_string()),
            ("page".into(), page.to_string()),
        ];
        if args.include_term_names {
            params.push(("includeFields".into(), "goName".into()));
        }
        if let Some(aspect) = args.aspect.as_deref() {
            params.push(("aspect".into(), aspect.into()));
        }
        if let Some(codes) = evidence.as_deref() {
            params.push(("evidenceCode".into(), codes.into()));
        }
        if let Some(taxon) = args.taxon_id {
            params.push(("taxonId".into(), taxon.to_string()));
        }
        let raw = json_request(
            bio,
            QUICKGO,
            Method::GET,
            &format!("{}/services/annotation/search", quickgo_base(bio)),
            &params,
        )
        .await?;
        total = raw
            .get("numberOfHits")
            .and_then(Value::as_u64)
            .unwrap_or(total);
        let page_rows = raw
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if page_rows.is_empty() {
            break;
        }
        for row in page_rows {
            records.push(project_go_annotation(&row, &accession));
            if records.len() as u32 >= cap {
                break;
            }
        }
        let pages = raw
            .get("pageInfo")
            .and_then(|info| info.get("total"))
            .and_then(Value::as_u64)
            .unwrap_or(1);
        if u64::from(page) >= pages {
            break;
        }
        page += 1;
        if page > 20 {
            break;
        }
    }
    if args.include_term_names {
        hydrate_go_names(bio, &mut records).await?;
    }
    let mut go_ids = BTreeSet::new();
    for record in &records {
        if let Some(id) = record.get("go_id").and_then(Value::as_str) {
            go_ids.insert(id.to_string());
        }
    }
    let truncated = total > records.len() as u64;
    Ok(json!({
        "source": "QuickGO",
        "source_url": format!("{QUICKGO_HOST}/services/annotation/search"),
        "gene_product": accession,
        "gene_product_url": format!("{QUICKGO_HOST}/annotations?geneProductId={accession}"),
        "aspect": args.aspect,
        "evidence": args.evidence,
        "taxon_id": args.taxon_id,
        "total_annotations": total,
        "returned": records.len(),
        "truncated": truncated,
        "distinct_go_ids": go_ids.into_iter().collect::<Vec<_>>(),
        "records": records
    }))
}

fn project_go_annotation(row: &Value, accession: &str) -> Value {
    let go_id = row
        .get("goId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    json!({
        "go_id": go_id,
        "go_name": row.get("goName"),
        "go_aspect": row.get("goAspect"),
        "qualifier": row.get("qualifier"),
        "go_evidence": row.get("goEvidence"),
        "eco_id": row.get("evidenceCode"),
        "reference": row.get("reference"),
        "assigned_by": row.get("assignedBy"),
        "taxon_id": row.get("taxonId"),
        "symbol": row.get("symbol"),
        "date": row.get("date"),
        "gene_product_id": row.get("geneProductId").cloned().unwrap_or_else(|| json!(format!("UniProtKB:{accession}"))),
        "url": if go_id.is_empty() {
            Value::Null
        } else {
            json!(format!("{QUICKGO_HOST}/term/{go_id}"))
        }
    })
}

async fn hydrate_go_names(bio: &NativeBio, records: &mut [Value]) -> Result<()> {
    let mut needed: Vec<String> = records
        .iter()
        .filter(|row| row.get("go_name").and_then(Value::as_str).is_none())
        .filter_map(|row| row.get("go_id").and_then(Value::as_str).map(str::to_string))
        .filter(|id| !id.is_empty())
        .collect();
    needed.sort();
    needed.dedup();
    if needed.is_empty() {
        return Ok(());
    }
    let mut names: BTreeMap<String, Value> = BTreeMap::new();
    for chunk in needed.chunks(20) {
        let ids = chunk
            .iter()
            .map(|id| percent_encode(id))
            .collect::<Vec<_>>()
            .join(",");
        let raw = json_request(
            bio,
            QUICKGO,
            Method::GET,
            &format!("{}/services/ontology/go/terms/{ids}", quickgo_base(bio)),
            &[],
        )
        .await?;
        if let Some(results) = raw.get("results").and_then(Value::as_array) {
            for term in results {
                if let Some(id) = term.get("id").and_then(Value::as_str) {
                    names.insert(id.to_string(), term.clone());
                }
            }
        }
    }
    for record in records.iter_mut() {
        let Some(id) = record.get("go_id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(meta) = names.get(id) {
            if record.get("go_name").and_then(Value::as_str).is_none() {
                record["go_name"] = meta.get("name").cloned().unwrap_or(Value::Null);
            }
            record["go_is_obsolete"] = json!(meta.get("isObsolete").and_then(Value::as_bool));
        }
    }
    Ok(())
}

fn evidence_codes(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("experimental_manual") {
        return Ok(
            "ECO:0000269,ECO:0000314,ECO:0000353,ECO:0000315,ECO:0000316,ECO:0000270".into(),
        );
    }
    if trimmed.eq_ignore_ascii_case("automatic_iea") {
        return Ok("ECO:0000501".into());
    }
    if (trimmed.len() == 2 || trimmed.len() == 3)
        && trimmed.bytes().all(|b| b.is_ascii_alphabetic())
    {
        bail!(
            "evidence must be an ECO identifier (ECO:0000314) or a preset (experimental_manual, automatic_iea); three-letter GO evidence codes are not a QuickGO filter"
        );
    }
    if trimmed.len() > 32
        || !trimmed.starts_with("ECO:")
        || trimmed.len() <= 4
        || !trimmed[4..].bytes().all(|b| b.is_ascii_digit())
    {
        bail!("evidence must be experimental_manual, automatic_iea, or an ECO: identifier");
    }
    Ok(trimmed.to_string())
}

async fn list_ontologies(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListOntologies =
        serde_json::from_value(args.clone()).context("invalid list_ontologies arguments")?;
    if let Some(ids) = args.ontology_ids {
        let ids = require_ontology_ids(&ids, MAX_OLS_IDS)?;
        let mut records = Vec::new();
        let mut not_found = Vec::new();
        for id in &ids {
            let response = bio
                .http()
                .send(
                    OLS,
                    Method::GET,
                    &format!("{}/api/ontologies/{}", ols_base(bio), path_segment(id)),
                    &[],
                )
                .await?;
            if response.status == StatusCode::NOT_FOUND {
                not_found.push(id.clone());
                continue;
            }
            let raw = decode_json(OLS, response)?;
            records.push(project_ontology(&raw)?);
        }
        return Ok(json!({
            "source": "OLS4",
            "source_url": format!("{OLS_HOST}/api/ontologies"),
            "records": records,
            "not_found": not_found
        }));
    }
    let mut records = Vec::new();
    let mut total = 0u64;
    let mut page = 0u32;
    loop {
        let raw = json_request(
            bio,
            OLS,
            Method::GET,
            &format!("{}/api/ontologies", ols_base(bio)),
            &[
                ("page".into(), page.to_string()),
                ("size".into(), OLS_PAGE.to_string()),
            ],
        )
        .await?;
        total = raw
            .pointer("/page/totalElements")
            .and_then(Value::as_u64)
            .unwrap_or(total);
        let rows = raw
            .pointer("/_embedded/ontologies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if rows.is_empty() {
            break;
        }
        for row in rows {
            records.push(project_ontology(&row)?);
        }
        let pages = raw
            .pointer("/page/totalPages")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        page += 1;
        if u64::from(page) >= pages || page >= OLS_MAX_PAGES {
            break;
        }
    }
    let complete = records.len() as u64 >= total && total > 0 || total == 0 && records.is_empty();
    Ok(json!({
        "source": "OLS4",
        "source_url": format!("{OLS_HOST}/api/ontologies"),
        "total_elements": total,
        "returned": records.len(),
        "complete": complete,
        "records": records
    }))
}

fn project_ontology(raw: &Value) -> Result<Value> {
    let id = raw
        .get("ontologyId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("OLS ontology record missing ontologyId"))?;
    let config = raw.get("config").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "ontology_id": id,
        "title": first_string(&config, &["title"]).or_else(|| first_string(raw, &["title"])),
        "version": first_string(&config, &["version"]).or_else(|| first_string(raw, &["version"])),
        "status": raw.get("status"),
        "num_terms": raw.get("numberOfTerms"),
        "num_properties": raw.get("numberOfProperties"),
        "description": first_string(&config, &["description"]),
        "homepage": first_string(&config, &["homepage"]),
        "url": format!("{OLS_HOST}/ontologies/{id}")
    }))
}

async fn search_ontology_terms(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchTerms =
        serde_json::from_value(args.clone()).context("invalid search_ontology_terms arguments")?;
    let query = args.query.trim();
    if query.is_empty() || query.len() > 256 {
        bail!("query must contain 1 to 256 characters");
    }
    let cap = bound_u32(args.max_results, 1, MAX_SEARCH, "max_results")?;
    let ontologies = args
        .ontologies
        .as_ref()
        .map(|ids| require_ontology_ids(ids, 20))
        .transpose()?;
    let mut params = vec![
        ("q".into(), query.to_string()),
        ("rows".into(), cap.to_string()),
        ("start".into(), "0".into()),
    ];
    if let Some(ids) = ontologies.as_ref() {
        params.push(("ontology".into(), ids.join(",")));
    }
    if args.exact {
        params.push(("exact".into(), "true".into()));
        params.push(("queryFields".into(), "label,synonym".into()));
    }
    if args.include_obsolete {
        params.push(("obsoletes".into(), "true".into()));
    }
    let raw = json_request(
        bio,
        OLS,
        Method::GET,
        &format!("{}/api/search", ols_base(bio)),
        &params,
    )
    .await?;
    let docs = raw
        .pointer("/response/docs")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| anyhow!("OLS search response missing response.docs"))?;
    let total = raw
        .pointer("/response/numFound")
        .and_then(Value::as_u64)
        .unwrap_or(docs.len() as u64);
    let terms: Vec<Value> = docs.iter().map(project_search_term).collect();
    Ok(json!({
        "source": "OLS4",
        "source_url": format!("{OLS_HOST}/api/search"),
        "query": query,
        "ontologies": ontologies,
        "total_found": total,
        "returned": terms.len(),
        "truncated": total > terms.len() as u64,
        "terms": terms
    }))
}

fn project_search_term(doc: &Value) -> Value {
    let iri = doc.get("iri").and_then(Value::as_str).unwrap_or("");
    let ontology = doc
        .get("ontology_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    json!({
        "curie": doc.get("obo_id").cloned().unwrap_or_else(|| doc.get("short_form").cloned().unwrap_or(Value::Null)),
        "iri": doc.get("iri"),
        "label": doc.get("label"),
        "short_form": doc.get("short_form"),
        "ontology": ontology,
        "description": string_or_first(doc.get("description")),
        "type": doc.get("type"),
        "is_defining_ontology": doc.get("is_defining_ontology"),
        "url": ols_class_url(ontology, iri)
    })
}

async fn get_ontology_term(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetTerm =
        serde_json::from_value(args.clone()).context("invalid get_ontology_term arguments")?;
    let ontology = require_ontology_id(&args.ontology)?;
    let term_id = args.term_id.trim();
    if term_id.is_empty() || term_id.len() > 512 {
        bail!("term_id must contain 1 to 512 characters");
    }
    let term = lookup_term(bio, &ontology, term_id).await?;
    if let Some(relation) = args.relation.as_deref() {
        if !RELATIONS.contains(&relation) {
            bail!("relation must be one of {}", RELATIONS.join(", "));
        }
        return related_terms(bio, &ontology, &term, relation).await;
    }
    let mut out = project_term_detail(&term, &ontology);
    if args.include_parents {
        out["parents"] = json!(fetch_related_page(bio, &ontology, &term, "parents", 20).await?);
    }
    Ok(out)
}

async fn lookup_term(bio: &NativeBio, ontology: &str, term_id: &str) -> Result<Value> {
    if term_id.starts_with("http://") || term_id.starts_with("https://") {
        return fetch_term_by_iri(bio, ontology, term_id).await;
    }
    for (key, value) in [
        ("obo_id", term_id.to_string()),
        ("short_form", term_id.replace(':', "_")),
    ] {
        let raw = json_request(
            bio,
            OLS,
            Method::GET,
            &format!(
                "{}/api/ontologies/{}/terms",
                ols_base(bio),
                path_segment(ontology)
            ),
            &[("size".into(), "5".into()), (key.into(), value)],
        )
        .await?;
        if let Some(term) = raw
            .pointer("/_embedded/terms")
            .and_then(Value::as_array)
            .and_then(|terms| terms.first())
            .cloned()
        {
            return Ok(term);
        }
    }
    bail!("OLS has no term {term_id} in ontology {ontology}");
}

async fn fetch_term_by_iri(bio: &NativeBio, ontology: &str, iri: &str) -> Result<Value> {
    let encoded = double_encode(iri);
    let response = bio
        .http()
        .send(
            OLS,
            Method::GET,
            &format!(
                "{}/api/ontologies/{}/terms/{encoded}",
                ols_base(bio),
                path_segment(ontology)
            ),
            &[],
        )
        .await?;
    if response.status == StatusCode::NOT_FOUND {
        bail!("OLS has no term {iri} in ontology {ontology}");
    }
    decode_json(OLS, response)
}

async fn related_terms(
    bio: &NativeBio,
    ontology: &str,
    term: &Value,
    relation: &str,
) -> Result<Value> {
    let mut terms = Vec::new();
    let mut total = 0u64;
    let mut page = 0u32;
    loop {
        let remaining = MAX_RELATED.saturating_sub(terms.len());
        if remaining == 0 {
            break;
        }
        let size = remaining.min(50);
        let raw = related_request(bio, ontology, term, relation, page, size).await?;
        total = raw
            .pointer("/page/totalElements")
            .and_then(Value::as_u64)
            .unwrap_or(total);
        let rows = raw
            .pointer("/_embedded/terms")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if rows.is_empty() {
            break;
        }
        for row in rows {
            terms.push(project_related_term(&row, ontology));
            if terms.len() >= MAX_RELATED {
                break;
            }
        }
        let pages = raw
            .pointer("/page/totalPages")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        page += 1;
        if u64::from(page) >= pages {
            break;
        }
    }
    Ok(json!({
        "source": "OLS4",
        "source_url": format!("{OLS_HOST}/ontologies/{ontology}"),
        "root": project_term_detail(term, ontology),
        "relation": relation,
        "total_elements": total,
        "returned": terms.len(),
        "truncated": total > terms.len() as u64,
        "terms": terms
    }))
}

async fn fetch_related_page(
    bio: &NativeBio,
    ontology: &str,
    term: &Value,
    relation: &str,
    size: usize,
) -> Result<Vec<Value>> {
    let raw = related_request(bio, ontology, term, relation, 0, size).await?;
    let rows = raw
        .pointer("/_embedded/terms")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(rows
        .iter()
        .map(|row| project_related_term(row, ontology))
        .collect())
}

async fn related_request(
    bio: &NativeBio,
    ontology: &str,
    term: &Value,
    relation: &str,
    page: u32,
    size: usize,
) -> Result<Value> {
    let iri = term
        .get("iri")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("OLS term record missing iri"))?;
    json_request(
        bio,
        OLS,
        Method::GET,
        &format!(
            "{}/api/ontologies/{}/terms/{}/{relation}",
            ols_base(bio),
            path_segment(ontology),
            double_encode(iri)
        ),
        &[
            ("page".into(), page.to_string()),
            ("size".into(), size.to_string()),
        ],
    )
    .await
}

fn project_term_detail(term: &Value, ontology: &str) -> Value {
    let iri = term.get("iri").and_then(Value::as_str).unwrap_or("");
    json!({
        "source": "OLS4",
        "source_url": ols_class_url(ontology, iri),
        "curie": term.get("obo_id").cloned().unwrap_or_else(|| term.get("short_form").cloned().unwrap_or(Value::Null)),
        "iri": term.get("iri"),
        "label": term.get("label"),
        "ontology": ontology,
        "short_form": term.get("short_form"),
        "synonyms": term.get("synonyms").cloned().unwrap_or(Value::Null),
        "description": string_or_first(term.get("description")),
        "is_obsolete": term.get("is_obsolete").and_then(Value::as_bool).unwrap_or(false),
        "has_children": term.get("has_children").and_then(Value::as_bool).unwrap_or(false),
        "url": ols_class_url(ontology, iri)
    })
}

fn project_related_term(term: &Value, ontology: &str) -> Value {
    let iri = term.get("iri").and_then(Value::as_str).unwrap_or("");
    json!({
        "curie": term.get("obo_id").cloned().unwrap_or_else(|| term.get("short_form").cloned().unwrap_or(Value::Null)),
        "iri": term.get("iri"),
        "label": term.get("label"),
        "ontology": term.get("ontology_name").and_then(Value::as_str).unwrap_or(ontology),
        "url": ols_class_url(ontology, iri)
    })
}

fn ols_class_url(ontology: &str, iri: &str) -> String {
    if iri.is_empty() {
        format!("{OLS_HOST}/ontologies/{ontology}")
    } else {
        format!(
            "{OLS_HOST}/ontologies/{}/classes/{}",
            ontology,
            double_encode(iri)
        )
    }
}

async fn map_reactome_pathways(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MapReactome =
        serde_json::from_value(args.clone()).context("invalid map_reactome_pathways arguments")?;
    let identifiers = require_unique(&args.identifiers, MAX_REACTOME, "identifier")?;
    if args.id_type != "symbol" && args.id_type != "uniprot" {
        bail!("id_type must be symbol or uniprot");
    }
    if !REACTOME_RESOURCES.contains(&args.resource.as_str()) {
        bail!("resource is not a Reactome Analysis Service resource");
    }
    let species = args.species.trim();
    if species.is_empty() || species.len() > 64 {
        bail!("species must contain 1 to 64 characters");
    }
    let analysis = post_plain(
        bio,
        REACTOME,
        &format!("{}/AnalysisService/identifiers/", reactome_base(bio)),
        &[
            ("interactors".into(), "false".into()),
            ("pageSize".into(), REACTOME_PATHWAYS.to_string()),
            ("page".into(), "1".into()),
            ("sortBy".into(), "ENTITIES_FDR".into()),
            ("order".into(), "ASC".into()),
            ("resource".into(), args.resource.clone()),
            (
                "includeDisease".into(),
                if args.include_disease {
                    "true"
                } else {
                    "false"
                }
                .into(),
            ),
            ("species".into(), species.into()),
        ],
        identifiers.join("\n"),
    )
    .await?;
    let token = analysis
        .pointer("/summary/token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Reactome analysis response did not include a token"))?;
    let token = decode_analysis_token(token)?;
    let pathways_found = analysis
        .get("pathwaysFound")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let identifiers_not_found = analysis
        .get("identifiersNotFound")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let raw_pathways = analysis
        .get("pathways")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| anyhow!("Reactome analysis response missing pathways"))?;
    let projected: Vec<Value> = raw_pathways.iter().map(project_pathway).collect();
    let version = reactome_version(bio).await;
    let found_ids =
        load_found_identifiers(bio, &token, &args.resource, &projected, args.compact).await?;
    let mut missing = Vec::new();
    let mut genes = serde_json::Map::new();
    for id in &identifiers {
        let pathways = found_ids.get(id).cloned().unwrap_or_default();
        let found = !pathways.is_empty();
        if !found {
            missing.push(id.clone());
        }
        genes.insert(
            id.clone(),
            json!({
                "found": found,
                "n_pathways": pathways.len(),
                "pathways": pathways
            }),
        );
    }
    let mut out = json!({
        "source": "Reactome Analysis Service",
        "source_url": format!("{REACTOME_HOST}/AnalysisService"),
        "reactome_version": version,
        "id_type": args.id_type,
        "species": species,
        "resource": args.resource,
        "include_disease": args.include_disease,
        "n_input": identifiers.len(),
        "identifiers_not_found_count": identifiers_not_found,
        "missing_identifiers": missing,
        "pathways_found": pathways_found,
        "returned_pathways": projected.len(),
        "truncated": pathways_found > projected.len() as u64,
        "genes": genes
    });
    if !args.compact {
        out["token"] = json!(token);
        out["pathways"] = json!(projected);
        out["browser_url"] = json!(format!(
            "{REACTOME_HOST}/PathwayBrowser/#DTAB=AN&ANALYSIS={}",
            path_segment(&token)
        ));
    }
    Ok(out)
}

// The service can percent-encode base64 padding. Decode once before URL encoding
// a path segment, and keep delimiters/path traversal out of the accepted alphabet.
fn decode_analysis_token(raw: &str) -> Result<String> {
    if raw.is_empty() || raw.len() > 200 || !raw.is_ascii() {
        bail!("Reactome returned an invalid analysis token");
    }
    let mut out = String::new();
    let mut i = 0;
    while i < raw.len() {
        let byte = if raw.as_bytes()[i] == b'%' {
            let hex = raw
                .get(i + 1..i + 3)
                .context("Reactome returned an invalid analysis token")?;
            i += 3;
            u8::from_str_radix(hex, 16).context("Reactome returned an invalid analysis token")?
        } else {
            let byte = raw.as_bytes()[i];
            i += 1;
            byte
        };
        if !(byte.is_ascii_alphanumeric() || b"-_/+=".contains(&byte)) {
            bail!("Reactome returned an invalid analysis token");
        }
        out.push(char::from(byte));
    }
    Ok(out)
}

fn project_pathway(raw: &Value) -> Value {
    let st_id = raw.get("stId").and_then(Value::as_str).unwrap_or("");
    let species = raw
        .get("species")
        .and_then(|s| s.get("name"))
        .cloned()
        .unwrap_or(Value::Null);
    let entities = raw.get("entities").cloned().unwrap_or(Value::Null);
    json!({
        "stId": st_id,
        "name": raw.get("name"),
        "species": species,
        "llp": raw.get("llp").and_then(Value::as_bool).unwrap_or(false),
        "in_disease": raw.get("inDisease").and_then(Value::as_bool).unwrap_or(false),
        "entities_found": entities.get("found"),
        "entities_total": entities.get("total"),
        "entities_fdr": entities.get("fdr"),
        "entities_pvalue": entities.get("pValue").cloned().or_else(|| entities.get("getpValue").cloned()),
        "reactions_found": raw.pointer("/reactions/found"),
        "reactions_total": raw.pointer("/reactions/total"),
        "url": format!("{REACTOME_HOST}/content/detail/{st_id}")
    })
}

async fn load_found_identifiers(
    bio: &NativeBio,
    token: &str,
    resource: &str,
    pathways: &[Value],
    compact: bool,
) -> Result<BTreeMap<String, Vec<Value>>> {
    let mut wanted: Vec<&Value> = pathways
        .iter()
        .filter(|p| !compact || p.get("llp").and_then(Value::as_bool) == Some(true))
        .collect();
    if wanted.is_empty() {
        wanted = pathways.iter().collect();
    }
    let ids: Vec<String> = wanted
        .iter()
        .filter_map(|p| p.get("stId").and_then(Value::as_str).map(str::to_string))
        .filter(|id| !id.is_empty())
        .collect();
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let by_st: BTreeMap<String, Value> = wanted
        .into_iter()
        .filter_map(|p| {
            p.get("stId")
                .and_then(Value::as_str)
                .map(|id| (id.to_string(), compact_pathway(p)))
        })
        .collect();
    let found = post_plain(
        bio,
        REACTOME,
        &format!(
            "{}/AnalysisService/token/{}/found/all",
            reactome_base(bio),
            path_segment(token)
        ),
        &[("resource".into(), resource.into())],
        ids.join(","),
    )
    .await?;
    let rows = match found {
        Value::Array(rows) => rows,
        Value::Object(_) => vec![found],
        _ => bail!("Reactome found/all response was not a JSON array"),
    };
    let mut map: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in rows {
        let pathway_id = row
            .get("pathway")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let summary = by_st.get(&pathway_id).cloned().unwrap_or_else(|| {
            json!({
                "stId": pathway_id,
                "url": format!("{REACTOME_HOST}/content/detail/{pathway_id}")
            })
        });
        let entities = row
            .get("entities")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for entity in entities {
            if let Some(id) = entity.get("id").and_then(Value::as_str) {
                map.entry(id.to_string()).or_default().push(summary.clone());
            }
        }
    }
    Ok(map)
}

fn compact_pathway(raw: &Value) -> Value {
    json!({
        "stId": raw.get("stId"),
        "name": raw.get("name"),
        "species": raw.get("species"),
        "url": raw.get("url")
    })
}

async fn reactome_version(bio: &NativeBio) -> Value {
    match text_request(
        bio,
        REACTOME,
        Method::GET,
        &format!("{}/AnalysisService/database/version", reactome_base(bio)),
        &[],
    )
    .await
    {
        Ok(text) => json!(text.trim().trim_matches('"')),
        Err(_) => Value::Null,
    }
}

async fn json_request(
    bio: &NativeBio,
    source: Source,
    method: Method,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    let response = bio.http().send(source, method, url, params).await?;
    decode_json(source, response)
}

fn decode_json(source: Source, response: crate::http::Response) -> Result<Value> {
    response.check()?;
    if looks_like_html(&response.body) {
        bail!("{} returned HTML instead of JSON", source.0);
    }
    serde_json::from_slice(&response.body)
        .with_context(|| format!("{} returned invalid JSON", source.0))
}

async fn text_request(
    bio: &NativeBio,
    source: Source,
    method: Method,
    url: &str,
    params: &[(String, String)],
) -> Result<String> {
    let response = bio.http().send(source, method, url, params).await?;
    response.check()?;
    if looks_like_html(&response.body) {
        bail!("{} returned HTML instead of text", source.0);
    }
    String::from_utf8(response.body).context(format!("{} returned invalid UTF-8", source.0))
}

async fn post_plain(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
    body: String,
) -> Result<Value> {
    for attempt in 0..2 {
        let mut response = bio
            .http()
            .0
            .request(Method::POST, url)
            .query(params)
            .header(reqwest::header::CONTENT_TYPE, "text/plain")
            .body(body.clone())
            .send()
            .await
            .map_err(|_| anyhow!("{} connection failed or timed out", source.0))?;
        let status = response.status();
        if attempt == 0 && (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) {
            let delay = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .map(|header| header.to_str().ok().and_then(retry_after_seconds))
                .unwrap_or(Some(2));
            if let Some(delay) = delay.filter(|seconds| *seconds <= 5) {
                drop(response);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
        }
        if !status.is_success() {
            bail!("{} returned HTTP {}", source.0, status.as_u16());
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE as u64)
        {
            bail!(
                "{} response exceeded 4 MiB; request fewer records",
                source.0
            );
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("{} response could not be read", source.0))?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                bail!(
                    "{} response exceeded 4 MiB; request fewer records",
                    source.0
                );
            }
            bytes.extend_from_slice(&chunk);
        }
        if looks_like_html(&bytes) {
            bail!("{} returned HTML instead of JSON", source.0);
        }
        return serde_json::from_slice(&bytes)
            .with_context(|| format!("{} returned invalid JSON", source.0));
    }
    unreachable!("second attempt returns a response")
}

fn retry_after_seconds(value: &str) -> Option<u64> {
    value.parse().ok()
}

pub(super) fn looks_like_html(body: &[u8]) -> bool {
    let text = std::str::from_utf8(body).unwrap_or("").trim_start();
    let prefix: String = text
        .chars()
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    prefix.starts_with("<!doctype") || prefix.starts_with("<html")
}

fn require_terms(values: &[String], bound: usize, what: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for value in values {
        let entry = value.trim();
        if entry.is_empty() {
            continue;
        }
        if entry.contains(',') || entry.chars().any(char::is_whitespace) {
            bail!(
                "{what} {entry:?} contains a comma or whitespace; pass each value as its own list item (at most {bound} per call)"
            );
        }
        if entry.len() > 128 {
            bail!("{what} exceeds 128 characters");
        }
        out.push(entry.to_string());
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

fn require_unique(values: &[String], bound: usize, what: &str) -> Result<Vec<String>> {
    let cleaned = require_terms(values, bound, what)?;
    let mut seen = HashSet::new();
    for item in &cleaned {
        if !seen.insert(item) {
            bail!("duplicate {what} {item}");
        }
    }
    Ok(cleaned)
}

fn require_uniprot(values: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let acc = parse_uniprot_one(value)?;
        if !seen.insert(acc.clone()) {
            bail!("duplicate UniProt accession {acc}");
        }
        out.push(acc);
    }
    if out.is_empty() {
        bail!("provide at least one UniProt accession");
    }
    if out.len() > MAX_UNIPROT {
        bail!(
            "{} accessions exceeds the per-call bound of {MAX_UNIPROT}",
            out.len()
        );
    }
    Ok(out)
}

fn parse_uniprot_one(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let stripped = trimmed
        .strip_prefix("UniProtKB:")
        .or_else(|| trimmed.strip_prefix("uniprotkb:"))
        .unwrap_or(trimmed)
        .trim();
    if stripped.is_empty() {
        bail!("UniProt accession is empty");
    }
    let upper = stripped.to_ascii_uppercase();
    if !is_uniprot_accession(&upper) {
        bail!("{trimmed:?} is not a UniProtKB accession");
    }
    Ok(upper)
}

fn is_uniprot_accession(value: &str) -> bool {
    let (core, isoform) = match value.split_once('-') {
        Some((core, iso)) => (core, Some(iso)),
        None => (value, None),
    };
    if let Some(iso) = isoform {
        if iso.is_empty() || iso.len() > 3 || !iso.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    let b = core.as_bytes();
    let up = |c: u8| c.is_ascii_uppercase();
    let digit = |c: u8| c.is_ascii_digit();
    let an = |c: u8| digit(c) || up(c);
    match b {
        [c0, c1, c2, c3, c4, c5] => {
            if !digit(*c1) || !digit(*c5) {
                return false;
            }
            if matches!(c0, b'O' | b'P' | b'Q') {
                an(*c2) && an(*c3) && an(*c4)
            } else if up(*c0) && *c0 != b'U' {
                up(*c2) && an(*c3) && an(*c4)
            } else {
                false
            }
        }
        [c0, c1, c2, c3, c4, c5, c6, c7, c8, c9] => {
            up(*c0)
                && !matches!(c0, b'O' | b'P' | b'Q')
                && digit(*c1)
                && up(*c2)
                && an(*c3)
                && an(*c4)
                && digit(*c5)
                && up(*c6)
                && an(*c7)
                && an(*c8)
                && digit(*c9)
        }
        _ => false,
    }
}

fn require_uniprot_fields(fields: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for field in fields {
        let name = field.trim();
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            bail!("UniProt field {field:?} is not a rest.uniprot.org return-field token");
        }
        out.push(name.to_string());
    }
    if out.is_empty() {
        bail!("provide at least one UniProt field");
    }
    if out.len() > 20 {
        bail!("at most 20 UniProt fields per call");
    }
    Ok(out)
}

fn require_ontology_ids(values: &[String], bound: usize) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let id = require_ontology_id(value)?;
        if !seen.insert(id.clone()) {
            continue;
        }
        out.push(id);
    }
    if out.is_empty() {
        bail!("provide at least one ontology id");
    }
    if out.len() > bound {
        bail!(
            "{} ontology ids exceeds the per-call bound of {bound}",
            out.len()
        );
    }
    Ok(out)
}

fn require_ontology_id(value: &str) -> Result<String> {
    let id = value.trim().to_ascii_lowercase();
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        bail!("{value:?} is not an OLS ontology id");
    }
    Ok(id)
}

fn csv_param(value: &str, what: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 512 {
        bail!("{what} must contain 1 to 512 characters");
    }
    if trimmed.contains('&') || trimmed.contains(' ') {
        bail!("{what} must be a comma-separated list without spaces or query delimiters");
    }
    Ok(trimmed.to_string())
}

fn token_param(value: &str, max: usize, what: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max {
        bail!("{what} must contain 1 to {max} characters");
    }
    if trimmed
        .chars()
        .any(|c| c == '&' || c == ',' || c.is_whitespace())
    {
        bail!("{what} must be a single token");
    }
    Ok(trimmed.to_string())
}

pub(super) fn bound_u32(value: u32, min: u32, max: u32, what: &str) -> Result<u32> {
    if !(min..=max).contains(&value) {
        bail!("{what} must be between {min} and {max}");
    }
    Ok(value)
}

fn tsv_field_name(label: &str) -> String {
    match label.trim() {
        "Entry" => "accession".into(),
        "Entry Name" => "id".into(),
        "Protein names" => "protein_name".into(),
        "Gene Names" => "gene_names".into(),
        "Gene Names (primary)" => "gene_primary".into(),
        "Organism" => "organism_name".into(),
        "Organism ID" => "organism_id".into(),
        "Length" => "length".into(),
        "Sequence" => "sequence".into(),
        "Reviewed" => "reviewed".into(),
        "Mass" => "mass".into(),
        other => other.to_string(),
    }
}

fn parse_fasta(text: &str) -> Result<BTreeMap<String, String>> {
    if looks_like_html(text.as_bytes()) {
        bail!("UniProt returned HTML instead of FASTA");
    }
    let mut records = BTreeMap::new();
    let mut current_id: Option<String> = None;
    let mut current = String::new();
    for line in text.lines() {
        if let Some(header) = line.strip_prefix('>') {
            if let Some(id) = current_id.take() {
                records.insert(id, std::mem::take(&mut current));
            }
            let acc = fasta_accession(header)
                .ok_or_else(|| anyhow!("UniProt FASTA header missing an accession"))?;
            current_id = Some(acc);
            current = format!(">{header}\n");
        } else if current_id.is_some() {
            current.push_str(line);
            current.push('\n');
        }
    }
    if let Some(id) = current_id {
        records.insert(id, current);
    }
    Ok(records)
}

fn fasta_accession(header: &str) -> Option<String> {
    if let Some(acc) = header
        .split('|')
        .nth(1)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(acc.to_ascii_uppercase());
    }
    header
        .split_whitespace()
        .next()
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
}

fn parse_uniprot_txt(text: &str) -> Result<BTreeMap<String, String>> {
    if looks_like_html(text.as_bytes()) {
        bail!("UniProt returned HTML instead of a UniProt flat file");
    }
    let mut records = BTreeMap::new();
    for chunk in text.split("\n//") {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let mut acc = None;
        for line in chunk.lines() {
            if let Some(rest) = line.strip_prefix("AC   ") {
                acc = rest
                    .split(';')
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_ascii_uppercase());
                break;
            }
        }
        if let Some(acc) = acc {
            records.insert(acc, format!("{chunk}\n//\n"));
        }
    }
    Ok(records)
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = string_or_first(value.get(*key)) {
            return Some(found);
        }
    }
    None
}

fn string_or_first(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(Value::Array(items)) => items.iter().find_map(|item| {
            item.as_str()
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        }),
        _ => None,
    }
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn double_encode(value: &str) -> String {
    percent_encode(&percent_encode(value))
}

fn path_segment(value: &str) -> String {
    percent_encode(value)
}

fn mygene_base(bio: &NativeBio) -> String {
    cred_base(bio, "MYGENE_BASE_URL", MYGENE_HOST)
}
fn uniprot_base(bio: &NativeBio) -> String {
    cred_base(bio, "UNIPROT_BASE_URL", UNIPROT_HOST)
}
fn ols_base(bio: &NativeBio) -> String {
    cred_base(bio, "OLS_BASE_URL", OLS_HOST)
}
fn quickgo_base(bio: &NativeBio) -> String {
    cred_base(bio, "QUICKGO_BASE_URL", QUICKGO_HOST)
}
fn reactome_base(bio: &NativeBio) -> String {
    cred_base(bio, "REACTOME_BASE_URL", REACTOME_HOST)
}

pub(super) fn cred_base(bio: &NativeBio, name: &str, fallback: &str) -> String {
    bio.credential(name)
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}
