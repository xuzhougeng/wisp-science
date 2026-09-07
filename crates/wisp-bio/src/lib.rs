//! Native scientific retrieval shared by desktop, CLI and the ACP MCP bridge.
//!
//! Each domain has `catalog()` and `call()`. Desktop, CLI and MCP bridge
//! register this catalog directly; no Python server is required.

mod biomart;
mod biorxiv;
mod cancer_models;
mod cellguide;
mod chembl;
mod chemistry;
mod clinical_genomics;
mod clinical_trials;
mod drug_regulatory;
mod expression;
mod genes_ontologies;
mod genomes;
mod http;
mod human_genetics;
mod literature;
mod omics_archives;
mod protein_annotation;
mod pubmed;
mod regulation;
mod research_resources;
mod rna;
mod structures_interactions;
mod variants;
mod xml;
mod zinc;

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

/// Process-wide native bio client. Credentials come from desktop 凭据 / CLI env.
pub struct NativeBio {
    pubmed: pubmed::PubMed,
    http: http::Http,
    credentials: Vec<(String, String)>,
}

/// Hosts historically constructed `PubMed`; that name now covers every native domain.
pub type PubMed = NativeBio;

impl NativeBio {
    pub fn new(credentials: &[(String, String)]) -> Result<Self> {
        Self::with_proxy(credentials, "")
    }

    /// Network settings apply equally to bundled connectors and custom MCP.
    pub fn with_proxy(credentials: &[(String, String)], proxy: &str) -> Result<Self> {
        let http = http::Http::with_proxy(proxy)?;
        Ok(Self {
            pubmed: pubmed::PubMed::with_http(credentials, http.clone()),
            http,
            credentials: credentials
                .iter()
                .filter(|(_, value)| !value.is_empty())
                .cloned()
                .collect(),
        })
    }

    pub async fn call(&self, name: &str, args: &Value) -> Result<Value> {
        let Some((domain, schema)) = catalog()
            .into_iter()
            .find(|(_, schema)| schema.function.name == name)
        else {
            bail!("unknown native biological tool: {name}");
        };
        let Some(object) = args.as_object() else {
            bail!("native biological tool arguments must be an object");
        };
        if schema.function.parameters["additionalProperties"] == false {
            for key in object.keys() {
                if schema.function.parameters["properties"].get(key).is_none() {
                    bail!("invalid {name} arguments: unknown field {key:?}");
                }
            }
        }
        match domain {
            "pubmed" => self.pubmed.call(name, args).await,
            _ => dispatch_domain(self, domain, name, args).await,
        }
    }

    pub(crate) fn http(&self) -> &http::Http {
        &self.http
    }

    pub(crate) fn credential(&self, name: &str) -> Option<&str> {
        self.credentials
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Domain tests inject a `no_proxy` client aimed at an in-process fake upstream.
    #[cfg(test)]
    pub(crate) fn test_client(credentials: &[(String, String)], http: http::Http) -> Result<Self> {
        Ok(Self {
            pubmed: pubmed::PubMed::new(credentials)?,
            http,
            credentials: credentials
                .iter()
                .filter(|(_, value)| !value.is_empty())
                .cloned()
                .collect(),
        })
    }
}

pub fn package_name(domain: &str) -> String {
    format!("mcp_{}", domain.replace('-', "_"))
}

pub fn package_selects(package: &str, domain: &str) -> bool {
    package == "mcp_bio" || package == package_name(domain)
}

pub fn selected_by_package(package: &str) -> bool {
    catalog()
        .iter()
        .any(|(domain, _)| package_selects(package, domain))
}

pub fn contains_tool(name: &str) -> bool {
    catalog()
        .iter()
        .any(|(_, schema)| schema.function.name == name)
}

pub fn domain_for_tool(name: &str) -> Option<&'static str> {
    catalog()
        .into_iter()
        .find(|(_, schema)| schema.function.name == name)
        .map(|(domain, _)| domain)
}

/// Only implemented operations belong in this catalog. Domain identifiers also
/// identify the host's persisted connector settings and capability grants.
pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    let mut out = pubmed_catalog();
    out.extend(biomart::catalog());
    out.extend(biorxiv::catalog());
    out.extend(cancer_models::catalog());
    out.extend(cellguide::catalog());
    out.extend(chembl::catalog());
    out.extend(chemistry::catalog());
    out.extend(clinical_genomics::catalog());
    out.extend(clinical_trials::catalog());
    out.extend(drug_regulatory::catalog());
    out.extend(expression::catalog());
    out.extend(genes_ontologies::catalog());
    out.extend(genomes::catalog());
    out.extend(human_genetics::catalog());
    out.extend(literature::catalog());
    out.extend(omics_archives::catalog());
    out.extend(protein_annotation::catalog());
    out.extend(regulation::catalog());
    out.extend(research_resources::catalog());
    out.extend(rna::catalog());
    out.extend(structures_interactions::catalog());
    out.extend(variants::catalog());
    out.extend(zinc::catalog());
    out
}

async fn dispatch_domain(bio: &NativeBio, domain: &str, name: &str, args: &Value) -> Result<Value> {
    match domain {
        "biomart" => biomart::call(bio, name, args).await,
        "biorxiv" => biorxiv::call(bio, name, args).await,
        "cancer-models" => cancer_models::call(bio, name, args).await,
        "cellguide" => cellguide::call(bio, name, args).await,
        "chembl" => chembl::call(bio, name, args).await,
        "chemistry" => chemistry::call(bio, name, args).await,
        "clinical-genomics" => clinical_genomics::call(bio, name, args).await,
        "clinical-trials" => clinical_trials::call(bio, name, args).await,
        "drug-regulatory" => drug_regulatory::call(bio, name, args).await,
        "expression" => expression::call(bio, name, args).await,
        "genes-ontologies" => genes_ontologies::call(bio, name, args).await,
        "genomes" => genomes::call(bio, name, args).await,
        "human-genetics" => human_genetics::call(bio, name, args).await,
        "literature" => literature::call(bio, name, args).await,
        "omics-archives" => omics_archives::call(bio, name, args).await,
        "protein-annotation" => protein_annotation::call(bio, name, args).await,
        "regulation" => regulation::call(bio, name, args).await,
        "research-resources" => research_resources::call(bio, name, args).await,
        "rna" => rna::call(bio, name, args).await,
        "structures-interactions" => structures_interactions::call(bio, name, args).await,
        "variants" => variants::call(bio, name, args).await,
        "zinc" => zinc::call(bio, name, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn pubmed_catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "pubmed",
            ToolSchema::new(
                "search_articles",
                "Search PubMed with an Entrez query. Returns matching PMIDs, the total match count and a bounded page. Use get_article_metadata to retrieve citation summaries. PubMed permits retrieval of only the first 10,000 search matches.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 8192},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 20},
                        "retstart": {"type": "integer", "minimum": 0, "maximum": 9999, "default": 0},
                        "sort": {"type": "string", "enum": ["relevance", "pub_date", "author", "journal_name"], "default": "relevance"},
                        "datetype": {"type": "string", "enum": ["pdat", "edat", "mdat"], "default": "pdat"},
                        "date_from": {"type": "string", "description": "Start date: YYYY, YYYY/MM or YYYY/MM/DD. Supply both dates."},
                        "date_to": {"type": "string", "description": "End date in the same format as date_from."}
                    }
                }),
            ),
        ),
        (
            "pubmed",
            ToolSchema::new(
                "get_article_metadata",
                "Retrieve NCBI PubMed metadata for up to 200 PMIDs: citation summaries, article identifiers and abstracts when available. Missing records are reported individually. This retrieves metadata, not article full text.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["pmids"],
                    "properties": {
                        "pmids": {"type": "array", "minItems": 1, "maxItems": 200,
                            "items": {"type": "string", "pattern": "^[1-9][0-9]{0,11}$"}}
                    }
                }),
            ),
        ),
        (
            "pubmed",
            ToolSchema::new(
                "convert_article_ids",
                "Convert a same-type batch of PMID, PMCID or DOI identifiers with the NCBI PMC ID Converter. Conversions are returned only when the article is in PubMed Central. Missing and unconverted identifiers are listed. Embargoed records include live status and release date when the converter supplies them. At most 200 identifiers per request.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["ids"],
                    "properties": {
                        "ids": {"type": "array", "minItems": 1, "maxItems": 200,
                            "items": {"type": "string", "minLength": 1, "maxLength": 256}},
                        "id_type": {"type": "string", "enum": ["pmid", "pmcid", "doi"], "default": "pmid"}
                    }
                }),
            ),
        ),
        (
            "pubmed",
            ToolSchema::new(
                "find_related_articles",
                "Find Entrez records linked to PubMed articles with NCBI ELink. Supported link names are pubmed_pubmed (similar articles), pubmed_pmc, pubmed_gene, pubmed_protein and pubmed_nucleotide. Related PubMed articles keep NCBI's ranking. The response is a bounded page, not the complete neighbor set.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["pmids"],
                    "properties": {
                        "pmids": {"type": "array", "minItems": 1, "maxItems": 20,
                            "items": {"type": "string", "pattern": "^[1-9][0-9]{0,11}$"}},
                        "link_type": {"type": "string",
                            "enum": ["pubmed_pubmed", "pubmed_pmc", "pubmed_gene", "pubmed_protein", "pubmed_nucleotide"],
                            "default": "pubmed_pubmed"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200, "default": 20}
                    }
                }),
            ),
        ),
        (
            "pubmed",
            ToolSchema::new(
                "lookup_article_by_citation",
                "Match bibliographic citations to PubMed IDs with NCBI ECitMatch. Each citation is encoded as journal|year|volume|first_page|author|key as documented by E-utilities. Matched and unmatched citations are reported separately.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["citations"],
                    "properties": {
                        "citations": {"type": "array", "minItems": 1, "maxItems": 25, "items": {
                            "type": "object", "additionalProperties": false,
                            "required": ["journal"],
                            "properties": {
                                "journal": {"type": "string", "minLength": 1, "maxLength": 256},
                                "year": {"type": ["integer", "string"]},
                                "volume": {"type": "string", "maxLength": 64},
                                "first_page": {"type": "string", "maxLength": 64},
                                "author": {"type": "string", "maxLength": 256},
                                "key": {"type": "string", "maxLength": 64}
                            }
                        }}
                    }
                }),
            ),
        ),
        (
            "pubmed",
            ToolSchema::new(
                "get_full_text_article",
                "Retrieve Open Access full-text XML from Europe PMC for PMC articles. Distinguishes identifiers that are not found, not in the Europe PMC open-access subset, and OA records whose XML is unavailable. An empty body is not treated as successful evidence. At most five PMCIDs per request.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["pmc_ids"],
                    "properties": {
                        "pmc_ids": {"type": "array", "minItems": 1, "maxItems": 5,
                            "items": {"type": "string", "minLength": 1, "maxLength": 32}}
                    }
                }),
            ),
        ),
        (
            "pubmed",
            ToolSchema::new(
                "get_copyright_status",
                "Report metadata availability, accessible full text, and stated licenses for PubMed articles using Europe PMC core metadata and PMC ID Converter embargo fields. An open-access flag is not a reuse grant. Does not call the retired PMC OA Web Service.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["pmids"],
                    "properties": {
                        "pmids": {"type": "array", "minItems": 1, "maxItems": 50,
                            "items": {"type": "string", "pattern": "^[1-9][0-9]{0,11}$"}}
                    }
                }),
            ),
        ),
    ]
}

pub fn tools(client: Arc<NativeBio>) -> Vec<Box<dyn Tool>> {
    tools_for_package(client, "mcp_bio")
}

pub fn tools_for_package(client: Arc<NativeBio>, package: &str) -> Vec<Box<dyn Tool>> {
    catalog()
        .into_iter()
        .filter(|(domain, _)| package_selects(package, domain))
        .map(|(_, schema)| {
            Box::new(BioTool {
                schema,
                client: client.clone(),
            }) as Box<dyn Tool>
        })
        .collect()
}

struct BioTool {
    schema: ToolSchema,
    client: Arc<NativeBio>,
}

#[async_trait]
impl Tool for BioTool {
    fn name(&self) -> &str {
        &self.schema.function.name
    }

    fn schema(&self) -> ToolSchema {
        self.schema.clone()
    }

    fn defer_schema(&self) -> bool {
        true
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match self.client.call(self.name(), args).await {
            Ok(value) => ToolResult::ok(value.to_string()),
            Err(error) => ToolResult::fail(error.to_string()),
        }
    }
}

#[cfg(test)]
mod api_tests {
    use super::*;

    #[tokio::test]
    async fn network_proxy_routes_both_pubmed_and_other_scientific_connectors() {
        let app = axum::Router::new().fallback(|| async { axum::Json(json!({"via": "proxy"})) });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = NativeBio::with_proxy(&[], &proxy).unwrap();
        for http in [&client.http, &client.pubmed.http] {
            let response = http
                .send(
                    crate::http::Source("network proxy test", std::time::Duration::ZERO),
                    reqwest::Method::GET,
                    "http://scientific-source.invalid/records",
                    &[],
                )
                .await
                .unwrap()
                .json()
                .unwrap();
            assert_eq!(response["via"], "proxy");
        }
        server.abort();
        assert!(NativeBio::with_proxy(&[], "none").is_ok());
        assert!(NativeBio::with_proxy(&[], "http://[").is_err());
    }

    #[tokio::test]
    async fn every_tool_rejects_non_objects_and_unknown_arguments_before_http() {
        let bio = NativeBio::new(&[]).unwrap();
        for (_, schema) in catalog() {
            let name = schema.function.name;
            assert!(
                bio.call(&name, &Value::Null)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("must be an object"),
                "{name}"
            );
            assert!(
                bio.call(&name, &json!({"__unknown": true}))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("unknown field"),
                "{name}"
            );
        }
    }

    #[test]
    fn credentials_are_available_to_domain_clients() {
        let bio = NativeBio::new(&[("OPENALEX_API_KEY".into(), "k".into())]).unwrap();
        assert_eq!(bio.credential("OPENALEX_API_KEY"), Some("k"));
        let _ = bio.http();
        assert!(selected_by_package("mcp_chembl"));
        assert!(selected_by_package("mcp_pubmed"));
        assert!(selected_by_package("mcp_bio"));
    }
}
