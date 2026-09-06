//! Native `expression` domain against the GTEx Portal API v2 and the frozen
//! PanglaoDB marker table. Independently implemented from:
//!
//! - [GTEx programmatic access](https://gtexportal.org/home/apiPage)
//! - [GTEx Portal API v2 docs](https://gtexportal.org/api/v2/docs)
//! - [GTEx Portal API v2 OpenAPI](https://gtexportal.org/api/v2/openapi.json)
//! - [PanglaoDB](https://panglaodb.se/)
//! - [PanglaoDB FAQ / marker download](https://panglaodb.se/faq.html)
//! - [PanglaoDB bulk download](https://panglaodb.se/bulk.html)
//! - [Franzén et al., Database (2019)](https://pmc.ncbi.nlm.nih.gov/articles/PMC6450036/)
//!
//! References reviewed 2026-09-06. GTEx is GET JSON, paginated with
//! `page` / `itemsPerPage` / `paging_info`, and is documented as
//! low-throughput (do not send parallel queries). `datasetId` is sent on
//! every dataset-scoped call; server defaults currently mix `gtex_v8` and
//! `gtex_v10`. No API key is published. PanglaoDB has no JSON API; tools
//! download the frozen 27 Mar 2020 `PanglaoDB_markers_27_Mar_2020.tsv.gz`
//! snapshot, verify a pinned sha256 of the gzip bytes, and query in memory.
//! Tests use invented records.

mod panglaodb;

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::time::Duration;
use wisp_llm::ToolSchema;

const GTEX_API: &str = "https://gtexportal.org/api/v2";
const GTEX_PORTAL: &str = "https://gtexportal.org";
const SOURCE: &str = "GTEx Portal";
const GTEX: Source = Source(SOURCE, Duration::from_millis(500));
const DEFAULT_DATASET: &str = "gtex_v8";
const DEFAULT_MAX: u32 = 200;
const DEFAULT_SAMPLES: u32 = 50;
const DEFAULT_EGENES: u32 = 100;
const DEFAULT_TOP: u32 = 50;
const MAX_RESULTS: u32 = 500;
const MAX_TOP: u32 = 200;
const MAX_GENE_IDS: usize = 25;
const MAX_TISSUES: usize = 54;
const PAGE_SIZE: usize = 250;
const MAX_WALK_PAGES: usize = 8;
const TOKEN_MAX: usize = 64;
const VARIANT_MAX: usize = 128;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    let mut tools = vec![
        tool(
            "gtex_calculate_eqtl",
            "Calculate a gene–variant eQTL in one GTEx tissue with GET /association/dyneqtl. Unlike the precomputed significant-association routes, this computes the test for any pair (including non-significant). Returns p-value, NES, t-statistic, MAF, genotype counts and the per-sample genotype/expression arrays. Sample order is not meaningful; pairs are sorted by (genotype, expression). gencode_id must be a versioned GENCODE ID for the pinned dataset.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gencode_id", "variant_id", "tissue_site_detail_id"],
                "properties": {
                    "gencode_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "variant_id": {"type": "string", "minLength": 1, "maxLength": 128, "description": "GTEx variant ID (chr_pos_ref_alt_build) or rsID."},
                    "tissue_site_detail_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_dataset_info",
            "List GTEx dataset releases from GET /metadata/dataset: datasetId, GENCODE version, genome build, dbSNP build, and sample/subject/tissue counts. Use a returned datasetId to pin other gtex_* calls. The v2 API currently defaults some routes to gtex_v10; this client always sends datasetId on dataset-scoped requests.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_eqtl_genes",
            "List eGenes (genes with at least one significant cis-eQTL) for a tissue from GET /association/egene. Rows include nominal and empirical p-values, q-values and log2 allelic fold change of the top eQTL. The API total is preserved; the response is a bounded page (not the complete eGene list, which can be thousands of genes).",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["tissue_site_detail_id"],
                "properties": {
                    "tissue_site_detail_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "max_genes": {"type": "integer", "minimum": 1, "maximum": 500, "default": 100},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_expression_summary",
            "Profile one gene across GTEx tissues: resolve a symbol or Ensembl ID to the pinned dataset's versioned GENCODE ID (GET /reference/gene), then rank tissues by median TPM (GET /expression/medianGeneExpression). Errors when the identifier is not in the GTEx reference. Pins GENCODE v26 for gtex_v8 and v39 for gtex_v10.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gene"],
                "properties": {
                    "gene": {"type": "string", "minLength": 1, "maxLength": 64, "description": "Gene symbol, versioned GENCODE ID, or unversioned ENSG ID."},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_gene_expression",
            "Sample-level (not median) normalized expression from GET /expression/geneExpression. Each tissue row includes the per-sample TPM array and n_samples. gencode_id must be versioned for the pinned dataset. Omit tissue_site_detail_ids for every tissue in the release (~54 rows in gtex_v8). The response is a bounded page.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gencode_id"],
                "properties": {
                    "gencode_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "tissue_site_detail_ids": {
                        "type": "array", "maxItems": 54,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_median_expression",
            "Median gene expression (TPM) for genes × tissues from GET /expression/medianGeneExpression. gencode_ids must be versioned GENCODE IDs for the pinned dataset (unversioned ENSG IDs typically match zero rows). Omit tissue_site_detail_ids for every tissue. A capped page is not the complete matrix; total is the API count.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gencode_ids"],
                "properties": {
                    "gencode_ids": {
                        "type": "array", "minItems": 1, "maxItems": 25,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "tissue_site_detail_ids": {
                        "type": "array", "maxItems": 54,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_multi_tissue_eqtls",
            "METASOFT multi-tissue eQTL meta-analysis from GET /association/metasoft. Requires a versioned GENCODE ID. Each row is one variant with per-tissue m-value (posterior probability of an effect), NES, p-value and standard error. Pass variant_id to narrow; otherwise the gene's tested variants are paged. A capped page is not the complete set.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gencode_id"],
                "properties": {
                    "gencode_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "variant_id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_resolve_genes",
            "Resolve gene symbols or Ensembl IDs to versioned GENCODE IDs with GET /reference/gene. Expression and eQTL routes need the version that belongs to the pinned dataset (gtex_v8 = GENCODE v26, gtex_v10 = v39). Identifiers with no match are listed in missing_ids. At most 25 identifiers per call.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gene_ids"],
                "properties": {
                    "gene_ids": {
                        "type": "array", "minItems": 1, "maxItems": 25,
                        "items": {"type": "string", "minLength": 1, "maxLength": 64}
                    },
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_sample_info",
            "GTEx analysis-sample and donor metadata from GET /dataset/sample. Optional filters: tissueSiteDetailId, dataType (RNASEQ, WGS, WES, OMNI, EXCLUDE), subjectId (GTEX-XXXXX). Unfiltered calls match tens of thousands of rows; this tool returns a bounded page and reports the API total. A capped page is not the complete sample set.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "tissue_site_detail_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "data_type": {"type": "string", "enum": ["RNASEQ", "WGS", "WES", "OMNI", "EXCLUDE"]},
                    "subject_id": {"type": "string", "minLength": 1, "maxLength": 16, "description": "GTEx subject ID, for example GTEX-14753."},
                    "max_samples": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_single_tissue_eqtls",
            "Precomputed significant single-tissue eQTLs from GET /association/singleTissueEqtl. Provide gencode_id (versioned) and/or variant_id; optionally restrict to one tissue. Returns p-value and normalized effect size (NES). A gene without a tissue filter can match a large set; the response is a bounded page and total is the API count.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gencode_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "variant_id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "tissue_site_detail_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_tissue_sites",
            "List GTEx tissue site details for a pinned dataset from GET /dataset/tissueSiteDetail (about 54 sites in gtex_v8). Each row includes tissueSiteDetailId, display name, sample summaries, eGene counts and ontology identifiers. Use the returned tissueSiteDetailId values as tissue arguments on other gtex_* tools.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "dataset_id": dataset_schema()
                }
            }),
        ),
        tool(
            "gtex_top_expressed_genes",
            "Top-n genes by median TPM in one tissue from GET /expression/topExpressedGene (API-side ranking). filter_mt_gene defaults to true so mitochondrial genes, which otherwise dominate, are excluded. total_genes_in_ranking is the full ranking size; returned is the requested head. n is 1–200 (default 50).",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["tissue_site_detail_id"],
                "properties": {
                    "tissue_site_detail_id": {"type": "string", "minLength": 1, "maxLength": 64},
                    "n": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "filter_mt_gene": {"type": "boolean", "default": true},
                    "dataset_id": dataset_schema()
                }
            }),
        ),
    ];
    tools.extend(panglaodb::catalog());
    tools
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    let timeout_msg = if name.starts_with("panglaodb_") {
        "PanglaoDB request exceeded 45 seconds"
    } else {
        "GTEx Portal request exceeded 45 seconds"
    };
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("{timeout_msg}"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "gtex_calculate_eqtl" => calculate_eqtl(bio, args).await,
        "gtex_dataset_info" => dataset_info(bio, args).await,
        "gtex_eqtl_genes" => eqtl_genes(bio, args).await,
        "gtex_expression_summary" => expression_summary(bio, args).await,
        "gtex_gene_expression" => gene_expression(bio, args).await,
        "gtex_median_expression" => median_expression(bio, args).await,
        "gtex_multi_tissue_eqtls" => multi_tissue_eqtls(bio, args).await,
        "gtex_resolve_genes" => resolve_genes(bio, args).await,
        "gtex_sample_info" => sample_info(bio, args).await,
        "gtex_single_tissue_eqtls" => single_tissue_eqtls(bio, args).await,
        "gtex_tissue_sites" => tissue_sites(bio, args).await,
        "gtex_top_expressed_genes" => top_expressed(bio, args).await,
        "panglaodb_cell_types_for_gene" => panglaodb::cell_types_for_gene(bio, args).await,
        "panglaodb_marker_genes" => panglaodb::marker_genes(bio, args).await,
        "panglaodb_options" => panglaodb::options(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    ("expression", ToolSchema::new(name, description, parameters))
}

fn dataset_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["gtex_v8", "gtex_v10", "gtex_snrnaseq_pilot"],
        "default": "gtex_v8",
        "description": "GTEx datasetId. gtex_v8 uses GENCODE v26 / GRCh38; gtex_v10 uses GENCODE v39 / GRCh38."
    })
}

fn default_dataset() -> String {
    DEFAULT_DATASET.into()
}

fn default_max() -> u32 {
    DEFAULT_MAX
}

fn default_samples() -> u32 {
    DEFAULT_SAMPLES
}

fn default_egenes() -> u32 {
    DEFAULT_EGENES
}

fn default_top() -> u32 {
    DEFAULT_TOP
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetArg {
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetInfo {
    dataset_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SampleInfo {
    tissue_site_detail_id: Option<String>,
    data_type: Option<String>,
    subject_id: Option<String>,
    #[serde(default = "default_samples")]
    max_samples: u32,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveGenes {
    gene_ids: Vec<String>,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MedianExpression {
    gencode_ids: Vec<String>,
    tissue_site_detail_ids: Option<Vec<String>>,
    #[serde(default = "default_max")]
    max_results: u32,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpressionSummary {
    gene: String,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneExpression {
    gencode_id: String,
    tissue_site_detail_ids: Option<Vec<String>>,
    #[serde(default = "default_max")]
    max_results: u32,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopExpressed {
    tissue_site_detail_id: String,
    #[serde(default = "default_top")]
    n: u32,
    #[serde(default = "default_true")]
    filter_mt_gene: bool,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EqtlGenes {
    tissue_site_detail_id: String,
    #[serde(default = "default_egenes")]
    max_genes: u32,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SingleTissueEqtls {
    gencode_id: Option<String>,
    variant_id: Option<String>,
    tissue_site_detail_id: Option<String>,
    #[serde(default = "default_max")]
    max_results: u32,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MultiTissueEqtls {
    gencode_id: String,
    variant_id: Option<String>,
    #[serde(default = "default_max")]
    max_results: u32,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CalculateEqtl {
    gencode_id: String,
    variant_id: String,
    tissue_site_detail_id: String,
    #[serde(default = "default_dataset")]
    dataset_id: String,
}

async fn tissue_sites(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: DatasetArg =
        serde_json::from_value(args.clone()).context("invalid GTEx tissue site arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let walk = walk(
        bio,
        "/dataset/tissueSiteDetail",
        vec![("datasetId".into(), dataset.clone())],
        MAX_RESULTS as usize,
    )
    .await?;
    let mut rows: Vec<Value> = walk
        .rows
        .iter()
        .map(|row| {
            let mut record = project(
                row,
                &[
                    "tissueSiteDetailId",
                    "tissueSiteDetail",
                    "tissueSite",
                    "tissueSiteDetailAbbr",
                    "ontologyId",
                    "eGeneCount",
                    "expressedGeneCount",
                    "sGeneCount",
                    "hasEGenes",
                    "hasSGenes",
                    "colorHex",
                    "samplingSite",
                    "datasetId",
                    "eqtlSampleSummary",
                    "rnaSeqSampleSummary",
                ],
            );
            if let Some(id) = row.get("tissueSiteDetailId").and_then(Value::as_str) {
                record["url"] = json!(tissue_url(id));
            }
            record
        })
        .collect();
    rows.sort_by(|a, b| {
        str_field(a, "tissueSiteDetailId").cmp(&str_field(b, "tissueSiteDetailId"))
    });
    Ok(list_result(
        json!({"dataset_id": dataset}),
        Some(&dataset),
        walk.total,
        "tissue_sites",
        rows,
    ))
}

async fn dataset_info(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: DatasetInfo =
        serde_json::from_value(args.clone()).context("invalid GTEx dataset info arguments")?;
    let mut params = Vec::new();
    let dataset = match args.dataset_id.as_deref() {
        Some(value) => {
            let id = dataset_id(value)?;
            params.push(("datasetId".into(), id.clone()));
            Some(id)
        }
        None => None,
    };
    let payload = gtex_json(bio, &url(bio, "/metadata/dataset"), &params).await?;
    let rows = match payload {
        Value::Array(rows) => rows,
        Value::Object(map) => map
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .context("GTEx Portal omitted dataset records")?,
        _ => bail!("GTEx Portal returned an unrecognized dataset list"),
    };
    let mut datasets: Vec<Value> = rows
        .iter()
        .filter(|row| row.is_object())
        .map(|row| {
            project(
                row,
                &[
                    "datasetId",
                    "displayName",
                    "description",
                    "gencodeVersion",
                    "genomeBuild",
                    "dbSnpBuild",
                    "dbgapId",
                    "organization",
                    "subjectCount",
                    "tissueCount",
                    "rnaSeqSampleCount",
                    "rnaSeqAndGenotypeSampleCount",
                    "eqtlSubjectCount",
                    "eqtlTissuesCount",
                ],
            )
        })
        .collect();
    datasets.sort_by(|a, b| str_field(a, "datasetId").cmp(&str_field(b, "datasetId")));
    Ok(list_result(
        json!({"dataset_id": dataset}),
        dataset.as_deref(),
        datasets.len() as u64,
        "datasets",
        datasets,
    ))
}

async fn sample_info(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SampleInfo =
        serde_json::from_value(args.clone()).context("invalid GTEx sample arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let cap = bound_page(args.max_samples, MAX_RESULTS, "max_samples")?;
    let mut params = vec![("datasetId".into(), dataset.clone())];
    if let Some(tissue) = args.tissue_site_detail_id.as_deref() {
        params.push(("tissueSiteDetailId".into(), tissue_id(tissue)?));
    }
    if let Some(data_type) = args.data_type.as_deref() {
        params.push(("dataType".into(), data_type_id(data_type)?));
    }
    if let Some(subject) = args.subject_id.as_deref() {
        params.push(("subjectId".into(), subject_id(subject)?));
    }
    let walk = walk(bio, "/dataset/sample", params, cap).await?;
    let samples = walk
        .rows
        .iter()
        .map(|row| {
            project(
                row,
                &[
                    "sampleId",
                    "subjectId",
                    "tissueSiteDetailId",
                    "tissueSiteDetail",
                    "dataType",
                    "ischemicTime",
                    "rin",
                    "hardyScale",
                    "pathologyNotes",
                    "ageBracket",
                    "sex",
                    "datasetId",
                    "aliquotId",
                    "uberonId",
                    "autolysisScore",
                ],
            )
        })
        .collect();
    Ok(list_result(
        json!({
            "dataset_id": dataset,
            "tissue_site_detail_id": args.tissue_site_detail_id,
            "data_type": args.data_type,
            "subject_id": args.subject_id,
            "max_samples": cap
        }),
        Some(&dataset),
        walk.total,
        "samples",
        samples,
    ))
}

async fn resolve_genes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ResolveGenes =
        serde_json::from_value(args.clone()).context("invalid GTEx gene resolution arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let ids = require_ids(&args.gene_ids, MAX_GENE_IDS, "gene id")?;
    let genes = fetch_reference(bio, &dataset, &ids).await?;
    let missing = missing_gene_ids(&ids, &genes);
    let mut result = list_result(
        json!({"dataset_id": dataset, "gene_ids": ids}),
        Some(&dataset),
        genes.len() as u64,
        "genes",
        genes,
    );
    result["missing_ids"] = json!(missing);
    Ok(result)
}

async fn median_expression(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MedianExpression =
        serde_json::from_value(args.clone()).context("invalid GTEx median expression arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let gencode_ids = require_ids(&args.gencode_ids, MAX_GENE_IDS, "gencode id")?;
    let tissues = optional_tissues(args.tissue_site_detail_ids.as_deref())?;
    let cap = bound_page(args.max_results, MAX_RESULTS, "max_results")?;
    let walk = walk_median(bio, &dataset, &gencode_ids, &tissues, cap).await?;
    let mut medians: Vec<Value> = walk
        .rows
        .iter()
        .map(|row| {
            let mut record = project(
                row,
                &[
                    "gencodeId",
                    "geneSymbol",
                    "tissueSiteDetailId",
                    "ontologyId",
                    "median",
                    "unit",
                    "datasetId",
                ],
            );
            if let Some(id) = row.get("tissueSiteDetailId").and_then(Value::as_str) {
                record["url"] = json!(tissue_url(id));
            }
            record
        })
        .collect();
    medians.sort_by(|a, b| {
        str_field(a, "gencodeId")
            .cmp(&str_field(b, "gencodeId"))
            .then_with(|| {
                str_field(a, "tissueSiteDetailId").cmp(&str_field(b, "tissueSiteDetailId"))
            })
    });
    Ok(maybe_unversioned_hint(
        list_result(
            json!({
                "dataset_id": dataset,
                "gencode_ids": gencode_ids,
                "tissue_site_detail_ids": if tissues.is_empty() { Value::Null } else { json!(tissues) }
            }),
            Some(&dataset),
            walk.total,
            "medians",
            medians,
        ),
        &args.gencode_ids,
    ))
}

async fn expression_summary(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ExpressionSummary = serde_json::from_value(args.clone())
        .context("invalid GTEx expression summary arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let gene = require_token(&args.gene, "gene", TOKEN_MAX)?;
    let resolved = fetch_reference(bio, &dataset, std::slice::from_ref(&gene)).await?;
    let matched = pick_gene(&resolved, &gene).with_context(|| {
        format!("gene {gene:?} was not found in the GTEx reference for {dataset}")
    })?;
    let gencode_id = matched
        .get("gencodeId")
        .and_then(Value::as_str)
        .context("GTEx Portal omitted gencodeId")?
        .to_string();
    let walk = walk_median(
        bio,
        &dataset,
        &[gencode_id.clone()],
        &[],
        MAX_RESULTS as usize,
    )
    .await?;
    let mut ranked = walk.rows;
    ranked.sort_by(|a, b| {
        let ma = a.get("median").and_then(Value::as_f64);
        let mb = b.get("median").and_then(Value::as_f64);
        match (mb, ma) {
            (Some(right), Some(left)) => right.partial_cmp(&left).unwrap_or(Ordering::Equal),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        }
        .then_with(|| str_field(a, "tissueSiteDetailId").cmp(&str_field(b, "tissueSiteDetailId")))
    });
    let tissues: Vec<Value> = ranked
        .iter()
        .map(|row| {
            let tissue = str_field(row, "tissueSiteDetailId");
            json!({
                "tissueSiteDetailId": row.get("tissueSiteDetailId"),
                "median": row.get("median"),
                "unit": row.get("unit"),
                "url": tissue_url(&tissue)
            })
        })
        .collect();
    Ok(json!({
        "source": SOURCE,
        "source_url": GTEX_API,
        "dataset_id": dataset,
        "query": {"gene": gene, "dataset_id": dataset},
        "gene": {
            "geneSymbol": matched.get("geneSymbol"),
            "gencodeId": matched.get("gencodeId"),
            "gencodeVersion": matched.get("gencodeVersion"),
            "genomeBuild": matched.get("genomeBuild"),
            "url": matched.get("url")
        },
        "n_tissues": tissues.len(),
        "unit": "TPM",
        "tissues_ranked": tissues
    }))
}

async fn gene_expression(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GeneExpression =
        serde_json::from_value(args.clone()).context("invalid GTEx gene expression arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let gencode_id = require_token(&args.gencode_id, "gencode_id", TOKEN_MAX)?;
    let tissues = optional_tissues(args.tissue_site_detail_ids.as_deref())?;
    let cap = bound_page(args.max_results, MAX_RESULTS, "max_results")?;
    let mut params = vec![
        ("gencodeId".into(), gencode_id.clone()),
        ("datasetId".into(), dataset.clone()),
    ];
    push_each(&mut params, "tissueSiteDetailId", &tissues);
    let walk = walk(bio, "/expression/geneExpression", params, cap).await?;
    let mut tissues_out: Vec<Value> = Vec::new();
    for row in &walk.rows {
        let tpm = row
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .context("GTEx Portal omitted sample-level expression values")?;
        let tissue = str_field(row, "tissueSiteDetailId");
        tissues_out.push(json!({
            "tissueSiteDetailId": row.get("tissueSiteDetailId"),
            "gencodeId": row.get("gencodeId"),
            "geneSymbol": row.get("geneSymbol"),
            "unit": row.get("unit"),
            "ontologyId": row.get("ontologyId"),
            "n_samples": tpm.len(),
            "tpm": tpm,
            "url": tissue_url(&tissue)
        }));
    }
    tissues_out.sort_by(|a, b| {
        str_field(a, "tissueSiteDetailId").cmp(&str_field(b, "tissueSiteDetailId"))
    });
    Ok(maybe_unversioned_hint(
        list_result(
            json!({
                "dataset_id": dataset,
                "gencode_id": gencode_id,
                "tissue_site_detail_ids": if tissues.is_empty() { Value::Null } else { json!(tissues) }
            }),
            Some(&dataset),
            walk.total,
            "tissues",
            tissues_out,
        ),
        &[args.gencode_id.clone()],
    ))
}

async fn top_expressed(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: TopExpressed = serde_json::from_value(args.clone())
        .context("invalid GTEx top-expressed gene arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let tissue = tissue_id(&args.tissue_site_detail_id)?;
    let n = bound_page(args.n, MAX_TOP, "n")?;
    let params = vec![
        ("tissueSiteDetailId".into(), tissue.clone()),
        ("datasetId".into(), dataset.clone()),
        (
            "filterMtGene".into(),
            if args.filter_mt_gene { "true" } else { "false" }.into(),
        ),
    ];
    let walk = walk(bio, "/expression/topExpressedGene", params, n).await?;
    let genes: Vec<Value> = walk
        .rows
        .iter()
        .map(|row| {
            let mut record = project(
                row,
                &[
                    "gencodeId",
                    "geneSymbol",
                    "median",
                    "unit",
                    "tissueSiteDetailId",
                    "ontologyId",
                    "datasetId",
                ],
            );
            if let Some(symbol) = row.get("geneSymbol").and_then(Value::as_str) {
                record["url"] = json!(gene_url(symbol));
            }
            record
        })
        .collect();
    Ok(json!({
        "source": SOURCE,
        "source_url": GTEX_API,
        "dataset_id": dataset,
        "query": {
            "dataset_id": dataset,
            "tissue_site_detail_id": tissue,
            "n": n,
            "filter_mt_gene": args.filter_mt_gene
        },
        "tissueSiteDetailId": tissue,
        "filter_mt_gene": args.filter_mt_gene,
        "total_genes_in_ranking": walk.total,
        "returned": genes.len(),
        "truncated": (genes.len() as u64) < walk.total,
        "genes": genes
    }))
}

async fn eqtl_genes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: EqtlGenes =
        serde_json::from_value(args.clone()).context("invalid GTEx eGene arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let tissue = tissue_id(&args.tissue_site_detail_id)?;
    let cap = bound_page(args.max_genes, MAX_RESULTS, "max_genes")?;
    let walk = walk(
        bio,
        "/association/egene",
        vec![
            ("tissueSiteDetailId".into(), tissue.clone()),
            ("datasetId".into(), dataset.clone()),
        ],
        cap,
    )
    .await?;
    let egenes = walk
        .rows
        .iter()
        .map(|row| {
            let mut record = project(
                row,
                &[
                    "gencodeId",
                    "geneSymbol",
                    "tissueSiteDetailId",
                    "ontologyId",
                    "pValue",
                    "pValueThreshold",
                    "empiricalPValue",
                    "qValue",
                    "log2AllelicFoldChange",
                    "datasetId",
                ],
            );
            if let Some(symbol) = row.get("geneSymbol").and_then(Value::as_str) {
                record["url"] = json!(gene_url(symbol));
            }
            record
        })
        .collect();
    Ok(list_result(
        json!({
            "dataset_id": dataset,
            "tissue_site_detail_id": tissue,
            "max_genes": cap
        }),
        Some(&dataset),
        walk.total,
        "egenes",
        egenes,
    ))
}

async fn single_tissue_eqtls(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SingleTissueEqtls = serde_json::from_value(args.clone())
        .context("invalid GTEx single-tissue eQTL arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let gencode_id = optional_token(args.gencode_id.as_deref(), "gencode_id", TOKEN_MAX)?;
    let variant_id = optional_token(args.variant_id.as_deref(), "variant_id", VARIANT_MAX)?;
    let tissue = optional_token(
        args.tissue_site_detail_id.as_deref(),
        "tissue_site_detail_id",
        TOKEN_MAX,
    )?;
    if gencode_id.is_none() && variant_id.is_none() {
        bail!("provide gencode_id and/or variant_id; a tissue-only eQTL dump is not bounded");
    }
    let cap = bound_page(args.max_results, MAX_RESULTS, "max_results")?;
    let mut params = vec![("datasetId".into(), dataset.clone())];
    if let Some(id) = &gencode_id {
        params.push(("gencodeId".into(), id.clone()));
    }
    if let Some(id) = &variant_id {
        params.push(("variantId".into(), id.clone()));
    }
    if let Some(id) = &tissue {
        params.push(("tissueSiteDetailId".into(), tissue_id(id)?));
    }
    let walk = walk(bio, "/association/singleTissueEqtl", params, cap).await?;
    let eqtls = walk.rows.iter().map(project_eqtl).collect();
    let ids: Vec<String> = gencode_id.iter().cloned().collect();
    Ok(maybe_unversioned_hint(
        list_result(
            json!({
                "dataset_id": dataset,
                "gencode_id": gencode_id,
                "variant_id": variant_id,
                "tissue_site_detail_id": tissue
            }),
            Some(&dataset),
            walk.total,
            "eqtls",
            eqtls,
        ),
        &ids,
    ))
}

async fn multi_tissue_eqtls(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MultiTissueEqtls =
        serde_json::from_value(args.clone()).context("invalid GTEx multi-tissue eQTL arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let gencode_id = require_token(&args.gencode_id, "gencode_id", TOKEN_MAX)?;
    let variant_id = optional_token(args.variant_id.as_deref(), "variant_id", VARIANT_MAX)?;
    let cap = bound_page(args.max_results, MAX_RESULTS, "max_results")?;
    let mut params = vec![
        ("gencodeId".into(), gencode_id.clone()),
        ("datasetId".into(), dataset.clone()),
    ];
    if let Some(id) = &variant_id {
        params.push(("variantId".into(), id.clone()));
    }
    let walk = walk(bio, "/association/metasoft", params, cap).await?;
    let associations = walk
        .rows
        .iter()
        .map(|row| {
            project(
                row,
                &["gencodeId", "variantId", "metaP", "datasetId", "tissues"],
            )
        })
        .collect();
    Ok(maybe_unversioned_hint(
        {
            let mut result = list_result(
                json!({
                    "dataset_id": dataset,
                    "gencode_id": gencode_id,
                    "variant_id": variant_id
                }),
                Some(&dataset),
                walk.total,
                "associations",
                associations,
            );
            result["gencodeId"] = json!(gencode_id);
            result
        },
        &[args.gencode_id.clone()],
    ))
}

async fn calculate_eqtl(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: CalculateEqtl =
        serde_json::from_value(args.clone()).context("invalid GTEx dynamic eQTL arguments")?;
    let dataset = dataset_id(&args.dataset_id)?;
    let gencode_id = require_token(&args.gencode_id, "gencode_id", TOKEN_MAX)?;
    let variant_id = require_token(&args.variant_id, "variant_id", VARIANT_MAX)?;
    let tissue = tissue_id(&args.tissue_site_detail_id)?;
    let params = vec![
        ("gencodeId".into(), gencode_id.clone()),
        ("variantId".into(), variant_id.clone()),
        ("tissueSiteDetailId".into(), tissue.clone()),
        ("datasetId".into(), dataset.clone()),
    ];
    let mut payload = gtex_json(bio, &url(bio, "/association/dyneqtl"), &params).await?;
    if payload.get("gencodeId").is_none() {
        if payload.get("message").is_some() {
            bail!("GTEx Portal could not calculate the eQTL");
        }
        bail!("GTEx Portal omitted the dynamic eQTL result");
    }
    sort_eqtl_pairs(&mut payload);
    let mut out = json!({
        "source": SOURCE,
        "source_url": GTEX_API,
        "dataset_id": dataset,
        "query": {
            "dataset_id": dataset,
            "gencode_id": gencode_id,
            "variant_id": variant_id,
            "tissue_site_detail_id": tissue
        }
    });
    for key in [
        "gencodeId",
        "geneSymbol",
        "variantId",
        "tissueSiteDetailId",
        "pValue",
        "pValueThreshold",
        "nes",
        "tStatistic",
        "maf",
        "error",
        "hetCount",
        "homoAltCount",
        "homoRefCount",
        "genotypes",
        "data",
    ] {
        if let Some(value) = payload.get(key) {
            out[key] = value.clone();
        }
    }
    out["url"] = json!(tissue_url(&tissue));
    Ok(out)
}

async fn fetch_reference(
    bio: &NativeBio,
    dataset: &str,
    gene_ids: &[String],
) -> Result<Vec<Value>> {
    let mut params = reference_params(dataset);
    push_each(&mut params, "geneId", gene_ids);
    let walk = walk(bio, "/reference/gene", params, MAX_RESULTS as usize).await?;
    let mut genes: Vec<Value> = walk.rows.iter().map(project_gene).collect();
    genes.sort_by(|a, b| {
        str_field(a, "geneSymbol")
            .cmp(&str_field(b, "geneSymbol"))
            .then_with(|| str_field(a, "gencodeId").cmp(&str_field(b, "gencodeId")))
    });
    Ok(genes)
}

async fn walk_median(
    bio: &NativeBio,
    dataset: &str,
    gencode_ids: &[String],
    tissues: &[String],
    cap: usize,
) -> Result<Walk> {
    let mut params = vec![("datasetId".into(), dataset.to_string())];
    push_each(&mut params, "gencodeId", gencode_ids);
    push_each(&mut params, "tissueSiteDetailId", tissues);
    walk(bio, "/expression/medianGeneExpression", params, cap).await
}

struct Walk {
    rows: Vec<Value>,
    total: u64,
}

async fn walk(
    bio: &NativeBio,
    path: &str,
    params: Vec<(String, String)>,
    cap: usize,
) -> Result<Walk> {
    if cap == 0 || cap > MAX_RESULTS as usize {
        bail!("max_results must be between 1 and {MAX_RESULTS}");
    }
    let mut rows = Vec::new();
    let mut total = 0u64;
    let mut seen_total = false;
    let mut exhausted = false;
    for page in 0..MAX_WALK_PAGES {
        let remaining = cap.saturating_sub(rows.len());
        if remaining == 0 {
            break;
        }
        let mut query = params.clone();
        query.push(("page".into(), page.to_string()));
        query.push(("itemsPerPage".into(), remaining.min(PAGE_SIZE).to_string()));
        let payload = gtex_json(bio, &url(bio, path), &query).await?;
        if payload.get("data").is_none() && payload.get("message").is_some() {
            bail!("GTEx Portal rejected the query");
        }
        let parsed = parse_page(&payload)?;
        if parsed.page != page as u64 {
            bail!("GTEx Portal returned inconsistent pagination");
        }
        if !seen_total {
            total = parsed.total;
            seen_total = true;
        } else if parsed.total != total {
            bail!("GTEx Portal returned inconsistent pagination");
        }
        let last = parsed.pages == 0 || (page as u64) + 1 >= parsed.pages;
        if parsed.rows.is_empty() && !last && total > 0 {
            bail!("GTEx Portal returned inconsistent pagination");
        }
        rows.extend(parsed.rows);
        if rows.len() > cap {
            rows.truncate(cap);
        }
        exhausted = last;
        if exhausted || rows.len() >= cap {
            break;
        }
    }
    if !seen_total {
        bail!("GTEx Portal omitted paging_info");
    }
    if rows.len() as u64 > total {
        bail!("GTEx Portal returned more records than paging_info.totalNumberOfItems");
    }
    if exhausted {
        let expected = total.min(cap as u64);
        if rows.len() as u64 != expected {
            bail!("GTEx Portal page count did not match paging_info.totalNumberOfItems");
        }
    }
    Ok(Walk { rows, total })
}

struct Page {
    rows: Vec<Value>,
    total: u64,
    page: u64,
    pages: u64,
}

fn parse_page(payload: &Value) -> Result<Page> {
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .context("GTEx Portal omitted the result page")?;
    let info = payload
        .get("paging_info")
        .context("GTEx Portal omitted paging_info")?;
    let total = json_u64(info.get("totalNumberOfItems"))
        .context("GTEx Portal omitted paging_info.totalNumberOfItems")?;
    let page = json_u64(info.get("page")).context("GTEx Portal omitted paging_info.page")?;
    let pages = json_u64(info.get("numberOfPages"))
        .context("GTEx Portal omitted paging_info.numberOfPages")?;
    if data.iter().any(|row| !row.is_object()) {
        bail!("GTEx Portal returned a non-record page item");
    }
    Ok(Page {
        rows: data.clone(),
        total,
        page,
        pages,
    })
}

async fn gtex_json(bio: &NativeBio, endpoint: &str, params: &[(String, String)]) -> Result<Value> {
    let response = bio.http().send(GTEX, Method::GET, endpoint, params).await?;
    response.check()?;
    if looks_like_html(&response.body) {
        bail!("GTEx Portal returned HTML instead of JSON");
    }
    serde_json::from_slice(&response.body).context("GTEx Portal returned invalid JSON")
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

fn list_result(
    query: Value,
    dataset: Option<&str>,
    total: u64,
    key: &str,
    rows: Vec<Value>,
) -> Value {
    let returned = rows.len();
    let mut out = json!({
        "source": SOURCE,
        "source_url": GTEX_API,
        "query": query,
        "total": total,
        "returned": returned,
        "truncated": (returned as u64) < total,
    });
    if let Some(id) = dataset {
        out["dataset_id"] = json!(id);
    }
    out[key] = Value::Array(rows);
    out
}

fn maybe_unversioned_hint(mut result: Value, ids: &[String]) -> Value {
    let total = result.get("total").and_then(Value::as_u64).unwrap_or(0);
    if total > 0 {
        return result;
    }
    let unversioned: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| is_unversioned_ensg(id))
        .collect();
    if unversioned.is_empty() {
        return result;
    }
    result["hint"] = json!(format!(
        "GTEx expression and eQTL routes expect versioned GENCODE IDs (GENCODE v26 on gtex_v8, v39 on gtex_v10), for example ENSG00000141510.16. Unversioned id(s) {unversioned:?} returned no rows. Resolve symbols or unversioned ENSG ids with gtex_resolve_genes and pass gencodeId, or use gtex_expression_summary."
    ));
    result
}

fn project_gene(row: &Value) -> Value {
    let mut record = project(
        row,
        &[
            "gencodeId",
            "geneSymbol",
            "geneSymbolUpper",
            "gencodeVersion",
            "genomeBuild",
            "geneType",
            "chromosome",
            "start",
            "end",
            "strand",
            "tss",
            "entrezGeneId",
            "description",
            "geneStatus",
            "dataSource",
        ],
    );
    let target = row
        .get("geneSymbol")
        .and_then(Value::as_str)
        .or_else(|| row.get("gencodeId").and_then(Value::as_str));
    if let Some(id) = target {
        record["url"] = json!(gene_url(id));
    }
    record
}

fn project_eqtl(row: &Value) -> Value {
    let mut record = project(
        row,
        &[
            "gencodeId",
            "geneSymbol",
            "variantId",
            "snpId",
            "tissueSiteDetailId",
            "ontologyId",
            "pValue",
            "nes",
            "pos",
            "chromosome",
            "datasetId",
        ],
    );
    if let Some(symbol) = row.get("geneSymbol").and_then(Value::as_str) {
        record["url"] = json!(gene_url(symbol));
    }
    record
}

fn project(row: &Value, keys: &[&str]) -> Value {
    let mut out = serde_json::Map::new();
    for key in keys {
        if let Some(value) = row.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(out)
}

fn pick_gene(genes: &[Value], query: &str) -> Option<Value> {
    let upper = query.to_ascii_uppercase();
    let exact_id = genes
        .iter()
        .find(|gene| gene.get("gencodeId").and_then(Value::as_str) == Some(query));
    if let Some(gene) = exact_id {
        return Some(gene.clone());
    }
    let exact_symbol = genes.iter().find(|gene| {
        gene.get("geneSymbolUpper").and_then(Value::as_str) == Some(upper.as_str())
            || gene
                .get("geneSymbol")
                .and_then(Value::as_str)
                .is_some_and(|symbol| symbol.eq_ignore_ascii_case(query))
    });
    if let Some(gene) = exact_symbol {
        return Some(gene.clone());
    }
    genes
        .iter()
        .find(|gene| {
            gene.get("gencodeId")
                .and_then(Value::as_str)
                .and_then(|id| id.split('.').next())
                == Some(query)
        })
        .cloned()
}

fn missing_gene_ids(requested: &[String], genes: &[Value]) -> Vec<String> {
    requested
        .iter()
        .filter(|id| pick_gene(genes, id).is_none())
        .cloned()
        .collect()
}

fn sort_eqtl_pairs(payload: &mut Value) {
    let Some(genotypes) = payload.get("genotypes").and_then(Value::as_array).cloned() else {
        return;
    };
    let Some(data) = payload.get("data").and_then(Value::as_array).cloned() else {
        return;
    };
    if genotypes.len() != data.len() {
        return;
    }
    let mut pairs: Vec<(Value, Value)> = genotypes.into_iter().zip(data).collect();
    pairs.sort_by(|a, b| cmp_json(&a.0, &b.0).then_with(|| cmp_json(&a.1, &b.1)));
    payload["genotypes"] = Value::Array(pairs.iter().map(|(g, _)| g.clone()).collect());
    payload["data"] = Value::Array(pairs.iter().map(|(_, d)| d.clone()).collect());
}

fn cmp_json(a: &Value, b: &Value) -> Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(left), Some(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        _ => a.to_string().cmp(&b.to_string()),
    }
}

fn reference_params(dataset: &str) -> Vec<(String, String)> {
    let (gencode, build) = match dataset {
        "gtex_v10" => ("v39", "GRCh38/hg38"),
        _ => ("v26", "GRCh38/hg38"),
    };
    vec![
        ("gencodeVersion".into(), gencode.into()),
        ("genomeBuild".into(), build.into()),
    ]
}

fn dataset_id(value: &str) -> Result<String> {
    match value.trim() {
        id @ ("gtex_v8" | "gtex_v10" | "gtex_snrnaseq_pilot") => Ok(id.to_string()),
        other => bail!(
            "dataset_id {other:?} is not a GTEx datasetId (gtex_v8, gtex_v10, gtex_snrnaseq_pilot)"
        ),
    }
}

fn data_type_id(value: &str) -> Result<String> {
    match value.trim() {
        id @ ("RNASEQ" | "WGS" | "WES" | "OMNI" | "EXCLUDE") => Ok(id.to_string()),
        other => {
            bail!("data_type {other:?} is not a GTEx dataType (RNASEQ, WGS, WES, OMNI, EXCLUDE)")
        }
    }
}

fn subject_id(value: &str) -> Result<String> {
    let token = require_token(value, "subject_id", 16)?;
    let rest = token.strip_prefix("GTEX-").unwrap_or("");
    if rest.len() < 4
        || rest.len() > 5
        || !rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        bail!("subject_id must match GTEX-XXXXX (four or five alphanumeric characters)");
    }
    Ok(token)
}

fn tissue_id(value: &str) -> Result<String> {
    require_token(value, "tissue_site_detail_id", TOKEN_MAX)
}

fn optional_tissues(values: Option<&[String]>) -> Result<Vec<String>> {
    match values {
        None | Some([]) => Ok(Vec::new()),
        Some(values) => require_ids(values, MAX_TISSUES, "tissue_site_detail_id"),
    }
}

fn optional_token(value: Option<&str>, what: &str, max: usize) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => Ok(Some(require_token(text, what, max)?)),
    }
}

fn require_ids(ids: &[String], bound: usize, what: &str) -> Result<Vec<String>> {
    let mut cleaned = Vec::new();
    for id in ids {
        cleaned.push(require_token(id, what, TOKEN_MAX)?);
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

fn require_token(value: &str, what: &str, max: usize) -> Result<String> {
    let token = value.trim();
    if token.is_empty() || token.len() > max {
        bail!("{what} must contain 1 to {max} characters");
    }
    if token.contains("..")
        || token.chars().any(|c| {
            c.is_whitespace() || matches!(c, ',' | '/' | '\\' | '?' | '&' | '#' | '%' | '"' | '\'')
        })
    {
        bail!("{what} {token:?} is not a valid identifier; pass each value as its own list item");
    }
    Ok(token.to_string())
}

fn bound_page(n: u32, max: u32, what: &str) -> Result<usize> {
    if !(1..=max).contains(&n) {
        bail!("{what} must be between 1 and {max}");
    }
    Ok(n as usize)
}

fn is_unversioned_ensg(id: &str) -> bool {
    match id.strip_prefix("ENSG") {
        Some(rest) => !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_f64()
                .filter(|f| *f >= 0.0 && f.fract() == 0.0)
                .map(|f| f as u64)
        }),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn str_field(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn push_each(params: &mut Vec<(String, String)>, key: &str, values: &[String]) {
    for value in values {
        params.push((key.to_string(), value.clone()));
    }
}

fn api_base(bio: &NativeBio) -> String {
    bio.credential("GTEX_BASE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| GTEX_API.to_string())
}

fn url(bio: &NativeBio, path: &str) -> String {
    format!("{}{path}", api_base(bio))
}

fn gene_url(id: &str) -> String {
    format!("{GTEX_PORTAL}/home/gene/{}", path_segment(id))
}

fn tissue_url(id: &str) -> String {
    format!("{GTEX_PORTAL}/home/tissue/{}", path_segment(id))
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
