use super::{
    bound_records, default_page, gwas_base, join_url, json_count, null_if_blank, optional_text,
    path_segment, require_efo_id, require_gcst, require_gene_symbol, require_pubmed_id,
    require_rs_id, require_text, send_json, send_json_not_found, string_list, GWAS, GWAS_API,
    GWAS_SITE, MAX_GWAS,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AssociationsForVariant {
    rs_id: String,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AssociationsForGene {
    gene_symbol: String,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AssociationsForTrait {
    efo_id: Option<String>,
    efo_trait: Option<String>,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchTraits {
    query: String,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchStudies {
    efo_id: Option<String>,
    efo_trait: Option<String>,
    pubmed_id: Option<String>,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GetStudy {
    accession_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GetVariant {
    rs_id: String,
}

pub(super) async fn associations_for_variant(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: AssociationsForVariant = serde_json::from_value(args.clone())
        .context("invalid GWAS Catalog variant association arguments")?;
    let rs_id = require_rs_id(&args.rs_id)?;
    let cap = bound_records(args.max_records, MAX_GWAS, "max_records")?;
    page(
        bio,
        "associations",
        "associations",
        vec![("rs_id".into(), rs_id.clone())],
        cap,
        true,
        json!({"rs_id": rs_id}),
    )
    .await
}

pub(super) async fn associations_for_gene(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: AssociationsForGene = serde_json::from_value(args.clone())
        .context("invalid GWAS Catalog gene association arguments")?;
    let gene = require_gene_symbol(&args.gene_symbol)?;
    let cap = bound_records(args.max_records, MAX_GWAS, "max_records")?;
    page(
        bio,
        "associations",
        "associations",
        vec![("mapped_gene".into(), gene.clone())],
        cap,
        true,
        json!({"gene_symbol": gene}),
    )
    .await
}

pub(super) async fn associations_for_trait(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: AssociationsForTrait = serde_json::from_value(args.clone())
        .context("invalid GWAS Catalog trait association arguments")?;
    let efo_id = optional_text(&args.efo_id);
    let efo_trait = optional_text(&args.efo_trait);
    if efo_id.is_some() == efo_trait.is_some() {
        bail!("pass exactly one of efo_id / efo_trait");
    }
    let cap = bound_records(args.max_records, MAX_GWAS, "max_records")?;
    let (filters, query) = if let Some(id) = efo_id {
        let id = require_efo_id(&id)?;
        (vec![("efo_id".into(), id.clone())], json!({"efo_id": id}))
    } else {
        let label = require_text(&efo_trait.unwrap(), "efo_trait", 1, 256)?;
        (
            vec![("efo_trait".into(), label.clone())],
            json!({"efo_trait": label}),
        )
    };
    page(
        bio,
        "associations",
        "associations",
        filters,
        cap,
        true,
        query,
    )
    .await
}

pub(super) async fn search_traits(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchTraits = serde_json::from_value(args.clone())
        .context("invalid GWAS Catalog trait search arguments")?;
    let query = require_text(&args.query, "query", 1, 256)?;
    let cap = bound_records(args.max_records, MAX_GWAS, "max_records")?;
    let mut result = page(
        bio,
        "efo-traits",
        "efo_traits",
        vec![("trait".into(), query.clone())],
        cap,
        false,
        json!({"query": query}),
    )
    .await?;
    if let Some(Value::Array(rows)) = result.get_mut("efo_traits") {
        rows.sort_by(|a, b| {
            trait_label(a)
                .cmp(&trait_label(b))
                .then_with(|| trait_id(a).cmp(&trait_id(b)))
        });
    }
    Ok(result)
}

pub(super) async fn search_studies(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchStudies = serde_json::from_value(args.clone())
        .context("invalid GWAS Catalog study search arguments")?;
    let mut filters = Vec::new();
    let mut query = serde_json::Map::new();
    if let Some(id) = optional_text(&args.efo_id) {
        let id = require_efo_id(&id)?;
        filters.push(("efo_id".into(), id.clone()));
        query.insert("efo_id".into(), json!(id));
    }
    if let Some(label) = optional_text(&args.efo_trait) {
        let label = require_text(&label, "efo_trait", 1, 256)?;
        filters.push(("efo_trait".into(), label.clone()));
        query.insert("efo_trait".into(), json!(label));
    }
    if let Some(pmid) = optional_text(&args.pubmed_id) {
        let pmid = require_pubmed_id(&pmid)?;
        filters.push(("pubmed_id".into(), pmid.clone()));
        query.insert("pubmed_id".into(), json!(pmid));
    }
    if filters.is_empty() {
        bail!("pass at least one of efo_id / efo_trait / pubmed_id");
    }
    let cap = bound_records(args.max_records, MAX_GWAS, "max_records")?;
    page(
        bio,
        "studies",
        "studies",
        filters,
        cap,
        false,
        json!({"filters": query}),
    )
    .await
}

pub(super) async fn get_study(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetStudy = serde_json::from_value(args.clone())
        .context("invalid GWAS Catalog study lookup arguments")?;
    let accession = require_gcst(&args.accession_id)?;
    let url = join_url(
        &gwas_base(bio),
        &format!("studies/{}", path_segment(&accession)),
    );
    match send_json_not_found(bio, GWAS, &url, &[]).await? {
        None => Ok(json!({
            "source": "NHGRI-EBI GWAS Catalog",
            "source_url": GWAS_SITE,
            "found": false,
            "accession_id": accession,
            "study": null
        })),
        Some(payload) => Ok(json!({
            "source": "NHGRI-EBI GWAS Catalog",
            "source_url": GWAS_SITE,
            "found": true,
            "accession_id": accession,
            "study": flatten_study(&payload)
        })),
    }
}

pub(super) async fn get_variant(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetVariant = serde_json::from_value(args.clone())
        .context("invalid GWAS Catalog variant lookup arguments")?;
    let rs_id = require_rs_id(&args.rs_id)?;
    let url = join_url(
        &gwas_base(bio),
        &format!("single-nucleotide-polymorphisms/{}", path_segment(&rs_id)),
    );
    match send_json_not_found(bio, GWAS, &url, &[]).await? {
        None => Ok(json!({
            "source": "NHGRI-EBI GWAS Catalog",
            "source_url": GWAS_SITE,
            "found": false,
            "rs_id": rs_id,
            "variant": null
        })),
        Some(payload) => Ok(json!({
            "source": "NHGRI-EBI GWAS Catalog",
            "source_url": GWAS_SITE,
            "found": true,
            "rs_id": rs_id,
            "variant": flatten_snp(&payload)
        })),
    }
}

async fn page(
    bio: &NativeBio,
    path: &str,
    embed_key: &str,
    filters: Vec<(String, String)>,
    cap: usize,
    sort_p: bool,
    query: Value,
) -> Result<Value> {
    let mut params = filters;
    params.push(("size".into(), cap.to_string()));
    params.push(("page".into(), "0".into()));
    if sort_p {
        params.push(("sort".into(), "p_value".into()));
        params.push(("direction".into(), "asc".into()));
    }
    let url = join_url(&gwas_base(bio), path);
    let payload = send_json(bio, GWAS, &url, &params).await?;
    let api_total = json_count(payload.pointer("/page/totalElements"))
        .context("GWAS Catalog page omitted totalElements")?;
    let batch = payload
        .get("_embedded")
        .and_then(|embedded| embedded.get(embed_key))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if batch.is_empty() && api_total > 0 {
        bail!("GWAS Catalog reported {api_total} {embed_key} but the page was empty");
    }
    let rows: Vec<Value> = batch
        .into_iter()
        .take(cap)
        .map(|row| match embed_key {
            "associations" => flatten_association(&row),
            "studies" => flatten_study(&row),
            "efo_traits" => flatten_efo_trait(&row),
            _ => row,
        })
        .collect();
    let returned = rows.len() as u64;
    let truncated = api_total > returned;
    if !truncated && returned != api_total {
        bail!("GWAS Catalog reported {api_total} {embed_key} but returned {returned}");
    }
    let mut result = json!({
        "source": "NHGRI-EBI GWAS Catalog",
        "source_url": GWAS_SITE,
        "api_url": GWAS_API,
        "api_total": api_total,
        "returned": returned,
        "truncated": truncated,
    });
    if let Value::Object(map) = query {
        if let Some(filters) = map.get("filters") {
            result["filters"] = filters.clone();
        } else {
            for (key, value) in map {
                result[key] = value;
            }
        }
    }
    result[embed_key] = Value::Array(rows);
    Ok(result)
}

pub(super) fn flatten_association(rec: &Value) -> Value {
    let accession = rec
        .get("accession_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    json!({
        "association_id": rec.get("association_id"),
        "p_value": rec.get("p_value"),
        "pvalue_mantissa": rec.get("pvalue_mantissa"),
        "pvalue_exponent": rec.get("pvalue_exponent"),
        "pvalue_description": null_if_blank(rec.get("pvalue_description")),
        "or_value": rec.get("or_per_copy_num"),
        "beta": null_if_blank(rec.get("beta")),
        "ci_lower": rec.get("ci_lower"),
        "ci_upper": rec.get("ci_upper"),
        "range": null_if_blank(rec.get("range")),
        "risk_frequency": rec.get("risk_frequency"),
        "snp_effect_alleles": string_list(rec.get("snp_effect_allele")),
        "rs_ids": rs_ids(rec),
        "locations": rec.get("locations").cloned().unwrap_or_else(|| json!([])),
        "mapped_genes": rec.get("mapped_genes").cloned().unwrap_or_else(|| json!([])),
        "efo_traits": lean_traits(rec.get("efo_traits")),
        "bg_efo_traits": lean_traits(rec.get("bg_efo_traits")),
        "reported_trait": rec.get("reported_trait").cloned().unwrap_or_else(|| json!([])),
        "multi_snp_haplotype": rec.get("multi_snp_haplotype"),
        "snp_interaction": rec.get("snp_interaction"),
        "study_accession_id": rec.get("accession_id"),
        "pubmed_id": rec.get("pubmed_id"),
        "first_author": rec.get("first_author"),
        "source_url": if accession.is_empty() {
            Value::Null
        } else {
            json!(format!("{GWAS_SITE}/studies/{accession}"))
        }
    })
}

pub(super) fn flatten_study(rec: &Value) -> Value {
    let accession = rec
        .get("accession_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    json!({
        "accession_id": rec.get("accession_id"),
        "disease_trait": rec.get("disease_trait"),
        "efo_traits": lean_traits(rec.get("efo_traits")),
        "bg_efo_traits": lean_traits(rec.get("bg_efo_traits")),
        "pubmed_id": rec.get("pubmed_id"),
        "initial_sample_size": rec.get("initial_sample_size"),
        "replication_sample_size": rec.get("replication_sample_size"),
        "discovery_ancestry": rec.get("discovery_ancestry").cloned().unwrap_or_else(|| json!([])),
        "replication_ancestry": rec.get("replication_ancestry").cloned().unwrap_or_else(|| json!([])),
        "genotyping_technologies": rec.get("genotyping_technologies").cloned().unwrap_or_else(|| json!([])),
        "platforms": rec.get("platforms"),
        "cohort": rec.get("cohort").cloned().unwrap_or_else(|| json!([])),
        "full_summary_stats_available": rec.get("full_summary_stats_available"),
        "imputed": rec.get("imputed"),
        "gxe": rec.get("gxe"),
        "source_url": if accession.is_empty() {
            Value::Null
        } else {
            json!(format!("{GWAS_SITE}/studies/{accession}"))
        }
    })
}

pub(super) fn flatten_snp(rec: &Value) -> Value {
    let rs_id = rec.get("rs_id").and_then(Value::as_str).unwrap_or("");
    json!({
        "rs_id": rec.get("rs_id"),
        "merged": rec.get("merged"),
        "functional_class": rec.get("functional_class"),
        "most_severe_consequence": rec.get("most_severe_consequence"),
        "alleles": rec.get("alleles"),
        "mapped_genes": rec.get("mapped_genes").cloned().unwrap_or_else(|| json!([])),
        "locations": snp_locations(rec.get("locations")),
        "last_update_date": rec.get("last_update_date"),
        "source_url": if rs_id.is_empty() {
            Value::Null
        } else {
            json!(format!("{GWAS_SITE}/variants/{rs_id}"))
        }
    })
}

pub(super) fn flatten_efo_trait(rec: &Value) -> Value {
    let efo_id = rec.get("efo_id").and_then(Value::as_str).unwrap_or("");
    json!({
        "efo_id": rec.get("efo_id"),
        "efo_trait": rec.get("efo_trait"),
        "uri": rec.get("uri"),
        "source_url": if efo_id.is_empty() {
            Value::Null
        } else {
            json!(format!("{GWAS_SITE}/efotraits/{efo_id}"))
        }
    })
}

fn lean_traits(value: Option<&Value>) -> Value {
    let Some(Value::Array(items)) = value else {
        return json!([]);
    };
    Value::Array(
        items
            .iter()
            .map(|trait_row| {
                json!({
                    "efo_id": trait_row.get("efo_id"),
                    "efo_trait": trait_row.get("efo_trait")
                })
            })
            .collect(),
    )
}

fn rs_ids(rec: &Value) -> Value {
    match rec.get("snp_allele") {
        Some(Value::Array(items)) => Value::Array(
            items
                .iter()
                .filter_map(|allele| allele.get("rs_id").cloned())
                .collect(),
        ),
        _ => json!([]),
    }
}

fn snp_locations(value: Option<&Value>) -> Value {
    let Some(Value::Array(items)) = value else {
        return json!([]);
    };
    Value::Array(
        items
            .iter()
            .map(|loc| {
                json!({
                    "chromosome": loc.get("chromosome_name"),
                    "position": loc.get("chromosome_position"),
                    "region": loc.get("region").and_then(|region| region.get("name")).cloned()
                })
            })
            .collect(),
    )
}

fn trait_label(row: &Value) -> String {
    row.get("efo_trait")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn trait_id(row: &Value) -> String {
    row.get("efo_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
