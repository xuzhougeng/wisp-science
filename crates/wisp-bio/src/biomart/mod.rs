//! Native BioMart domain against the Ensembl martservice. Independently
//! implemented from:
//!
//! - [Ensembl BioMart](https://www.ensembl.org/info/data/biomart/index.html)
//! - [How to use BioMart](https://www.ensembl.org/info/data/biomart/how_to_use_biomart.html)
//! - [BioMart RESTful access](https://www.ensembl.org/info/data/biomart/biomart_restful.html)
//! - [martservice usage](https://www.ensembl.org/biomart/martservice)
//!
//! References reviewed 2026-09-06. Metadata is GET `type=registry|datasets|
//! attributes|filters`. Queries are a form POST of Query XML (`query=`), TSV
//! formatter, `completionStamp=1` (trailing `[success]`). No API key is
//! published. Tests use invented records.

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;
use wisp_llm::ToolSchema;

const MARTSERVICE: &str = "https://www.ensembl.org/biomart/martservice";
const MARTVIEW: &str = "https://www.ensembl.org/biomart/martview";
const ENSEMBL_ID: &str = "https://www.ensembl.org/id";
const BIOMART: Source = Source("Ensembl BioMart", Duration::from_millis(400));
const COMPLETION_STAMP: &str = "[success]";
const FEATURE_PAGE: &str = "feature_page";
const DEFAULT_PAGE: u32 = 50;
const DEFAULT_LIST: u32 = 200;
const MAX_PAGE: u32 = 500;
const MAX_ATTRIBUTES: usize = 20;
const MAX_FILTERS: usize = 20;
const MAX_TARGETS: usize = 200;
const MAX_IDENT: usize = 128;
const MAX_VALUE: usize = 256;
const MAX_FILTER_JOIN: usize = 8192;

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "biomart",
            ToolSchema::new(
                "list_marts",
                "List Ensembl BioMart marts from GET /biomart/martservice?type=registry. A mart is a BioMart database (Ensembl Genes, Variation, Regulation, …). Returns a bounded page of names, display names, visibility and source URLs; it is not a dataset listing.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                    }
                }),
            ),
        ),
        (
            "biomart",
            ToolSchema::new(
                "list_datasets",
                "List datasets in one Ensembl BioMart mart via GET type=datasets&mart=. Each dataset is a species/table set (for example hsapiens_gene_ensembl). Returns a bounded page of dataset names, display names, assembly/version and visibility. A capped page is not every dataset in the mart.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["mart"],
                    "properties": {
                        "mart": {"type": "string", "minLength": 1, "maxLength": 128,
                            "description": "Mart identifier from list_marts, e.g. ENSEMBL_MART_ENSEMBL"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                    }
                }),
            ),
        ),
        (
            "biomart",
            ToolSchema::new(
                "list_common_attributes",
                "List attributes on the Ensembl Genes Features page (attribute page feature_page) for a dataset, via GET type=attributes&dataset=. That is the default BioMart export panel (gene/transcript/protein IDs, names, coordinates). If the dataset has no feature_page, the first page in the TSV is used. Homolog and microarray pages are omitted. The response is a bounded page.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["mart", "dataset"],
                    "properties": {
                        "mart": {"type": "string", "minLength": 1, "maxLength": 128},
                        "dataset": {"type": "string", "minLength": 1, "maxLength": 128,
                            "description": "Dataset identifier, e.g. hsapiens_gene_ensembl"},
                        "page": {"type": "string", "minLength": 1, "maxLength": 128,
                            "description": "Attribute page to list; default is feature_page when present"},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                    }
                }),
            ),
        ),
        (
            "biomart",
            ToolSchema::new(
                "list_all_attributes",
                "List every BioMart attribute for a dataset via GET type=attributes&dataset=, including homolog and microarray pages. Optional page restricts to one attribute panel. The response is a bounded page and reports total_available; it is not the complete attribute catalogue when truncated.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["mart", "dataset"],
                    "properties": {
                        "mart": {"type": "string", "minLength": 1, "maxLength": 128},
                        "dataset": {"type": "string", "minLength": 1, "maxLength": 128},
                        "page": {"type": "string", "minLength": 1, "maxLength": 128},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                    }
                }),
            ),
        ),
        (
            "biomart",
            ToolSchema::new(
                "list_filters",
                "List BioMart filters for a dataset via GET type=filters&dataset=. Filters restrict a query (ID lists, regions, biotype, boolean with_/without_ flags). Option lists such as chromosome names are summarized by count, not expanded. The response is a bounded page.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["mart", "dataset"],
                    "properties": {
                        "mart": {"type": "string", "minLength": 1, "maxLength": 128},
                        "dataset": {"type": "string", "minLength": 1, "maxLength": 128},
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                    }
                }),
            ),
        ),
        (
            "biomart",
            ToolSchema::new(
                "get_data",
                "Query Ensembl BioMart for a bounded TSV table. Requires a dataset, 1–20 attributes and at least one filter (unfiltered queries can dump a whole annotation set). Posts Query XML with formatter=TSV and completionStamp=1; a missing [success] stamp is an error, not a short table. Returns records keyed by attribute name, source URLs, and truncated when more rows were available than max_results (default 50, max 500).",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["mart", "dataset", "attributes", "filters"],
                    "properties": {
                        "mart": {"type": "string", "minLength": 1, "maxLength": 128},
                        "dataset": {"type": "string", "minLength": 1, "maxLength": 128},
                        "attributes": {
                            "type": "array", "minItems": 1, "maxItems": 20,
                            "items": {"type": "string", "minLength": 1, "maxLength": 128}
                        },
                        "filters": {
                            "type": "object",
                            "minProperties": 1,
                            "additionalProperties": {
                                "type": ["string", "boolean", "integer", "array"],
                                "items": {"type": "string", "minLength": 1, "maxLength": 256}
                            }
                        },
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50}
                    }
                }),
            ),
        ),
        (
            "biomart",
            ToolSchema::new(
                "get_translation",
                "Translate one identifier between BioMart attributes (for example hgnc_symbol → ensembl_gene_id) by querying the dataset with an ID-list filter, not a full-table scan. from_attr must be a filter on that dataset. Missing identifiers are reported as found=false; a Query ERROR is a tool failure. At most 200 characters per identifier.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["mart", "dataset", "from_attr", "to_attr", "target"],
                    "properties": {
                        "mart": {"type": "string", "minLength": 1, "maxLength": 128},
                        "dataset": {"type": "string", "minLength": 1, "maxLength": 128},
                        "from_attr": {"type": "string", "minLength": 1, "maxLength": 128},
                        "to_attr": {"type": "string", "minLength": 1, "maxLength": 128},
                        "target": {"type": "string", "minLength": 1, "maxLength": 256}
                    }
                }),
            ),
        ),
        (
            "biomart",
            ToolSchema::new(
                "batch_translate",
                "Translate up to 200 identifiers between BioMart attributes in one ID-list query (from_attr must be a filter). Returns translations for identifiers that matched, not_found for the rest, per-row records when a source maps to several values, and source URLs. A capped row page is not every transcript/homolog row BioMart can emit.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["mart", "dataset", "from_attr", "to_attr", "targets"],
                    "properties": {
                        "mart": {"type": "string", "minLength": 1, "maxLength": 128},
                        "dataset": {"type": "string", "minLength": 1, "maxLength": 128},
                        "from_attr": {"type": "string", "minLength": 1, "maxLength": 128},
                        "to_attr": {"type": "string", "minLength": 1, "maxLength": 128},
                        "targets": {
                            "type": "array", "minItems": 1, "maxItems": 200,
                            "items": {"type": "string", "minLength": 1, "maxLength": 256}
                        },
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 500, "default": 200}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "list_marts" => list_marts(bio, args).await,
        "list_datasets" => list_datasets(bio, args).await,
        "list_common_attributes" => list_attributes(bio, args, true).await,
        "list_all_attributes" => list_attributes(bio, args, false).await,
        "list_filters" => list_filters(bio, args).await,
        "get_data" => get_data(bio, args).await,
        "get_translation" => get_translation(bio, args).await,
        "batch_translate" => batch_translate(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListMarts {
    #[serde(default = "default_list")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDatasets {
    mart: String,
    #[serde(default = "default_list")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListMeta {
    mart: String,
    dataset: String,
    page: Option<String>,
    #[serde(default = "default_list")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListFilters {
    mart: String,
    dataset: String,
    #[serde(default = "default_list")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetData {
    mart: String,
    dataset: String,
    attributes: Vec<String>,
    filters: BTreeMap<String, Value>,
    #[serde(default = "default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetTranslation {
    mart: String,
    dataset: String,
    from_attr: String,
    to_attr: String,
    target: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchTranslate {
    mart: String,
    dataset: String,
    from_attr: String,
    to_attr: String,
    targets: Vec<String>,
    #[serde(default = "default_list")]
    max_results: u32,
}

fn default_page() -> u32 {
    DEFAULT_PAGE
}

fn default_list() -> u32 {
    DEFAULT_LIST
}

async fn list_marts(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListMarts =
        serde_json::from_value(args.clone()).context("invalid BioMart list_marts arguments")?;
    let cap = bound_page(args.max_results)?;
    let body = mart_get(bio, vec![("type".into(), "registry".into())]).await?;
    let marts = parse_registry(&body)?;
    Ok(listing(
        "marts",
        marts,
        cap,
        json!({}),
        json!({"endpoint": "type=registry"}),
    ))
}

async fn list_datasets(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListDatasets =
        serde_json::from_value(args.clone()).context("invalid BioMart list_datasets arguments")?;
    let mart = require_ident(&args.mart, "mart")?;
    let cap = bound_page(args.max_results)?;
    let body = mart_get(
        bio,
        vec![
            ("type".into(), "datasets".into()),
            ("mart".into(), mart.clone()),
        ],
    )
    .await?;
    let datasets = parse_datasets(&body, &mart)?;
    Ok(listing(
        "datasets",
        datasets,
        cap,
        json!({"mart": mart}),
        json!({"endpoint": "type=datasets"}),
    ))
}

async fn list_attributes(bio: &NativeBio, args: &Value, common: bool) -> Result<Value> {
    let args: ListMeta = serde_json::from_value(args.clone())
        .context("invalid BioMart list_attributes arguments")?;
    let mart = require_ident(&args.mart, "mart")?;
    let dataset = require_ident(&args.dataset, "dataset")?;
    let requested = args
        .page
        .as_deref()
        .map(|page| require_ident(page, "page"))
        .transpose()?;
    let cap = bound_page(args.max_results)?;
    let body = mart_get(
        bio,
        vec![
            ("type".into(), "attributes".into()),
            ("dataset".into(), dataset.clone()),
        ],
    )
    .await?;
    let parsed = parse_attributes(&body, &dataset)?;
    let pages = attribute_pages(&parsed);
    let page = select_page(&parsed, requested.as_deref(), common);
    let filtered: Vec<Value> = parsed
        .into_iter()
        .filter(|attr| page.as_deref().is_none_or(|want| attr.page == want))
        .map(attribute_json)
        .collect();
    let mut result = listing(
        "attributes",
        filtered,
        cap,
        json!({"mart": mart, "dataset": dataset, "page": page}),
        json!({"endpoint": "type=attributes", "pages": pages}),
    );
    result["common"] = json!(common);
    Ok(result)
}

async fn list_filters(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListFilters =
        serde_json::from_value(args.clone()).context("invalid BioMart list_filters arguments")?;
    let mart = require_ident(&args.mart, "mart")?;
    let dataset = require_ident(&args.dataset, "dataset")?;
    let cap = bound_page(args.max_results)?;
    let body = mart_get(
        bio,
        vec![
            ("type".into(), "filters".into()),
            ("dataset".into(), dataset.clone()),
        ],
    )
    .await?;
    let filters = parse_filters(&body, &dataset)?;
    Ok(listing(
        "filters",
        filters,
        cap,
        json!({"mart": mart, "dataset": dataset}),
        json!({"endpoint": "type=filters"}),
    ))
}

async fn get_data(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetData =
        serde_json::from_value(args.clone()).context("invalid BioMart get_data arguments")?;
    let mart = require_ident(&args.mart, "mart")?;
    let dataset = require_ident(&args.dataset, "dataset")?;
    let attributes = require_attributes(&args.attributes)?;
    let filters = require_filters(&args.filters)?;
    let cap = bound_page(args.max_results)?;
    let xml = build_query_xml(&dataset, &attributes, &filters)?;
    let body = mart_query(bio, &xml).await?;
    let rows = parse_tsv_rows(&body, attributes.len())?;
    Ok(data_page(
        &mart,
        &dataset,
        &attributes,
        &args.filters,
        rows,
        cap,
    ))
}

async fn get_translation(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetTranslation = serde_json::from_value(args.clone())
        .context("invalid BioMart get_translation arguments")?;
    let mart = require_ident(&args.mart, "mart")?;
    let dataset = require_ident(&args.dataset, "dataset")?;
    let from_attr = require_ident(&args.from_attr, "from_attr")?;
    let to_attr = require_ident(&args.to_attr, "to_attr")?;
    if from_attr == to_attr {
        bail!("from_attr and to_attr must be different BioMart attributes");
    }
    let target = require_value(&args.target, "target")?;
    let mapped = translate(bio, &dataset, &from_attr, &to_attr, &[target.clone()], 500).await?;
    let values = mapped.get(&target).cloned().unwrap_or_default();
    let value = values.first().cloned();
    let found = value.is_some();
    Ok(json!({
        "source": "Ensembl BioMart",
        "source_url": MARTSERVICE,
        "martview_url": MARTVIEW,
        "query": {
            "mart": mart,
            "dataset": dataset,
            "from_attr": from_attr,
            "to_attr": to_attr,
            "target": target,
        },
        "found": found,
        "value": value,
        "values": values,
        "match_count": mapped.get(&target).map(Vec::len).unwrap_or(0),
        "url": value.as_deref().and_then(ensembl_id_url),
    }))
}

async fn batch_translate(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: BatchTranslate = serde_json::from_value(args.clone())
        .context("invalid BioMart batch_translate arguments")?;
    let mart = require_ident(&args.mart, "mart")?;
    let dataset = require_ident(&args.dataset, "dataset")?;
    let from_attr = require_ident(&args.from_attr, "from_attr")?;
    let to_attr = require_ident(&args.to_attr, "to_attr")?;
    if from_attr == to_attr {
        bail!("from_attr and to_attr must be different BioMart attributes");
    }
    let targets = require_targets(&args.targets)?;
    let cap = bound_page(args.max_results)?;
    let mapped = translate(bio, &dataset, &from_attr, &to_attr, &targets, cap).await?;
    let mut translations = Map::new();
    let mut not_found = Vec::new();
    let mut records = Vec::new();
    for target in &targets {
        match mapped.get(target) {
            Some(values) if !values.is_empty() => {
                translations.insert(target.clone(), json!(values[0]));
                let mut record = json!({
                    "from": target,
                    "to": values[0],
                    "values": values,
                    "found": true,
                });
                if let Some(url) = ensembl_id_url(&values[0]) {
                    record["url"] = json!(url);
                }
                records.push(record);
            }
            _ => {
                not_found.push(target.clone());
                records.push(json!({
                    "from": target,
                    "found": false,
                    "to": Value::Null,
                    "values": []
                }));
            }
        }
    }
    let found_count = translations.len();
    Ok(json!({
        "source": "Ensembl BioMart",
        "source_url": MARTSERVICE,
        "martview_url": MARTVIEW,
        "query": {
            "mart": mart,
            "dataset": dataset,
            "from_attr": from_attr,
            "to_attr": to_attr,
            "targets": targets,
        },
        "translations": translations,
        "not_found": not_found,
        "found_count": found_count,
        "not_found_count": not_found.len(),
        "returned": records.len(),
        "records": records,
    }))
}

async fn translate(
    bio: &NativeBio,
    dataset: &str,
    from_attr: &str,
    to_attr: &str,
    targets: &[String],
    cap: usize,
) -> Result<BTreeMap<String, Vec<String>>> {
    let attributes = vec![from_attr.to_string(), to_attr.to_string()];
    let filters = vec![Filter::List(from_attr.to_string(), targets.to_vec())];
    let xml = build_query_xml(dataset, &attributes, &filters)?;
    let body = mart_query(bio, &xml).await?;
    let rows = parse_tsv_rows(&body, 2)?;
    let mut mapped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows.into_iter().take(cap) {
        let from = row[0].trim();
        let to = row[1].trim();
        if from.is_empty() || to.is_empty() {
            continue;
        }
        let values = mapped.entry(from.to_string()).or_default();
        if !values.iter().any(|existing| existing == to) {
            values.push(to.to_string());
        }
    }
    Ok(mapped)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Filter {
    Value(String, String),
    List(String, Vec<String>),
    Include(String),
    Exclude(String),
}

struct Attribute {
    name: String,
    display_name: String,
    description: String,
    page: String,
}

fn build_query_xml(dataset: &str, attributes: &[String], filters: &[Filter]) -> Result<String> {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE Query><Query virtualSchemaName="default" formatter="TSV" header="0" uniqueRows="0" datasetConfigVersion="0.6" completionStamp="1"><Dataset name=""#,
    );
    xml.push_str(&xml_escape(dataset));
    xml.push_str(r#"" interface="default">"#);
    for filter in filters {
        match filter {
            Filter::Value(name, value) => {
                xml.push_str("<Filter name=\"");
                xml.push_str(&xml_escape(name));
                xml.push_str("\" value=\"");
                xml.push_str(&xml_escape(value));
                xml.push_str("\"/>");
            }
            Filter::List(name, values) => {
                xml.push_str("<Filter name=\"");
                xml.push_str(&xml_escape(name));
                xml.push_str("\" value=\"");
                xml.push_str(&xml_escape(&values.join(",")));
                xml.push_str("\"/>");
            }
            Filter::Include(name) => {
                xml.push_str("<Filter name=\"");
                xml.push_str(&xml_escape(name));
                xml.push_str("\" excluded=\"0\"/>");
            }
            Filter::Exclude(name) => {
                xml.push_str("<Filter name=\"");
                xml.push_str(&xml_escape(name));
                xml.push_str("\" excluded=\"1\"/>");
            }
        }
    }
    for attr in attributes {
        xml.push_str("<Attribute name=\"");
        xml.push_str(&xml_escape(attr));
        xml.push_str("\"/>");
    }
    xml.push_str("</Dataset></Query>");
    if xml.len() > MAX_FILTER_JOIN * 2 {
        bail!(
            "BioMart query XML exceeded the per-call size bound; use fewer filters or identifiers"
        );
    }
    Ok(xml)
}

fn parse_registry(text: &str) -> Result<Vec<Value>> {
    let doc = crate::xml::parse(text)?;
    if !doc.root_element().has_tag_name("MartRegistry") {
        bail!("Ensembl BioMart registry was not a MartRegistry document");
    }
    let mut marts = Vec::new();
    for node in doc
        .descendants()
        .filter(|node| node.has_tag_name("MartURLLocation"))
    {
        let name = node.attribute("name").unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        marts.push(json!({
            "name": name,
            "display_name": node.attribute("displayName").unwrap_or(""),
            "database": node.attribute("database").unwrap_or(""),
            "host": node.attribute("host").unwrap_or(""),
            "path": node.attribute("path").unwrap_or(""),
            "virtual_schema": node.attribute("serverVirtualSchema").unwrap_or("default"),
            "visible": node.attribute("visible") == Some("1"),
            "default": node.attribute("default") == Some("1"),
        }));
    }
    if marts.is_empty() {
        bail!("Ensembl BioMart registry listed no marts");
    }
    Ok(marts)
}

fn parse_datasets(text: &str, mart: &str) -> Result<Vec<Value>> {
    let mut datasets = Vec::new();
    for line in tsv_lines(text) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.first().copied() != Some("TableSet") {
            continue;
        }
        if fields.len() < 3 {
            bail!("Ensembl BioMart datasets response had a malformed TableSet row");
        }
        let name = fields[1].trim();
        if name.is_empty() {
            continue;
        }
        datasets.push(json!({
            "name": name,
            "display_name": fields[2],
            "visible": fields.get(3).copied() == Some("1"),
            "assembly": fields.get(4).copied().unwrap_or(""),
            "interface": fields.get(7).copied().unwrap_or("default"),
            "last_updated": fields.get(8).copied().unwrap_or(""),
        }));
    }
    if datasets.is_empty() {
        bail!("Ensembl BioMart returned no datasets for mart {mart}");
    }
    Ok(datasets)
}

fn parse_attributes(text: &str, dataset: &str) -> Result<Vec<Attribute>> {
    let mut attributes = Vec::new();
    for line in tsv_lines(text) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 4 {
            bail!("Ensembl BioMart attributes response had a malformed row");
        }
        let name = fields[0].trim();
        if name.is_empty() {
            continue;
        }
        attributes.push(Attribute {
            name: name.to_string(),
            display_name: fields[1].to_string(),
            description: fields[2].to_string(),
            page: fields[3].to_string(),
        });
    }
    if attributes.is_empty() {
        bail!("Ensembl BioMart returned no attributes for dataset {dataset}");
    }
    Ok(attributes)
}

fn parse_filters(text: &str, dataset: &str) -> Result<Vec<Value>> {
    let mut filters = Vec::new();
    for line in tsv_lines(text) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            bail!("Ensembl BioMart filters response had a malformed row");
        }
        let name = fields[0].trim();
        if name.is_empty() {
            continue;
        }
        filters.push(json!({
            "name": name,
            "display_name": fields[1],
            "n_options": option_count(fields[2]),
            "description": empty_to_null(fields[3]),
            "page": fields[4],
            "type": fields[5],
            "operator": fields[6],
        }));
    }
    if filters.is_empty() {
        bail!("Ensembl BioMart returned no filters for dataset {dataset}");
    }
    Ok(filters)
}

fn parse_tsv_rows(text: &str, n_cols: usize) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    for line in tsv_lines(text) {
        let fields: Vec<String> = line.split('\t').map(str::to_string).collect();
        if fields.len() != n_cols {
            bail!(
                "Ensembl BioMart TSV row had {} columns, expected {n_cols}",
                fields.len()
            );
        }
        rows.push(fields);
    }
    Ok(rows)
}

fn data_page(
    mart: &str,
    dataset: &str,
    attributes: &[String],
    filters: &BTreeMap<String, Value>,
    rows: Vec<Vec<String>>,
    cap: usize,
) -> Value {
    let total = rows.len();
    let records: Vec<Value> = rows
        .into_iter()
        .take(cap)
        .map(|fields| record_object(attributes, &fields))
        .collect();
    json!({
        "source": "Ensembl BioMart",
        "source_url": MARTSERVICE,
        "martview_url": MARTVIEW,
        "query": {
            "mart": mart,
            "dataset": dataset,
            "attributes": attributes,
            "filters": filters,
        },
        "columns": attributes,
        "total_available": total,
        "returned": records.len(),
        "truncated": total > records.len(),
        "records": records,
    })
}

fn record_object(columns: &[String], fields: &[String]) -> Value {
    let mut record = Map::new();
    for (column, value) in columns.iter().zip(fields) {
        record.insert(column.clone(), json!(value));
    }
    if !record.contains_key("url") {
        if let Some(url) = columns
            .iter()
            .zip(fields)
            .find(|(column, _)| *column == "ensembl_gene_id" || *column == "ensembl_transcript_id")
            .and_then(|(_, value)| ensembl_id_url(value))
        {
            record.insert("url".into(), json!(url));
        }
    }
    Value::Object(record)
}

fn listing(key: &str, items: Vec<Value>, cap: usize, query: Value, extra: Value) -> Value {
    let total = items.len();
    let returned: Vec<Value> = items.into_iter().take(cap).collect();
    let mut result = json!({
        "source": "Ensembl BioMart",
        "source_url": MARTSERVICE,
        "martview_url": MARTVIEW,
        "query": query,
        "total_available": total,
        "returned": returned.len(),
        "truncated": total > returned.len(),
    });
    if let Value::Object(extra) = extra {
        for (k, v) in extra {
            result[k] = v;
        }
    }
    result[key] = json!(returned);
    result
}

fn attribute_json(attr: Attribute) -> Value {
    json!({
        "name": attr.name,
        "display_name": attr.display_name,
        "description": empty_to_null(&attr.description),
        "page": attr.page,
    })
}

fn attribute_pages(attrs: &[Attribute]) -> Vec<String> {
    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    for attr in attrs {
        if !attr.page.is_empty() && seen.insert(attr.page.clone()) {
            pages.push(attr.page.clone());
        }
    }
    pages
}

fn select_page(attrs: &[Attribute], requested: Option<&str>, common: bool) -> Option<String> {
    if let Some(page) = requested {
        return Some(page.to_string());
    }
    if !common {
        return None;
    }
    if attrs.iter().any(|attr| attr.page == FEATURE_PAGE) {
        return Some(FEATURE_PAGE.to_string());
    }
    attrs
        .iter()
        .map(|attr| attr.page.as_str())
        .find(|page| !page.is_empty())
        .map(str::to_string)
}

fn option_count(options: &str) -> usize {
    let trimmed = options.trim();
    if let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        let inner = inner.trim();
        if inner.is_empty() {
            0
        } else {
            inner.split(',').count()
        }
    } else {
        0
    }
}

fn empty_to_null(value: &str) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        json!(value)
    }
}

fn tsv_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
}

async fn mart_get(bio: &NativeBio, params: Vec<(String, String)>) -> Result<String> {
    read_mart(bio, Method::GET, &params, false).await
}

async fn mart_query(bio: &NativeBio, xml: &str) -> Result<String> {
    let text = read_mart(
        bio,
        Method::POST,
        &[("query".into(), xml.to_string())],
        true,
    )
    .await?;
    complete_tsv(&text)
}

async fn read_mart(
    bio: &NativeBio,
    method: Method,
    params: &[(String, String)],
    query: bool,
) -> Result<String> {
    let url = martservice(bio);
    let response = bio.http().send(BIOMART, method, &url, params).await?;
    response.check()?;
    if looks_like_html(&response.body) {
        bail!(
            "Ensembl BioMart returned an HTML page instead of martservice data (outage or maintenance)"
        );
    }
    let text =
        String::from_utf8(response.body).context("Ensembl BioMart returned invalid UTF-8")?;
    if is_query_error(&text) {
        if query {
            bail!("Ensembl BioMart rejected the query (Query ERROR)");
        }
        bail!("Ensembl BioMart rejected the metadata request");
    }
    Ok(text)
}

fn complete_tsv(text: &str) -> Result<String> {
    let stripped = text.trim_end_matches(['\r', '\n', ' ', '\t']);
    let Some(body) = stripped.strip_suffix(COMPLETION_STAMP) else {
        bail!("Ensembl BioMart response was truncated (missing [success] completion stamp)");
    };
    Ok(body.trim_end_matches(['\r', '\n']).to_string())
}

fn is_query_error(text: &str) -> bool {
    let head = text.trim_start();
    head.starts_with("Query ERROR")
        || head.starts_with("Problem retrieving")
        || head.contains("BioMart::Exception")
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

fn martservice(bio: &NativeBio) -> String {
    bio.credential("BIOMART_BASE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| MARTSERVICE.to_string())
}

fn ensembl_id_url(id: &str) -> Option<String> {
    let id = id.trim();
    if id.len() < 8 || id.len() > 32 {
        return None;
    }
    let rest = id.strip_prefix("ENS")?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some(format!("{ENSEMBL_ID}/{id}"))
}

fn require_ident(value: &str, what: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_IDENT {
        bail!("{what} must contain 1 to {MAX_IDENT} characters");
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        bail!("{what} must contain 1 to {MAX_IDENT} characters");
    };
    if !first.is_ascii_alphabetic() || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!(
            "{what} {value:?} is not a BioMart identifier (ASCII letters, digits and underscore)"
        );
    }
    Ok(value.to_string())
}

fn require_value(value: &str, what: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_VALUE {
        bail!("{what} must contain 1 to {MAX_VALUE} characters");
    }
    if value.chars().any(|c| c.is_control() || c == ',') {
        bail!("{what} must not contain commas or control characters; pass list items separately");
    }
    Ok(value.to_string())
}

fn require_attributes(attrs: &[String]) -> Result<Vec<String>> {
    if attrs.is_empty() {
        bail!("provide at least one attribute");
    }
    if attrs.len() > MAX_ATTRIBUTES {
        bail!(
            "{} attributes exceeds the per-call bound of {MAX_ATTRIBUTES}",
            attrs.len()
        );
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for attr in attrs {
        let name = require_ident(attr, "attribute")?;
        if !seen.insert(name.clone()) {
            bail!("duplicate attribute {name}");
        }
        out.push(name);
    }
    Ok(out)
}

fn require_filters(filters: &BTreeMap<String, Value>) -> Result<Vec<Filter>> {
    if filters.is_empty() {
        bail!(
            "get_data requires at least one filter; unfiltered BioMart queries can dump an entire annotation set"
        );
    }
    if filters.len() > MAX_FILTERS {
        bail!(
            "{} filters exceeds the per-call bound of {MAX_FILTERS}",
            filters.len()
        );
    }
    let mut out = Vec::new();
    for (name, value) in filters {
        let name = require_ident(name, "filter")?;
        out.push(encode_filter(name, value)?);
    }
    Ok(out)
}

fn encode_filter(name: String, value: &Value) -> Result<Filter> {
    match value {
        Value::Bool(true) => Ok(Filter::Include(name)),
        Value::Bool(false) => Ok(Filter::Exclude(name)),
        Value::Number(number) => Ok(Filter::Value(name, number.to_string())),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.eq_ignore_ascii_case("only") || trimmed.eq_ignore_ascii_case("included") {
                Ok(Filter::Include(name))
            } else if trimmed.eq_ignore_ascii_case("excluded") {
                Ok(Filter::Exclude(name))
            } else {
                Ok(Filter::Value(name, require_value(text, "filter value")?))
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                bail!("filter {name} list must contain at least one value");
            }
            if items.len() > MAX_TARGETS {
                bail!("filter {name} list exceeds the per-call bound of {MAX_TARGETS} identifiers");
            }
            let mut values = Vec::new();
            for item in items {
                let Some(text) = item.as_str() else {
                    bail!("filter {name} list items must be strings");
                };
                values.push(require_value(text, "filter value")?);
            }
            let joined_len =
                values.iter().map(String::len).sum::<usize>() + values.len().saturating_sub(1);
            if joined_len > MAX_FILTER_JOIN {
                bail!("filter {name} values exceeded the per-call size bound");
            }
            Ok(Filter::List(name, values))
        }
        Value::Null => bail!("filter {name} is missing a value"),
        Value::Object(_) => bail!("filter {name} must be a string, boolean, integer or list"),
    }
}

fn require_targets(targets: &[String]) -> Result<Vec<String>> {
    if targets.is_empty() {
        bail!("provide at least one target identifier");
    }
    if targets.len() > MAX_TARGETS {
        bail!(
            "{} targets exceeds the per-call bound of {MAX_TARGETS}",
            targets.len()
        );
    }
    let mut out = Vec::new();
    for target in targets {
        out.push(require_value(target, "target")?);
    }
    Ok(out)
}

fn bound_page(n: u32) -> Result<usize> {
    if !(1..=MAX_PAGE).contains(&n) {
        bail!("max_results must be between 1 and {MAX_PAGE}");
    }
    Ok(n as usize)
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}
