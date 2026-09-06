use super::{
    bound_records, default_page, eqtl_base, join_url, optional_text, path_segment, require_ensg,
    require_eqtl_pos, require_eqtl_variant, require_qtd, require_rs_id, require_text,
    send_json_or_empty, EQTL, EQTL_API, EQTL_SITE, MAX_EQTL,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const EQTL_PAGE: usize = 1000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListDatasets {
    study_label: Option<String>,
    tissue_label: Option<String>,
    quant_method: Option<String>,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Associations {
    dataset_id: String,
    gene_id: Option<String>,
    rsid: Option<String>,
    variant: Option<String>,
    pos: Option<String>,
    nlog10p_min: Option<f64>,
    #[serde(default = "default_page")]
    max_records: u32,
}

pub(super) async fn list_datasets(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListDatasets = serde_json::from_value(args.clone())
        .context("invalid eQTL Catalogue dataset listing arguments")?;
    let cap = bound_records(args.max_records, MAX_EQTL, "max_records")?;
    let mut filters = Vec::new();
    let mut query = serde_json::Map::new();
    if let Some(label) = optional_text(&args.study_label) {
        let label = require_text(&label, "study_label", 1, 64)?;
        filters.push(("study_label".into(), label.clone()));
        query.insert("study_label".into(), json!(label));
    }
    if let Some(label) = optional_text(&args.tissue_label) {
        let label = require_text(&label, "tissue_label", 1, 64)?;
        filters.push(("tissue_label".into(), label.clone()));
        query.insert("tissue_label".into(), json!(label));
    }
    if let Some(method) = optional_text(&args.quant_method) {
        if !matches!(
            method.as_str(),
            "ge" | "exon" | "tx" | "txrev" | "microarray" | "leafcutter" | "aptamer"
        ) {
            bail!(
                "quant_method must be one of ge, exon, tx, txrev, microarray, leafcutter, aptamer"
            );
        }
        filters.push(("quant_method".into(), method.clone()));
        query.insert("quant_method".into(), json!(method));
    }
    let (mut rows, truncated) = walk(bio, "datasets", &filters, cap).await?;
    rows.sort_by(|a, b| dataset_id(a).cmp(&dataset_id(b)));
    let datasets: Vec<Value> = rows.iter().take(cap).map(flatten_dataset).collect();
    Ok(json!({
        "source": "eQTL Catalogue",
        "source_url": EQTL_SITE,
        "api_url": EQTL_API,
        "filters": query,
        "returned": datasets.len(),
        "truncated": truncated,
        "datasets": datasets
    }))
}

pub(super) async fn associations(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Associations = serde_json::from_value(args.clone())
        .context("invalid eQTL Catalogue association arguments")?;
    let dataset_id = require_qtd(&args.dataset_id)?;
    let cap = bound_records(args.max_records, MAX_EQTL, "max_records")?;
    let mut filters = Vec::new();
    let mut query = serde_json::Map::new();
    if let Some(gene) = optional_text(&args.gene_id) {
        let gene = require_ensg(&gene)?;
        filters.push(("gene_id".into(), gene.clone()));
        query.insert("gene_id".into(), json!(gene));
    }
    if let Some(rsid) = optional_text(&args.rsid) {
        let rsid = require_rs_id(&rsid)?;
        filters.push(("rsid".into(), rsid.clone()));
        query.insert("rsid".into(), json!(rsid));
    }
    if let Some(variant) = optional_text(&args.variant) {
        let variant = require_eqtl_variant(&variant)?;
        filters.push(("variant".into(), variant.clone()));
        query.insert("variant".into(), json!(variant));
    }
    if let Some(pos) = optional_text(&args.pos) {
        let pos = require_eqtl_pos(&pos)?;
        filters.push(("pos".into(), pos.clone()));
        query.insert("pos".into(), json!(pos));
    }
    if filters.is_empty() {
        bail!("pass at least one of gene_id / rsid / variant / pos — the API rejects unfiltered association scans");
    }
    if let Some(floor) = args.nlog10p_min {
        if !floor.is_finite() || !(0.0..=1000.0).contains(&floor) {
            bail!("nlog10p_min must be a finite number between 0 and 1000");
        }
        filters.push(("nlog10p".into(), floor.to_string()));
        query.insert("nlog10p".into(), json!(floor));
    }
    let path = format!("datasets/{}/associations", path_segment(&dataset_id));
    let (rows, truncated) = walk(bio, &path, &filters, cap).await?;
    let associations: Vec<Value> = rows
        .iter()
        .take(cap)
        .map(|row| flatten_association(row, &dataset_id))
        .collect();
    Ok(json!({
        "source": "eQTL Catalogue",
        "source_url": EQTL_SITE,
        "api_url": EQTL_API,
        "dataset_id": dataset_id,
        "filters": query,
        "returned": associations.len(),
        "truncated": truncated,
        "associations": associations
    }))
}

async fn walk(
    bio: &NativeBio,
    path: &str,
    filters: &[(String, String)],
    cap: usize,
) -> Result<(Vec<Value>, bool)> {
    let url = join_url(&eqtl_base(bio), path);
    let size = cap.saturating_add(1).min(EQTL_PAGE);
    let mut params = filters.to_vec();
    params.push(("start".into(), "0".into()));
    params.push(("size".into(), size.to_string()));
    let batch = list_payload(send_json_or_empty(bio, EQTL, &url, &params).await?, path)?;
    if batch.len() < size {
        let truncated = batch.len() > cap;
        return Ok((batch, truncated));
    }
    if cap < EQTL_PAGE {
        return Ok((batch, true));
    }
    let mut peek = filters.to_vec();
    peek.push(("start".into(), batch.len().to_string()));
    peek.push(("size".into(), "1".into()));
    let extra = list_payload(send_json_or_empty(bio, EQTL, &url, &peek).await?, path)?;
    Ok((batch, !extra.is_empty()))
}

fn list_payload(payload: Value, path: &str) -> Result<Vec<Value>> {
    match payload {
        Value::Array(rows) => Ok(rows),
        Value::Null => Ok(Vec::new()),
        _ => bail!("eQTL Catalogue {path} returned a non-list payload"),
    }
}

fn flatten_dataset(row: &Value) -> Value {
    let dataset_id = row.get("dataset_id").and_then(Value::as_str).unwrap_or("");
    json!({
        "dataset_id": row.get("dataset_id"),
        "study_id": row.get("study_id"),
        "study_label": row.get("study_label"),
        "sample_group": row.get("sample_group"),
        "tissue_id": row.get("tissue_id"),
        "tissue_label": row.get("tissue_label"),
        "condition_label": row.get("condition_label"),
        "quant_method": row.get("quant_method"),
        "sample_size": row.get("sample_size"),
        "source_url": if dataset_id.is_empty() {
            Value::Null
        } else {
            json!(format!("{EQTL_API}/datasets/{dataset_id}"))
        }
    })
}

fn flatten_association(row: &Value, dataset_id: &str) -> Value {
    json!({
        "molecular_trait_id": row.get("molecular_trait_id"),
        "gene_id": row.get("gene_id"),
        "variant": row.get("variant"),
        "rsid": row.get("rsid"),
        "chromosome": row.get("chromosome"),
        "position": row.get("position"),
        "ref": row.get("ref"),
        "alt": row.get("alt"),
        "type": row.get("type"),
        "beta": row.get("beta"),
        "se": row.get("se"),
        "pvalue": row.get("pvalue"),
        "nlog10p": row.get("nlog10p"),
        "maf": row.get("maf"),
        "ac": row.get("ac"),
        "an": row.get("an"),
        "r2": row.get("r2"),
        "median_tpm": row.get("median_tpm"),
        "source_url": format!("{EQTL_API}/datasets/{dataset_id}")
    })
}

fn dataset_id(row: &Value) -> String {
    row.get("dataset_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
