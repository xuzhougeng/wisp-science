use super::*;
use anyhow::Context;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListStudies {
    #[serde(default = "default_list")]
    max_returned: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetStudies {
    accessions: Vec<String>,
    #[serde(default = "default_false")]
    include_samples: bool,
    #[serde(default = "default_rows")]
    max_sample_rows_returned: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StudyFiles {
    accession: String,
    #[serde(default = "default_true")]
    include_data_files: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchFiles {
    accession: String,
    pattern: Option<String>,
}

pub(super) async fn list_studies(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ListStudies =
        serde_json::from_value(args.clone()).context("invalid MetaboLights list arguments")?;
    if !(1..=2000).contains(&args.max_returned) {
        bail!("max_returned must be between 1 and 2000");
    }
    let cap = args.max_returned as usize;
    let base = api_base(bio, "METABOLIGHTS_BASE_URL", METABOLIGHTS);
    let raw = get_json(bio, MTBLS, &format!("{base}/studies"), &[]).await?;
    let mut accessions: Vec<String> = raw
        .get("content")
        .and_then(Value::as_array)
        .context("MetaboLights omitted the public study list")?
        .iter()
        .filter_map(as_text)
        .map(|acc| acc.trim().to_ascii_uppercase())
        .filter(|acc| matches_prefix_digits(acc, "MTBLS"))
        .collect();
    accessions.sort_by_key(mtbls_key);
    accessions.dedup();
    let reported = field_u64(&raw, &["studies", "reported_count", "count"]);
    let truncated = accessions.len() > cap;
    accessions.truncate(cap);
    Ok(json!({
        "source": "MetaboLights",
        "source_url": "https://www.ebi.ac.uk/metabolights/",
        "count": accessions.len(),
        "reported_count": reported,
        "truncated": truncated,
        "accessions": accessions,
    }))
}

pub(super) async fn get_studies(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetStudies =
        serde_json::from_value(args.clone()).context("invalid MetaboLights study arguments")?;
    let mut accessions = unique_ids(&args.accessions, MAX_IDS, "MetaboLights accession")?;
    for acc in &accessions {
        if !matches_prefix_digits(acc, "MTBLS") {
            bail!("MetaboLights accession {acc:?} must look like MTBLS1");
        }
    }
    accessions.sort_by_key(mtbls_key);
    let cap = bound_rows(args.max_sample_rows_returned)?;
    let base = api_base(bio, "METABOLIGHTS_BASE_URL", METABOLIGHTS);
    let mut records = Vec::new();
    let mut not_found = Vec::new();
    for acc in &accessions {
        let response = send(
            bio,
            MTBLS,
            Method::GET,
            &format!("{base}/studies/public/study/{}", path_seg(acc)),
            &[],
        )
        .await?;
        if missing_status(response.status) {
            not_found.push(acc.clone());
            continue;
        }
        let payload = response.json()?;
        reject_error_payload("MetaboLights", &payload)?;
        let mut record = extract_study(&payload)?;
        let content = payload.get("content").unwrap_or(&Value::Null);
        record["protocols"] = json!(protocols(content));
        if args.include_samples {
            record["sample_table"] = json!(sample_table(content.get("sampleTable"), cap));
        }
        record["url"] = json!(mtbls_url(acc));
        records.push(record);
    }
    Ok(json!({
        "source": "MetaboLights",
        "source_url": "https://www.ebi.ac.uk/metabolights/",
        "n_requested": accessions.len(),
        "returned": records.len(),
        "not_found": not_found,
        "records": records,
    }))
}

pub(super) async fn get_study_files(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: StudyFiles =
        serde_json::from_value(args.clone()).context("invalid MetaboLights files arguments")?;
    let accession = require_mtbls(&args.accession)?;
    let base = api_base(bio, "METABOLIGHTS_BASE_URL", METABOLIGHTS);
    let payload = get_json(
        bio,
        MTBLS,
        &format!("{base}/studies/{}/files", path_seg(&accession)),
        &[("include_raw_data".into(), "true".into())],
    )
    .await?;
    let mut entries: Vec<Value> = payload
        .get("study")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(slim_file)
        .collect();
    entries.sort_by(|a, b| {
        let da = a.get("directory").and_then(Value::as_bool).unwrap_or(false);
        let db = b.get("directory").and_then(Value::as_bool).unwrap_or(false);
        db.cmp(&da)
            .then_with(|| field_text(a, &["file"]).cmp(&field_text(b, &["file"])))
    });
    let metadata: Vec<_> = entries
        .iter()
        .filter(|entry| {
            field_text(entry, &["type"])
                .unwrap_or_default()
                .starts_with("metadata")
        })
        .filter_map(|entry| field_text(entry, &["file"]))
        .collect();
    let mut record = json!({
        "source": "MetaboLights",
        "source_url": mtbls_url(&accession),
        "accession": accession,
        "latest_version": payload.get("latest"),
        "study_folder": entries,
        "n_study_folder_entries": entries.len(),
        "metadata_files": metadata,
        "url": mtbls_url(&accession),
    });
    if args.include_data_files {
        let data = list_data_files(bio, &accession, None).await?;
        record["data_files"] = json!(data.0);
        record["n_data_files"] = json!(data.0.len());
        record["data_files_truncated"] = json!(data.1);
    }
    Ok(record)
}

pub(super) async fn search_data_files(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchFiles =
        serde_json::from_value(args.clone()).context("invalid MetaboLights data-file arguments")?;
    let accession = require_mtbls(&args.accession)?;
    let pattern = args
        .pattern
        .as_deref()
        .map(|value| require_text(value, "pattern", 256))
        .transpose()?
        .map(str::to_string);
    if let Some(pattern) = pattern.as_deref() {
        if pattern.contains("..") || pattern.contains('\\') {
            bail!("pattern must be a filename glob without path traversal");
        }
    }
    let (files, truncated) = list_data_files(bio, &accession, pattern.as_deref()).await?;
    Ok(json!({
        "source": "MetaboLights",
        "source_url": mtbls_url(&accession),
        "accession": accession,
        "pattern": pattern,
        "n_files": files.len(),
        "truncated": truncated,
        "files": files,
    }))
}

async fn list_data_files(
    bio: &NativeBio,
    accession: &str,
    pattern: Option<&str>,
) -> Result<(Vec<String>, bool)> {
    let base = api_base(bio, "METABOLIGHTS_BASE_URL", METABOLIGHTS);
    let mut params = vec![
        ("file_match".into(), "true".into()),
        ("folder_match".into(), "false".into()),
    ];
    if let Some(pattern) = pattern {
        params.push(("search_pattern".into(), pattern.to_string()));
    }
    let payload = get_json(
        bio,
        MTBLS,
        &format!("{base}/studies/{}/public-data-files", path_seg(accession)),
        &params,
    )
    .await?;
    let mut names: Vec<String> = payload
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| field_text(&entry, &["name", "file"]))
        .collect();
    if let Some(pattern) = pattern {
        names.retain(|name| glob_match(pattern, file_name(name)));
    }
    names.sort();
    names.dedup();
    let truncated = names.len() > MAX_ROWS as usize;
    names.truncate(MAX_ROWS as usize);
    Ok((names, truncated))
}

fn require_mtbls(value: &str) -> Result<String> {
    let accession = value.trim().to_ascii_uppercase();
    if !matches_prefix_digits(&accession, "MTBLS") {
        bail!("MetaboLights accession {value:?} must look like MTBLS1");
    }
    Ok(accession)
}

fn mtbls_url(accession: &str) -> String {
    format!("https://www.ebi.ac.uk/metabolights/{}", path_seg(accession))
}

fn mtbls_key(acc: &String) -> (u64, String) {
    let n = acc
        .strip_prefix("MTBLS")
        .and_then(|rest| rest.parse().ok())
        .unwrap_or(u64::MAX);
    (n, acc.clone())
}

fn slim_file(raw: Value) -> Value {
    json!({
        "file": field_text(&raw, &["file", "name"]),
        "type": field_text(&raw, &["type"]),
        "status": field_text(&raw, &["status"]),
        "directory": raw.get("directory").and_then(as_bool).unwrap_or(false),
    })
}

pub(super) fn extract_study(payload: &Value) -> Result<Value> {
    let content = payload
        .get("content")
        .filter(|value| value.is_object())
        .context("MetaboLights omitted ISA study content")?;
    let accession = field_text(content, &["studyIdentifier", "accession"])
        .context("MetaboLights study omitted its accession")?;
    let mut organisms = Vec::new();
    if let Some(Value::Array(items)) = content.get("organism") {
        for item in items {
            let name = field_text(item, &["organismName", "organism"]);
            let part = field_text(item, &["organismPart", "organism_part"]);
            if name.is_some() || part.is_some() {
                organisms.push(json!({"organism": name, "organism_part": part}));
            }
        }
    }
    organisms.sort_by(|a, b| {
        field_text(a, &["organism"])
            .cmp(&field_text(b, &["organism"]))
            .then_with(|| field_text(a, &["organism_part"]).cmp(&field_text(b, &["organism_part"])))
    });
    let mut assays = Vec::new();
    if let Some(Value::Array(items)) = content.get("assays") {
        for item in items {
            assays.push(json!({
                "assay_number": field_u64(item, &["assayNumber", "assay_number"]),
                "measurement": field_text(item, &["measurement"]),
                "technology": field_text(item, &["technology"]),
                "platform": field_text(item, &["platform"]),
                "filename": field_text(item, &["fileName", "filename"]),
            }));
        }
    }
    let technologies = {
        let mut values: Vec<_> = assays
            .iter()
            .filter_map(|assay| field_text(assay, &["technology"]))
            .collect();
        values.sort();
        values.dedup();
        values
    };
    let factors = names(content.get("factors"));
    let descriptors = {
        let mut out = Vec::new();
        if let Some(Value::Array(items)) = content.get("descriptors") {
            for item in items {
                if let Some(desc) = field_text(item, &["description", "name"]) {
                    out.push(desc);
                }
            }
        }
        out.sort();
        out.dedup();
        out
    };
    let sample_count = content
        .get("sampleTable")
        .and_then(|table| table.get("data"))
        .and_then(Value::as_array)
        .map(|rows| rows.len())
        .unwrap_or(0);
    let derived = content.get("derivedData").unwrap_or(&Value::Null);
    Ok(json!({
        "accession": accession,
        "title": field_text(content, &["title"]),
        "description": field_text(content, &["studyDescription", "description"]),
        "study_status": field_text(content, &["studyStatus", "status"]),
        "release_year": field_u64(derived, &["releaseYear", "release_year"]),
        "submission_year": field_u64(derived, &["submissionYear", "submission_year"]),
        "organisms": organisms,
        "assays": assays,
        "assay_count": assays.len(),
        "technologies": technologies,
        "factors": factors,
        "descriptors": descriptors,
        "sample_count": sample_count,
    }))
}

fn protocols(content: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = content.get("protocols") {
        for item in items {
            let name = field_text(item, &["name"]);
            let description = field_text(item, &["description"]);
            if name.is_some() || description.is_some() {
                out.push(json!({"name": name, "description": description}));
            }
        }
    }
    out
}

pub(super) fn sample_table(table: Option<&Value>, cap: usize) -> Value {
    let table = table.unwrap_or(&Value::Null);
    let mut columns: Vec<(u64, String)> = Vec::new();
    match table.get("fields") {
        Some(Value::Object(map)) => {
            for field in map.values() {
                if let Some(index) = field_u64(field, &["index"]) {
                    let header = field_text(field, &["header", "name"])
                        .unwrap_or_else(|| format!("column_{index}"));
                    columns.push((index, header));
                }
            }
        }
        Some(Value::Array(items)) => {
            for (i, field) in items.iter().enumerate() {
                let header =
                    field_text(field, &["header", "name"]).unwrap_or_else(|| format!("column_{i}"));
                columns.push((i as u64, header));
            }
        }
        _ => {}
    }
    columns.sort_by_key(|(index, _)| *index);
    let headers: Vec<String> = columns.into_iter().map(|(_, name)| name).collect();
    let data = table.get("data").and_then(Value::as_array);
    let n_total = data.map(|rows| rows.len()).unwrap_or(0);
    let truncated = n_total > cap;
    let mut rows = Vec::new();
    if let Some(data) = data {
        for raw in data.iter().take(cap) {
            let cells = match raw {
                Value::Array(items) => items.iter().filter_map(as_text).collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            let mut row = serde_json::Map::new();
            for (i, header) in headers.iter().enumerate() {
                row.insert(
                    header.clone(),
                    json!(cells.get(i).cloned().unwrap_or_default()),
                );
            }
            rows.push(Value::Object(row));
        }
    }
    json!({
        "headers": headers,
        "rows": rows,
        "n_rows_total": n_total,
        "rows_truncated": truncated,
    })
}
