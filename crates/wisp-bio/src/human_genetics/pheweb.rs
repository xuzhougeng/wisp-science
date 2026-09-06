use super::{
    bound_records, default_page, default_pheno_list, join_url, json_f64, override_base,
    path_segment, require_gene_symbol, require_text, send_json, send_json_not_found, DEFAULT_PAGE,
    MAX_PHENOS, MAX_PHENO_LIST, PHEWEB,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const FINNGEN_API: &str = "https://r12.finngen.fi";
const BBJ_API: &str = "https://pheweb.jp";

#[derive(Clone, Copy)]
struct Instance {
    key: &'static str,
    label: &'static str,
    base_url: &'static str,
    genome_build: &'static str,
    capabilities: &'static [&'static str],
    notes: &'static str,
}

const INSTANCES: [Instance; 2] = [
    Instance {
        key: "finngen",
        label: "FinnGen R12",
        base_url: FINNGEN_API,
        genome_build: "GRCh38",
        capabilities: &["variant", "gene", "phenotypes", "autocomplete"],
        notes: "~500k Finnish biobank participants; variant ids are chrom-pos-ref-alt on GRCh38. Public DF12 PheWeb remains at r12.finngen.fi.",
    },
    Instance {
        key: "bbj",
        label: "BioBank Japan (pheweb.jp)",
        base_url: BBJ_API,
        genome_build: "GRCh37",
        capabilities: &["variant", "autocomplete"],
        notes: "Variant ids are chrom-pos-ref-alt on GRCh37/hg19, not GRCh38. No gene or full-phenotype-list JSON endpoints.",
    },
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VariantArgs {
    instance: String,
    variant: String,
    #[serde(default = "default_page")]
    max_phenos: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GeneArgs {
    gene_symbol: String,
    #[serde(default = "default_page")]
    max_phenos: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListPhenotypes {
    #[serde(default = "default_finngen")]
    instance: String,
    #[serde(default = "default_pheno_list")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchPhenotypes {
    query: String,
    #[serde(default = "default_finngen")]
    instance: String,
    #[serde(default = "default_page")]
    max_records: u32,
}

fn default_finngen() -> String {
    "finngen".into()
}

pub(super) fn instances() -> Result<Value> {
    let mut map = serde_json::Map::new();
    for instance in INSTANCES {
        map.insert(
            instance.key.into(),
            json!({
                "label": instance.label,
                "base_url": instance.base_url,
                "genome_build": instance.genome_build,
                "capabilities": instance.capabilities,
                "notes": instance.notes,
                "source_url": instance.base_url
            }),
        );
    }
    Ok(json!({
        "source": "PheWeb",
        "instances": map
    }))
}

pub(super) async fn variant(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: VariantArgs =
        serde_json::from_value(args.clone()).context("invalid PheWeb variant arguments")?;
    let instance = require_instance(&args.instance)?;
    require_capability(instance, "variant")?;
    let variant = normalize_variant_id(&args.variant)?;
    let cap = bound_records(args.max_phenos, MAX_PHENOS, "max_phenos")?;
    let url = join_url(
        &instance_base(bio, instance),
        &format!("api/variant/{}", path_segment(&variant)),
    );
    let payload = send_json_not_found(bio, PHEWEB, &url, &[])
        .await?
        .with_context(|| {
            format!(
                "PheWeb instance {} has no record for variant {variant}",
                instance.key
            )
        })?;
    let (meta, mut rows) = variant_payload(&payload)?;
    rows.sort_by_key(phewas_rank);
    let page = cap_rows(rows, cap);
    Ok(json!({
        "source": instance.label,
        "source_url": format!("{}/variant/{variant}", instance.base_url),
        "instance": instance.key,
        "genome_build": instance.genome_build,
        "variant": variant,
        "variant_meta": meta,
        "total": page.total,
        "returned": page.returned,
        "truncated": page.truncated,
        "phenotypes": page.rows
    }))
}

pub(super) async fn finngen_gene(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GeneArgs =
        serde_json::from_value(args.clone()).context("invalid FinnGen gene PheWAS arguments")?;
    let instance = require_instance("finngen")?;
    require_capability(instance, "gene")?;
    let gene = require_gene_symbol(&args.gene_symbol)?;
    let cap = bound_records(args.max_phenos, MAX_PHENOS, "max_phenos")?;
    let url = join_url(
        &instance_base(bio, instance),
        &format!("api/gene_phenos/{}", path_segment(&gene)),
    );
    let payload = send_json_not_found(bio, PHEWEB, &url, &[])
        .await?
        .with_context(|| format!("PheWeb instance finngen has no record for gene {gene}"))?;
    let mut rows = gene_payload(&payload)?;
    rows.sort_by_key(phewas_rank);
    let page = cap_rows(rows, cap);
    Ok(json!({
        "source": instance.label,
        "source_url": format!("{}/gene/{gene}", instance.base_url),
        "instance": instance.key,
        "genome_build": instance.genome_build,
        "gene_symbol": gene,
        "total": page.total,
        "returned": page.returned,
        "truncated": page.truncated,
        "phenotypes": page.rows
    }))
}

pub(super) async fn list_phenotypes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListPhenotypes = serde_json::from_value(args.clone())
        .context("invalid PheWeb phenotype listing arguments")?;
    let instance = require_instance(&args.instance)?;
    require_capability(instance, "phenotypes")?;
    let cap = bound_records(args.max_records, MAX_PHENO_LIST, "max_records")?;
    let url = join_url(&instance_base(bio, instance), "api/phenos");
    let payload = send_json(bio, PHEWEB, &url, &[]).await?;
    let mut rows = pheno_list(&payload)?;
    rows.sort_by(|a, b| phenocode(a).cmp(&phenocode(b)));
    let page = cap_rows(rows, cap);
    Ok(json!({
        "source": instance.label,
        "source_url": format!("{}/", instance.base_url),
        "instance": instance.key,
        "total": page.total,
        "returned": page.returned,
        "truncated": page.truncated,
        "phenotypes": page.rows
    }))
}

pub(super) async fn search_phenotypes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchPhenotypes = serde_json::from_value(args.clone())
        .context("invalid PheWeb phenotype search arguments")?;
    let instance = require_instance(&args.instance)?;
    require_capability(instance, "autocomplete")?;
    let query = require_text(&args.query, "query", 1, 256)?;
    let cap = bound_records(
        args.max_records,
        DEFAULT_PAGE.max(MAX_PHENOS),
        "max_records",
    )?;
    let url = join_url(&instance_base(bio, instance), "api/autocomplete");
    let payload = send_json(bio, PHEWEB, &url, &[("query".into(), query.clone())]).await?;
    let rows = autocomplete_rows(&payload)?;
    let page = cap_rows(rows, cap);
    Ok(json!({
        "source": instance.label,
        "source_url": format!("{}/", instance.base_url),
        "instance": instance.key,
        "query": query,
        "total": page.total,
        "returned": page.returned,
        "truncated": page.truncated,
        "matches": page.rows
    }))
}

fn require_instance(name: &str) -> Result<&'static Instance> {
    let key = name.trim().to_ascii_lowercase();
    INSTANCES
        .iter()
        .find(|instance| instance.key == key)
        .ok_or_else(|| anyhow::anyhow!("unknown PheWeb instance {name:?}; known: finngen, bbj"))
}

fn require_capability(instance: &Instance, capability: &str) -> Result<()> {
    if instance.capabilities.contains(&capability) {
        Ok(())
    } else {
        bail!(
            "instance {} ({}) has no {capability} endpoint; capabilities: {:?}",
            instance.key,
            instance.label,
            instance.capabilities
        )
    }
}

fn instance_base(bio: &NativeBio, instance: &Instance) -> String {
    let credential = match instance.key {
        "finngen" => "PHEWEB_FINNGEN_BASE_URL",
        "bbj" => "PHEWEB_BBJ_BASE_URL",
        _ => return instance.base_url.to_string(),
    };
    override_base(bio, credential, instance.base_url)
}

pub(super) fn normalize_variant_id(variant: &str) -> Result<String> {
    let trimmed = variant.trim();
    if trimmed.is_empty() {
        bail!("variant must be chrom-pos-ref-alt, e.g. 19-44908822-C-T");
    }
    let mut value = trimmed.replace([':', '_', '/'], "-");
    if value.to_ascii_lowercase().starts_with("chr") {
        value = value[3..].to_string();
    }
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 4 || parts[1].is_empty() || !parts[1].bytes().all(|b| b.is_ascii_digit()) {
        bail!("variant must be chrom-pos-ref-alt, e.g. 19-44908822-C-T");
    }
    if parts
        .iter()
        .any(|part| part.is_empty() || part.contains(".."))
    {
        bail!("variant must be chrom-pos-ref-alt, e.g. 19-44908822-C-T");
    }
    Ok(parts.join("-"))
}

fn variant_payload(payload: &Value) -> Result<(Value, Vec<Value>)> {
    if payload.get("results").is_some() {
        let var = payload.get("variant").cloned().unwrap_or(json!({}));
        let annotation = var.get("annotation").cloned().unwrap_or(json!({}));
        let meta = json!({
            "chrom": stringify_field(var.get("chr")),
            "pos": var.get("pos"),
            "ref": var.get("ref"),
            "alt": var.get("alt"),
            "rsids": annotation.get("rsids"),
            "gnomad": lean_gnomad(annotation.get("gnomad")),
            "nearest_genes": null
        });
        let rows = payload
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|row| lean_assoc(&row, None))
            .collect();
        Ok((meta, rows))
    } else {
        let meta = json!({
            "chrom": stringify_field(payload.get("chrom")),
            "pos": payload.get("pos"),
            "ref": payload.get("ref"),
            "alt": payload.get("alt"),
            "rsids": payload.get("rsids").cloned().filter(|v| !v.is_null()),
            "gnomad": null,
            "nearest_genes": payload.get("nearest_genes")
        });
        let rows = payload
            .get("phenos")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|row| lean_assoc(&row, None))
            .collect();
        Ok((meta, rows))
    }
}

fn gene_payload(payload: &Value) -> Result<Vec<Value>> {
    let rows = if let Some(Value::Array(items)) = payload.get("phenotypes") {
        items.clone()
    } else if let Value::Array(items) = payload {
        items.clone()
    } else {
        bail!("PheWeb gene_phenos returned an unrecognized payload")
    };
    Ok(rows
        .into_iter()
        .map(|row| {
            let assoc = row.get("assoc").cloned().unwrap_or(json!({}));
            let var = row.get("variant").cloned().unwrap_or(json!({}));
            let variant = json!({
                "chrom": stringify_field(var.get("chr")),
                "pos": var.get("pos"),
                "ref": var.get("ref"),
                "alt": var.get("alt"),
                "varid": var.get("varid"),
                "rsids": var.get("annotation").and_then(|ann| ann.get("rsids")).cloned()
            });
            lean_assoc(&assoc, Some(variant))
        })
        .collect())
}

fn pheno_list(payload: &Value) -> Result<Vec<Value>> {
    let Value::Array(items) = payload else {
        bail!("PheWeb /api/phenos returned a non-list payload");
    };
    Ok(items
        .iter()
        .map(|row| {
            json!({
                "phenocode": row.get("phenocode"),
                "phenostring": row.get("phenostring"),
                "category": row.get("category"),
                "num_cases": row.get("num_cases"),
                "num_controls": row.get("num_controls"),
                "num_gw_significant": row.get("num_gw_significant")
            })
        })
        .collect())
}

fn autocomplete_rows(payload: &Value) -> Result<Vec<Value>> {
    let Value::Array(items) = payload else {
        bail!("PheWeb /api/autocomplete returned a non-list payload");
    };
    Ok(items
        .iter()
        .map(|row| {
            json!({
                "display": row.get("display"),
                "phenocode": row.get("pheno").cloned().or_else(|| row.get("value").cloned()),
                "url": row.get("url")
            })
        })
        .collect())
}

fn lean_assoc(row: &Value, variant: Option<Value>) -> Value {
    let mut out = json!({
        "phenocode": row.get("phenocode"),
        "phenostring": row.get("phenostring"),
        "category": row.get("category"),
        "pval": row.get("pval"),
        "mlogp": row.get("mlogp"),
        "beta": row.get("beta"),
        "sebeta": row.get("sebeta"),
        "af": row.get("af"),
        "maf": row.get("maf"),
        "maf_case": row.get("maf_case"),
        "maf_control": row.get("maf_control"),
        "n_cases": row.get("n_case").cloned().or_else(|| row.get("num_cases").cloned()),
        "n_controls": row.get("n_control").cloned().or_else(|| row.get("num_controls").cloned()),
        "n_samples": row.get("n_sample").cloned().or_else(|| row.get("num_samples").cloned())
    });
    if let Some(variant) = variant {
        out["variant"] = variant;
    }
    out
}

fn lean_gnomad(value: Option<&Value>) -> Value {
    let Some(Value::Object(map)) = value else {
        return Value::Null;
    };
    let mut lean = serde_json::Map::new();
    for key in ["AF", "AF_fin", "AF_nfe", "AF_popmax", "filters", "rsid"] {
        if let Some(item) = map.get(key) {
            lean.insert(key.to_string(), item.clone());
        }
    }
    if lean.is_empty() {
        Value::Null
    } else {
        Value::Object(lean)
    }
}

fn stringify_field(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => json!(text),
        Some(other) => json!(other.to_string()),
        None => json!(""),
    }
}

pub(super) fn phewas_rank(row: &Value) -> (bool, OrderedFloat) {
    if let Some(mlogp) = json_f64(row.get("mlogp")) {
        return (false, OrderedFloat(-mlogp));
    }
    match json_f64(row.get("pval")) {
        Some(pval) if pval <= 0.0 => (false, OrderedFloat(f64::NEG_INFINITY)),
        Some(pval) => (false, OrderedFloat(pval.log10())),
        None => (true, OrderedFloat(0.0)),
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct OrderedFloat(pub f64);

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

struct Capped {
    total: usize,
    returned: usize,
    truncated: bool,
    rows: Vec<Value>,
}

fn cap_rows(rows: Vec<Value>, cap: usize) -> Capped {
    let total = rows.len();
    let returned: Vec<Value> = rows.into_iter().take(cap).collect();
    Capped {
        total,
        returned: returned.len(),
        truncated: total > returned.len(),
        rows: returned,
    }
}

fn phenocode(row: &Value) -> String {
    row.get("phenocode")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
