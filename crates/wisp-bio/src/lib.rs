//! Native scientific retrieval shared by desktop, CLI and the ACP MCP bridge.
//!
//! Provider behavior is implemented from the operators' API documentation.
//! This crate does not import the legacy Python bundle or its schemas.

mod http;
mod pubmed;
mod xml;

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolResult};

pub use pubmed::PubMed;

pub fn selected_by_package(package: &str) -> bool {
    matches!(package, "mcp_bio" | "mcp_pubmed")
}

pub fn contains_tool(name: &str) -> bool {
    catalog()
        .iter()
        .any(|(_, schema)| schema.function.name == name)
}

/// Only implemented operations belong in this catalog. Domain identifiers also
/// identify the host's persisted connector settings and capability grants.
pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
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

pub fn tools(client: Arc<PubMed>) -> Vec<Box<dyn Tool>> {
    catalog()
        .into_iter()
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
    client: Arc<PubMed>,
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
