use super::*;
use anyhow::Context;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: Option<String>,
    biome_lineage: Option<String>,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetStudies {
    accessions: Vec<String>,
    #[serde(default = "default_false")]
    include_analyses: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StudyAnalyses {
    accession: String,
    #[serde(default = "default_page")]
    max_records: u32,
}

pub(super) async fn search_studies(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid MGnify search arguments")?;
    let cap = bound_page(args.max_records)?;
    let query = args
        .query
        .as_deref()
        .map(|value| require_text(value, "query", 512))
        .transpose()?;
    let biome = args
        .biome_lineage
        .as_deref()
        .map(|value| require_text(value, "biome_lineage", 512))
        .transpose()?;
    match (query, biome) {
        (Some(_), Some(_)) | (None, None) => {
            bail!("provide exactly one of query or biome_lineage")
        }
        (Some(query), None) => {
            let (records, total, truncated) =
                list_studies(bio, "studies", &[("search".into(), query.to_string())], cap).await?;
            Ok(page(
                records,
                total,
                truncated,
                json!({"type": "search", "query": query}),
            ))
        }
        (None, Some(lineage)) => {
            if lineage.contains("..") || lineage.contains('/') || lineage.contains('\\') {
                bail!("biome_lineage must be a GOLD-style lineage without path separators");
            }
            let path = format!("biomes/{}/studies", path_seg(lineage));
            let (records, total, truncated) = list_studies(bio, &path, &[], cap).await?;
            Ok(page(
                records,
                total,
                truncated,
                json!({"type": "biome", "lineage": lineage}),
            ))
        }
    }
}

pub(super) async fn get_studies(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetStudies =
        serde_json::from_value(args.clone()).context("invalid MGnify study arguments")?;
    let mut accessions = unique_ids(&args.accessions, MAX_IDS, "MGnify accession")?;
    for acc in &accessions {
        if !matches_prefix_digits(acc, "MGYS") {
            bail!("MGnify accession {acc:?} must look like MGYS00000001");
        }
    }
    accessions.sort();
    let base = api_base(bio, "MGNIFY_BASE_URL", MGNIFY);
    let mut studies = Vec::new();
    let mut missing = Vec::new();
    for acc in &accessions {
        let response = send(
            bio,
            MGNIFY_SRC,
            Method::GET,
            &format!("{base}/studies/{}", path_seg(acc)),
            &[],
        )
        .await?;
        if missing_status(response.status) {
            missing.push(acc.clone());
            continue;
        }
        let payload = response.json()?;
        reject_error_payload("MGnify", &payload)?;
        let mut record = flatten_study(payload_item(&payload));
        if args.include_analyses {
            let (analyses, total, truncated) = fetch_analyses(bio, acc, MAX_PAGE as usize).await?;
            let breakdown = analysis_breakdowns(&analyses);
            record["analyses_total"] = json!(total);
            record["analyses_truncated"] = json!(truncated);
            record["analyses_by_pipeline_version"] = json!(breakdown.0);
            record["analyses_by_experiment_type"] = json!(breakdown.1);
            record["analyses"] = json!(analyses);
        }
        studies.push(record);
    }
    Ok(json!({
        "source": "MGnify",
        "source_url": "https://www.ebi.ac.uk/metagenomics",
        "n_requested": accessions.len(),
        "returned": studies.len(),
        "missing": missing,
        "studies": studies,
    }))
}

pub(super) async fn get_study_analyses(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: StudyAnalyses =
        serde_json::from_value(args.clone()).context("invalid MGnify analyses arguments")?;
    let accession = require_mgys(&args.accession)?;
    let cap = bound_page(args.max_records)?;
    let (analyses, total, truncated) = fetch_analyses(bio, &accession, cap).await?;
    Ok(json!({
        "source": "MGnify",
        "source_url": mgnify_url(&accession),
        "study_accession": accession,
        "analyses_count": total,
        "returned": analyses.len(),
        "truncated": truncated,
        "analyses": analyses,
    }))
}

async fn list_studies(
    bio: &NativeBio,
    path: &str,
    extra: &[(String, String)],
    cap: usize,
) -> Result<(Vec<Value>, Option<u64>, bool)> {
    let base = api_base(bio, "MGNIFY_BASE_URL", MGNIFY);
    let mut records = Vec::new();
    let mut total = None;
    let mut truncated = false;
    let mut page = 1u32;
    loop {
        let mut params = extra.to_vec();
        params.push(("page".into(), page.to_string()));
        params.push(("page_size".into(), PAGE_SIZE.to_string()));
        let raw = get_json(bio, MGNIFY_SRC, &format!("{base}/{path}/"), &params).await?;
        if total.is_none() {
            total = list_count(&raw);
        }
        let items = list_items(&raw);
        let batch_len = items.len();
        for item in items {
            records.push(flatten_study(&item));
            if records.len() >= cap {
                truncated = true;
                break;
            }
        }
        if truncated || batch_len < PAGE_SIZE as usize {
            break;
        }
        if let Some(count) = total {
            if records.len() as u64 >= count {
                break;
            }
        }
        page += 1;
        if page > 5 {
            truncated = true;
            break;
        }
    }
    if !truncated {
        if let Some(count) = total {
            truncated = (records.len() as u64) < count;
        }
    }
    records.sort_by(|a, b| field_text(a, &["accession"]).cmp(&field_text(b, &["accession"])));
    Ok((records, total, truncated))
}

async fn fetch_analyses(
    bio: &NativeBio,
    accession: &str,
    cap: usize,
) -> Result<(Vec<Value>, Option<u64>, bool)> {
    let base = api_base(bio, "MGNIFY_BASE_URL", MGNIFY);
    let mut records = Vec::new();
    let mut total = None;
    let mut truncated = false;
    let mut page = 1u32;
    loop {
        let params = vec![
            ("page".into(), page.to_string()),
            ("page_size".into(), PAGE_SIZE.to_string()),
        ];
        let raw = get_json(
            bio,
            MGNIFY_SRC,
            &format!("{base}/studies/{}/analyses/", path_seg(accession)),
            &params,
        )
        .await?;
        if total.is_none() {
            total = list_count(&raw);
        }
        let items = list_items(&raw);
        let batch_len = items.len();
        for item in items {
            records.push(flatten_analysis(&item, accession));
            if records.len() >= cap {
                truncated = true;
                break;
            }
        }
        if truncated || batch_len < PAGE_SIZE as usize {
            break;
        }
        if let Some(count) = total {
            if records.len() as u64 >= count {
                break;
            }
        }
        page += 1;
        if page > 5 {
            truncated = true;
            break;
        }
    }
    if !truncated {
        if let Some(count) = total {
            truncated = (records.len() as u64) < count;
        }
    }
    records.sort_by(|a, b| {
        field_text(a, &["analysis_accession"]).cmp(&field_text(b, &["analysis_accession"]))
    });
    Ok((records, total, truncated))
}

fn page(records: Vec<Value>, total: Option<u64>, truncated: bool, spec: Value) -> Value {
    json!({
        "source": "MGnify",
        "source_url": "https://www.ebi.ac.uk/metagenomics",
        "query": spec,
        "count": total,
        "returned": records.len(),
        "truncated": truncated,
        "records": records,
    })
}

fn require_mgys(value: &str) -> Result<String> {
    let accession = value.trim().to_ascii_uppercase();
    if !matches_prefix_digits(&accession, "MGYS") {
        bail!("MGnify accession {value:?} must look like MGYS00000001");
    }
    Ok(accession)
}

fn mgnify_url(accession: &str) -> String {
    format!(
        "https://www.ebi.ac.uk/metagenomics/studies/{}",
        path_seg(accession)
    )
}

fn list_items(raw: &Value) -> Vec<Value> {
    if let Some(items) = raw.get("items").and_then(Value::as_array) {
        return items.clone();
    }
    if let Some(data) = raw.get("data") {
        match data {
            Value::Array(items) => return items.clone(),
            Value::Object(_) => return vec![data.clone()],
            _ => {}
        }
    }
    Vec::new()
}

fn list_count(raw: &Value) -> Option<u64> {
    field_u64(raw, &["count"]).or_else(|| {
        raw.get("meta")
            .and_then(|meta| meta.get("pagination"))
            .and_then(|page| field_u64(page, &["count"]))
    })
}

fn payload_item(raw: &Value) -> &Value {
    raw.get("data").unwrap_or(raw)
}

pub(super) fn flatten_study(obj: &Value) -> Value {
    let attrs = obj.get("attributes").unwrap_or(obj);
    let accession = field_text(obj, &["id", "accession"])
        .or_else(|| field_text(attrs, &["accession"]))
        .unwrap_or_default();
    json!({
        "accession": accession,
        "secondary_accession": field_text(attrs, &["secondary-accession", "secondary_accession"]),
        "bioproject": field_text(attrs, &["bioproject"]),
        "study_name": field_text(attrs, &["study-name", "study_name", "name"]),
        "abstract": field_text(attrs, &["study-abstract", "study_abstract", "abstract"]),
        "biome_lineages": biome_lineages(obj, attrs),
        "samples_count": field_u64(attrs, &["samples-count", "samples_count"]),
        "centre_name": field_text(attrs, &["centre-name", "centre_name"]),
        "data_origination": field_text(attrs, &["data-origination", "data_origination"]),
        "is_private": attrs.get("is-private").and_then(as_bool).or_else(|| attrs.get("is_private").and_then(as_bool)),
        "last_update": field_text(attrs, &["last-update", "last_update"]),
        "url": mgnify_url(&accession),
    })
}

fn biome_lineages(obj: &Value, attrs: &Value) -> Vec<String> {
    let mut lineages = Vec::new();
    if let Some(Value::Array(items)) = attrs.get("biomes").or_else(|| attrs.get("biome_lineages")) {
        for item in items {
            if let Some(id) = as_text(item).or_else(|| field_text(item, &["id", "lineage"])) {
                lineages.push(id);
            }
        }
    }
    if let Some(rel) = obj.get("relationships").and_then(|rel| rel.get("biomes")) {
        match rel.get("data") {
            Some(Value::Array(items)) => {
                for item in items {
                    if let Some(id) = field_text(item, &["id"]) {
                        lineages.push(id);
                    }
                }
            }
            Some(Value::Object(map)) => {
                if let Some(id) = map.get("id").and_then(as_text) {
                    lineages.push(id);
                }
            }
            _ => {}
        }
    }
    lineages.sort();
    lineages.dedup();
    lineages
}

fn flatten_analysis(obj: &Value, study: &str) -> Value {
    let attrs = obj.get("attributes").unwrap_or(obj);
    let accession = field_text(obj, &["id", "accession"])
        .or_else(|| field_text(attrs, &["accession"]))
        .unwrap_or_default();
    json!({
        "analysis_accession": accession,
        "study_accession": field_text(attrs, &["study_accession", "study-accession"]).unwrap_or_else(|| study.to_string()),
        "pipeline_version": field_text(attrs, &["pipeline-version", "pipeline_version"]),
        "experiment_type": field_text(attrs, &["experiment-type", "experiment_type"]),
        "analysis_status": field_text(attrs, &["analysis-status", "analysis_status", "status"]),
        "run_accession": rel_id(obj, "run").or_else(|| field_text(attrs, &["run_accession"])),
        "assembly_accession": rel_id(obj, "assembly").or_else(|| field_text(attrs, &["assembly_accession"])),
        "sample_accession": rel_id(obj, "sample").or_else(|| field_text(attrs, &["sample_accession"])),
        "url": format!("https://www.ebi.ac.uk/metagenomics/analyses/{}", path_seg(&accession)),
    })
}

fn rel_id(obj: &Value, name: &str) -> Option<String> {
    let data = obj.get("relationships")?.get(name)?.get("data")?;
    match data {
        Value::Object(_) => field_text(data, &["id"]),
        Value::Array(items) => items.first().and_then(|item| field_text(item, &["id"])),
        _ => as_text(data),
    }
}

fn analysis_breakdowns(records: &[Value]) -> (BTreeMap<String, u64>, BTreeMap<String, u64>) {
    let mut by_pipeline = BTreeMap::new();
    let mut by_experiment = BTreeMap::new();
    for record in records {
        let pipeline =
            field_text(record, &["pipeline_version"]).unwrap_or_else(|| "unknown".into());
        *by_pipeline.entry(pipeline).or_insert(0) += 1;
        let experiment =
            field_text(record, &["experiment_type"]).unwrap_or_else(|| "unknown".into());
        *by_experiment.entry(experiment).or_insert(0) += 1;
    }
    (by_pipeline, by_experiment)
}
