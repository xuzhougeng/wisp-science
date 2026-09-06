use super::*;
use anyhow::Context;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    keyword: Option<String>,
    organism: Option<String>,
    instrument: Option<String>,
    disease: Option<String>,
    #[serde(default)]
    extra_filters: BTreeMap<String, String>,
    #[serde(default = "default_page")]
    max_records_returned: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetProjects {
    accessions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectProteins {
    project_accession: String,
    keyword: Option<String>,
    #[serde(default = "default_page")]
    max_records: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProteinProjects {
    protein_accession: String,
}

pub(super) async fn search_projects(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid PRIDE search arguments")?;
    let cap = bound_page(args.max_records_returned)?;
    let mut params = vec![
        ("pageSize".into(), PAGE_SIZE.to_string()),
        ("sortFields".into(), "accession".into()),
        ("sortDirection".into(), "ASC".into()),
    ];
    if let Some(keyword) = args.keyword.as_deref() {
        params.push((
            "keyword".into(),
            require_text(keyword, "keyword", 512)?.to_string(),
        ));
    }
    let mut filters = Vec::new();
    if let Some(organism) = args.organism.as_deref() {
        filters.push(format!(
            "organisms=={}",
            require_text(organism, "organism", 256)?
        ));
    }
    if let Some(instrument) = args.instrument.as_deref() {
        filters.push(format!(
            "instruments=={}",
            require_text(instrument, "instrument", 256)?
        ));
    }
    if let Some(disease) = args.disease.as_deref() {
        filters.push(format!(
            "diseases=={}",
            require_text(disease, "disease", 256)?
        ));
    }
    for (key, value) in &args.extra_filters {
        let key = require_filter_field(key)?;
        let value = require_text(value, "filter value", 256)?;
        if value.contains(',') || value.contains('=') {
            bail!("PRIDE filter values must not contain comma or '='");
        }
        filters.push(format!("{key}=={value}"));
    }
    if args.keyword.is_none() && filters.is_empty() {
        bail!("provide a keyword or at least one PRIDE filter");
    }
    if !filters.is_empty() {
        params.push(("filter".into(), filters.join(",")));
    }
    let base = api_base(bio, "PRIDE_BASE_URL", PRIDE);
    let mut records = Vec::new();
    let mut total = None;
    let mut truncated = false;
    let mut page = 0u32;
    loop {
        let mut page_params = params.clone();
        page_params.push(("page".into(), page.to_string()));
        let raw = get_json(
            bio,
            PRIDE_SRC,
            &format!("{base}/search/projects"),
            &page_params,
        )
        .await?;
        if total.is_none() {
            total = collection_total(&raw);
        }
        let items = collection_items(&raw);
        let batch_len = items.len();
        for item in items {
            records.push(project_record(&item, "search"));
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
        if page >= 5 {
            truncated = true;
            break;
        }
    }
    if !truncated {
        if let Some(count) = total {
            truncated = (records.len() as u64) < count;
        }
    }
    if total.is_none() && !truncated {
        total = Some(records.len() as u64);
    }
    Ok(json!({
        "source": "PRIDE Archive",
        "source_url": "https://www.ebi.ac.uk/pride/archive",
        "query": {
            "keyword": args.keyword,
            "filter": filters.join(","),
        },
        "total": total,
        "returned": records.len(),
        "truncated": truncated,
        "records": records,
    }))
}

pub(super) async fn get_projects(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetProjects =
        serde_json::from_value(args.clone()).context("invalid PRIDE project arguments")?;
    let mut accessions = unique_ids(&args.accessions, MAX_IDS, "PRIDE accession")?;
    for acc in &accessions {
        if !is_pride_accession(acc) {
            bail!("PRIDE accession {acc:?} must look like PXD000001");
        }
    }
    accessions.sort();
    let base = api_base(bio, "PRIDE_BASE_URL", PRIDE);
    let mut records = Vec::new();
    let mut not_found = Vec::new();
    for acc in &accessions {
        let response = send(
            bio,
            PRIDE_SRC,
            Method::GET,
            &format!("{base}/projects/{}", path_seg(acc)),
            &[],
        )
        .await?;
        if missing_status(response.status) {
            not_found.push(acc.clone());
            continue;
        }
        let raw = response.json()?;
        reject_error_payload("PRIDE", &raw)?;
        records.push(project_record(&raw, "detail"));
    }
    Ok(json!({
        "source": "PRIDE Archive",
        "source_url": "https://www.ebi.ac.uk/pride/archive",
        "n_requested": accessions.len(),
        "returned": records.len(),
        "not_found": not_found,
        "records": records,
    }))
}

pub(super) async fn search_project_proteins(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ProjectProteins =
        serde_json::from_value(args.clone()).context("invalid PRIDE protein-table arguments")?;
    let accession = require_pride(&args.project_accession)?;
    let cap = bound_page(args.max_records)?;
    let keyword = args
        .keyword
        .as_deref()
        .map(|value| require_text(value, "keyword", 256))
        .transpose()?;
    let base = api_base(bio, "PRIDE_BASE_URL", PRIDE);
    let mut proteins = Vec::new();
    let mut truncated = false;
    let mut page = 0u32;
    loop {
        let mut params = vec![
            ("projectAccession".into(), accession.clone()),
            ("pageSize".into(), PAGE_SIZE.to_string()),
            ("page".into(), page.to_string()),
        ];
        if let Some(keyword) = keyword {
            params.push(("keyword".into(), keyword.to_string()));
        }
        let raw = get_json(
            bio,
            PRIDE_SRC,
            &format!("{base}/pride-ap/search/proteins"),
            &params,
        )
        .await?;
        let items = collection_items(&raw);
        let batch_len = items.len();
        if batch_len == 0 {
            break;
        }
        for item in items {
            proteins.push(json!({
                "protein_accession": field_text(&item, &["proteinAccession", "protein_accession", "accession"]),
                "protein_name": field_text(&item, &["proteinName", "protein_name", "name"]),
                "gene": field_text(&item, &["gene", "geneName", "gene_name"]),
                "project_count": field_u64(&item, &["projectCount", "project_count"]),
            }));
            if proteins.len() >= cap {
                truncated = true;
                break;
            }
        }
        if truncated || batch_len < PAGE_SIZE as usize {
            break;
        }
        page += 1;
        if page >= 5 {
            truncated = true;
            break;
        }
    }
    proteins.sort_by(|a, b| {
        field_text(a, &["protein_accession"]).cmp(&field_text(b, &["protein_accession"]))
    });
    Ok(json!({
        "source": "PRIDE Archive",
        "source_url": pride_url(&accession),
        "project_accession": accession,
        "keyword": keyword,
        "n_proteins": proteins.len(),
        "truncated": truncated,
        "proteins": proteins,
        "note": "This index serves affinity-proteomics (PAD) projects; classic PXD submissions typically have no queryable per-project protein table.",
    }))
}

pub(super) async fn find_projects_for_protein(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: ProteinProjects =
        serde_json::from_value(args.clone()).context("invalid PRIDE protein-search arguments")?;
    let protein =
        require_text(&args.protein_accession, "protein_accession", 32)?.to_ascii_uppercase();
    if protein
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
    {
        bail!("protein_accession must be a UniProt-style identifier");
    }
    let base = api_base(bio, "PRIDE_BASE_URL", PRIDE);
    let raw = get_json(
        bio,
        PRIDE_SRC,
        &format!("{base}/proteins/search"),
        &[("accession".into(), protein.clone())],
    )
    .await?;
    let mut records = Vec::new();
    for item in collection_items(&raw) {
        let mut projects = names(
            item.get("projects")
                .or_else(|| item.get("projectAccessions")),
        );
        projects.sort();
        projects.dedup();
        let accession = field_text(
            &item,
            &["proteinAccession", "protein_accession", "accession"],
        )
        .unwrap_or_else(|| protein.clone());
        records.push(json!({
            "protein_accession": accession,
            "n_projects": projects.len(),
            "projects": projects,
            "project_urls": projects.iter().map(|acc| pride_url(acc)).collect::<Vec<_>>(),
        }));
    }
    records.sort_by(|a, b| {
        field_text(a, &["protein_accession"]).cmp(&field_text(b, &["protein_accession"]))
    });
    Ok(json!({
        "source": "PRIDE Archive",
        "source_url": "https://www.ebi.ac.uk/pride/archive",
        "query_accession": protein,
        "n_records": records.len(),
        "records": records,
    }))
}

fn require_pride(value: &str) -> Result<String> {
    let accession = value.trim().to_ascii_uppercase();
    if !is_pride_accession(&accession) {
        bail!("PRIDE accession {value:?} must look like PXD000001");
    }
    Ok(accession)
}

fn is_pride_accession(value: &str) -> bool {
    matches_prefix_digits(value, "PXD")
        || matches_prefix_digits(value, "PAD")
        || matches_prefix_digits(value, "PRD")
}

fn pride_url(accession: &str) -> String {
    format!(
        "https://www.ebi.ac.uk/pride/archive/projects/{}",
        path_seg(accession)
    )
}

fn require_filter_field(key: &str) -> Result<String> {
    let key = key.trim();
    if key.is_empty()
        || key.len() > 64
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        bail!("PRIDE extra_filters keys must be alphanumeric field names");
    }
    Ok(key.to_string())
}

fn collection_items(raw: &Value) -> Vec<Value> {
    match raw {
        Value::Array(items) => items.clone(),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map
                .get("content")
                .or_else(|| map.get("list"))
                .or_else(|| map.get("projects"))
                .or_else(|| map.get("records"))
                .or_else(|| map.get("files"))
            {
                return items.clone();
            }
            if let Some(embedded) = map.get("_embedded") {
                if let Some(Value::Array(items)) = embedded
                    .get("compactprojects")
                    .or_else(|| embedded.get("projects"))
                    .or_else(|| embedded.get("proteins"))
                {
                    return items.clone();
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn collection_total(raw: &Value) -> Option<u64> {
    field_u64(raw, &["total", "totalElements", "total_records", "count"]).or_else(|| {
        raw.get("page")
            .and_then(|page| field_u64(page, &["totalElements", "total_elements", "total"]))
    })
}

pub(super) fn project_record(raw: &Value, origin: &str) -> Value {
    let accession = field_text(raw, &["accession"]).unwrap_or_default();
    json!({
        "accession": accession,
        "title": field_text(raw, &["title"]),
        "description": field_text(raw, &["projectDescription", "description"]),
        "organisms": names(raw.get("organisms")),
        "organism_parts": names(raw.get("organismsPart").or_else(|| raw.get("organismParts"))),
        "diseases": names(raw.get("diseases")),
        "instruments": names(raw.get("instruments")),
        "experiment_types": names(raw.get("experimentTypes").or_else(|| raw.get("experiment_types"))),
        "quantification_methods": names(raw.get("quantificationMethods").or_else(|| raw.get("quantification_methods"))),
        "keywords": names(raw.get("keywords")),
        "submission_date": date_field(raw, &["submissionDate", "submission_date"]),
        "publication_date": date_field(raw, &["publicationDate", "publication_date"]),
        "submitters": person_names(raw.get("submitters")),
        "lab_pis": person_names(raw.get("labPIs").or_else(|| raw.get("labPis"))),
        "references": references(raw.get("references")),
        "origin": origin,
        "url": pride_url(&accession),
    })
}

fn date_field(raw: &Value, keys: &[&str]) -> Option<String> {
    field_text(raw, keys).map(|value| value.chars().take(10).collect())
}

fn person_names(value: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = value {
        for item in items {
            if let Some(name) = as_text(item).or_else(|| field_text(item, &["name"])) {
                out.push(name);
                continue;
            }
            let first = field_text(item, &["firstName", "first_name"]).unwrap_or_default();
            let last = field_text(item, &["lastName", "last_name"]).unwrap_or_default();
            let combined = format!("{first} {last}");
            let combined = combined.trim();
            if !combined.is_empty() {
                out.push(combined.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn references(value: Option<&Value>) -> Vec<Value> {
    let mut refs = Vec::new();
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                if item.is_object() {
                    refs.push(json!({
                        "pubmed_id": field_text(item, &["pubmedID", "pubmed_id", "pubmedId"]),
                        "doi": field_text(item, &["doi", "DOI"]),
                        "reference_line": field_text(item, &["referenceLine", "reference_line", "reference"]),
                    }));
                } else if let Some(text) = as_text(item) {
                    refs.push(parse_reference_line(&text));
                }
            }
        }
        Some(other) => {
            if let Some(text) = as_text(other) {
                refs.push(parse_reference_line(&text));
            }
        }
        None => {}
    }
    refs
}

fn parse_reference_line(text: &str) -> Value {
    let pubmed = text
        .split("--pubMed:")
        .nth(1)
        .and_then(|rest| rest.split("--doi:").next())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null");
    let doi = text
        .split("--doi:")
        .nth(1)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null");
    let line = text.split("--pubMed:").next().unwrap_or(text).trim();
    json!({
        "pubmed_id": pubmed,
        "doi": doi,
        "reference_line": line,
    })
}
