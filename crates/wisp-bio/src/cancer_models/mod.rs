//! Native `cancer-models` domain against the public cBioPortal REST API.
//! Independently implemented from:
//!
//! - [cBioPortal REST API](https://www.cbioportal.org/api/swagger-ui/index.html)
//! - [OpenAPI v2](https://www.cbioportal.org/api/v2/api-docs) and
//!   [v3](https://www.cbioportal.org/api/v3/api-docs)
//! - [API clients](https://docs.cbioportal.org/web-API-and-Clients/)
//! - [Clinical file format (OS_/DFS_/PFS_/DSS_ pairs)](https://docs.cbioportal.org/file-formats/)
//!
//! References reviewed 2026-09-06. The public instance is keyless. Sanger Cell
//! Model Passports tools are not implemented (commercial-use license gate).
//! Tests use invented records.

#[cfg(test)]
mod tests;

use crate::http::Source;
use crate::NativeBio;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Duration;
use wisp_llm::ToolSchema;

const PORTAL: &str = "https://www.cbioportal.org";
const API: &str = "https://www.cbioportal.org/api";
const CBIOPORTAL: Source = Source("cBioPortal", Duration::from_millis(500));
const MAX_RECORDS: u32 = 200;
const DEFAULT_STUDIES: u32 = 50;
const DEFAULT_MUTATIONS: u32 = 50;
const DEFAULT_CNA: u32 = 50;
const DEFAULT_CLINICAL: u32 = 100;
const MAX_FREQUENCY_STUDIES: usize = 12;
const FETCH_PAGE: u32 = 200;
const MAX_STUDY_PAGES: u32 = 8;
const FREQUENCY_PAGE: u32 = 500;
const DESCRIPTION_MAX: usize = 256;
const CNA_EVENT_TYPES: &[&str] = &[
    "HOMDEL_AND_AMP",
    "HOMDEL",
    "AMP",
    "GAIN",
    "HETLOSS",
    "DIPLOID",
    "ALL",
];
const SURVIVAL_PREFIXES: &[&str] = &["OS_", "DFS_", "PFS_", "DSS_"];

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        (
            "cancer-models",
            ToolSchema::new(
                "cbioportal_clinical_attributes",
                "Catalogue clinical attributes for one cBioPortal study (patient- or sample-level; STRING or NUMBER). Returns a bounded page and the upstream total-count when the portal supplies it. Survival-style OS_/DFS_/PFS_/DSS_ attribute IDs from the clinical file format are listed when present; their presence is not a survival analysis.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["study_id"],
                    "properties": {
                        "study_id": {"type": "string", "minLength": 1, "maxLength": 128},
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100}
                    }
                }),
            ),
        ),
        (
            "cancer-models",
            ToolSchema::new(
                "cbioportal_cna_in_gene",
                "Discrete copy-number calls (GISTIC-style −2..2) for one gene in one cBioPortal study. Resolves the study's DISCRETE COPY_NUMBER_ALTERATION profile and POSTs a gene filter to /discrete-copy-number/fetch (the GET form is not gene-filtered). event_type selects HOMDEL_AND_AMP (default), HOMDEL, AMP, GAIN, HETLOSS, DIPLOID, or ALL. The listing is bounded; ALL/DIPLOID in large cohorts can exceed the 4 MiB response limit.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["gene_symbol", "study_id"],
                    "properties": {
                        "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                        "study_id": {"type": "string", "minLength": 1, "maxLength": 128},
                        "event_type": {
                            "type": "string",
                            "enum": ["HOMDEL_AND_AMP", "HOMDEL", "AMP", "GAIN", "HETLOSS", "DIPLOID", "ALL"],
                            "default": "HOMDEL_AND_AMP"
                        },
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                    }
                }),
            ),
        ),
        (
            "cancer-models",
            ToolSchema::new(
                "cbioportal_get_study",
                "Retrieve one cBioPortal study by studyId: citation, platform sample counts, and molecular profiles. Sample and patient collection sizes use the META total-count header when supplied; unknown is not reported as zero. Unknown study identifiers fail rather than returning empty evidence.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["study_id"],
                    "properties": {
                        "study_id": {"type": "string", "minLength": 1, "maxLength": 128}
                    }
                }),
            ),
        ),
        (
            "cancer-models",
            ToolSchema::new(
                "cbioportal_list_studies",
                "List public cBioPortal cancer genomics studies. Optional keyword is matched by the portal against study name and cancer type. Optional cancer_type_id is applied locally after retrieval (it is not a documented query parameter). Returns a bounded page with the upstream total-count when present. A capped page is not the complete catalog.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "keyword": {"type": "string", "minLength": 1, "maxLength": 256},
                        "cancer_type_id": {"type": "string", "minLength": 1, "maxLength": 64},
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                    }
                }),
            ),
        ),
        (
            "cancer-models",
            ToolSchema::new(
                "cbioportal_mutation_frequency",
                "Compare the fraction of sequenced samples with at least one mutation in a gene across 1–12 cBioPortal studies. Unknown study IDs and studies without a MUTATION_EXTENDED profile or {studyId}_all sample list are listed separately rather than dropped. Frequency uses sequencedSampleCount as the denominator. A truncated mutation page is a lower bound on mutated samples, not a complete prevalence.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["gene_symbol", "study_ids"],
                    "properties": {
                        "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                        "study_ids": {
                            "type": "array", "minItems": 1, "maxItems": 12,
                            "items": {"type": "string", "minLength": 1, "maxLength": 128}
                        }
                    }
                }),
            ),
        ),
        (
            "cancer-models",
            ToolSchema::new(
                "cbioportal_mutations_in_gene",
                "List somatic mutations for one HUGO symbol or Entrez gene ID in one cBioPortal study. Resolves the study's MUTATION_EXTENDED molecular profile and the {studyId}_all sample list documented by the API examples. Returns a bounded page plus the upstream total-count. Type counts and top protein changes are computed from the returned page; a capped page is not the complete mutation set.",
                json!({
                    "type": "object", "additionalProperties": false,
                    "required": ["gene_symbol", "study_id"],
                    "properties": {
                        "gene_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                        "study_id": {"type": "string", "minLength": 1, "maxLength": 128},
                        "max_records": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                    }
                }),
            ),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(45), dispatch(bio, name, args))
        .await
        .map_err(|_| anyhow!("cBioPortal request exceeded 45 seconds"))?
}

async fn dispatch(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "cbioportal_clinical_attributes" => clinical_attributes(bio, args).await,
        "cbioportal_cna_in_gene" => cna_in_gene(bio, args).await,
        "cbioportal_get_study" => get_study(bio, args).await,
        "cbioportal_list_studies" => list_studies(bio, args).await,
        "cbioportal_mutation_frequency" => mutation_frequency(bio, args).await,
        "cbioportal_mutations_in_gene" => mutations_in_gene(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListStudies {
    keyword: Option<String>,
    cancer_type_id: Option<String>,
    #[serde(default = "default_studies")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetStudy {
    study_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationsInGene {
    gene_symbol: String,
    study_id: String,
    #[serde(default = "default_mutations")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationFrequency {
    gene_symbol: String,
    study_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CnaInGene {
    gene_symbol: String,
    study_id: String,
    #[serde(default = "default_event")]
    event_type: String,
    #[serde(default = "default_cna")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClinicalAttributes {
    study_id: String,
    #[serde(default = "default_clinical")]
    max_records: u32,
}

fn default_studies() -> u32 {
    DEFAULT_STUDIES
}
fn default_mutations() -> u32 {
    DEFAULT_MUTATIONS
}
fn default_cna() -> u32 {
    DEFAULT_CNA
}
fn default_clinical() -> u32 {
    DEFAULT_CLINICAL
}
fn default_event() -> String {
    "HOMDEL_AND_AMP".into()
}

async fn list_studies(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListStudies =
        serde_json::from_value(args.clone()).context("invalid cBioPortal study list arguments")?;
    let cap = bound_page(args.max_records)?;
    let keyword = optional_keyword(args.keyword.as_deref())?;
    let cancer_type_id = optional_id(args.cancer_type_id.as_deref(), "cancer_type_id", 64)?;
    let portal = Portal::new(bio);
    let page_size = if cancer_type_id.is_some() {
        FETCH_PAGE
    } else {
        cap as u32
    };
    let mut collected = Vec::new();
    let mut api_total = None;
    let mut exhausted = false;
    for page in 0..MAX_STUDY_PAGES {
        let mut params = vec![
            ("projection".into(), "DETAILED".into()),
            ("pageSize".into(), page_size.to_string()),
            ("pageNumber".into(), page.to_string()),
            ("sortBy".into(), "studyId".into()),
            ("direction".into(), "ASC".into()),
        ];
        if let Some(keyword) = &keyword {
            params.push(("keyword".into(), keyword.clone()));
        }
        let fetched = portal.get("studies", &params).await?;
        fetched.require_ok("studies")?;
        if page == 0 {
            api_total = fetched.total_count;
        }
        let rows = as_array(fetched.value)?;
        let page_len = rows.len();
        for row in rows {
            let study = shape_study(&row, true);
            if let Some(want) = &cancer_type_id {
                let got = study
                    .get("cancer_type_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !got.eq_ignore_ascii_case(want) {
                    continue;
                }
            }
            collected.push(study);
            if collected.len() > cap {
                break;
            }
        }
        let reached_end = page_len < page_size as usize
            || api_total.is_some_and(|total| {
                u64::from(page.saturating_add(1)).saturating_mul(u64::from(page_size)) >= total
            });
        if reached_end {
            exhausted = true;
            break;
        }
        if collected.len() > cap {
            break;
        }
    }
    let truncated = collected.len() > cap || (!exhausted && cancer_type_id.is_some());
    collected.truncate(cap);
    let header_total = api_total.map(|n| n as usize);
    Ok(json!({
        "source": "cBioPortal",
        "source_url": API,
        "query": {
            "keyword": keyword,
            "cancer_type_id": cancer_type_id,
            "max_records": cap
        },
        "total": header_total,
        "returned": collected.len(),
        "truncated": match header_total {
            Some(total) if cancer_type_id.is_none() => total > collected.len(),
            _ => truncated,
        },
        "studies": collected,
    }))
}

async fn get_study(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetStudy =
        serde_json::from_value(args.clone()).context("invalid cBioPortal study arguments")?;
    let study_id = require_id(&args.study_id, "study_id", 128)?;
    let portal = Portal::new(bio);
    let fetched = portal
        .get(&format!("studies/{}", path_segment(&study_id)), &[])
        .await?;
    if fetched.not_found {
        bail!("cBioPortal study {study_id} was not found");
    }
    fetched.require_ok(&format!("study {study_id}"))?;
    let raw = as_object(fetched.value)?;
    let mut record = shape_study(&raw, false);
    record["sample_count"] = meta_count(
        &portal,
        &format!("studies/{}/samples", path_segment(&study_id)),
    )
    .await?;
    record["patient_count"] = meta_count(
        &portal,
        &format!("studies/{}/patients", path_segment(&study_id)),
    )
    .await?;
    let profiles = portal
        .get(
            &format!("studies/{}/molecular-profiles", path_segment(&study_id)),
            &[
                ("projection".into(), "SUMMARY".into()),
                ("pageSize".into(), "250".into()),
                ("pageNumber".into(), "0".into()),
                ("sortBy".into(), "molecularProfileId".into()),
                ("direction".into(), "ASC".into()),
            ],
        )
        .await?;
    profiles.require_ok(&format!("molecular profiles for {study_id}"))?;
    let mut molecular_profiles: Vec<Value> = as_array(profiles.value)?
        .iter()
        .map(shape_profile)
        .collect();
    molecular_profiles
        .sort_by(|a, b| text(a, "molecular_profile_id").cmp(&text(b, "molecular_profile_id")));
    record["molecular_profiles"] = json!(molecular_profiles);
    record["source"] = json!("cBioPortal");
    record["source_url"] = json!(API);
    Ok(record)
}

async fn mutations_in_gene(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MutationsInGene =
        serde_json::from_value(args.clone()).context("invalid cBioPortal mutation arguments")?;
    let cap = bound_page(args.max_records)?;
    let portal = Portal::new(bio);
    let gene = resolve_gene(&portal, &args.gene_symbol).await?;
    let study_id = require_id(&args.study_id, "study_id", 128)?;
    let profile = profile_for(&portal, &study_id, "MUTATION_EXTENDED", None).await?;
    require_all_sample_list(&portal, &study_id).await?;
    let entrez = gene
        .get("entrez_gene_id")
        .and_then(Value::as_i64)
        .context("cBioPortal gene record omitted entrezGeneId")?;
    let fetched = portal
        .get(
            &format!("molecular-profiles/{}/mutations", path_segment(&profile)),
            &[
                ("sampleListId".into(), format!("{study_id}_all")),
                ("entrezGeneId".into(), entrez.to_string()),
                ("projection".into(), "SUMMARY".into()),
                ("pageSize".into(), cap.to_string()),
                ("pageNumber".into(), "0".into()),
                ("sortBy".into(), "startPosition".into()),
                ("direction".into(), "ASC".into()),
            ],
        )
        .await?;
    fetched.require_ok(&format!("mutations for {study_id}"))?;
    let rows = as_array(fetched.value)?;
    let (total, truncated) = page_meta(rows.len(), cap, fetched.total_count);
    let mut mutations: Vec<Value> = rows.iter().take(cap).map(shape_mutation).collect();
    mutations.sort_by(|a, b| mutation_sort(a).cmp(&mutation_sort(b)));
    let (type_counts, protein_changes, mutated_samples) = mutation_aggregates(&mutations);
    Ok(json!({
        "source": "cBioPortal",
        "source_url": API,
        "gene": gene,
        "study_id": study_id,
        "study_url": study_url(&study_id),
        "molecular_profile_id": profile,
        "total": total,
        "returned": mutations.len(),
        "truncated": truncated,
        "mutated_sample_count": mutated_samples,
        "mutation_type_counts": type_counts,
        "distinct_protein_changes": protein_changes.len(),
        "top_protein_changes": top_n(&protein_changes, 20),
        "aggregates_from_returned": true,
        "mutations": mutations,
    }))
}

async fn mutation_frequency(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: MutationFrequency = serde_json::from_value(args.clone())
        .context("invalid cBioPortal mutation frequency arguments")?;
    let study_ids = require_ids(&args.study_ids, MAX_FREQUENCY_STUDIES, "study_id")?;
    let portal = Portal::new(bio);
    let gene = resolve_gene(&portal, &args.gene_symbol).await?;
    let entrez = gene
        .get("entrez_gene_id")
        .and_then(Value::as_i64)
        .context("cBioPortal gene record omitted entrezGeneId")?;
    let mut frequencies = Vec::new();
    let mut unknown = Vec::new();
    let mut no_mutation_data = Vec::new();
    for study_id in &study_ids {
        let study = portal
            .get(&format!("studies/{}", path_segment(study_id)), &[])
            .await?;
        if study.not_found {
            unknown.push(study_id.clone());
            continue;
        }
        study.require_ok(&format!("study {study_id}"))?;
        let raw = as_object(study.value)?;
        let study_name = text(&raw, "name");
        let sequenced = int_field(&raw, "sequencedSampleCount").unwrap_or(0);
        let profile = match profile_for(&portal, study_id, "MUTATION_EXTENDED", None).await {
            Ok(profile) => profile,
            Err(_) => {
                no_mutation_data.push(study_id.clone());
                continue;
            }
        };
        if !sample_list_exists(&portal, study_id).await? {
            no_mutation_data.push(study_id.clone());
            continue;
        }
        let fetched = portal
            .get(
                &format!("molecular-profiles/{}/mutations", path_segment(&profile)),
                &[
                    ("sampleListId".into(), format!("{study_id}_all")),
                    ("entrezGeneId".into(), entrez.to_string()),
                    ("projection".into(), "ID".into()),
                    ("pageSize".into(), FREQUENCY_PAGE.to_string()),
                    ("pageNumber".into(), "0".into()),
                ],
            )
            .await?;
        if fetched.not_found {
            no_mutation_data.push(study_id.clone());
            continue;
        }
        fetched.require_ok(&format!("mutations for {study_id}"))?;
        let rows = as_array(fetched.value)?;
        let (total, truncated) =
            page_meta(rows.len(), FREQUENCY_PAGE as usize, fetched.total_count);
        let mutated_samples = unique_samples(&rows);
        let frequency = if sequenced > 0 {
            Value::from(
                ((mutated_samples as f64) / (sequenced as f64) * 10_000.0).round() / 10_000.0,
            )
        } else {
            Value::Null
        };
        frequencies.push(json!({
            "study_id": study_id,
            "study_name": study_name,
            "study_url": study_url(study_id),
            "molecular_profile_id": profile,
            "mutation_count": total,
            "mutated_samples": mutated_samples,
            "sequenced_samples": sequenced,
            "frequency": frequency,
            "truncated": truncated,
        }));
    }
    frequencies.sort_by(|a, b| {
        let fa = a.get("frequency").and_then(Value::as_f64);
        let fb = b.get("frequency").and_then(Value::as_f64);
        match (fb, fa) {
            (Some(right), Some(left)) => right
                .partial_cmp(&left)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| {
            text(a, "study_id")
                .unwrap_or_default()
                .cmp(&text(b, "study_id").unwrap_or_default())
        })
    });
    Ok(json!({
        "source": "cBioPortal",
        "source_url": API,
        "gene": gene,
        "count": frequencies.len(),
        "frequencies": frequencies,
        "unknown_studies": unknown,
        "no_mutation_data": no_mutation_data,
    }))
}

async fn cna_in_gene(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: CnaInGene =
        serde_json::from_value(args.clone()).context("invalid cBioPortal CNA arguments")?;
    let cap = bound_page(args.max_records)?;
    let event_type = normalize_event_type(&args.event_type)?;
    let portal = Portal::new(bio);
    let gene = resolve_gene(&portal, &args.gene_symbol).await?;
    let study_id = require_id(&args.study_id, "study_id", 128)?;
    let profile = profile_for(
        &portal,
        &study_id,
        "COPY_NUMBER_ALTERATION",
        Some("DISCRETE"),
    )
    .await?;
    require_all_sample_list(&portal, &study_id).await?;
    let entrez = gene
        .get("entrez_gene_id")
        .and_then(Value::as_i64)
        .context("cBioPortal gene record omitted entrezGeneId")?;
    let body = json!({
        "sampleListId": format!("{study_id}_all"),
        "entrezGeneIds": [entrez]
    });
    let fetched = portal
        .post(
            &format!(
                "molecular-profiles/{}/discrete-copy-number/fetch",
                path_segment(&profile)
            ),
            &[
                ("discreteCopyNumberEventType".into(), event_type.clone()),
                ("projection".into(), "SUMMARY".into()),
            ],
            body,
        )
        .await?;
    fetched.require_ok(&format!("copy-number events for {study_id}"))?;
    let rows = as_array(fetched.value)?;
    let header_total = fetched.total_count;
    let mut events: Vec<Value> = rows.iter().map(shape_cna).collect();
    events.sort_by(|a, b| {
        text(a, "sample_id")
            .unwrap_or_default()
            .cmp(&text(b, "sample_id").unwrap_or_default())
    });
    let altered = unique_sample_ids(&events);
    let mut alteration_counts = BTreeMap::new();
    for event in &events {
        let key = event
            .get("alteration_label")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                event
                    .get("alteration")
                    .and_then(Value::as_i64)
                    .map(|n| n.to_string())
            });
        if let Some(key) = key {
            *alteration_counts.entry(key).or_insert(0u64) += 1;
        }
    }
    let total = header_total.map(|n| n as usize).unwrap_or(events.len());
    let truncated = total > cap || events.len() > cap;
    events.truncate(cap);
    Ok(json!({
        "source": "cBioPortal",
        "source_url": API,
        "gene": gene,
        "study_id": study_id,
        "study_url": study_url(&study_id),
        "molecular_profile_id": profile,
        "event_type": event_type,
        "total": total,
        "returned": events.len(),
        "truncated": truncated,
        "altered_sample_count": altered,
        "alteration_counts": alteration_counts,
        "aggregates_from_fetched": true,
        "events": events,
    }))
}

async fn clinical_attributes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ClinicalAttributes = serde_json::from_value(args.clone())
        .context("invalid cBioPortal clinical attribute arguments")?;
    let cap = bound_page(args.max_records)?;
    let study_id = require_id(&args.study_id, "study_id", 128)?;
    let portal = Portal::new(bio);
    let fetched = portal
        .get(
            &format!("studies/{}/clinical-attributes", path_segment(&study_id)),
            &[
                ("projection".into(), "SUMMARY".into()),
                ("pageSize".into(), cap.to_string()),
                ("pageNumber".into(), "0".into()),
                ("sortBy".into(), "clinicalAttributeId".into()),
                ("direction".into(), "ASC".into()),
            ],
        )
        .await?;
    if fetched.not_found {
        bail!("cBioPortal study {study_id} was not found");
    }
    fetched.require_ok(&format!("clinical attributes for {study_id}"))?;
    let rows = as_array(fetched.value)?;
    let (total, truncated) = page_meta(rows.len(), cap, fetched.total_count);
    let mut attributes: Vec<Value> = rows.iter().take(cap).map(shape_clinical).collect();
    attributes.sort_by(|a, b| {
        text(a, "attribute_id")
            .unwrap_or_default()
            .cmp(&text(b, "attribute_id").unwrap_or_default())
    });
    let ids: BTreeSet<String> = attributes
        .iter()
        .filter_map(|row| text(row, "attribute_id"))
        .collect();
    let survival: Vec<String> = ids
        .iter()
        .filter(|id| {
            SURVIVAL_PREFIXES
                .iter()
                .any(|prefix| id.starts_with(prefix))
        })
        .cloned()
        .collect();
    Ok(json!({
        "source": "cBioPortal",
        "source_url": API,
        "study_id": study_id,
        "study_url": study_url(&study_id),
        "total": total,
        "returned": attributes.len(),
        "truncated": truncated,
        "patient_level_count": attributes.iter().filter(|row| text(row, "level").as_deref() == Some("patient")).count(),
        "sample_level_count": attributes.iter().filter(|row| text(row, "level").as_deref() == Some("sample")).count(),
        "survival_attributes": survival,
        "has_overall_survival": ids.contains("OS_STATUS") && ids.contains("OS_MONTHS"),
        "attributes": attributes,
    }))
}

struct Portal<'a> {
    bio: &'a NativeBio,
}

struct Fetched {
    not_found: bool,
    value: Value,
    total_count: Option<u64>,
    status: StatusCode,
}

impl Fetched {
    fn require_ok(&self, what: &str) -> Result<()> {
        if self.not_found {
            bail!("cBioPortal did not find {what}");
        }
        if !self.status.is_success() {
            bail!(
                "cBioPortal returned HTTP {} for {what}",
                self.status.as_u16()
            );
        }
        Ok(())
    }
}

impl<'a> Portal<'a> {
    fn new(bio: &'a NativeBio) -> Self {
        Self { bio }
    }

    fn base(&self) -> String {
        self.bio
            .credential("CBIOPORTAL_BASE_URL")
            .map(|value| value.trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| API.to_string())
    }

    async fn get(&self, path: &str, params: &[(String, String)]) -> Result<Fetched> {
        let url = format!("{}/{}", self.base(), path.trim_start_matches('/'));
        let response = self
            .bio
            .http()
            .send(CBIOPORTAL, Method::GET, &url, params)
            .await?;
        decode(response)
    }

    async fn post(&self, path: &str, params: &[(String, String)], body: Value) -> Result<Fetched> {
        let url = format!("{}/{}", self.base(), path.trim_start_matches('/'));
        let response = self
            .bio
            .http()
            .send_json(CBIOPORTAL, Method::POST, &url, params, &body)
            .await?;
        decode(response)
    }
}

fn decode(response: crate::http::Response) -> Result<Fetched> {
    if response.status == StatusCode::NOT_FOUND {
        return Ok(Fetched {
            not_found: true,
            value: Value::Null,
            total_count: None,
            status: response.status,
        });
    }
    if !response.status.is_success() {
        return Ok(Fetched {
            not_found: false,
            value: Value::Null,
            total_count: None,
            status: response.status,
        });
    }
    let value = if response.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&response.body).context("cBioPortal returned invalid JSON")?
    };
    Ok(Fetched {
        not_found: false,
        value,
        total_count: response.total_count,
        status: response.status,
    })
}

async fn meta_count(portal: &Portal<'_>, path: &str) -> Result<Value> {
    let fetched = portal
        .get(
            path,
            &[
                ("projection".into(), "META".into()),
                ("pageSize".into(), "1".into()),
                ("pageNumber".into(), "0".into()),
            ],
        )
        .await?;
    if fetched.not_found || !fetched.status.is_success() {
        return Ok(Value::Null);
    }
    Ok(fetched.total_count.map(Value::from).unwrap_or(Value::Null))
}

async fn resolve_gene(portal: &Portal<'_>, symbol: &str) -> Result<Value> {
    let raw = require_id(symbol, "gene_symbol", 64)?;
    let gene_id = if raw.bytes().all(|b| b.is_ascii_digit()) {
        raw
    } else {
        raw.to_ascii_uppercase()
    };
    let fetched = portal
        .get(&format!("genes/{}", path_segment(&gene_id)), &[])
        .await?;
    if fetched.not_found {
        bail!("cBioPortal gene {gene_id} was not found");
    }
    fetched.require_ok(&format!("gene {gene_id}"))?;
    let raw = as_object(fetched.value)?;
    let symbol = text(&raw, "hugoGeneSymbol")
        .filter(|value| !value.is_empty())
        .unwrap_or(gene_id);
    let entrez =
        int_field(&raw, "entrezGeneId").context("cBioPortal gene record omitted entrezGeneId")?;
    Ok(json!({
        "symbol": symbol,
        "entrez_gene_id": entrez,
        "type": text(&raw, "type"),
    }))
}

async fn profile_for(
    portal: &Portal<'_>,
    study_id: &str,
    alteration: &str,
    datatype: Option<&str>,
) -> Result<String> {
    let fetched = portal
        .get(
            &format!("studies/{}/molecular-profiles", path_segment(study_id)),
            &[
                ("projection".into(), "SUMMARY".into()),
                ("pageSize".into(), "250".into()),
                ("pageNumber".into(), "0".into()),
                ("sortBy".into(), "molecularProfileId".into()),
                ("direction".into(), "ASC".into()),
            ],
        )
        .await?;
    if fetched.not_found {
        bail!("cBioPortal study {study_id} was not found");
    }
    fetched.require_ok(&format!("molecular profiles for {study_id}"))?;
    let rows = as_array(fetched.value)?;
    let mut available = BTreeSet::new();
    let mut matches = Vec::new();
    for row in &rows {
        if let Some(kind) = text(row, "molecularAlterationType") {
            available.insert(kind.clone());
            if kind != alteration {
                continue;
            }
        } else {
            continue;
        }
        if let Some(want) = datatype {
            if text(row, "datatype").as_deref() != Some(want) {
                continue;
            }
        }
        if let Some(id) = text(row, "molecularProfileId") {
            matches.push(id);
        }
    }
    matches.sort();
    matches.into_iter().next().ok_or_else(|| {
        let extra = datatype
            .map(|value| format!("/{value}"))
            .unwrap_or_default();
        anyhow!(
            "study {study_id} has no {alteration}{extra} molecular profile (available alteration types: {:?})",
            available.into_iter().collect::<Vec<_>>()
        )
    })
}

async fn require_all_sample_list(portal: &Portal<'_>, study_id: &str) -> Result<String> {
    if sample_list_exists(portal, study_id).await? {
        Ok(format!("{study_id}_all"))
    } else {
        bail!("study {study_id} has no '{study_id}_all' sample list; cannot scope an all-samples query")
    }
}

async fn sample_list_exists(portal: &Portal<'_>, study_id: &str) -> Result<bool> {
    let list_id = format!("{study_id}_all");
    let fetched = portal
        .get(&format!("sample-lists/{}", path_segment(&list_id)), &[])
        .await?;
    if fetched.not_found {
        return Ok(false);
    }
    fetched.require_ok(&format!("sample list {list_id}"))?;
    Ok(true)
}

fn shape_study(raw: &Value, trim_description: bool) -> Value {
    let cancer_type = raw.get("cancerType");
    let description = text(raw, "description").map(|value| {
        if trim_description {
            trim_text(&value, DESCRIPTION_MAX)
        } else {
            value
        }
    });
    json!({
        "study_id": text(raw, "studyId"),
        "name": text(raw, "name"),
        "description": description,
        "cancer_type_id": text(raw, "cancerTypeId"),
        "cancer_type": cancer_type.and_then(|value| text(value, "name")),
        "reference_genome": text(raw, "referenceGenome"),
        "pmid": text(raw, "pmid"),
        "citation": text(raw, "citation"),
        "public": raw.get("publicStudy"),
        "groups": text(raw, "groups"),
        "import_date": text(raw, "importDate"),
        "sequenced_sample_count": int_field(raw, "sequencedSampleCount"),
        "cna_sample_count": int_field(raw, "cnaSampleCount"),
        "structural_variant_count": int_field(raw, "structuralVariantCount"),
        "mrna_rnaseq_v2_sample_count": int_field(raw, "mrnaRnaSeqV2SampleCount"),
        "rppa_sample_count": int_field(raw, "rppaSampleCount"),
        "treatment_count": int_field(raw, "treatmentCount"),
        "url": text(raw, "studyId").as_deref().map(study_url),
    })
}

fn shape_profile(raw: &Value) -> Value {
    json!({
        "molecular_profile_id": text(raw, "molecularProfileId"),
        "alteration_type": text(raw, "molecularAlterationType"),
        "datatype": text(raw, "datatype"),
        "name": text(raw, "name"),
        "description": text(raw, "description").map(|value| trim_text(&value, DESCRIPTION_MAX)),
    })
}

fn shape_mutation(raw: &Value) -> Value {
    json!({
        "sample_id": text(raw, "sampleId"),
        "patient_id": text(raw, "patientId"),
        "protein_change": text(raw, "proteinChange"),
        "mutation_type": text(raw, "mutationType"),
        "mutation_status": text(raw, "mutationStatus"),
        "chromosome": text(raw, "chr"),
        "start_position": int_field(raw, "startPosition"),
        "end_position": int_field(raw, "endPosition"),
        "reference_allele": text(raw, "referenceAllele"),
        "variant_allele": text(raw, "variantAllele"),
        "variant_type": text(raw, "variantType"),
        "ncbi_build": text(raw, "ncbiBuild"),
        "protein_pos_start": int_field(raw, "proteinPosStart"),
        "protein_pos_end": int_field(raw, "proteinPosEnd"),
        "tumor_alt_count": int_field(raw, "tumorAltCount"),
        "tumor_ref_count": int_field(raw, "tumorRefCount"),
        "refseq_mrna_id": text(raw, "refseqMrnaId"),
    })
}

fn shape_cna(raw: &Value) -> Value {
    let alteration = int_field(raw, "alteration");
    json!({
        "sample_id": text(raw, "sampleId"),
        "patient_id": text(raw, "patientId"),
        "alteration": alteration,
        "alteration_label": alteration.and_then(cna_label),
    })
}

fn shape_clinical(raw: &Value) -> Value {
    let patient = raw
        .get("patientAttribute")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "attribute_id": text(raw, "clinicalAttributeId"),
        "display_name": text(raw, "displayName"),
        "description": text(raw, "description").map(|value| trim_text(&value, DESCRIPTION_MAX)),
        "datatype": text(raw, "datatype"),
        "level": if patient { "patient" } else { "sample" },
        "priority": text(raw, "priority"),
    })
}

fn cna_label(code: i64) -> Option<&'static str> {
    match code {
        -2 => Some("deep_deletion"),
        -1 => Some("shallow_deletion"),
        0 => Some("diploid"),
        1 => Some("gain"),
        2 => Some("amplification"),
        _ => None,
    }
}

fn mutation_aggregates(rows: &[Value]) -> (BTreeMap<String, u64>, BTreeMap<String, u64>, usize) {
    let mut types = BTreeMap::new();
    let mut proteins = BTreeMap::new();
    let mut samples = HashSet::new();
    for row in rows {
        if let Some(kind) = text(row, "mutation_type") {
            *types.entry(kind).or_insert(0) += 1;
        }
        if let Some(change) = text(row, "protein_change") {
            *proteins.entry(change).or_insert(0) += 1;
        }
        if let Some(sample) = text(row, "sample_id") {
            samples.insert(sample);
        }
    }
    (types, proteins, samples.len())
}

fn top_n(counts: &BTreeMap<String, u64>, n: usize) -> BTreeMap<String, u64> {
    let mut ranked: Vec<_> = counts.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    ranked
        .into_iter()
        .take(n)
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}

fn unique_samples(rows: &[Value]) -> usize {
    rows.iter()
        .filter_map(|row| text(row, "sampleId").or_else(|| text(row, "sample_id")))
        .collect::<HashSet<_>>()
        .len()
}

fn unique_sample_ids(rows: &[Value]) -> usize {
    rows.iter()
        .filter_map(|row| text(row, "sample_id"))
        .collect::<HashSet<_>>()
        .len()
}

fn mutation_sort(row: &Value) -> (i64, String, String) {
    (
        row.get("start_position")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        text(row, "protein_change").unwrap_or_default(),
        text(row, "sample_id").unwrap_or_default(),
    )
}

fn page_meta(returned: usize, cap: usize, header_total: Option<u64>) -> (usize, bool) {
    match header_total {
        Some(total) => (total as usize, total as usize > returned.min(cap)),
        None => (returned.min(cap), returned >= cap),
    }
}

fn as_array(value: Value) -> Result<Vec<Value>> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(rows) => Ok(rows),
        _ => bail!("cBioPortal returned an unexpected list"),
    }
}

fn as_object(value: Value) -> Result<Value> {
    match value {
        Value::Object(_) => Ok(value),
        _ => bail!("cBioPortal returned an unexpected record"),
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn int_field(value: &Value, key: &str) -> Option<i64> {
    match value.get(key) {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.parse().ok(),
        _ => None,
    }
}

fn trim_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index + 3 >= limit {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn bound_page(n: u32) -> Result<usize> {
    if !(1..=MAX_RECORDS).contains(&n) {
        bail!("max_records must be between 1 and {MAX_RECORDS}");
    }
    Ok(n as usize)
}

fn require_id(value: &str, what: &str, max: usize) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{what} is required");
    }
    if trimmed.len() > max {
        bail!("{what} exceeds {max} characters");
    }
    if trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#') {
        bail!("{what} must not contain URL path characters");
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        bail!("{what} contains unsupported characters");
    }
    Ok(trimmed.to_string())
}

fn optional_id(value: Option<&str>, what: &str, max: usize) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => Ok(Some(require_id(text, what, max)?)),
    }
}

fn optional_keyword(value: Option<&str>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.len() > 256 {
                bail!("keyword exceeds 256 characters");
            }
            if trimmed.chars().any(|ch| ch.is_control()) {
                bail!("keyword must not contain control characters");
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

fn require_ids(ids: &[String], bound: usize, what: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        let cleaned = require_id(id, what, 128)?;
        if seen.insert(cleaned.clone()) {
            out.push(cleaned);
        }
    }
    if out.is_empty() {
        bail!("provide at least one {what}");
    }
    if out.len() > bound {
        bail!(
            "{} {what}s exceeds the per-call bound of {bound}",
            out.len()
        );
    }
    Ok(out)
}

fn normalize_event_type(value: &str) -> Result<String> {
    let event = value.trim().to_ascii_uppercase();
    if CNA_EVENT_TYPES.contains(&event.as_str()) {
        Ok(event)
    } else {
        bail!(
            "event_type must be one of {} (got {value:?})",
            CNA_EVENT_TYPES.join(", ")
        )
    }
}

fn study_url(study_id: &str) -> String {
    format!("{PORTAL}/study/summary?id={}", path_segment(study_id))
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
