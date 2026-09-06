//! Sanger Cell Model Passports JSON:API client.
//!
//! Independently implemented from:
//! - <https://depmap.sanger.ac.uk/documentation/api/>
//! - <https://depmap.sanger.ac.uk/documentation/api/endpoints/>
//! - <https://depmap.sanger.ac.uk/documentation/api/modifiers/>
//! - <https://api.cellmodelpassports.sanger.ac.uk/swagger>
//! - <https://jsonapi.org/>
//!
//! Reviewed 2026-09-06. Individual non-commercial use is keyless; commercial
//! use and embedding in third-party websites/apps need prior consent.

use super::{bound_page, path_segment, require_id, NativeBio};
use crate::http::Source;
use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use wisp_llm::ToolSchema;

const CMP_API: &str = "https://api.cellmodelpassports.sanger.ac.uk";
const CMP: Source = Source("Cell Model Passports", Duration::from_millis(500));
const CMP_USAGE: &str = "Cell Model Passports / Sanger DepMap API is free for individual non-commercial use. Commercial use and embedding in third-party websites/apps require prior consent (depmap@sanger.ac.uk). https://depmap.sanger.ac.uk/documentation/api/";
const MODEL_INCLUDE: &str = "sample.tissue,sample.cancer_type,model_msi_status";
const FETCH_PAGE: u32 = 100;
const MAX_PAGES: u32 = 8;
const DEFAULT_RECORDS: u32 = 50;
const LABEL_MAX: usize = 256;
const LEAN_FLAGS: &[&str] = &[
    "crispr_ko_available",
    "mutations_available",
    "rnaseq_available",
];
const DETAIL_FLAGS: &[&str] = &[
    "mutations_available",
    "cnv_available",
    "expression_available",
    "rnaseq_available",
    "crispr_ko_available",
    "drugs_available",
    "fusions_available",
    "methylation_available",
    "proteomics_available",
    "commercial_available",
];

pub(super) fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        cmp_tool(
            "gene_dependencies",
            "CRISPR knock-out dependency scores for one official CMP gene symbol from GET /genes/{SIDG}/datasets/crispr_ko (never the unscoped /datasets/crispr_ko table). Resolves the symbol exactly, then returns a bounded page of Bayes-factor rows (higher BF = more essential) plus meta.count. Optional model_id (SIDM) restricts to one model. A capped page is not the complete screen.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gene_symbol"],
                "properties": {
                    "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                    "model_id": {"type": "string", "minLength": 1, "maxLength": 128},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        cmp_tool(
            "get_model",
            "Retrieve one Sanger Cell Model Passports model by SIDM id (case-insensitive prefix) or exact name/synonym. SIDM uses GET /models/{id} with include=sample.tissue,sample.cancer_type,model_msi_status; names use a names/any filter. Unknown ids and empty documents fail; an ambiguous synonym fails listing candidate SIDM ids.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["model_id_or_name"],
                "properties": {
                    "model_id_or_name": {"type": "string", "minLength": 1, "maxLength": 256}
                }
            }),
        ),
        cmp_tool(
            "list_models",
            "List Sanger Cell Model Passports models (SIDM ids) from GET /models. Optional tissue and cancer_type are case-sensitive exact CMP names (e.g. Lung, Small Cell Lung Carcinoma) applied as nested JSON:API relationship filters. Returns a bounded page; total is meta.count when supplied. Unfiltered /models is ~2000 rows — a capped page is not the complete catalog.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "tissue": {"type": "string", "minLength": 1, "maxLength": 256},
                    "cancer_type": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        cmp_tool(
            "search_genes",
            "Search Sanger Cell Model Passports genes by official CMP symbol (GET /genes). exact=true uses op eq; otherwise case-insensitive substring (ilike %query%). Synonym search is not supported. Returns SIDG rows on a bounded page plus meta.count.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 64},
                    "exact": {"type": "boolean", "default": false},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
        cmp_tool(
            "search_models",
            "Search Sanger Cell Model Passports models via GET /search/{query} and keep resources with type=model. Returns the same lean rows as list_models on a bounded page. Prefer this when the exact CMP name/synonym is unknown.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }),
        ),
    ]
}

fn cmp_tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
) -> (&'static str, ToolSchema) {
    (
        "cancer-models",
        ToolSchema::new(name, &format!("{description} {CMP_USAGE}"), parameters),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListModels {
    tissue: Option<String>,
    cancer_type: Option<String>,
    #[serde(default = "default_records")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetModel {
    model_id_or_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchModels {
    query: String,
    #[serde(default = "default_records")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchGenes {
    query: String,
    #[serde(default)]
    exact: bool,
    #[serde(default = "default_records")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneDependencies {
    gene_symbol: String,
    model_id: Option<String>,
    #[serde(default = "default_records")]
    max_records: u32,
}

fn default_records() -> u32 {
    DEFAULT_RECORDS
}

pub(super) async fn list_models(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListModels = serde_json::from_value(args.clone())
        .context("invalid Cell Model Passports model list arguments")?;
    let cap = bound_page(args.max_records)?;
    let tissue = optional_label(args.tissue.as_deref(), "tissue")?;
    let cancer_type = optional_label(args.cancer_type.as_deref(), "cancer_type")?;
    let mut extra = Vec::new();
    let mut filters = Vec::new();
    if let Some(tissue) = &tissue {
        filters.push(has_rel(
            "sample",
            has_rel("tissue", eq_field("name", tissue)),
        ));
    }
    if let Some(cancer_type) = &cancer_type {
        filters.push(has_rel(
            "sample",
            has_rel("cancer_type", eq_field("name", cancer_type)),
        ));
    }
    if !filters.is_empty() {
        extra.push(("filter".into(), encode_filter(&filters)?));
    }
    let cmp = Cmp::new(bio);
    let (mut models, total, truncated) =
        collect_pages(&cmp, "models", &extra, cap, shape_model_row).await?;
    models.sort_by(|a, b| field_text(a, "model_id").cmp(&field_text(b, "model_id")));
    Ok(json!({
        "source": CMP.0,
        "source_url": CMP_API,
        "tissue": tissue,
        "cancer_type": cancer_type,
        "total": total,
        "returned": models.len(),
        "truncated": truncated,
        "models": models,
    }))
}

pub(super) async fn get_model(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetModel = serde_json::from_value(args.clone())
        .context("invalid Cell Model Passports model arguments")?;
    let ident = require_label(&args.model_id_or_name, "model_id_or_name")?;
    let cmp = Cmp::new(bio);
    let include = vec![("include".into(), MODEL_INCLUDE.to_string())];
    let (resource, included) = if looks_like_sidm(&ident) {
        let model_id = require_id(&ident.to_ascii_uppercase(), "model_id_or_name", 128)?;
        let fetched = cmp
            .get(&format!("models/{}", path_segment(&model_id)), &include)
            .await?;
        if fetched.not_found {
            bail!("Cell Model Passports model {model_id} was not found");
        }
        fetched.require_ok(&format!("model {model_id}"))?;
        let resource = data_resources(&fetched.value)
            .into_iter()
            .next()
            .with_context(|| format!("Cell Model Passports model {model_id} was not found"))?;
        (resource, included_resources(&fetched.value))
    } else {
        let filter = encode_filter(&[json!({"name": "names", "op": "any", "val": ident})])?;
        let fetched = cmp
            .get(
                "models",
                &[
                    ("filter".into(), filter),
                    ("include".into(), MODEL_INCLUDE.to_string()),
                    ("page[size]".into(), "5".into()),
                ],
            )
            .await?;
        fetched.require_ok("model name lookup")?;
        let resources = data_resources(&fetched.value);
        if resources.is_empty() {
            bail!("Cell Model Passports model {ident:?} was not found");
        }
        if resources.len() > 1 {
            let ids: Vec<String> = resources
                .iter()
                .filter_map(|row| field_text(row, "id"))
                .collect();
            bail!("Cell Model Passports name {ident:?} is ambiguous: {ids:?}");
        }
        let resource = resources
            .into_iter()
            .next()
            .context("Cell Model Passports omitted model data")?;
        (resource, included_resources(&fetched.value))
    };
    let mut record = shape_model_detail(&resource, &included);
    record["source"] = json!(CMP.0);
    record["source_url"] = json!(CMP_API);
    Ok(record)
}

pub(super) async fn search_models(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchModels = serde_json::from_value(args.clone())
        .context("invalid Cell Model Passports model search arguments")?;
    let cap = bound_page(args.max_records)?;
    let query = require_label(&args.query, "query")?;
    let cmp = Cmp::new(bio);
    let fetched = cmp
        .get(&format!("search/{}", path_segment(&query)), &[])
        .await?;
    fetched.require_ok(&format!("search {query}"))?;
    let resources = data_resources(&fetched.value);
    let dropped_other_types = resources
        .iter()
        .any(|row| field_text(row, "type").as_deref() != Some("model"));
    let mut models: Vec<Value> = resources
        .into_iter()
        .filter(|row| field_text(row, "type").as_deref() == Some("model"))
        .map(|row| shape_model_row(&row))
        .collect();
    let seen = models.len();
    models.sort_by(|a, b| field_text(a, "model_id").cmp(&field_text(b, "model_id")));
    let total = if dropped_other_types {
        seen as u64
    } else {
        meta_count(&fetched.value).unwrap_or(seen as u64)
    };
    models.truncate(cap);
    let truncated = (models.len() as u64) < total;
    Ok(json!({
        "source": CMP.0,
        "source_url": CMP_API,
        "query": query,
        "total": total,
        "returned": models.len(),
        "truncated": truncated,
        "models": models,
    }))
}

pub(super) async fn search_genes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchGenes = serde_json::from_value(args.clone())
        .context("invalid Cell Model Passports gene search arguments")?;
    let cap = bound_page(args.max_records)?;
    let query = require_id(&args.query, "query", 64)?;
    let filters = if args.exact {
        vec![json!({"name": "symbol", "op": "eq", "val": query})]
    } else {
        vec![json!({"name": "symbol", "op": "ilike", "val": format!("%{query}%")})]
    };
    let extra = vec![("filter".into(), encode_filter(&filters)?)];
    let cmp = Cmp::new(bio);
    let (mut genes, total, truncated) =
        collect_pages(&cmp, "genes", &extra, cap, shape_gene).await?;
    genes.sort_by(|a, b| field_text(a, "gene_id").cmp(&field_text(b, "gene_id")));
    Ok(json!({
        "source": CMP.0,
        "source_url": CMP_API,
        "query": query,
        "exact": args.exact,
        "total": total,
        "returned": genes.len(),
        "truncated": truncated,
        "genes": genes,
    }))
}

pub(super) async fn gene_dependencies(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GeneDependencies = serde_json::from_value(args.clone())
        .context("invalid Cell Model Passports gene dependency arguments")?;
    let cap = bound_page(args.max_records)?;
    let gene_symbol = require_id(&args.gene_symbol, "gene_symbol", 64)?;
    let model_id = match args.model_id.as_deref() {
        None => None,
        Some(value) if value.trim().is_empty() => None,
        Some(value) => {
            let cleaned = require_id(value, "model_id", 128)?;
            Some(if looks_like_sidm(&cleaned) {
                cleaned.to_ascii_uppercase()
            } else {
                cleaned
            })
        }
    };
    let cmp = Cmp::new(bio);
    let gene = resolve_gene(&cmp, &gene_symbol).await?;
    let gene_id = field_text(&gene, "gene_id").context("Cell Model Passports gene omitted id")?;
    // Unscoped /datasets/crispr_ko is ~19.5M rows; always gene-scoped.
    let path = format!("genes/{}/datasets/crispr_ko", path_segment(&gene_id));
    let mut extra = Vec::new();
    if let Some(model_id) = &model_id {
        extra.push((
            "filter".into(),
            encode_filter(&[has_rel("model", eq_field("id", model_id))])?,
        ));
    }
    let (mut dependencies, total, truncated) =
        collect_pages(&cmp, &path, &extra, cap, shape_dependency).await?;
    dependencies.sort_by(|a, b| {
        field_text(a, "model_id")
            .cmp(&field_text(b, "model_id"))
            .then_with(|| field_text(a, "source").cmp(&field_text(b, "source")))
    });
    Ok(json!({
        "source": CMP.0,
        "source_url": CMP_API,
        "gene": gene,
        "model_id": model_id,
        "total": total,
        "returned": dependencies.len(),
        "truncated": truncated,
        "dependencies": dependencies,
    }))
}

struct Cmp<'a> {
    bio: &'a NativeBio,
}

struct Fetched {
    not_found: bool,
    value: Value,
    status: StatusCode,
}

impl Fetched {
    fn require_ok(&self, what: &str) -> Result<()> {
        if self.not_found {
            bail!("Cell Model Passports did not find {what}");
        }
        if !self.status.is_success() {
            bail!(
                "Cell Model Passports returned HTTP {} for {what}",
                self.status.as_u16()
            );
        }
        Ok(())
    }
}

impl<'a> Cmp<'a> {
    fn new(bio: &'a NativeBio) -> Self {
        Self { bio }
    }

    fn base(&self) -> String {
        self.bio
            .credential("CMP_BASE_URL")
            .map(|value| value.trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CMP_API.to_string())
    }

    async fn get(&self, path: &str, params: &[(String, String)]) -> Result<Fetched> {
        let url = format!("{}/{}", self.base(), path.trim_start_matches('/'));
        let response = self.bio.http().send(CMP, Method::GET, &url, params).await?;
        decode(response)
    }
}

fn decode(response: crate::http::Response) -> Result<Fetched> {
    if response.status == StatusCode::NOT_FOUND {
        return Ok(Fetched {
            not_found: true,
            value: Value::Null,
            status: response.status,
        });
    }
    if !response.status.is_success() {
        return Ok(Fetched {
            not_found: false,
            value: Value::Null,
            status: response.status,
        });
    }
    let value: Value = serde_json::from_slice(&response.body)
        .context("Cell Model Passports returned invalid JSON")?;
    reject_jsonapi_errors(&value)?;
    Ok(Fetched {
        not_found: false,
        value,
        status: response.status,
    })
}

fn reject_jsonapi_errors(value: &Value) -> Result<()> {
    match value.get("errors").and_then(Value::as_array) {
        Some(errors) if !errors.is_empty() => {
            bail!("Cell Model Passports returned an error document")
        }
        _ => Ok(()),
    }
}

async fn collect_pages(
    cmp: &Cmp<'_>,
    path: &str,
    extra: &[(String, String)],
    cap: usize,
    map: fn(&Value) -> Value,
) -> Result<(Vec<Value>, Option<u64>, bool)> {
    let mut collected = Vec::new();
    let mut total = None;
    let mut exhausted = false;
    for page in 1..=MAX_PAGES {
        let mut params = extra.to_vec();
        params.push(("page[size]".into(), FETCH_PAGE.to_string()));
        params.push(("page[number]".into(), page.to_string()));
        let fetched = cmp.get(path, &params).await?;
        fetched.require_ok(path)?;
        if page == 1 {
            total = meta_count(&fetched.value);
        }
        let items = data_resources(&fetched.value);
        let page_len = items.len();
        for item in items {
            collected.push(map(&item));
            if collected.len() >= cap {
                break;
            }
        }
        if collected.len() >= cap {
            break;
        }
        if page_len < FETCH_PAGE as usize {
            exhausted = true;
            break;
        }
        if total.is_some_and(|count| u64::from(page).saturating_mul(u64::from(FETCH_PAGE)) >= count)
        {
            exhausted = true;
            break;
        }
    }
    let truncated = match total {
        Some(count) => (collected.len() as u64) < count,
        None => !exhausted && collected.len() >= cap,
    };
    collected.truncate(cap);
    Ok((collected, total, truncated))
}

async fn resolve_gene(cmp: &Cmp<'_>, symbol: &str) -> Result<Value> {
    let filter = encode_filter(&[json!({"name": "symbol", "op": "eq", "val": symbol})])?;
    let fetched = cmp
        .get(
            "genes",
            &[("filter".into(), filter), ("page[size]".into(), "5".into())],
        )
        .await?;
    fetched.require_ok(&format!("gene {symbol}"))?;
    let resources = data_resources(&fetched.value);
    if resources.is_empty() {
        bail!("Cell Model Passports gene {symbol:?} was not found");
    }
    if resources.len() > 1 {
        let ids: Vec<String> = resources
            .iter()
            .filter_map(|row| field_text(row, "id"))
            .collect();
        bail!("Cell Model Passports gene {symbol:?} is ambiguous: {ids:?}");
    }
    Ok(shape_gene(&resources[0]))
}

fn shape_model_row(resource: &Value) -> Value {
    let attrs = attributes(resource);
    let mut row = json!({
        "model_id": field_text(resource, "id"),
        "names": sorted_names(attrs),
        "model_type": attrs.get("model_type").cloned().unwrap_or(Value::Null),
        "growth_properties": attrs.get("growth_properties").cloned().unwrap_or(Value::Null),
    });
    copy_flags(&mut row, attrs, LEAN_FLAGS);
    row
}

fn shape_model_detail(resource: &Value, included: &[Value]) -> Value {
    let attrs = attributes(resource);
    let mut record = json!({
        "model_id": field_text(resource, "id"),
        "names": sorted_names(attrs),
        "model_type": attrs.get("model_type").cloned().unwrap_or(Value::Null),
        "growth_properties": attrs.get("growth_properties").cloned().unwrap_or(Value::Null),
        "model_treatment": attrs.get("model_treatment").cloned().unwrap_or(Value::Null),
        "ploidy_wes": attrs.get("ploidy_wes").cloned().unwrap_or(Value::Null),
        "ploidy_wgs": attrs.get("ploidy_wgs").cloned().unwrap_or(Value::Null),
        "mutations_per_mb": attrs.get("mutations_per_mb").cloned().unwrap_or(Value::Null),
        "tissue": Value::Null,
        "cancer_type": Value::Null,
        "msi_status": Value::Null,
        "sample_id": rel_id(resource, "sample"),
    });
    copy_flags(&mut record, attrs, DETAIL_FLAGS);
    for item in included {
        let Some(kind) = field_text(item, "type") else {
            continue;
        };
        let item_attrs = attributes(item);
        match kind.as_str() {
            "tissue" => record["tissue"] = json!(field_text(item_attrs, "name")),
            "cancer_type" => record["cancer_type"] = json!(field_text(item_attrs, "name")),
            "sample" => {
                if record.get("sample_id").and_then(Value::as_str).is_none() {
                    record["sample_id"] = json!(field_text(item, "id"));
                }
            }
            "model_msi_status" if item_attrs.get("current") == Some(&Value::Bool(true)) => {
                record["msi_status"] = item_attrs.get("msi_status").cloned().unwrap_or(Value::Null);
            }
            _ => {}
        }
    }
    record
}

fn shape_gene(resource: &Value) -> Value {
    let attrs = attributes(resource);
    let mut row = json!({
        "gene_id": field_text(resource, "id"),
        "symbol": attrs.get("symbol").cloned().unwrap_or(Value::Null),
        "hgnc_id": attrs.get("hgnc_id").cloned().unwrap_or(Value::Null),
        "hgnc_status": attrs.get("hgnc_status").cloned().unwrap_or(Value::Null),
        "location": attrs.get("location").cloned().unwrap_or(Value::Null),
        "cancer_driver": attrs.get("cancer_driver").cloned().unwrap_or(Value::Null),
        "tumour_suppressor": attrs.get("tumour_suppressor").cloned().unwrap_or(Value::Null),
    });
    if let Some(flag) = attrs.get("in_yusa_lib") {
        row["in_yusa_lib"] = flag.clone();
    }
    row
}

fn shape_dependency(resource: &Value) -> Value {
    let attrs = attributes(resource);
    json!({
        "gene_id": rel_id(resource, "gene"),
        "model_id": rel_id(resource, "model"),
        "source": attrs.get("source").cloned().unwrap_or(Value::Null),
        "bf": attrs.get("bf").cloned().unwrap_or(Value::Null),
        "bf_scaled": attrs.get("bf_scaled").cloned().unwrap_or(Value::Null),
        "fc_clean": attrs.get("fc_clean").cloned().unwrap_or(Value::Null),
        "fc_clean_qn": attrs.get("fc_clean_qn").cloned().unwrap_or(Value::Null),
        "mageck_fdr": attrs.get("mageck_fdr").cloned().unwrap_or(Value::Null),
        "qc_pass": attrs.get("qc_pass").cloned().unwrap_or(Value::Null),
    })
}

fn copy_flags(target: &mut Value, attrs: &Value, keys: &[&str]) {
    for key in keys {
        if let Some(value) = attrs.get(*key) {
            target[*key] = value.clone();
        }
    }
}

fn sorted_names(attrs: &Value) -> Vec<String> {
    let mut names = match attrs.get("names") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(name)) if !name.is_empty() => vec![name.clone()],
        _ => Vec::new(),
    };
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    names
}

fn attributes(resource: &Value) -> &Value {
    resource.get("attributes").unwrap_or(resource)
}

fn data_resources(doc: &Value) -> Vec<Value> {
    match doc.get("data") {
        Some(Value::Array(items)) => items.clone(),
        Some(obj) if obj.is_object() && obj.get("id").is_some() => vec![obj.clone()],
        _ => Vec::new(),
    }
}

fn included_resources(doc: &Value) -> Vec<Value> {
    match doc.get("included") {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn meta_count(doc: &Value) -> Option<u64> {
    let count = doc.get("meta")?.get("count")?;
    match count {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|n| u64::try_from(n).ok())),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn rel_id(resource: &Value, name: &str) -> Option<String> {
    let data = resource.get("relationships")?.get(name)?.get("data")?;
    match data {
        Value::Object(_) => field_text(data, "id"),
        Value::Array(items) => items.first().and_then(|item| field_text(item, "id")),
        _ => None,
    }
}

fn has_rel(name: &str, val: Value) -> Value {
    json!({"name": name, "op": "has", "val": val})
}

fn eq_field(name: &str, val: &str) -> Value {
    json!({"name": name, "op": "eq", "val": val})
}

fn encode_filter(filters: &[Value]) -> Result<String> {
    serde_json::to_string(filters).context("failed to encode Cell Model Passports filter")
}

fn looks_like_sidm(value: &str) -> bool {
    value.len() >= 4 && value[..4].eq_ignore_ascii_case("SIDM")
}

pub(super) fn require_label(value: &str, what: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{what} is required");
    }
    if trimmed.len() > LABEL_MAX {
        bail!("{what} exceeds {LABEL_MAX} characters");
    }
    if trimmed.chars().any(|ch| ch.is_control()) {
        bail!("{what} must not contain control characters");
    }
    if trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#') {
        bail!("{what} must not contain URL path characters");
    }
    Ok(trimmed.to_string())
}

fn optional_label(value: Option<&str>, what: &str) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => Ok(Some(require_label(text, what)?)),
    }
}

fn field_text(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}
