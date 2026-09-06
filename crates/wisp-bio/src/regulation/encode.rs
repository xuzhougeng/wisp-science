use super::*;
use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const ORGANISM_EXPERIMENT: &str = "replicates.library.biosample.donor.organism.scientific_name";

const EXPERIMENT_FIELDS: &[&str] = &[
    "accession",
    "assay_title",
    "assay_term_name",
    "target.label",
    "biosample_ontology.term_name",
    "status",
    "date_released",
    "lab.title",
];
const BIOSAMPLE_FIELDS: &[&str] = &[
    "accession",
    "biosample_ontology.term_name",
    "biosample_ontology.classification",
    "organism.scientific_name",
    "status",
    "lab.title",
    "summary",
    "date_created",
];
const FILE_FIELDS: &[&str] = &[
    "accession",
    "file_format",
    "output_type",
    "assay_term_name",
    "assembly",
    "dataset",
    "status",
    "file_size",
    "date_created",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchExperiments {
    assay_title: Option<String>,
    target: Option<String>,
    organism: Option<String>,
    #[serde(default = "super::default_status")]
    status: String,
    date_released_before: Option<String>,
    extra_filters: Option<BTreeMap<String, String>>,
    #[serde(default = "super::default_rows")]
    max_rows: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchBiosamples {
    term_name: Option<String>,
    classification: Option<String>,
    organism: Option<String>,
    #[serde(default = "super::default_status")]
    status: String,
    date_created_before: Option<String>,
    extra_filters: Option<BTreeMap<String, String>>,
    #[serde(default = "super::default_rows")]
    max_rows: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListFiles {
    file_format: Option<String>,
    assay_term_name: Option<String>,
    biosample_term_name: Option<String>,
    #[serde(default = "super::default_status")]
    status: String,
    date_created_before: Option<String>,
    extra_filters: Option<BTreeMap<String, String>>,
    #[serde(default = "super::default_rows")]
    max_rows: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetAccession {
    accession: String,
}

pub async fn search_experiments(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchExperiments = serde_json::from_value(args.clone())
        .context("invalid ENCODE experiment search arguments")?;
    let mut filters = BTreeMap::new();
    filters.insert("status".into(), query_text(&args.status, 64, "status")?);
    if let Some(value) = optional_query(&args.assay_title, 128, "assay_title")? {
        filters.insert("assay_title".into(), value);
    }
    if let Some(value) = optional_query(&args.target, 128, "target")? {
        filters.insert("target.label".into(), value);
    }
    if let Some(value) = optional_query(&args.organism, 128, "organism")? {
        filters.insert(ORGANISM_EXPERIMENT.into(), value);
    }
    if let Some(date) = args.date_released_before.as_deref() {
        let date = iso_date(date, "date_released_before")?;
        filters.insert("date_released".into(), format!("lte:{date}"));
    }
    filters.extend(extra_filters(args.extra_filters)?);
    search(
        bio,
        "Experiment",
        EXPERIMENT_FIELDS,
        filters,
        bound_rows(args.max_rows, ENCODE_MAX_ROWS)?,
        project_experiment_row,
    )
    .await
}

pub async fn search_biosamples(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchBiosamples = serde_json::from_value(args.clone())
        .context("invalid ENCODE biosample search arguments")?;
    let mut filters = BTreeMap::new();
    filters.insert("status".into(), query_text(&args.status, 64, "status")?);
    if let Some(value) = optional_query(&args.term_name, 128, "term_name")? {
        filters.insert("biosample_ontology.term_name".into(), value);
    }
    if let Some(value) = optional_query(&args.classification, 128, "classification")? {
        filters.insert("biosample_ontology.classification".into(), value);
    }
    if let Some(value) = optional_query(&args.organism, 128, "organism")? {
        filters.insert("organism.scientific_name".into(), value);
    }
    if let Some(date) = args.date_created_before.as_deref() {
        let date = iso_date(date, "date_created_before")?;
        filters.insert("date_created".into(), format!("lte:{date}"));
    }
    filters.extend(extra_filters(args.extra_filters)?);
    search(
        bio,
        "Biosample",
        BIOSAMPLE_FIELDS,
        filters,
        bound_rows(args.max_rows, ENCODE_MAX_ROWS)?,
        project_biosample_row,
    )
    .await
}

pub async fn list_files(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListFiles =
        serde_json::from_value(args.clone()).context("invalid ENCODE file list arguments")?;
    let mut filters = BTreeMap::new();
    filters.insert("status".into(), query_text(&args.status, 64, "status")?);
    if let Some(value) = optional_query(&args.file_format, 64, "file_format")? {
        filters.insert("file_format".into(), value);
    }
    if let Some(value) = optional_query(&args.assay_term_name, 128, "assay_term_name")? {
        filters.insert("assay_term_name".into(), value);
    }
    if let Some(value) = optional_query(&args.biosample_term_name, 128, "biosample_term_name")? {
        filters.insert("biosample_ontology.term_name".into(), value);
    }
    if let Some(date) = args.date_created_before.as_deref() {
        let date = iso_date(date, "date_created_before")?;
        filters.insert("date_created".into(), format!("lte:{date}"));
    }
    filters.extend(extra_filters(args.extra_filters)?);
    search(
        bio,
        "File",
        FILE_FIELDS,
        filters,
        bound_rows(args.max_rows, ENCODE_MAX_ROWS)?,
        project_file_row,
    )
    .await
}

pub async fn get_experiment(bio: &NativeBio, args: &Value) -> Result<Value> {
    let doc = get_object(bio, args, "ENCSR", "experiments").await?;
    Ok(project_experiment(&doc))
}

pub async fn get_file(bio: &NativeBio, args: &Value) -> Result<Value> {
    let doc = get_object(bio, args, "ENCFF", "files").await?;
    Ok(project_file(&doc))
}

pub async fn get_biosample(bio: &NativeBio, args: &Value) -> Result<Value> {
    let doc = get_object(bio, args, "ENCBS", "biosamples").await?;
    Ok(project_biosample(&doc))
}

async fn search(
    bio: &NativeBio,
    type_name: &str,
    fields: &[&str],
    filters: BTreeMap<String, String>,
    limit: u32,
    project: fn(&Value) -> Option<Value>,
) -> Result<Value> {
    let base = encode_base(bio);
    let url = join_url(&base, "search/");
    let mut params = vec![
        ("type".into(), type_name.into()),
        ("format".into(), "json".into()),
        ("limit".into(), limit.to_string()),
    ];
    for field in fields {
        params.push(("field".into(), (*field).into()));
    }
    for (key, value) in &filters {
        params.push((key.clone(), value.clone()));
    }
    let (status, body) = get_json(bio, ENCODE, &url, &params).await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(search_page(
            type_name,
            &filters,
            0,
            Vec::new(),
            format!("{ENCODE_PORTAL}/search/?type={type_name}"),
        ));
    }
    let payload =
        body.ok_or_else(|| anyhow::anyhow!("ENCODE returned HTTP {}", status.as_u16()))?;
    if payload.get("status").and_then(Value::as_str) == Some("error") {
        bail!(
            "ENCODE search failed ({})",
            payload
                .get("code")
                .and_then(Value::as_u64)
                .unwrap_or(status.as_u16() as u64)
        );
    }
    let notification = payload.get("notification").and_then(Value::as_str);
    if notification == Some("No results found") {
        return Ok(search_page(
            type_name,
            &filters,
            0,
            Vec::new(),
            portal_search_url(&payload, type_name),
        ));
    }
    if let Some(message) = notification {
        if message != "Success" {
            bail!("ENCODE search reported {message}");
        }
    }
    let total = payload
        .get("total")
        .and_then(Value::as_u64)
        .context("ENCODE search omitted total")?;
    let graph = payload
        .get("@graph")
        .and_then(Value::as_array)
        .context("ENCODE search omitted @graph")?;
    let rows: Vec<Value> = graph.iter().filter_map(project).collect();
    Ok(search_page(
        type_name,
        &filters,
        total,
        rows,
        portal_search_url(&payload, type_name),
    ))
}

fn search_page(
    type_name: &str,
    filters: &BTreeMap<String, String>,
    total: u64,
    rows: Vec<Value>,
    source_url: String,
) -> Value {
    let returned = rows.len();
    json!({
        "source": "ENCODE",
        "source_url": source_url,
        "query": {
            "type": type_name,
            "filters": filters,
        },
        "total": total,
        "returned": returned,
        "truncated": (returned as u64) < total,
        "has_more": (returned as u64) < total,
        "records": rows,
    })
}

fn portal_search_url(payload: &Value, type_name: &str) -> String {
    payload
        .get("@id")
        .and_then(Value::as_str)
        .filter(|id| id.starts_with('/'))
        .map(|id| format!("{ENCODE_PORTAL}{id}"))
        .unwrap_or_else(|| format!("{ENCODE_PORTAL}/search/?type={type_name}"))
}

async fn get_object(
    bio: &NativeBio,
    args: &Value,
    prefix: &str,
    collection: &str,
) -> Result<Value> {
    let args: GetAccession =
        serde_json::from_value(args.clone()).context("invalid ENCODE accession arguments")?;
    let accession = encode_accession(&args.accession, prefix)?;
    let url = join_url(
        &encode_base(bio),
        &format!("{collection}/{}/", path_segment(&accession)),
    );
    let params = [
        ("format".into(), "json".into()),
        ("frame".into(), "object".into()),
    ];
    let (status, body) = get_json(bio, ENCODE, &url, &params).await?;
    if status == StatusCode::NOT_FOUND {
        bail!("ENCODE has no {collection} record {accession}");
    }
    let doc = body.ok_or_else(|| anyhow::anyhow!("ENCODE returned HTTP {}", status.as_u16()))?;
    if doc.get("status").and_then(Value::as_str) == Some("error") {
        bail!("ENCODE has no {collection} record {accession}");
    }
    Ok(doc)
}

fn encode_accession(value: &str, prefix: &str) -> Result<String> {
    let accession = value.trim().to_ascii_uppercase();
    if accession.len() < 8 || accession.len() > 32 {
        bail!("ENCODE accession must be 8–32 characters");
    }
    if !accession.starts_with(prefix)
        || !accession
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        bail!("ENCODE accession must match {prefix} followed by uppercase letters or digits");
    }
    Ok(accession)
}

fn project_experiment_row(row: &Value) -> Option<Value> {
    let accession = row.get("accession").and_then(Value::as_str)?;
    Some(json!({
        "accession": accession,
        "assay_title": row.get("assay_title"),
        "assay_term_name": row.get("assay_term_name"),
        "target": nested_name(row.get("target").unwrap_or(&Value::Null)),
        "biosample_term_name": nested_name(row.get("biosample_ontology").unwrap_or(&Value::Null))
            .or_else(|| row.get("biosample_ontology.term_name").and_then(as_string)),
        "status": row.get("status"),
        "date_released": row.get("date_released"),
        "lab": nested_name(row.get("lab").unwrap_or(&Value::Null)),
        "url": format!("{ENCODE_PORTAL}/experiments/{}/", path_segment(accession)),
    }))
}

fn project_biosample_row(row: &Value) -> Option<Value> {
    let accession = row.get("accession").and_then(Value::as_str)?;
    let ontology = row.get("biosample_ontology").unwrap_or(&Value::Null);
    Some(json!({
        "accession": accession,
        "term_name": nested_name(ontology)
            .or_else(|| row.get("biosample_ontology.term_name").and_then(as_string)),
        "classification": ontology.get("classification").and_then(as_string)
            .or_else(|| row.get("biosample_ontology.classification").and_then(as_string)),
        "organism": nested_name(row.get("organism").unwrap_or(&Value::Null))
            .or_else(|| row.get("organism.scientific_name").and_then(as_string)),
        "status": row.get("status"),
        "lab": nested_name(row.get("lab").unwrap_or(&Value::Null)),
        "summary": row.get("summary"),
        "date_created": row.get("date_created"),
        "url": format!("{ENCODE_PORTAL}/biosamples/{}/", path_segment(accession)),
    }))
}

fn project_file_row(row: &Value) -> Option<Value> {
    let accession = row.get("accession").and_then(Value::as_str)?;
    Some(json!({
        "accession": accession,
        "file_format": row.get("file_format"),
        "output_type": row.get("output_type"),
        "assay_term_name": row.get("assay_term_name"),
        "assembly": row.get("assembly"),
        "dataset": row.get("dataset"),
        "status": row.get("status"),
        "file_size": row.get("file_size"),
        "date_created": row.get("date_created"),
        "url": format!("{ENCODE_PORTAL}/files/{}/", path_segment(accession)),
    }))
}

fn project_experiment(doc: &Value) -> Value {
    let accession = doc
        .get("accession")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target = doc.get("target").unwrap_or(&Value::Null);
    let bio = doc.get("biosample_ontology").unwrap_or(&Value::Null);
    json!({
        "source": "ENCODE",
        "source_url": format!("{ENCODE_PORTAL}/experiments/{}/", path_segment(accession)),
        "record_type": "experiment",
        "accession": accession,
        "status": doc.get("status"),
        "assay_term_name": doc.get("assay_term_name"),
        "assay_title": doc.get("assay_title"),
        "target_label": nested_name(target),
        "biosample_term_name": nested_name(bio),
        "biosample_classification": bio.get("classification"),
        "biosample_summary": doc.get("biosample_summary"),
        "description": doc.get("description"),
        "lab": nested_name(doc.get("lab").unwrap_or(&Value::Null)),
        "award_project": doc.get("award").and_then(|award| award.get("project")).cloned()
            .or_else(|| nested_name(doc.get("award").unwrap_or(&Value::Null)).map(Value::String)),
        "date_released": doc.get("date_released"),
        "date_submitted": doc.get("date_submitted"),
        "assembly": doc.get("assembly"),
        "bio_replicate_count": doc.get("bio_replicate_count"),
        "tech_replicate_count": doc.get("tech_replicate_count"),
        "replication_type": doc.get("replication_type"),
        "dbxrefs": doc.get("dbxrefs"),
        "doi": doc.get("doi"),
        "uuid": doc.get("uuid"),
    })
}

fn project_file(doc: &Value) -> Value {
    let accession = doc
        .get("accession")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let href = doc.get("href").and_then(Value::as_str);
    json!({
        "source": "ENCODE",
        "source_url": format!("{ENCODE_PORTAL}/files/{}/", path_segment(accession)),
        "record_type": "file",
        "accession": accession,
        "status": doc.get("status"),
        "file_format": doc.get("file_format"),
        "file_format_type": doc.get("file_format_type"),
        "output_type": doc.get("output_type"),
        "output_category": doc.get("output_category"),
        "assay_term_name": doc.get("assay_term_name"),
        "assembly": doc.get("assembly"),
        "dataset": doc.get("dataset"),
        "biological_replicates": doc.get("biological_replicates"),
        "file_size": doc.get("file_size"),
        "md5sum": doc.get("md5sum"),
        "content_md5sum": doc.get("content_md5sum"),
        "run_type": doc.get("run_type"),
        "read_length": doc.get("read_length"),
        "lab": nested_name(doc.get("lab").unwrap_or(&Value::Null)),
        "date_created": doc.get("date_created"),
        "href": href,
        "download_url": href.filter(|path| path.starts_with('/')).map(|path| format!("{ENCODE_PORTAL}{path}")),
        "uuid": doc.get("uuid"),
    })
}

fn project_biosample(doc: &Value) -> Value {
    let accession = doc
        .get("accession")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let bio = doc.get("biosample_ontology").unwrap_or(&Value::Null);
    let organism = doc.get("organism").unwrap_or(&Value::Null);
    let donor = doc.get("donor").unwrap_or(&Value::Null);
    json!({
        "source": "ENCODE",
        "source_url": format!("{ENCODE_PORTAL}/biosamples/{}/", path_segment(accession)),
        "record_type": "biosample",
        "accession": accession,
        "status": doc.get("status"),
        "term_name": nested_name(bio),
        "classification": bio.get("classification"),
        "organism": nested_name(organism),
        "donor": donor.get("accession").cloned().or_else(|| nested_name(donor).map(Value::String)),
        "source_lab": nested_name(doc.get("source").unwrap_or(&Value::Null)),
        "lab": nested_name(doc.get("lab").unwrap_or(&Value::Null)),
        "summary": doc.get("summary"),
        "life_stage": doc.get("life_stage"),
        "age_display": doc.get("age_display"),
        "sex": doc.get("sex"),
        "date_created": doc.get("date_created"),
        "uuid": doc.get("uuid"),
    })
}
