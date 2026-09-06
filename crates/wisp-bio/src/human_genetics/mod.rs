//! Native `human-genetics` domain against the NHGRI-EBI GWAS Catalog REST
//! API v2, the eQTL Catalogue REST API v2, and public PheWeb portals.
//! Independently implemented from:
//!
//! - [GWAS Catalog REST API v2](https://www.ebi.ac.uk/gwas/rest/api/v2/docs)
//! - [GWAS Catalog API overview](https://www.ebi.ac.uk/gwas/docs/api)
//! - [GWAS Catalog training: v2 endpoints](https://www.ebi.ac.uk/training/online/courses/gwas-catalogue-exploring-snp-trait-associations/getting-data-from-gwas-catalog/the-gwas-catalog-api/)
//! - [eQTL Catalogue data access](https://www.ebi.ac.uk/eqtl/Data_access/)
//! - [eQTL Catalogue API v2 tutorial](https://github.com/eQTL-Catalogue/eQTL-Catalogue-resources/blob/master/tutorials/API_v2/eQTL_API_tutorial.md)
//! - [FinnGen PheWeb](https://finngen.gitbook.io/documentation/methods/pheweb)
//! - [statgen PheWeb](https://github.com/statgen/pheweb)
//!
//! References reviewed 2026-09-06. No API keys are published. Tests use
//! invented records.

mod eqtl;
mod gwas;
mod pheweb;

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

pub(super) const DOMAIN: &str = "human-genetics";
pub(super) const GWAS: Source = Source("GWAS Catalog", Duration::from_millis(500));
pub(super) const EQTL: Source = Source("eQTL Catalogue", Duration::from_millis(500));
pub(super) const PHEWEB: Source = Source("PheWeb", Duration::from_millis(500));

pub(super) const GWAS_API: &str = "https://www.ebi.ac.uk/gwas/rest/api/v2";
pub(super) const GWAS_SITE: &str = "https://www.ebi.ac.uk/gwas";
pub(super) const EQTL_API: &str = "https://www.ebi.ac.uk/eqtl/api/v2";
pub(super) const EQTL_SITE: &str = "https://www.ebi.ac.uk/eqtl";

pub(super) const DEFAULT_PAGE: u32 = 200;
pub(super) const MAX_GWAS: u32 = 500;
pub(super) const MAX_EQTL: u32 = 1000;
pub(super) const MAX_PHENOS: u32 = 500;
pub(super) const DEFAULT_PHENO_LIST: u32 = 500;
pub(super) const MAX_PHENO_LIST: u32 = 3000;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "eqtl_associations",
            "Retrieve cis molecular-QTL rows from one eQTL Catalogue v2 dataset (QTD accession). Requires a gene_id, rsid, variant (chr_pos_ref_alt) or pos window (chrom:start-end, GRCh38). The catalogue only tests a local window around each gene; an empty page means not tested or not present, not genome-wide absence. The API publishes no total — truncated is true when the cap filled. At most 1000 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["dataset_id"],
                "properties": {
                    "dataset_id": {"type": "string", "minLength": 4, "maxLength": 32, "pattern": "^QTD[0-9]+$"},
                    "gene_id": {"type": "string", "minLength": 8, "maxLength": 32, "description": "Unversioned Ensembl gene id, e.g. ENSG00000130203."},
                    "rsid": {"type": "string", "minLength": 3, "maxLength": 32},
                    "variant": {"type": "string", "minLength": 7, "maxLength": 128, "description": "eQTL Catalogue variant id, e.g. chr19_44908822_C_T."},
                    "pos": {"type": "string", "minLength": 5, "maxLength": 64, "description": "GRCh38 window chrom:start-end without a chr prefix, e.g. 19:44900000-44920000."},
                    "nlog10p_min": {"type": "number", "minimum": 0, "maximum": 1000, "description": "Keep rows with -log10(p) at least this value. Applied upstream."},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}
                }
            }),
        ),
        tool(
            "eqtl_list_datasets",
            "List eQTL Catalogue v2 datasets (one study × tissue or cell type × quantification method). Optional exact filters: study_label, tissue_label, quant_method (ge is gene-level expression). The API publishes no total; truncated is false when the listing is exhausted. Default cap 200, maximum 1000.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "study_label": {"type": "string", "minLength": 1, "maxLength": 64},
                    "tissue_label": {"type": "string", "minLength": 1, "maxLength": 64},
                    "quant_method": {"type": "string", "enum": ["ge", "exon", "tx", "txrev", "microarray", "leafcutter", "aptamer"]},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200}
                }
            }),
        ),
        tool(
            "gwas_associations_for_gene",
            "List NHGRI-EBI GWAS Catalog v2 associations whose variants map to one HGNC gene symbol (Ensembl mapping, including flanking genes for intergenic variants). Sorted by p-value ascending, so a capped page is the most-significant prefix. Reports the catalog total. Unknown symbols return api_total 0. Maximum 500 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gene_symbol"],
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 40},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "gwas_associations_for_trait",
            "List NHGRI-EBI GWAS Catalog v2 associations annotated to one EFO/MONDO/HP trait. Pass exactly one of efo_id (short form, e.g. MONDO_0005010) or efo_trait (exact label). Resolve current ids with gwas_search_traits — historical EFO ids may have migrated. Sorted by p-value ascending. Unknown ids return api_total 0. Maximum 500 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "efo_id": {"type": "string", "minLength": 3, "maxLength": 64},
                    "efo_trait": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "gwas_associations_for_variant",
            "List NHGRI-EBI GWAS Catalog v2 associations for one dbSNP rsID. Use the catalog's current rsID; merged or retired ids may return api_total 0 rather than an error. Sorted by p-value ascending, so a capped page is the most-significant prefix. Reports the catalog total. Maximum 500 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["rs_id"],
                "properties": {
                    "rs_id": {"type": "string", "minLength": 3, "maxLength": 32, "pattern": "^rs[0-9]+$"},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "gwas_get_study",
            "Fetch one NHGRI-EBI GWAS Catalog v2 study by GCST accession. Returns found=false and study=null when the accession is unknown. Positions and sample fields are catalog metadata, not full summary statistics (the summary-statistics API is retired).",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["accession_id"],
                "properties": {
                    "accession_id": {"type": "string", "minLength": 5, "maxLength": 32, "pattern": "^GCST[0-9]+$"}
                }
            }),
        ),
        tool(
            "gwas_get_variant",
            "Fetch one NHGRI-EBI GWAS Catalog v2 variant record (GRCh38 location, mapped genes, consequence) by dbSNP rsID. Lighter than listing its associations. Returns found=false and variant=null when the rsID is not in the catalog. merged=1 means the rsID was merged upstream.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["rs_id"],
                "properties": {
                    "rs_id": {"type": "string", "minLength": 3, "maxLength": 32, "pattern": "^rs[0-9]+$"}
                }
            }),
        ),
        tool(
            "gwas_search_studies",
            "Search NHGRI-EBI GWAS Catalog v2 studies by EFO id, exact EFO trait label, and/or PubMed id (filters combine). At least one filter is required — the unfiltered catalog is hundreds of thousands of studies. A capped page is not the complete hit list; api_total is the catalog count. Maximum 500 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "efo_id": {"type": "string", "minLength": 3, "maxLength": 64},
                    "efo_trait": {"type": "string", "minLength": 1, "maxLength": 256},
                    "pubmed_id": {"type": "string", "minLength": 1, "maxLength": 12, "pattern": "^[1-9][0-9]{0,11}$"},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "gwas_search_traits",
            "Search NHGRI-EBI GWAS Catalog v2 EFO trait annotations by case-insensitive label substring. Use the returned efo_id values with gwas_associations_for_trait and gwas_search_studies. The catalog mixes EFO, MONDO, HP and OBA ids. A capped page is not the complete hit list. Maximum 500 rows.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "phewas_finngen_gene",
            "Gene-region PheWAS from the FinnGen R12 public PheWeb (GRCh38): for each disease endpoint, the best-associated variant in the padded gene region. Unknown symbols are an error. Most rows are null results; filter by p-value for hits. Sorted most-significant first. Default 200 phenotypes, maximum 500. FinnGen publishes ~2470 endpoints — a capped page is not the complete phenome.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gene_symbol"],
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 40},
                    "max_phenos": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "phewas_instances",
            "List the public PheWeb PheWAS portals this domain can query. finngen is FinnGen R12 (GRCh38); bbj is BioBank Japan / pheweb.jp (GRCh37/hg19). capabilities name the JSON endpoints each instance exposes. Lift coordinates before cross-querying builds.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {}
            }),
        ),
        tool(
            "phewas_list_phenotypes",
            "List disease endpoints from a PheWeb instance that publishes /api/phenos (currently FinnGen R12). BioBank Japan does not expose this listing — use phewas_search_phenotypes there. Sorted by phenocode. Default 500, maximum 3000 (enough for the complete FinnGen catalogue). A capped page reports truncated.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "instance": {"type": "string", "enum": ["finngen"], "default": "finngen"},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 3000, "default": 500}
                }
            }),
        ),
        tool(
            "phewas_search_phenotypes",
            "Search a PheWeb instance's phenotype autocomplete by free text (disease name, code, and on some instances gene or rsID). instance is finngen (default, GRCh38) or bbj (GRCh37). Returns display labels, phenocodes and instance-relative URLs. Autocomplete lists are short; the cap still reports truncated when hit.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 256},
                    "instance": {"type": "string", "enum": ["finngen", "bbj"], "default": "finngen"},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
        tool(
            "phewas_variant",
            "PheWAS for one variant against every phenotype in a public PheWeb portal, most-significant first. instance is finngen (FinnGen R12, GRCh38) or bbj (BioBank Japan, GRCh37). Variant coordinates must match that build. Variant id is chrom-pos-ref-alt (colon/underscore/chr prefix tolerated). Unknown variants are an error. Default 200 phenotypes, maximum 500. FinnGen returns ~2470 rows — a capped page is not the complete phenome.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["instance", "variant"],
                "properties": {
                    "instance": {"type": "string", "enum": ["finngen", "bbj"]},
                    "variant": {"type": "string", "minLength": 7, "maxLength": 128},
                    "max_phenos": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                }
            }),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "eqtl_associations" => eqtl::associations(bio, args).await,
        "eqtl_list_datasets" => eqtl::list_datasets(bio, args).await,
        "gwas_associations_for_gene" => gwas::associations_for_gene(bio, args).await,
        "gwas_associations_for_trait" => gwas::associations_for_trait(bio, args).await,
        "gwas_associations_for_variant" => gwas::associations_for_variant(bio, args).await,
        "gwas_get_study" => gwas::get_study(bio, args).await,
        "gwas_get_variant" => gwas::get_variant(bio, args).await,
        "gwas_search_studies" => gwas::search_studies(bio, args).await,
        "gwas_search_traits" => gwas::search_traits(bio, args).await,
        "phewas_finngen_gene" => pheweb::finngen_gene(bio, args).await,
        "phewas_instances" => pheweb::instances(),
        "phewas_list_phenotypes" => pheweb::list_phenotypes(bio, args).await,
        "phewas_search_phenotypes" => pheweb::search_phenotypes(bio, args).await,
        "phewas_variant" => pheweb::variant(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    (DOMAIN, ToolSchema::new(name, description, parameters))
}

pub(super) fn default_page() -> u32 {
    DEFAULT_PAGE
}

pub(super) fn default_pheno_list() -> u32 {
    DEFAULT_PHENO_LIST
}

pub(super) fn bound_records(n: u32, max: u32, name: &str) -> Result<usize> {
    if n < 1 || n > max {
        bail!("{name} must be between 1 and {max}");
    }
    Ok(n as usize)
}

pub(super) fn require_text(value: &str, what: &str, min: usize, max: usize) -> Result<String> {
    let text = value.trim();
    if text.len() < min || text.len() > max {
        bail!("{what} must be {min}–{max} characters");
    }
    if text
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '?' || c == '#')
    {
        bail!("{what} contains reserved path or query characters");
    }
    Ok(text.to_string())
}

pub(super) fn optional_text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub(super) fn require_rs_id(value: &str) -> Result<String> {
    let text = require_text(value, "rs_id", 3, 32)?;
    let lower = text.to_ascii_lowercase();
    if !lower.starts_with("rs")
        || lower.len() == 2
        || !lower[2..].bytes().all(|b| b.is_ascii_digit())
    {
        bail!("rs_id must be a dbSNP identifier such as rs7412");
    }
    Ok(lower)
}

pub(super) fn require_gene_symbol(value: &str) -> Result<String> {
    let text = require_text(value, "gene_symbol", 1, 40)?;
    if !text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        bail!("gene_symbol must be an HGNC symbol (letters, digits, hyphen)");
    }
    Ok(text)
}

pub(super) fn require_efo_id(value: &str) -> Result<String> {
    let text = require_text(value, "efo_id", 3, 64)?;
    let mut parts = text.split('_');
    let prefix = parts.next().unwrap_or("");
    let digits = parts.next().unwrap_or("");
    if parts.next().is_some()
        || prefix.is_empty()
        || !prefix.bytes().all(|b| b.is_ascii_alphabetic())
        || digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        bail!("efo_id must be an ontology short form such as MONDO_0005010 or EFO_0004340");
    }
    Ok(text.to_ascii_uppercase())
}

pub(super) fn require_gcst(value: &str) -> Result<String> {
    let text = require_text(value, "accession_id", 5, 32)?;
    let upper = text.to_ascii_uppercase();
    if !upper.starts_with("GCST") || !upper[4..].bytes().all(|b| b.is_ascii_digit()) {
        bail!("accession_id must be a GWAS Catalog GCST accession");
    }
    Ok(upper)
}

pub(super) fn require_qtd(value: &str) -> Result<String> {
    let text = require_text(value, "dataset_id", 4, 32)?;
    let upper = text.to_ascii_uppercase();
    if !upper.starts_with("QTD") || !upper[3..].bytes().all(|b| b.is_ascii_digit()) {
        bail!("dataset_id must be an eQTL Catalogue QTD accession such as QTD000266");
    }
    Ok(upper)
}

pub(super) fn require_ensg(value: &str) -> Result<String> {
    let text = require_text(value, "gene_id", 8, 32)?;
    let upper = text.to_ascii_uppercase();
    if !upper.starts_with("ENSG") || !upper[4..].bytes().all(|b| b.is_ascii_digit()) {
        bail!("gene_id must be an unversioned Ensembl gene id such as ENSG00000130203");
    }
    Ok(upper)
}

pub(super) fn require_pubmed_id(value: &str) -> Result<String> {
    let text = require_text(value, "pubmed_id", 1, 12)?;
    if !text.starts_with(|c: char| c.is_ascii_digit() && c != '0')
        || !text.bytes().all(|b| b.is_ascii_digit())
    {
        bail!("pubmed_id must be a PubMed identifier");
    }
    Ok(text)
}

pub(super) fn require_eqtl_pos(value: &str) -> Result<String> {
    let text = require_text(value, "pos", 5, 64)?;
    let Some((chrom, span)) = text.split_once(':') else {
        bail!("pos must be chrom:start-end without a chr prefix, e.g. 19:44900000-44920000");
    };
    let Some((start, end)) = span.split_once('-') else {
        bail!("pos must be chrom:start-end without a chr prefix, e.g. 19:44900000-44920000");
    };
    if chrom.eq_ignore_ascii_case("chr")
        || chrom.to_ascii_lowercase().starts_with("chr")
        || !valid_chrom(chrom)
        || !start.bytes().all(|b| b.is_ascii_digit())
        || !end.bytes().all(|b| b.is_ascii_digit())
        || start.is_empty()
        || end.is_empty()
    {
        bail!("pos must be chrom:start-end without a chr prefix, e.g. 19:44900000-44920000");
    }
    Ok(format!("{chrom}:{start}-{end}"))
}

pub(super) fn require_eqtl_variant(value: &str) -> Result<String> {
    let text = require_text(value, "variant", 7, 128)?;
    let parts: Vec<&str> = text.split('_').collect();
    if parts.len() != 4
        || !parts[0].to_ascii_lowercase().starts_with("chr")
        || !parts[1].bytes().all(|b| b.is_ascii_digit())
        || !allele(parts[2])
        || !allele(parts[3])
    {
        bail!("variant must be an eQTL Catalogue id such as chr19_44908822_C_T");
    }
    Ok(text.to_string())
}

fn valid_chrom(value: &str) -> bool {
    matches!(value.to_ascii_uppercase().as_str(), "X" | "Y" | "M" | "MT")
        || (value.bytes().all(|b| b.is_ascii_digit()) && !value.is_empty() && value != "0")
}

fn allele(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| {
            matches!(
                b,
                b'A' | b'C' | b'G' | b'T' | b'N' | b'a' | b'c' | b'g' | b't' | b'n'
            )
        })
}

pub(super) fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub(super) fn join_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub(super) fn gwas_base(bio: &NativeBio) -> String {
    override_base(bio, "GWAS_CATALOG_BASE_URL", GWAS_API)
}

pub(super) fn eqtl_base(bio: &NativeBio) -> String {
    override_base(bio, "EQTL_CATALOGUE_BASE_URL", EQTL_API)
}

pub(super) fn override_base(bio: &NativeBio, credential: &str, fallback: &str) -> String {
    bio.credential(credential)
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

pub(super) async fn send_json(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    let response = bio.http().send(source, Method::GET, url, params).await?;
    parse_json(response, source.0)
}

pub(super) async fn send_json_not_found(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Option<Value>> {
    let response = bio.http().send(source, Method::GET, url, params).await?;
    if response.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    parse_json(response, source.0).map(Some)
}

pub(super) async fn send_json_or_empty(
    bio: &NativeBio,
    source: Source,
    url: &str,
    params: &[(String, String)],
) -> Result<Value> {
    let response = bio.http().send(source, Method::GET, url, params).await?;
    if response.status == StatusCode::BAD_REQUEST {
        // eQTL Catalogue documents empty association/dataset hits as HTTP 400
        // `{"message":"No results"}`. Transport strips error bodies, so a
        // validated query that still 400s is treated as an empty list.
        return Ok(json!([]));
    }
    parse_json(response, source.0)
}

fn parse_json(response: crate::http::Response, source: &str) -> Result<Value> {
    response.check()?;
    serde_json::from_slice(&response.body)
        .with_context(|| format!("{source} returned invalid JSON"))
}

pub(super) fn json_count(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|n| u64::try_from(n).ok()))
            .or_else(|| {
                number.as_f64().and_then(|n| {
                    (n >= 0.0 && n.fract() == 0.0 && n <= u64::MAX as f64).then_some(n as u64)
                })
            }),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

pub(super) fn json_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

pub(super) fn null_if_blank(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) if text.is_empty() || text == "-" => Value::Null,
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

pub(super) fn string_list(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Array(items)) => Value::Array(items.clone()),
        Some(Value::String(text)) if !text.is_empty() => json!([text]),
        _ => json!([]),
    }
}
