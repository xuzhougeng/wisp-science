//! Native `drug-regulatory` domain against openFDA Drugs@FDA and drug labels.
//! Independently implemented from:
//!
//! - [Drugs@FDA endpoint](https://open.fda.gov/apis/drug/drugsfda/)
//! - [Drugs@FDA fields](https://open.fda.gov/fields/drugsfda.yaml)
//! - [Drug labels endpoint](https://open.fda.gov/apis/drug/label/)
//! - [Query parameters](https://open.fda.gov/apis/query-parameters/)
//! - [Query syntax](https://open.fda.gov/apis/query-syntax/)
//! - [Paging](https://open.fda.gov/apis/paging/)
//! - [Authentication](https://open.fda.gov/apis/authentication/)
//!
//! References reviewed 2026-09-06. Calls are GET `https://api.fda.gov/drug/{drugsfda|label}.json`.
//! An `OPENFDA_API_KEY` raises the daily quota; anonymous access remains valid.
//! openFDA 404 `NOT_FOUND` means zero matches. Skip pagination cannot pass 25,000
//! (`limit` ≤ 1,000, reachable depth 26,000). Tests use invented records.

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use wisp_llm::ToolSchema;

const ORIGIN: &str = "https://api.fda.gov";
const DRUGSFDA_PATH: &str = "/drug/drugsfda.json";
const LABEL_PATH: &str = "/drug/label.json";
const DRUGSFDA_DOCS: &str = "https://open.fda.gov/apis/drug/drugsfda/";
const LABEL_DOCS: &str = "https://open.fda.gov/apis/drug/label/";
const DAF: &str =
    "https://www.accessdata.fda.gov/scripts/cder/daf/index.cfm?event=overview.process";
const DAILYMED: &str = "https://dailymed.nlm.nih.gov/dailymed/drugInfo.cfm";
const OPENFDA: Source = Source("openFDA", Duration::from_millis(250));
const SKIP_CAP: u32 = 25_000;
const API_LIMIT: u32 = 1_000;
const APP_PAGE: u32 = 100;
const LABEL_PAGE: u32 = 50;
const COUNT_CAP: u32 = 1_000;
const STATS_TOP: usize = 25;
const MAX_TERM: usize = 256;
const MAX_RAW: usize = 2_048;
const MAX_SECTIONS: usize = 20;
const MAX_INGREDIENT_SETS: usize = 5;
const SUBMISSION_CAP: usize = 50;
const EQUIV_PAGE: u32 = 100;

const COUNT_FIELDS: &[(&str, &str)] = &[
    ("sponsor_name", "sponsor_name"),
    ("application_number", "application_number"),
    ("dosage_form", "products.dosage_form.exact"),
    ("route", "products.route.exact"),
    ("marketing_status", "products.marketing_status"),
    ("te_code", "products.te_code"),
    ("pharm_class_epc", "openfda.pharm_class_epc.exact"),
    ("pharm_class_moa", "openfda.pharm_class_moa.exact"),
    ("pharm_class_cs", "openfda.pharm_class_cs.exact"),
    ("pharm_class_pe", "openfda.pharm_class_pe.exact"),
];

const APP_FIELDS: &[(&str, &str)] = &[
    ("brand", "products.brand_name"),
    ("generic", "openfda.generic_name"),
    ("active_ingredient", "products.active_ingredients.name"),
    ("sponsor", "sponsor_name"),
    ("marketing_status", "products.marketing_status"),
    ("dosage_form", "products.dosage_form"),
    ("route", "products.route"),
];

const LABEL_FIELDS: &[(&str, &str)] = &[
    ("active_ingredient", "openfda.substance_name"),
    ("generic_name", "openfda.generic_name"),
    ("brand_name", "openfda.brand_name"),
    ("route", "openfda.route"),
    ("product_type", "openfda.product_type"),
];

const WARNING_SECTIONS: &[&str] = &[
    "boxed_warning",
    "warnings",
    "warnings_and_cautions",
    "contraindications",
    "precautions",
    "general_precautions",
    "adverse_reactions",
    "drug_interactions",
    "do_not_use",
    "stop_use",
    "ask_doctor",
    "ask_doctor_or_pharmacist",
    "when_using",
];

const OPENFDA_LISTS: &[&str] = &[
    "generic_name",
    "brand_name",
    "manufacturer_name",
    "substance_name",
    "unii",
    "rxcui",
    "product_ndc",
    "pharm_class_epc",
    "pharm_class_moa",
    "pharm_class_cs",
    "pharm_class_pe",
];

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "drug-regulatory",
            ToolSchema::new(
                "count_drug_applications",
                "Count Drugs@FDA applications by one field (sponsor, dosage form, route, marketing status, TE code, or pharmacologic class). Optional name/status filters narrow the set before counting. Returns the most frequent terms as a bounded bucket page. openFDA count is at most 1,000 terms; a full-corpus count is not a complete vocabulary. Uses GET /drug/drugsfda.json?count=.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["count_field"],
                    "properties": {
                        "count_field": {
                            "type": "string",
                            "enum": ["sponsor_name", "application_number", "dosage_form", "route", "marketing_status", "te_code", "pharm_class_epc", "pharm_class_moa", "pharm_class_cs", "pharm_class_pe"]
                        },
                        "brand": {"type": "string", "minLength": 1, "maxLength": 256},
                        "generic": {"type": "string", "minLength": 1, "maxLength": 256},
                        "active_ingredient": {"type": "string", "minLength": 1, "maxLength": 256},
                        "sponsor": {"type": "string", "minLength": 1, "maxLength": 256},
                        "marketing_status": {"type": "string", "minLength": 1, "maxLength": 256, "description": "Prescription, Over-the-counter, Discontinued, or None (Tentative Approval)."},
                        "dosage_form": {"type": "string", "minLength": 1, "maxLength": 256},
                        "route": {"type": "string", "minLength": 1, "maxLength": 256},
                        "pharm_class": {"type": "string", "minLength": 1, "maxLength": 256},
                        "pharm_class_type": {"type": "string", "enum": ["epc", "moa", "cs", "pe"], "default": "epc"},
                        "search_type": {"type": "string", "enum": ["and", "or"], "default": "and"},
                        "max_buckets": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                    }
                }),
            ),
        ),
        (
            "drug-regulatory",
            ToolSchema::new(
                "get_drug_application",
                "Fetch one Drugs@FDA application by NDA, ANDA, or BLA number (prefix plus six digits). Returns sponsor, products (ingredients, dosage form, route, marketing status, TE code), a bounded submission history, and the harmonized openfda block when present. Missing numbers are reported as found=false rather than as an error. 404 NOT_FOUND is an empty match.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["application_number"],
                    "properties": {
                        "application_number": {
                            "type": "string", "minLength": 9, "maxLength": 16,
                            "pattern": "^([Nn][Dd][Aa]|[Aa][Nn][Dd][Aa]|[Bb][Ll][Aa])[0-9]{6}$"
                        }
                    }
                }),
            ),
        ),
        (
            "drug-regulatory",
            ToolSchema::new(
                "get_drug_statistics",
                "Corpus snapshot of Drugs@FDA: application total, last_updated, marketing-status counts, and the top dosage forms, routes, and sponsors. Distinct form/route figures are capped by openFDA's 1,000-term count window. Not a complete dump of the dataset.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {}
                }),
            ),
        ),
        (
            "drug-regulatory",
            ToolSchema::new(
                "get_generic_equivalents",
                "Find Drugs@FDA applications whose product active-ingredient set matches a brand-name reference. Resolves the brand, collects ingredient sets, and searches those ingredients. Therapeutic-equivalence (TE) codes and marketing status are on each product. The response is a bounded page; a large ingredient class is not fully retrieved. Brand matches with no ingredients are reported rather than guessed.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["brand"],
                    "properties": {
                        "brand": {"type": "string", "minLength": 1, "maxLength": 256},
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 100, "default": 50}
                    }
                }),
            ),
        ),
        (
            "drug-regulatory",
            ToolSchema::new(
                "list_pharmacologic_classes",
                "List pharmacologic classes attached to Drugs@FDA records, with application counts. class_type is epc (Established Pharmacologic Class), moa (mechanism of action), cs (chemical structure), or pe (physiologic effect). Counts only cover applications that carry the harmonized openfda block. At most 1,000 classes.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "class_type": {"type": "string", "enum": ["epc", "moa", "cs", "pe"], "default": "epc"},
                        "max_buckets": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100}
                    }
                }),
            ),
        ),
        (
            "drug-regulatory",
            ToolSchema::new(
                "search_drug_applications",
                "Search FDA Drugs@FDA applications (NDA/ANDA/BLA) by brand, generic name, active ingredient, sponsor, marketing status, dosage form, route, pharmacologic class, or submission-status date range. Phrase queries use openFDA field:\"term\" syntax. The response is one bounded page (skip/limit; skip cannot exceed 25,000) with the upstream total. A truncated page is not the complete hit list. Harmonized openfda.* fields are missing on many older applications.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "brand": {"type": "string", "minLength": 1, "maxLength": 256},
                        "generic": {"type": "string", "minLength": 1, "maxLength": 256},
                        "active_ingredient": {"type": "string", "minLength": 1, "maxLength": 256},
                        "sponsor": {"type": "string", "minLength": 1, "maxLength": 256},
                        "marketing_status": {"type": "string", "minLength": 1, "maxLength": 256},
                        "dosage_form": {"type": "string", "minLength": 1, "maxLength": 256},
                        "route": {"type": "string", "minLength": 1, "maxLength": 256},
                        "pharm_class": {"type": "string", "minLength": 1, "maxLength": 256},
                        "pharm_class_type": {"type": "string", "enum": ["epc", "moa", "cs", "pe"], "default": "epc"},
                        "search_type": {"type": "string", "enum": ["and", "or"], "default": "and"},
                        "submission_date_from": {"type": "string", "description": "YYYY-MM-DD or YYYYMMDD; supply both dates."},
                        "submission_date_to": {"type": "string", "description": "YYYY-MM-DD or YYYYMMDD; supply both dates."},
                        "raw_search": {"type": "string", "minLength": 1, "maxLength": 2048, "description": "Verbatim openFDA search= string. Mutually exclusive with mapped filters."},
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25},
                        "skip": {"type": "integer", "minimum": 0, "maximum": 25000, "default": 0}
                    }
                }),
            ),
        ),
        (
            "drug-regulatory",
            ToolSchema::new(
                "search_drug_labels",
                "Search FDA Structured Product Labels via openFDA /drug/label.json by substance, generic or brand name, route, or product type (HUMAN PRESCRIPTION DRUG / HUMAN OTC DRUG). Default records carry identifiers, boxed-warning presence, which warning sections exist, and indications_and_usage. Pass sections to extract those label fields instead. Analyzed fields match tokens; exact=true queries the .exact variant. The response is a bounded page with the upstream total.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "active_ingredient": {"type": "string", "minLength": 1, "maxLength": 256},
                        "generic_name": {"type": "string", "minLength": 1, "maxLength": 256},
                        "brand_name": {"type": "string", "minLength": 1, "maxLength": 256},
                        "route": {"type": "string", "minLength": 1, "maxLength": 256},
                        "product_type": {"type": "string", "minLength": 1, "maxLength": 256},
                        "exact": {"type": "boolean", "default": false},
                        "raw_search": {"type": "string", "minLength": 1, "maxLength": 2048},
                        "sections": {
                            "type": "array", "minItems": 1, "maxItems": 20,
                            "items": {"type": "string", "minLength": 1, "maxLength": 64, "pattern": "^[a-z][a-z0-9_]*$"}
                        },
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20},
                        "skip": {"type": "integer", "minimum": 0, "maximum": 25000, "default": 0}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("openFDA request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "count_drug_applications" => count_applications(bio, args).await,
        "get_drug_application" => get_application(bio, args).await,
        "get_drug_statistics" => statistics(bio, args).await,
        "get_generic_equivalents" => generic_equivalents(bio, args).await,
        "list_pharmacologic_classes" => pharmacologic_classes(bio, args).await,
        "search_drug_applications" => search_applications(bio, args).await,
        "search_drug_labels" => search_labels(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchApplications {
    brand: Option<String>,
    generic: Option<String>,
    active_ingredient: Option<String>,
    sponsor: Option<String>,
    marketing_status: Option<String>,
    dosage_form: Option<String>,
    route: Option<String>,
    pharm_class: Option<String>,
    #[serde(default = "default_class_type")]
    pharm_class_type: String,
    #[serde(default = "default_search_type")]
    search_type: String,
    submission_date_from: Option<String>,
    submission_date_to: Option<String>,
    raw_search: Option<String>,
    #[serde(default = "default_app_page")]
    max_records: u32,
    #[serde(default)]
    skip: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CountApplications {
    count_field: String,
    brand: Option<String>,
    generic: Option<String>,
    active_ingredient: Option<String>,
    sponsor: Option<String>,
    marketing_status: Option<String>,
    dosage_form: Option<String>,
    route: Option<String>,
    pharm_class: Option<String>,
    #[serde(default = "default_class_type")]
    pharm_class_type: String,
    #[serde(default = "default_search_type")]
    search_type: String,
    #[serde(default = "default_count_page")]
    max_buckets: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetApplication {
    application_number: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenericEquivalents {
    brand: String,
    #[serde(default = "default_equiv_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PharmClasses {
    #[serde(default = "default_class_type")]
    class_type: String,
    #[serde(default = "default_count_page")]
    max_buckets: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchLabels {
    active_ingredient: Option<String>,
    generic_name: Option<String>,
    brand_name: Option<String>,
    route: Option<String>,
    product_type: Option<String>,
    #[serde(default)]
    exact: bool,
    raw_search: Option<String>,
    sections: Option<Vec<String>>,
    #[serde(default = "default_label_page")]
    max_records: u32,
    #[serde(default)]
    skip: u32,
}

fn default_class_type() -> String {
    "epc".into()
}
fn default_search_type() -> String {
    "and".into()
}
fn default_app_page() -> u32 {
    25
}
fn default_label_page() -> u32 {
    20
}
fn default_count_page() -> u32 {
    100
}
fn default_equiv_page() -> u32 {
    50
}

async fn search_applications(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchApplications = serde_json::from_value(args.clone())
        .context("invalid drug application search arguments")?;
    let search = application_search(&args)?;
    let limit = bound(args.max_records, 1, APP_PAGE, "max_records")?;
    let skip = bound_skip(args.skip)?;
    let page = fda_search(
        bio,
        DRUGSFDA_PATH,
        &search,
        limit,
        skip,
        Some("application_number:asc"),
    )
    .await?;
    let records: Vec<Value> = page
        .results
        .iter()
        .filter_map(|row| project_application(row, false))
        .collect();
    Ok(search_page(
        "openFDA Drugs@FDA",
        DRUGSFDA_DOCS,
        DRUGSFDA_PATH,
        &search,
        &page,
        records,
        skip,
        limit,
    ))
}

async fn get_application(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetApplication = serde_json::from_value(args.clone())
        .context("invalid drug application lookup arguments")?;
    let number = normalize_application_number(&args.application_number)?;
    let search = phrase("application_number", &number);
    let page = fda_search(
        bio,
        DRUGSFDA_PATH,
        &search,
        2,
        0,
        Some("application_number:asc"),
    )
    .await?;
    if page.results.len() > 1 {
        bail!("openFDA returned more than one record for {number}");
    }
    let record = page
        .results
        .first()
        .and_then(|row| project_application(row, true));
    let found = record.is_some();
    Ok(json!({
        "source": "openFDA Drugs@FDA",
        "source_url": DRUGSFDA_DOCS,
        "query_url": public_url(DRUGSFDA_PATH, &[("search".into(), search.clone()), ("limit".into(), "2".into())]),
        "application_number": number,
        "found": found,
        "last_updated": page.last_updated,
        "record": record,
        "url": application_api_url(&number),
        "fda_url": daf_url(&number),
    }))
}

async fn count_applications(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: CountApplications =
        serde_json::from_value(args.clone()).context("invalid drug application count arguments")?;
    let api_field = resolve_count_field(&args.count_field)?;
    let limit = bound(args.max_buckets, 1, COUNT_CAP, "max_buckets")?;
    let search = count_search(&args)?;
    let page = fda_count(bio, DRUGSFDA_PATH, search.as_deref(), api_field, limit).await?;
    let returned = page.buckets.len();
    Ok(json!({
        "source": "openFDA Drugs@FDA",
        "source_url": DRUGSFDA_DOCS,
        "query_url": page.query_url,
        "count_field": args.count_field,
        "api_field": api_field,
        "search": search,
        "returned": returned,
        "truncated": returned as u32 >= limit,
        "last_updated": page.last_updated,
        "buckets": page.buckets,
    }))
}

async fn statistics(bio: &NativeBio, args: &Value) -> Result<Value> {
    let _: EmptyArgs =
        serde_json::from_value(args.clone()).context("invalid drug statistics arguments")?;
    let total_page = fda_search(bio, DRUGSFDA_PATH, "", 1, 0, None).await?;
    let status = fda_count(
        bio,
        DRUGSFDA_PATH,
        None,
        "products.marketing_status",
        COUNT_CAP,
    )
    .await?;
    let forms = fda_count(
        bio,
        DRUGSFDA_PATH,
        None,
        "products.dosage_form.exact",
        COUNT_CAP,
    )
    .await?;
    let routes = fda_count(bio, DRUGSFDA_PATH, None, "products.route.exact", COUNT_CAP).await?;
    let sponsors = fda_count(bio, DRUGSFDA_PATH, None, "sponsor_name", STATS_TOP as u32).await?;
    Ok(json!({
        "source": "openFDA Drugs@FDA",
        "source_url": DRUGSFDA_DOCS,
        "query_url": public_url(DRUGSFDA_PATH, &[("limit".into(), "1".into())]),
        "total_applications": total_page.total,
        "last_updated": total_page.last_updated,
        "retrieval_ceiling": SKIP_CAP + API_LIMIT,
        "marketing_status": status.buckets,
        "dosage_form_top": take(&forms.buckets, STATS_TOP),
        "dosage_form_distinct": forms.buckets.len(),
        "route_top": take(&routes.buckets, STATS_TOP),
        "route_distinct": routes.buckets.len(),
        "sponsor_top": sponsors.buckets,
        "count_cap": COUNT_CAP,
    }))
}

async fn pharmacologic_classes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: PharmClasses =
        serde_json::from_value(args.clone()).context("invalid pharmacologic class arguments")?;
    let class_type = require_class_type(&args.class_type)?;
    let limit = bound(args.max_buckets, 1, COUNT_CAP, "max_buckets")?;
    let api_field = format!("openfda.pharm_class_{class_type}.exact");
    let page = fda_count(bio, DRUGSFDA_PATH, None, &api_field, limit).await?;
    let returned = page.buckets.len();
    Ok(json!({
        "source": "openFDA Drugs@FDA",
        "source_url": DRUGSFDA_DOCS,
        "query_url": page.query_url,
        "class_type": class_type,
        "api_field": api_field,
        "returned": returned,
        "truncated": returned as u32 >= limit,
        "last_updated": page.last_updated,
        "classes": page.buckets,
    }))
}

async fn generic_equivalents(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GenericEquivalents =
        serde_json::from_value(args.clone()).context("invalid generic equivalent arguments")?;
    let brand = require_term(Some(&args.brand), "brand")?;
    let cap = bound(args.max_records, 1, EQUIV_PAGE, "max_records")?;
    let brand_search = phrase("products.brand_name", brand);
    let refs = fda_search(
        bio,
        DRUGSFDA_PATH,
        &brand_search,
        20,
        0,
        Some("application_number:asc"),
    )
    .await?;
    let reference: Vec<Value> = refs
        .results
        .iter()
        .filter_map(|row| project_application(row, false))
        .collect();
    let mut ingredient_sets: BTreeSet<BTreeSet<String>> = BTreeSet::new();
    for record in &reference {
        if let Some(products) = record.get("products").and_then(Value::as_array) {
            for product in products {
                let names = ingredient_names(product);
                if !names.is_empty() {
                    ingredient_sets.insert(names);
                }
            }
        }
    }
    let sets_truncated = ingredient_sets.len() > MAX_INGREDIENT_SETS;
    let chosen: Vec<BTreeSet<String>> = ingredient_sets
        .iter()
        .take(MAX_INGREDIENT_SETS)
        .cloned()
        .collect();
    let mut by_number: BTreeMap<String, Value> = BTreeMap::new();
    let mut candidates_truncated = refs.total > reference.len() as u64;
    for names in &chosen {
        if names.is_empty() {
            continue;
        }
        let mut clauses: Vec<String> = names
            .iter()
            .map(|name| phrase("products.active_ingredients.name", name))
            .collect();
        clauses.sort();
        let search = clauses.join(" AND ");
        let page = fda_search(
            bio,
            DRUGSFDA_PATH,
            &search,
            EQUIV_PAGE,
            0,
            Some("application_number:asc"),
        )
        .await?;
        if page.total > page.results.len() as u64 {
            candidates_truncated = true;
        }
        for row in &page.results {
            let Some(record) = project_application(row, false) else {
                continue;
            };
            if product_matches_set(&record, names) {
                if let Some(number) = record.get("application_number").and_then(Value::as_str) {
                    by_number.entry(number.to_string()).or_insert(record);
                }
            }
        }
    }
    let mut equivalents: Vec<Value> = by_number.into_values().collect();
    let total_equivalents = equivalents.len();
    equivalents.truncate(cap as usize);
    let set_list: Vec<Value> = chosen
        .iter()
        .map(|set| json!(set.iter().cloned().collect::<Vec<_>>()))
        .collect();
    let ref_numbers: Vec<String> = reference
        .iter()
        .filter_map(|row| row.get("application_number").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    Ok(json!({
        "source": "openFDA Drugs@FDA",
        "source_url": DRUGSFDA_DOCS,
        "query_url": public_url(DRUGSFDA_PATH, &[("search".into(), brand_search), ("limit".into(), "20".into())]),
        "brand": brand,
        "reference_applications": ref_numbers,
        "active_ingredient_sets": set_list,
        "ingredient_sets_truncated": sets_truncated,
        "total_equivalents": total_equivalents,
        "returned": equivalents.len(),
        "truncated": candidates_truncated || total_equivalents > equivalents.len(),
        "last_updated": refs.last_updated,
        "equivalents": equivalents,
    }))
}

async fn search_labels(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchLabels =
        serde_json::from_value(args.clone()).context("invalid drug label search arguments")?;
    let search = label_search(&args)?;
    let sections = normalize_sections(args.sections.as_deref())?;
    let limit = bound(args.max_records, 1, LABEL_PAGE, "max_records")?;
    let skip = bound_skip(args.skip)?;
    let page = fda_search(bio, LABEL_PATH, &search, limit, skip, Some("set_id:asc")).await?;
    let records: Vec<Value> = page
        .results
        .iter()
        .map(|row| project_label(row, sections.as_deref()))
        .collect();
    Ok(search_page(
        "openFDA drug label",
        LABEL_DOCS,
        LABEL_PATH,
        &search,
        &page,
        records,
        skip,
        limit,
    ))
}

fn application_search(args: &SearchApplications) -> Result<String> {
    if let Some(raw) = trim_opt(&args.raw_search) {
        if has_mapped_app_filters(args)
            || args.submission_date_from.is_some()
            || args.submission_date_to.is_some()
        {
            bail!("raw_search cannot be combined with mapped filters or submission dates");
        }
        return require_raw(raw);
    }
    let clauses = mapped_app_clauses(
        &args.brand,
        &args.generic,
        &args.active_ingredient,
        &args.sponsor,
        &args.marketing_status,
        &args.dosage_form,
        &args.route,
        &args.pharm_class,
        &args.pharm_class_type,
    )?;
    let dates = date_clause(&args.submission_date_from, &args.submission_date_to)?;
    join_clauses(&clauses, &args.search_type, dates)?
        .ok_or_else(|| anyhow!("provide at least one Drugs@FDA search field"))
}

fn count_search(args: &CountApplications) -> Result<Option<String>> {
    let clauses = mapped_app_clauses(
        &args.brand,
        &args.generic,
        &args.active_ingredient,
        &args.sponsor,
        &args.marketing_status,
        &args.dosage_form,
        &args.route,
        &args.pharm_class,
        &args.pharm_class_type,
    )?;
    join_clauses(&clauses, &args.search_type, None)
}

fn label_search(args: &SearchLabels) -> Result<String> {
    if let Some(raw) = trim_opt(&args.raw_search) {
        if has_mapped_label_filters(args) {
            bail!("raw_search cannot be combined with mapped label filters");
        }
        return require_raw(raw);
    }
    let suffix = if args.exact { ".exact" } else { "" };
    let mut clauses = Vec::new();
    for (key, field) in LABEL_FIELDS {
        let value = match *key {
            "active_ingredient" => trim_opt(&args.active_ingredient),
            "generic_name" => trim_opt(&args.generic_name),
            "brand_name" => trim_opt(&args.brand_name),
            "route" => trim_opt(&args.route),
            "product_type" => trim_opt(&args.product_type),
            _ => None,
        };
        if let Some(term) = value {
            clauses.push(phrase(
                &format!("{field}{suffix}"),
                require_term(Some(term), key)?,
            ));
        }
    }
    if clauses.is_empty() {
        bail!("provide at least one drug label search field");
    }
    Ok(clauses.join(" AND "))
}

fn mapped_app_clauses(
    brand: &Option<String>,
    generic: &Option<String>,
    active_ingredient: &Option<String>,
    sponsor: &Option<String>,
    marketing_status: &Option<String>,
    dosage_form: &Option<String>,
    route: &Option<String>,
    pharm_class: &Option<String>,
    pharm_class_type: &str,
) -> Result<Vec<String>> {
    let mut clauses = Vec::new();
    let values = [
        ("brand", brand),
        ("generic", generic),
        ("active_ingredient", active_ingredient),
        ("sponsor", sponsor),
        ("marketing_status", marketing_status),
        ("dosage_form", dosage_form),
        ("route", route),
    ];
    for (key, field) in APP_FIELDS {
        if let Some((_, value)) = values.iter().find(|(name, _)| name == key) {
            if let Some(term) = trim_opt(value) {
                clauses.push(phrase(field, require_term(Some(term), key)?));
            }
        }
    }
    if let Some(term) = trim_opt(pharm_class) {
        let class_type = require_class_type(pharm_class_type)?;
        clauses.push(phrase(
            &format!("openfda.pharm_class_{class_type}"),
            require_term(Some(term), "pharm_class")?,
        ));
    } else {
        require_class_type(pharm_class_type)?;
    }
    Ok(clauses)
}

fn has_mapped_app_filters(args: &SearchApplications) -> bool {
    [
        &args.brand,
        &args.generic,
        &args.active_ingredient,
        &args.sponsor,
        &args.marketing_status,
        &args.dosage_form,
        &args.route,
        &args.pharm_class,
    ]
    .iter()
    .any(|value| trim_opt(value).is_some())
}

fn has_mapped_label_filters(args: &SearchLabels) -> bool {
    [
        &args.active_ingredient,
        &args.generic_name,
        &args.brand_name,
        &args.route,
        &args.product_type,
    ]
    .iter()
    .any(|value| trim_opt(value).is_some())
}

fn join_clauses(
    clauses: &[String],
    search_type: &str,
    date: Option<String>,
) -> Result<Option<String>> {
    require_search_type(search_type)?;
    if clauses.is_empty() {
        return Ok(date);
    }
    let joiner = if search_type == "or" { " OR " } else { " AND " };
    let mut body = clauses.join(joiner);
    if search_type == "or" && clauses.len() > 1 {
        body = format!("({body})");
    }
    Ok(Some(match date {
        Some(range) => format!("{body} AND {range}"),
        None => body,
    }))
}

fn date_clause(from: &Option<String>, to: &Option<String>) -> Result<Option<String>> {
    match (trim_opt(from), trim_opt(to)) {
        (None, None) => Ok(None),
        (Some(start), Some(end)) => {
            let lo = yyyymmdd(start)?;
            let hi = yyyymmdd(end)?;
            Ok(Some(format!(
                "submissions.submission_status_date:[{lo} TO {hi}]"
            )))
        }
        _ => bail!("provide both submission_date_from and submission_date_to"),
    }
}

fn yyyymmdd(value: &str) -> Result<String> {
    let compact = value.replace('-', "");
    if compact.len() != 8 || !compact.bytes().all(|b| b.is_ascii_digit()) {
        bail!("dates must be YYYY-MM-DD or YYYYMMDD");
    }
    let month: u32 = compact[4..6].parse().unwrap_or(0);
    let day: u32 = compact[6..8].parse().unwrap_or(0);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        bail!("dates must be a real calendar day");
    }
    Ok(compact)
}

fn phrase(field: &str, value: &str) -> String {
    let cleaned = value.replace(['"', '\\'], "");
    format!("{field}:\"{cleaned}\"")
}

struct FdaPage {
    total: u64,
    last_updated: Option<String>,
    results: Vec<Value>,
    query_url: String,
}

struct CountPage {
    last_updated: Option<String>,
    buckets: Vec<Value>,
    query_url: String,
}

impl FdaPage {
    fn empty(query_url: String) -> Self {
        Self {
            total: 0,
            last_updated: None,
            results: Vec::new(),
            query_url,
        }
    }
}

async fn fda_search(
    bio: &NativeBio,
    path: &str,
    search: &str,
    limit: u32,
    skip: u32,
    sort: Option<&str>,
) -> Result<FdaPage> {
    let mut params = Vec::new();
    if !search.is_empty() {
        params.push(("search".into(), search.to_string()));
    }
    if let Some(sort) = sort {
        params.push(("sort".into(), sort.to_string()));
    }
    params.push(("limit".into(), limit.to_string()));
    params.push(("skip".into(), skip.to_string()));
    match fda_get(bio, path, &params).await? {
        None => Ok(FdaPage::empty(public_url(path, &params))),
        Some(body) => {
            let total = meta_total(&body)?;
            let last_updated = meta_updated(&body);
            let results = body
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if total > 0 && results.is_empty() && skip == 0 {
                bail!("openFDA omitted result records");
            }
            Ok(FdaPage {
                total,
                last_updated,
                results,
                query_url: public_url(path, &params),
            })
        }
    }
}

async fn fda_count(
    bio: &NativeBio,
    path: &str,
    search: Option<&str>,
    field: &str,
    limit: u32,
) -> Result<CountPage> {
    let mut params = Vec::new();
    if let Some(search) = search.filter(|value| !value.is_empty()) {
        params.push(("search".into(), search.to_string()));
    }
    params.push(("count".into(), field.to_string()));
    params.push(("limit".into(), limit.to_string()));
    let query_url = public_url(path, &params);
    match fda_get(bio, path, &params).await? {
        None => Ok(CountPage {
            last_updated: None,
            buckets: Vec::new(),
            query_url,
        }),
        Some(body) => Ok(CountPage {
            last_updated: meta_updated(&body),
            buckets: count_buckets(&body)?,
            query_url,
        }),
    }
}

async fn fda_get(
    bio: &NativeBio,
    path: &str,
    params: &[(String, String)],
) -> Result<Option<Value>> {
    let mut query = Vec::new();
    if let Some(key) = bio.credential("OPENFDA_API_KEY") {
        query.push(("api_key".into(), key.to_string()));
    }
    query.extend(params.iter().cloned());
    let url = format!("{}{path}", origin(bio));
    let response = bio.http().send(OPENFDA, Method::GET, &url, &query).await?;
    if response.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    response.check()?;
    if looks_like_html(&response.body) {
        bail!("openFDA returned HTML instead of JSON");
    }
    let body: Value =
        serde_json::from_slice(&response.body).context("openFDA returned invalid JSON")?;
    if let Some(code) = error_code(&body) {
        if code == "NOT_FOUND" {
            return Ok(None);
        }
        bail!("openFDA rejected the request ({code})");
    }
    Ok(Some(body))
}

fn origin(bio: &NativeBio) -> String {
    bio.credential("OPENFDA_BASE_URL")
        .map(|value| value.trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ORIGIN.to_string())
}

fn public_url(path: &str, params: &[(String, String)]) -> String {
    let pairs: Vec<(&str, &str)> = params
        .iter()
        .filter(|(key, _)| key != "api_key")
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    reqwest::Url::parse_with_params(&format!("{ORIGIN}{path}"), pairs)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| format!("{ORIGIN}{path}"))
}

fn meta_total(body: &Value) -> Result<u64> {
    body.get("meta")
        .and_then(|meta| meta.get("results"))
        .and_then(|results| results.get("total"))
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("openFDA omitted the result total"))
}

fn meta_updated(body: &Value) -> Option<String> {
    body.get("meta")
        .and_then(|meta| meta.get("last_updated"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn error_code(body: &Value) -> Option<&str> {
    body.get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
}

fn count_buckets(body: &Value) -> Result<Vec<Value>> {
    let Some(rows) = body.get("results") else {
        return Ok(Vec::new());
    };
    let Some(rows) = rows.as_array() else {
        bail!("openFDA count results were not a list");
    };
    let mut buckets = Vec::new();
    for row in rows {
        let term = row
            .get("term")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let count = row.get("count").and_then(Value::as_u64).unwrap_or(0);
        buckets.push(json!({"term": term, "count": count}));
    }
    Ok(buckets)
}

fn search_page(
    source: &str,
    docs: &str,
    path: &str,
    search: &str,
    page: &FdaPage,
    records: Vec<Value>,
    skip: u32,
    limit: u32,
) -> Value {
    let returned = records.len() as u64;
    let next_skip = skip as u64 + returned;
    let has_more = next_skip < page.total;
    json!({
        "source": source,
        "source_url": docs,
        "query_url": page.query_url,
        "search": search,
        "total": page.total,
        "returned": returned,
        "skip": skip,
        "limit": limit,
        "next_skip": if has_more && next_skip <= SKIP_CAP as u64 { json!(next_skip) } else { Value::Null },
        "has_more": has_more,
        "truncated": has_more,
        "beyond_skip_cap": page.total > (SKIP_CAP + API_LIMIT) as u64,
        "retrieval_ceiling": SKIP_CAP + API_LIMIT,
        "last_updated": page.last_updated,
        "records": records,
        "endpoint": format!("{ORIGIN}{path}"),
    })
}

fn project_application(raw: &Value, include_submissions: bool) -> Option<Value> {
    let number = scalar(raw.get("application_number"))?;
    if number.is_empty() {
        return None;
    }
    let mut products: Vec<Value> = raw
        .get("products")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(project_product)
        .collect();
    products.sort_by(|a, b| scalar(a.get("product_number")).cmp(&scalar(b.get("product_number"))));
    let mut record = json!({
        "application_number": number,
        "sponsor_name": scalar(raw.get("sponsor_name")),
        "products": products,
        "url": application_api_url(&number),
        "fda_url": daf_url(&number),
    });
    if let Some(openfda) = raw.get("openfda") {
        record["openfda"] = project_openfda(openfda);
    }
    if include_submissions {
        let mut submissions: Vec<Value> = raw
            .get("submissions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(project_submission)
            .collect();
        submissions.sort_by(|a, b| {
            (
                scalar(a.get("submission_status_date")),
                scalar(a.get("submission_type")),
                scalar(a.get("submission_number")),
            )
                .cmp(&(
                    scalar(b.get("submission_status_date")),
                    scalar(b.get("submission_type")),
                    scalar(b.get("submission_number")),
                ))
        });
        let total = submissions.len();
        if submissions.len() > SUBMISSION_CAP {
            let start = submissions.len() - SUBMISSION_CAP;
            submissions = submissions.split_off(start);
        }
        record["submissions_total"] = json!(total);
        record["submissions"] = json!(submissions);
    }
    Some(record)
}

fn project_product(raw: &Value) -> Value {
    let mut ingredients: Vec<Value> = raw
        .get("active_ingredients")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            json!({
                "name": scalar(item.get("name")),
                "strength": scalar(item.get("strength")),
            })
        })
        .collect();
    ingredients.sort_by(|a, b| {
        (
            scalar(a.get("name")).unwrap_or_default(),
            scalar(a.get("strength")).unwrap_or_default(),
        )
            .cmp(&(
                scalar(b.get("name")).unwrap_or_default(),
                scalar(b.get("strength")).unwrap_or_default(),
            ))
    });
    json!({
        "product_number": scalar(raw.get("product_number")),
        "brand_name": scalar(raw.get("brand_name")),
        "dosage_form": scalar(raw.get("dosage_form")),
        "route": scalar(raw.get("route")),
        "marketing_status": scalar(raw.get("marketing_status")),
        "te_code": scalar(raw.get("te_code")),
        "reference_drug": scalar(raw.get("reference_drug")),
        "reference_standard": scalar(raw.get("reference_standard")),
        "active_ingredients": ingredients,
    })
}

fn project_submission(raw: &Value) -> Value {
    json!({
        "submission_type": scalar(raw.get("submission_type")),
        "submission_number": scalar(raw.get("submission_number")),
        "submission_status": scalar(raw.get("submission_status")),
        "submission_status_date": scalar(raw.get("submission_status_date")),
        "submission_class_code": scalar(raw.get("submission_class_code")),
        "submission_class_code_description": scalar(raw.get("submission_class_code_description")),
        "review_priority": scalar(raw.get("review_priority")),
    })
}

fn project_openfda(raw: &Value) -> Value {
    let mut out = BTreeMap::new();
    for key in OPENFDA_LISTS {
        if let Some(values) = list_values(raw.get(key)) {
            out.insert(*key, json!(values));
        }
    }
    json!(out)
}

fn project_label(raw: &Value, sections: Option<&[String]>) -> Value {
    let set_id = scalar(raw.get("set_id"));
    let openfda = raw.get("openfda").unwrap_or(&Value::Null);
    let mut record = json!({
        "set_id": set_id,
        "spl_id": scalar(raw.get("id")),
        "spl_version": scalar(raw.get("version")),
        "effective_time": scalar(raw.get("effective_time")),
        "brand_name": list_values(openfda.get("brand_name")).unwrap_or_default(),
        "generic_name": list_values(openfda.get("generic_name")).unwrap_or_default(),
        "url": set_id.as_deref().map(label_api_url),
        "dailymed_url": set_id.as_deref().map(dailymed_url),
    });
    if let Some(sections) = sections {
        for key in sections {
            record[key] = json!(section_text(raw.get(key)));
        }
        return record;
    }
    record["substance_name"] =
        json!(list_values(openfda.get("substance_name")).unwrap_or_default());
    record["manufacturer_name"] =
        json!(list_values(openfda.get("manufacturer_name")).unwrap_or_default());
    record["route"] = json!(list_values(openfda.get("route")).unwrap_or_default());
    record["product_type"] = json!(list_values(openfda.get("product_type")).unwrap_or_default());
    record["application_number"] =
        json!(list_values(openfda.get("application_number")).unwrap_or_default());
    record["has_boxed_warning"] = json!(raw.get("boxed_warning").is_some());
    record["warning_sections_present"] = json!(WARNING_SECTIONS
        .iter()
        .filter(|key| raw.get(**key).is_some())
        .cloned()
        .collect::<Vec<_>>());
    record["indications_and_usage"] = json!(section_text(raw.get("indications_and_usage")));
    record
}

fn section_text(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => {
            let text = items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(other) => Some(other.to_string()),
    }
}

fn ingredient_names(product: &Value) -> BTreeSet<String> {
    product
        .get("active_ingredients")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| scalar(item.get("name")))
        .map(|name| name.to_ascii_uppercase())
        .collect()
}

fn product_matches_set(record: &Value, names: &BTreeSet<String>) -> bool {
    record
        .get("products")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|product| &ingredient_names(product) == names)
}

fn scalar(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => {
            let cleaned = collapse_ws(text);
            if cleaned.is_empty() {
                None
            } else {
                Some(iso_date(&cleaned))
            }
        }
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn list_values(value: Option<&Value>) -> Option<Vec<String>> {
    let items = value?.as_array()?;
    let mut values: Vec<String> = items.iter().filter_map(|item| scalar(Some(item))).collect();
    values.sort();
    values.dedup();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn iso_date(value: &str) -> String {
    if value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit()) {
        format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..])
    } else {
        value.to_string()
    }
}

fn application_api_url(number: &str) -> String {
    format!("{ORIGIN}{DRUGSFDA_PATH}?search=application_number:\"{number}\"")
}

fn daf_url(number: &str) -> String {
    let digits: String = number.chars().filter(|c| c.is_ascii_digit()).collect();
    format!("{DAF}&ApplNo={digits}")
}

fn label_api_url(set_id: &str) -> String {
    format!("{ORIGIN}{LABEL_PATH}?search=set_id:\"{set_id}\"")
}

fn dailymed_url(set_id: &str) -> String {
    format!("{DAILYMED}?setid={set_id}")
}

fn take(buckets: &[Value], n: usize) -> Vec<Value> {
    buckets.iter().take(n).cloned().collect()
}

fn bound(value: u32, min: u32, max: u32, name: &str) -> Result<u32> {
    if !(min..=max).contains(&value) {
        bail!("{name} must be between {min} and {max}");
    }
    Ok(value)
}

fn bound_skip(skip: u32) -> Result<u32> {
    if skip > SKIP_CAP {
        bail!("skip cannot exceed {SKIP_CAP}; narrow the query or download the openFDA dataset");
    }
    Ok(skip)
}

fn require_term<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str> {
    let term = value.map(str::trim).filter(|item| !item.is_empty());
    let Some(term) = term else {
        bail!("{name} must be a non-empty string");
    };
    if term.len() > MAX_TERM {
        bail!("{name} exceeds {MAX_TERM} characters");
    }
    Ok(term)
}

fn require_raw(value: &str) -> Result<String> {
    let raw = value.trim();
    if raw.is_empty() || raw.len() > MAX_RAW {
        bail!("raw_search must contain 1 to {MAX_RAW} characters");
    }
    Ok(raw.to_string())
}

fn require_class_type(value: &str) -> Result<&str> {
    match value.trim() {
        "epc" | "moa" | "cs" | "pe" => Ok(value.trim()),
        _ => bail!("pharm_class_type must be epc, moa, cs or pe"),
    }
}

fn require_search_type(value: &str) -> Result<()> {
    if matches!(value, "and" | "or") {
        Ok(())
    } else {
        bail!("search_type must be and or or")
    }
}

fn resolve_count_field(name: &str) -> Result<&str> {
    let field = name.trim();
    COUNT_FIELDS
        .iter()
        .find(|(key, _)| *key == field)
        .map(|(_, api)| *api)
        .ok_or_else(|| {
            anyhow!(
                "count_field must be one of {}",
                COUNT_FIELDS
                    .iter()
                    .map(|(key, _)| *key)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn normalize_application_number(value: &str) -> Result<String> {
    let number = value.trim().to_ascii_uppercase();
    let (prefix, rest) = if let Some(rest) = number.strip_prefix("ANDA") {
        ("ANDA", rest)
    } else if let Some(rest) = number.strip_prefix("BLA") {
        ("BLA", rest)
    } else if let Some(rest) = number.strip_prefix("NDA") {
        ("NDA", rest)
    } else {
        bail!("application_number must look like NDA, ANDA or BLA followed by six digits");
    };
    if rest.len() != 6 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        bail!("application_number must look like NDA, ANDA or BLA followed by six digits");
    }
    Ok(format!("{prefix}{rest}"))
}

fn normalize_sections(sections: Option<&[String]>) -> Result<Option<Vec<String>>> {
    let Some(sections) = sections else {
        return Ok(None);
    };
    if sections.is_empty() || sections.len() > MAX_SECTIONS {
        bail!("sections must contain 1 to {MAX_SECTIONS} label field names");
    }
    let mut out = Vec::new();
    for section in sections {
        let name = section.trim();
        if name.is_empty() || name.len() > 64 {
            bail!("each label section name must contain 1 to 64 characters");
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || !name.as_bytes()[0].is_ascii_lowercase()
        {
            bail!("label section names must be lowercase openFDA field identifiers");
        }
        out.push(name.to_string());
    }
    Ok(Some(out))
}

fn trim_opt(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
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
