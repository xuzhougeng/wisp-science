//! Native `variants` domain against CADD, gnomAD, ClinVar and dbSNP.
//! Independently implemented from:
//!
//! - [CADD API](https://cadd.gs.washington.edu/api)
//! - [CADD info / PHRED](https://cadd.gs.washington.edu/info)
//! - [CADD v1.7 (NAR 2024)](https://academic.oup.com/nar/article/52/D1/D1143/7511313)
//! - [gnomAD GraphQL API](https://gnomad.broadinstitute.org/help/how-do-i-query-a-batch-of-variants-do-you-have-an-api)
//!   (10 requests / IP / 60 s; POST `https://gnomad.broadinstitute.org/api`)
//! - [gnomAD DatasetId / StructuralVariantDatasetId](https://github.com/broadinstitute/gnomad-browser/blob/main/graphql-api/src/graphql/types/dataset-id.graphql)
//! - [NCBI E-utilities](https://www.ncbi.nlm.nih.gov/books/NBK25497/)
//! - [ClinVar programmatic access](https://www.ncbi.nlm.nih.gov/clinvar/docs/maintenance_use)
//! - [ClinVar review status / gold stars](https://www.ncbi.nlm.nih.gov/clinvar/docs/review_status/)
//! - [NCBI Variation Services](https://api.ncbi.nlm.nih.gov/variation/v0/) (`GET /refsnp/{rsid}`)
//! - [dbSNP Entrez fields](https://www.ncbi.nlm.nih.gov/snp/docs/entrez_help)
//!
//! References reviewed 2026-09-06. CADD scores are free for non-commercial
//! use; commercial use requires a license from the University of Washington.
//! Tests use invented records.

mod cadd;
mod clinvar;
mod dbsnp;
mod gnomad;
#[cfg(test)]
mod tests;

use crate::http::{Source, MAX_RESPONSE, NCBI};
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;
use wisp_llm::ToolSchema;

const NCBI_EUTILS: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/";
const NCBI_VARIATION: &str = "https://api.ncbi.nlm.nih.gov/variation/v0";
const CADD_API: &str = "https://cadd.gs.washington.edu/api/v1.0";
pub(super) const GNOMAD_API: &str = "https://gnomad.broadinstitute.org/api";
const GNOMAD_BROWSER: &str = "https://gnomad.broadinstitute.org";
const CLINVAR_BROWSER: &str = "https://www.ncbi.nlm.nih.gov/clinvar/";
const DBSNP_BROWSER: &str = "https://www.ncbi.nlm.nih.gov/snp/";
const GNOMAD: Source = Source("gnomAD", Duration::from_millis(600));
const LIST_CAP: usize = 2000;
pub(super) const REGION_SPAN: i64 = 1_000_000;
const SEARCH_CAP: usize = 200;
const BATCH_DEADLINE: Duration = Duration::from_secs(40);
const CALL_TIMEOUT: Duration = Duration::from_secs(45);

const GNOMAD_DATASETS: &[&str] = &[
    "gnomad_r4",
    "gnomad_r4_non_ukb",
    "gnomad_r3",
    "gnomad_r3_controls_and_biobanks",
    "gnomad_r3_non_cancer",
    "gnomad_r3_non_neuro",
    "gnomad_r3_non_topmed",
    "gnomad_r3_non_v2",
    "gnomad_r2_1",
    "gnomad_r2_1_controls",
    "gnomad_r2_1_non_neuro",
    "gnomad_r2_1_non_cancer",
    "gnomad_r2_1_non_topmed",
    "exac",
];
const SV_DATASETS: &[&str] = &[
    "gnomad_sv_r4",
    "gnomad_sv_r2_1",
    "gnomad_sv_r2_1_controls",
    "gnomad_sv_r2_1_non_neuro",
];

static GNOMAD_PACE: Mutex<Option<Instant>> = Mutex::const_new(None);

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "cadd_position_scores",
            "Return CADD raw and PHRED scores for every possible SNV at one nuclear position (up to three alts). PHRED is a rank score relative to all possible substitutions (PHRED ≥ 20 ≈ top 1%). Default version is GRCh38-v1.7; versions must be GRCh37-vX.Y or GRCh38-vX.Y (optional _inclAnno). CADD scores are free for non-commercial use; commercial use requires a license from the University of Washington. The API is experimental and not for high-throughput retrieval of thousands of variants.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["chrom", "pos"],
                "properties": {
                    "chrom": {"type": "string", "minLength": 1, "maxLength": 8,
                        "description": "Chromosome 1–22, X or Y; a chr prefix is stripped. Mitochondrial contigs are rejected."},
                    "pos": {"type": "integer", "minimum": 1,
                        "description": "1-based position on the build embedded in version."},
                    "version": {"type": "string", "minLength": 8, "maxLength": 32, "default": "GRCh38-v1.7",
                        "description": "CADD release with genome-build prefix, e.g. GRCh38-v1.7. A bare v1.7 is rejected."}
                }
            }),
        ),
        tool(
            "cadd_range_scores",
            "Return CADD raw and PHRED scores for every SNV in a nuclear window of at most 100 bp (1-based inclusive). PHRED is a rank score relative to all possible substitutions (PHRED ≥ 20 ≈ top 1%). Default version is GRCh38-v1.7; versions must be GRCh37-vX.Y or GRCh38-vX.Y (optional _inclAnno). The 100 bp cap is enforced client-side. CADD scores are free for non-commercial use; commercial use requires a license from the University of Washington. The API is experimental and not for high-throughput retrieval of thousands of variants.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["chrom", "start", "end"],
                "properties": {
                    "chrom": {"type": "string", "minLength": 1, "maxLength": 8,
                        "description": "Chromosome 1–22, X or Y; a chr prefix is stripped. Mitochondrial contigs are rejected."},
                    "start": {"type": "integer", "minimum": 1,
                        "description": "Window start (1-based, inclusive)."},
                    "end": {"type": "integer", "minimum": 1,
                        "description": "Window end (inclusive); end − start + 1 must be ≤ 100."},
                    "version": {"type": "string", "minLength": 8, "maxLength": 32, "default": "GRCh38-v1.7",
                        "description": "CADD release with genome-build prefix, e.g. GRCh38-v1.7. A bare v1.7 is rejected."}
                }
            }),
        ),
        tool(
            "cadd_variant_score",
            "Return the CADD raw and PHRED score for one nuclear SNV (chrom, pos, ref, alt). PHRED is a rank score relative to all possible substitutions (PHRED ≥ 20 ≈ top 1%). The reference allele is checked against CADD at that position so a wrong build or typo fails instead of returning a score for the wrong locus. Default version is GRCh38-v1.7; versions must be GRCh37-vX.Y or GRCh38-vX.Y (optional _inclAnno). CADD scores are free for non-commercial use; commercial use requires a license from the University of Washington. The API is experimental and not for high-throughput retrieval of thousands of variants.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["chrom", "pos", "ref", "alt"],
                "properties": {
                    "chrom": {"type": "string", "minLength": 1, "maxLength": 8,
                        "description": "Chromosome 1–22, X or Y; a chr prefix is stripped. Mitochondrial contigs are rejected."},
                    "pos": {"type": "integer", "minimum": 1,
                        "description": "1-based position on the build embedded in version."},
                    "ref": {"type": "string", "minLength": 1, "maxLength": 1,
                        "description": "Reference allele A/C/G/T; must match the genome at pos."},
                    "alt": {"type": "string", "minLength": 1, "maxLength": 1,
                        "description": "Alternate allele A/C/G/T, different from ref."},
                    "version": {"type": "string", "minLength": 8, "maxLength": 32, "default": "GRCh38-v1.7",
                        "description": "CADD release with genome-build prefix, e.g. GRCh38-v1.7. A bare v1.7 is rejected."}
                }
            }),
        ),
        tool(
            "get_variant",
            "Look up one gnomAD short variant by chrom-pos-ref-alt on the chosen dataset and return population allele counts. Default dataset is gnomad_r4 (GRCh38 exomes+genomes). Absent variants set found=false rather than inventing frequencies. Use search_variants to resolve an rsID first.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["variant_id"],
                "properties": {
                    "variant_id": {"type": "string", "minLength": 5, "maxLength": 128,
                        "description": "gnomAD ID chrom-pos-ref-alt without a chr prefix, e.g. 19-44908822-C-T."},
                    "dataset": dataset_property("gnomad_r4")
                }
            }),
        ),
        tool(
            "search_variants",
            "Search gnomAD for variant IDs matching an rsID, a chrom-pos-ref-alt ID, or a prefix. Returns a bounded, sorted list of variant_id values for get_variant. A capped page is not the complete hit list.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 128},
                    "dataset": dataset_property("gnomad_r4")
                }
            }),
        ),
        tool(
            "gene_variants",
            "List gnomAD short variants in one gene (HGNC symbol or Ensembl gene ID). Pass exactly one identifier. The response is a bounded page (at most 2000 rows) sorted by position; truncated is true when gnomAD returned more.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                    "gene_id": {"type": "string", "minLength": 1, "maxLength": 32},
                    "dataset": dataset_property("gnomad_r4")
                }
            }),
        ),
        tool(
            "gene_constraint",
            "Fetch gnomAD gene constraint metrics (pLI, observed/expected LoF/missense/synonymous ratios with 90% CI, z-scores) for one HGNC symbol or Ensembl gene ID. Pass exactly one identifier. pLI ≥ 0.9 or oe_lof_upper (LOEUF) < 0.6 is commonly treated as LoF-intolerant.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                    "gene_id": {"type": "string", "minLength": 1, "maxLength": 32}
                }
            }),
        ),
        tool(
            "region_variants",
            "List gnomAD short variants in a nuclear genomic window of at most 1 Mb (1-based inclusive). Chromosome is 1–22, X or Y without a chr prefix; mitochondrial windows use mitochondrial_variants. Coordinates follow the dataset reference (GRCh38 for r3/r4). The listing is capped at 2000 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["chrom", "start", "stop"],
                "properties": {
                    "chrom": {"type": "string", "minLength": 1, "maxLength": 8},
                    "start": {"type": "integer", "minimum": 1},
                    "stop": {"type": "integer", "minimum": 1},
                    "dataset": dataset_property("gnomad_r4")
                }
            }),
        ),
        tool(
            "liftover_variant",
            "Map a chrom-pos-ref-alt variant ID between GRCh37 and GRCh38 using gnomAD's liftover table. source_build is the build of the input ID. A directional miss returns zero results, not an error.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["variant_id"],
                "properties": {
                    "variant_id": {"type": "string", "minLength": 5, "maxLength": 128},
                    "source_build": {"type": "string", "enum": ["GRCh37", "GRCh38"], "default": "GRCh37"}
                }
            }),
        ),
        tool(
            "clinvar_variants",
            "List ClinVar variants in a gene as mirrored by gnomAD, including clinical significance, review status and gold stars. The gnomAD ClinVar snapshot date is returned as clinvar_release_date. Pass exactly one of gene_symbol or gene_id. Direct NCBI ClinVar tools provide live classifications this mirror may lack.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                    "gene_id": {"type": "string", "minLength": 1, "maxLength": 32}
                }
            }),
        ),
        tool(
            "structural_variants",
            "List gnomAD structural variants overlapping a gene. Default dataset is gnomad_sv_r4 (GRCh38); gnomad_sv_r2_1 is GRCh37. SV identifiers do not carry across releases. Pass exactly one of gene_symbol or gene_id. Listing capped at 2000 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                    "gene_id": {"type": "string", "minLength": 1, "maxLength": 32},
                    "dataset": sv_dataset_property()
                }
            }),
        ),
        tool(
            "get_structural_variant",
            "Look up one gnomAD structural variant by its release-specific identifier (for example DEL_chr17_599b1512 in gnomad_sv_r4). dataset must match the release that issued the ID. Absent IDs set found=false.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["sv_id"],
                "properties": {
                    "sv_id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "dataset": sv_dataset_property()
                }
            }),
        ),
        tool(
            "mitochondrial_variants",
            "List gnomAD mitochondrial variants with heteroplasmy-aware counts (ac_het, ac_hom, max_heteroplasmy) for one MT gene or a chrM window. Pass a gene (symbol or ID) or region_start+region_stop, not both. Listing capped at 2000 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                    "gene_id": {"type": "string", "minLength": 1, "maxLength": 32},
                    "region_start": {"type": "integer", "minimum": 1},
                    "region_stop": {"type": "integer", "minimum": 1},
                    "dataset": dataset_property("gnomad_r4")
                }
            }),
        ),
        tool(
            "clinvar_search",
            "Search live NCBI ClinVar (ESearch+ESummary, db=clinvar) and return variation records with germline, somatic clinical-impact and oncogenicity classifications, review status and gold stars. Requires NCBI_EMAIL. max_records is a page cap (1–200); total is the upstream match count and truncated marks a capped page. Missing summaries are listed separately from truncation.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 8192},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "clinvar_get_records",
            "Fetch live NCBI ClinVar variation records for up to 50 VCV/RCV accessions or bare variation IDs. VCV and numeric IDs resolve locally; each RCV costs one ESearch. rsIDs are rejected (use clinvar_variant_by_rsid). not_found is definitive absence for an RCV; missing_uids are dropped summaries (retry); not_processed are RCVs skipped when the per-call deadline ran out. Requires NCBI_EMAIL.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accessions"],
                "properties": {
                    "accessions": {
                        "type": "array", "minItems": 1, "maxItems": 50,
                        "items": {"type": "string", "minLength": 1, "maxLength": 32}
                    }
                }
            }),
        ),
        tool(
            "clinvar_variant_by_rsid",
            "Return live NCBI ClinVar variation records that reference a dbSNP rsID (one rsID can map to several VCVs). Requires NCBI_EMAIL. total is the upstream match count; truncated marks a capped page.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["rsid"],
                "properties": {
                    "rsid": {"type": "string", "minLength": 3, "maxLength": 24,
                        "pattern": "^[Rr][Ss][0-9]+$"},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        tool(
            "dbsnp_get_rsids",
            "Fetch NCBI Variation Services RefSNP records for up to 20 rsIDs: GRCh38/GRCh37 placements, alleles, gene context, per-study frequencies and ClinVar xrefs. 404 is reported in not_found; rsIDs skipped when the per-call deadline runs out land in not_processed. Requires NCBI_EMAIL.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["rsids"],
                "properties": {
                    "rsids": {
                        "type": "array", "minItems": 1, "maxItems": 20,
                        "items": {"type": "string", "minLength": 3, "maxLength": 24,
                            "pattern": "^[Rr][Ss][0-9]+$"}
                    }
                }
            }),
        ),
        tool(
            "dbsnp_search_by_region",
            "List dbSNP rsIDs in a genomic window via ESearch db=snp (Variation Services has no region endpoint). Chromosome 1–22, X, Y or MT; span at most 1 Mb. assembly selects POSITION (GRCh38) or POSITION_GRCH37. total is the upstream count; truncated marks a capped listing. Feed rsIDs to dbsnp_get_rsids. Requires NCBI_EMAIL.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["chrom", "start", "stop"],
                "properties": {
                    "chrom": {"type": "string", "minLength": 1, "maxLength": 8},
                    "start": {"type": "integer", "minimum": 1},
                    "stop": {"type": "integer", "minimum": 1},
                    "assembly": {"type": "string", "enum": ["GRCh38", "GRCh37"], "default": "GRCh38"},
                    "max_rsids": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}
                }
            }),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(CALL_TIMEOUT, dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("variants request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "cadd_position_scores" => cadd::position_scores(bio, args).await,
        "cadd_range_scores" => cadd::range_scores(bio, args).await,
        "cadd_variant_score" => cadd::variant_score(bio, args).await,
        "get_variant" => gnomad::get_variant(bio, args).await,
        "search_variants" => gnomad::search_variants(bio, args).await,
        "gene_variants" => gnomad::gene_variants(bio, args).await,
        "gene_constraint" => gnomad::gene_constraint(bio, args).await,
        "region_variants" => gnomad::region_variants(bio, args).await,
        "liftover_variant" => gnomad::liftover_variant(bio, args).await,
        "clinvar_variants" => gnomad::clinvar_variants(bio, args).await,
        "structural_variants" => gnomad::structural_variants(bio, args).await,
        "get_structural_variant" => gnomad::get_structural_variant(bio, args).await,
        "mitochondrial_variants" => gnomad::mitochondrial_variants(bio, args).await,
        "clinvar_search" => clinvar::search(bio, args).await,
        "clinvar_get_records" => clinvar::get_records(bio, args).await,
        "clinvar_variant_by_rsid" => clinvar::by_rsid(bio, args).await,
        "dbsnp_get_rsids" => dbsnp::get_rsids(bio, args).await,
        "dbsnp_search_by_region" => dbsnp::search_by_region(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    ("variants", ToolSchema::new(name, description, parameters))
}

fn dataset_property(default: &str) -> Value {
    json!({
        "type": "string",
        "enum": GNOMAD_DATASETS,
        "default": default
    })
}

fn sv_dataset_property() -> Value {
    json!({
        "type": "string",
        "enum": SV_DATASETS,
        "default": "gnomad_sv_r4"
    })
}

fn gnomad_api(bio: &NativeBio) -> String {
    override_url(bio, "GNOMAD_API_URL", GNOMAD_API)
}

fn cadd_base(bio: &NativeBio) -> String {
    override_url(bio, "CADD_BASE_URL", CADD_API)
}

fn eutils_base(bio: &NativeBio) -> String {
    match bio
        .credential("NCBI_EUTILS_URL")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(url) => format!("{}/", url.trim_end_matches('/')),
        None => NCBI_EUTILS.to_string(),
    }
}

fn variation_base(bio: &NativeBio) -> String {
    override_url(bio, "NCBI_VARIATION_URL", NCBI_VARIATION)
}

fn override_url(bio: &NativeBio, name: &str, fallback: &str) -> String {
    bio.credential(name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
        .unwrap_or_else(|| fallback.to_string())
}

fn contact_email(bio: &NativeBio) -> Result<String> {
    for name in ["NCBI_EMAIL", "OPERON_CONTACT_EMAIL"] {
        if let Some(email) = bio
            .credential(name)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if email.contains('@') && email.len() <= 256 {
                return Ok(email.to_string());
            }
        }
    }
    bail!(
        "NCBI E-utilities require a contact email. Set NCBI_EMAIL, or enable sharing a contact email with research data services."
    )
}

fn ncbi_identity(bio: &NativeBio) -> Result<Vec<(String, String)>> {
    let mut params = vec![
        ("tool".into(), "wisp-science".into()),
        ("email".into(), contact_email(bio)?),
    ];
    if let Some(key) = bio
        .credential("NCBI_API_KEY")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params.push(("api_key".into(), key.to_string()));
    }
    Ok(params)
}

async fn ncbi_json(
    bio: &NativeBio,
    path: &str,
    mut params: Vec<(String, String)>,
) -> Result<Value> {
    params.push(("retmode".into(), "json".into()));
    params.extend(ncbi_identity(bio)?);
    let url = format!("{}{path}", eutils_base(bio));
    let value = bio
        .http()
        .send(NCBI, Method::POST, &url, &params)
        .await?
        .json()?;
    if value.get("error").is_some() || value.get("ERROR").is_some() {
        bail!("NCBI rejected the request");
    }
    Ok(value)
}

async fn graphql(bio: &NativeBio, query: &str, variables: Value) -> Result<Gql> {
    let url = gnomad_api(bio);
    let payload = json!({"query": query, "variables": variables});
    let raw = json_post(bio, &url, &payload).await?;
    let errors = raw.get("errors").and_then(Value::as_array);
    if let Some(errors) = errors.filter(|rows| !rows.is_empty()) {
        if errors.iter().all(is_not_found_error) {
            return Ok(Gql::NotFound);
        }
        bail!("gnomAD rejected the GraphQL query");
    }
    let data = raw
        .get("data")
        .cloned()
        .context("gnomAD omitted GraphQL data")?;
    Ok(Gql::Data(data))
}

enum Gql {
    Data(Value),
    NotFound,
}

fn is_not_found_error(error: &Value) -> bool {
    error
        .get("message")
        .and_then(Value::as_str)
        .is_some_and(|message| message.to_ascii_lowercase().contains("not found"))
}

async fn json_post(bio: &NativeBio, url: &str, body: &Value) -> Result<Value> {
    let pace = url.starts_with("https://gnomad.broadinstitute.org");
    for attempt in 0..2 {
        if pace {
            let mut last = GNOMAD_PACE.lock().await;
            if let Some(previous) = *last {
                tokio::time::sleep_until(previous + GNOMAD.1).await;
            }
            *last = Some(Instant::now());
        }
        let mut response = bio
            .http()
            .0
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(|_| anyhow!("gnomAD connection failed or timed out"))?;
        let status = response.status();
        if attempt == 0 && (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) {
            let delay = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|header| header.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .or(Some(2));
            if let Some(delay) = delay.filter(|seconds| *seconds <= 5) {
                drop(response);
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }
        }
        if !status.is_success() {
            bail!("gnomAD returned HTTP {}", status.as_u16());
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE as u64)
        {
            bail!("gnomAD response exceeded 4 MiB; request fewer records");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("gnomAD response could not be read"))?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                bail!("gnomAD response exceeded 4 MiB; request fewer records");
            }
            bytes.extend_from_slice(&chunk);
        }
        return serde_json::from_slice(&bytes).context("gnomAD returned invalid JSON");
    }
    unreachable!("second attempt returns a response")
}

pub(super) fn require_dataset(value: &str) -> Result<String> {
    if GNOMAD_DATASETS.contains(&value) {
        Ok(value.to_string())
    } else {
        bail!("dataset must be a gnomAD DatasetId (default gnomad_r4)")
    }
}

fn require_sv_dataset(value: &str) -> Result<String> {
    if SV_DATASETS.contains(&value) {
        Ok(value.to_string())
    } else {
        bail!("dataset must be a gnomAD StructuralVariantDatasetId (default gnomad_sv_r4)")
    }
}

pub(super) fn reference_genome(dataset: &str) -> &'static str {
    if dataset.starts_with("gnomad_r2") || dataset == "exac" || dataset.contains("sv_r2") {
        "GRCh37"
    } else {
        "GRCh38"
    }
}

pub(super) fn gene_args(
    symbol: &Option<String>,
    gene_id: &Option<String>,
) -> Result<(Option<String>, Option<String>)> {
    let symbol = clean_opt(symbol, 64, "gene_symbol")?;
    let gene_id = clean_opt(gene_id, 32, "gene_id")?;
    match (symbol, gene_id) {
        (Some(symbol), None) => Ok((Some(symbol), None)),
        (None, Some(gene_id)) => Ok((None, Some(gene_id))),
        _ => bail!("pass exactly one of gene_symbol or gene_id"),
    }
}

fn clean_opt(value: &Option<String>, max: usize, what: &str) -> Result<Option<String>> {
    match value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        None => Ok(None),
        Some(text) if text.len() > max => bail!("{what} exceeds {max} characters"),
        Some(text) if text.chars().any(char::is_whitespace) => {
            bail!("{what} must not contain whitespace")
        }
        Some(text) => Ok(Some(text.to_string())),
    }
}

fn require_text(value: &str, min: usize, max: usize, what: &str) -> Result<String> {
    let text = value.trim();
    if text.len() < min || text.len() > max {
        bail!("{what} must contain {min} to {max} characters");
    }
    if text.chars().any(|c| c == '\0') {
        bail!("{what} contains invalid characters");
    }
    Ok(text.to_string())
}

pub(super) fn require_variant_id(value: &str) -> Result<String> {
    let text = require_text(value, 5, 128, "variant_id")?;
    let text = text
        .strip_prefix("chr")
        .or_else(|| text.strip_prefix("CHR"))
        .unwrap_or(&text);
    let mut parts = text.split('-');
    let chrom = parts
        .next()
        .context("variant_id must be chrom-pos-ref-alt")?;
    let pos = parts
        .next()
        .context("variant_id must be chrom-pos-ref-alt")?;
    let reference = parts
        .next()
        .context("variant_id must be chrom-pos-ref-alt")?;
    let alt = parts
        .next()
        .context("variant_id must be chrom-pos-ref-alt")?;
    if parts.next().is_some() {
        bail!("variant_id must be chrom-pos-ref-alt");
    }
    let chrom = normalize_chrom(chrom, ChromKind::NuclearAllowMito)?;
    if pos.parse::<u64>().ok().filter(|n| *n > 0).is_none() {
        bail!("variant_id position must be a positive integer");
    }
    if !allele(reference) || !allele(alt) {
        bail!("variant_id alleles must be A/C/G/T/N sequences");
    }
    Ok(format!(
        "{chrom}-{pos}-{}-{}",
        reference.to_ascii_uppercase(),
        alt.to_ascii_uppercase()
    ))
}

fn allele(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|b| {
            matches!(
                b,
                b'A' | b'C' | b'G' | b'T' | b'N' | b'a' | b'c' | b'g' | b't' | b'n'
            )
        })
}

#[derive(Clone, Copy)]
pub(super) enum ChromKind {
    Nuclear,
    NuclearAllowMito,
    Dbsnp,
}

pub(super) fn normalize_chrom(raw: &str, kind: ChromKind) -> Result<String> {
    let text = raw.trim();
    let text = text
        .strip_prefix("chr")
        .or_else(|| text.strip_prefix("CHR"))
        .unwrap_or(text);
    if text.is_empty() || text.len() > 6 {
        bail!("chromosome must be 1–22, X, Y or mitochondrial");
    }
    let upper = text.to_ascii_uppercase();
    let chrom = if let Ok(n) = upper.parse::<u8>() {
        if (1..=22).contains(&n) {
            n.to_string()
        } else {
            bail!("chromosome must be 1–22, X, Y or mitochondrial");
        }
    } else {
        match upper.as_str() {
            "X" | "Y" => upper,
            "M" | "MT" => {
                match kind {
                    ChromKind::Nuclear => {
                        bail!("mitochondrial coordinates use mitochondrial_variants, not region_variants")
                    }
                    ChromKind::NuclearAllowMito => "M".into(),
                    ChromKind::Dbsnp => "MT".into(),
                }
            }
            _ => bail!("chromosome must be 1–22, X, Y or mitochondrial"),
        }
    };
    Ok(chrom)
}

pub(super) fn require_region(start: i64, stop: i64) -> Result<(i64, i64)> {
    if start < 1 || stop < start {
        bail!("region requires 1 ≤ start ≤ stop");
    }
    if stop - start > REGION_SPAN {
        bail!("region span exceeds 1 Mb; split into consecutive windows");
    }
    Ok((start, stop))
}

pub(super) fn require_rsid(value: &str) -> Result<(u64, String)> {
    let text = value.trim();
    let digits = text
        .strip_prefix("rs")
        .or_else(|| text.strip_prefix("RS"))
        .unwrap_or(text);
    if digits.is_empty()
        || digits.len() > 16
        || digits.starts_with('0')
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        bail!("rsID must look like rs7412");
    }
    let number: u64 = digits.parse().context("rsID must look like rs7412")?;
    Ok((number, format!("rs{number}")))
}

fn json_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn json_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn cap_rows(mut rows: Vec<Value>, cap: usize) -> (Vec<Value>, bool) {
    let truncated = rows.len() > cap;
    if truncated {
        rows.truncate(cap);
    }
    (rows, truncated)
}

fn sort_by_pos(rows: &mut [Value]) {
    rows.sort_by(|a, b| {
        let pos = json_i64(&a["pos"])
            .unwrap_or(0)
            .cmp(&json_i64(&b["pos"]).unwrap_or(0));
        pos.then_with(|| {
            a["variant_id"]
                .as_str()
                .unwrap_or("")
                .cmp(b["variant_id"].as_str().unwrap_or(""))
        })
    });
}

fn sorted_strings(value: Option<&Value>) -> Value {
    let mut items: Vec<String> = value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(json_string)
        .collect();
    items.sort();
    json!(items)
}

fn page_bound(n: u32, min: u32, max: u32, what: &str) -> Result<usize> {
    if !(min..=max).contains(&n) {
        bail!("{what} must be between {min} and {max}");
    }
    Ok(n as usize)
}

fn gnomad_variant_url(variant_id: &str, dataset: &str) -> String {
    format!("{GNOMAD_BROWSER}/variant/{variant_id}?dataset={dataset}")
}

fn gnomad_gene_url(gene: &str, dataset: &str) -> String {
    format!("{GNOMAD_BROWSER}/gene/{gene}?dataset={dataset}")
}

fn clinvar_url(variation_id: &str) -> String {
    format!("https://www.ncbi.nlm.nih.gov/clinvar/variation/{variation_id}/")
}

fn dbsnp_url(rsid: &str) -> String {
    format!("{DBSNP_BROWSER}{rsid}")
}
