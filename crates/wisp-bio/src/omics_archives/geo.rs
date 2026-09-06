use super::*;
use crate::http::NCBI;
use anyhow::Context;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchSeries {
    term: String,
    #[serde(default = "default_page")]
    retmax: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetSeries {
    accessions: Vec<String>,
}

pub(super) async fn search_series(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchSeries =
        serde_json::from_value(args.clone()).context("invalid GEO search arguments")?;
    let term = require_text(&args.term, "term", MAX_QUERY)?;
    let cap = bound_page(args.retmax)?;
    let (total, ids) = esearch(bio, term, cap).await?;
    let docs = esummary(bio, &ids).await?;
    let mut records = Vec::new();
    for id in &ids {
        if let Some(doc) = docs.get(id) {
            records.push(project_summary(doc));
        }
    }
    records.sort_by(|a, b| field_text(a, &["accession"]).cmp(&field_text(b, &["accession"])));
    let returned = records.len();
    Ok(json!({
        "source": "NCBI GEO DataSets",
        "source_url": "https://www.ncbi.nlm.nih.gov/geo/",
        "query": {"term": term, "retmax": cap},
        "total": total,
        "returned": returned,
        "truncated": (cap as u64) < total || ids.len() < total as usize,
        "retrieval_ceiling": 10_000,
        "records": records,
    }))
}

pub(super) async fn get_series(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetSeries =
        serde_json::from_value(args.clone()).context("invalid GEO series arguments")?;
    let accessions = unique_ids(&args.accessions, MAX_GEO_SERIES, "GSE accession")?;
    for acc in &accessions {
        if !matches_prefix_digits(acc, "GSE") {
            bail!("GSE accession {acc:?} is not a GEO series identifier");
        }
    }
    let term = format!(
        "({}) AND gse[ETYP]",
        accessions
            .iter()
            .map(|acc| format!("{acc}[ACCN]"))
            .collect::<Vec<_>>()
            .join(" OR ")
    );
    let (_total, ids) = esearch(bio, &term, accessions.len().max(1) * 2).await?;
    let docs = esummary(bio, &ids).await?;
    let mut by_acc = std::collections::BTreeMap::new();
    for doc in docs.values() {
        if let Some(acc) = field_text(doc, &["accession"]) {
            by_acc.insert(acc.to_ascii_uppercase(), doc.clone());
        }
    }
    let mut records = Vec::new();
    let mut missing = Vec::new();
    for acc in &accessions {
        let Some(doc) = by_acc.get(acc) else {
            missing.push(acc.clone());
            continue;
        };
        let header = fetch_soft(bio, acc, "self").await?;
        let series = parse_series_header(&header)?;
        let samples_text = fetch_soft(bio, acc, "gsm").await.unwrap_or_default();
        let mut samples = parse_sample_headers(&samples_text);
        if samples.is_empty() {
            samples = esummary_samples(doc);
        }
        records.push(assemble_series(acc, doc, &series, samples));
    }
    Ok(json!({
        "source": "NCBI GEO DataSets",
        "source_url": "https://www.ncbi.nlm.nih.gov/geo/",
        "n_requested": accessions.len(),
        "returned": records.len(),
        "missing": missing,
        "records": records,
    }))
}

async fn esearch(bio: &NativeBio, term: &str, retmax: usize) -> Result<(u64, Vec<String>)> {
    let base = api_base(bio, "NCBI_EUTILS_URL", NCBI_EUTILS);
    let mut params = vec![
        ("db".into(), "gds".into()),
        ("term".into(), term.to_string()),
        ("retmode".into(), "json".into()),
        ("retmax".into(), retmax.min(10_000).to_string()),
        ("retstart".into(), "0".into()),
    ];
    params.extend(ncbi_identity(bio));
    let raw = post_json(bio, NCBI, &format!("{base}/esearch.fcgi"), &params).await?;
    let result = raw
        .get("esearchresult")
        .context("GEO omitted search results")?;
    if result.get("ERROR").is_some() || result.get("errorlist").is_some() {
        bail!("GEO rejected the search expression");
    }
    let total = result
        .get("count")
        .and_then(as_u64)
        .context("GEO omitted the search count")?;
    let ids: Vec<String> = result
        .get("idlist")
        .and_then(Value::as_array)
        .context("GEO returned an invalid identifier list")?
        .iter()
        .filter_map(as_text)
        .collect();
    if ids.len() > retmax {
        bail!("GEO returned more identifiers than requested");
    }
    Ok((total, ids))
}

async fn esummary(
    bio: &NativeBio,
    ids: &[String],
) -> Result<std::collections::BTreeMap<String, Value>> {
    let mut docs = std::collections::BTreeMap::new();
    if ids.is_empty() {
        return Ok(docs);
    }
    let base = api_base(bio, "NCBI_EUTILS_URL", NCBI_EUTILS);
    let mut params = vec![
        ("db".into(), "gds".into()),
        ("id".into(), ids.join(",")),
        ("retmode".into(), "json".into()),
        ("version".into(), "2.0".into()),
    ];
    params.extend(ncbi_identity(bio));
    let raw = post_json(bio, NCBI, &format!("{base}/esummary.fcgi"), &params).await?;
    let result = raw
        .get("result")
        .and_then(Value::as_object)
        .context("GEO omitted document summaries")?;
    for id in ids {
        if let Some(doc) = result
            .get(id)
            .filter(|doc| doc.is_object() && doc.get("error").is_none())
        {
            docs.insert(id.clone(), doc.clone());
        }
    }
    Ok(docs)
}

async fn fetch_soft(bio: &NativeBio, accession: &str, targ: &str) -> Result<String> {
    let url = bio
        .credential("GEO_ACC_URL")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| GEO_ACC.to_string());
    let params = vec![
        ("acc".into(), accession.to_string()),
        ("targ".into(), targ.to_string()),
        ("form".into(), "text".into()),
        ("view".into(), "brief".into()),
    ];
    let text = get_text(bio, GEO_SOFT, &url, &params).await?;
    if looks_like_html(&text) {
        bail!("GEO acc.cgi returned HTML instead of SOFT text for {accession}");
    }
    Ok(text)
}

fn project_summary(doc: &Value) -> Value {
    let accession = field_text(doc, &["accession"]).unwrap_or_default();
    json!({
        "uid": field_text(doc, &["uid"]),
        "accession": accession,
        "title": field_text(doc, &["title"]),
        "summary": field_text(doc, &["summary"]),
        "entry_type": field_text(doc, &["entrytype", "entry_type"]),
        "gds_type": field_text(doc, &["gdstype", "gds_type"]),
        "taxon": field_text(doc, &["taxon"]),
        "n_samples": field_u64(doc, &["n_samples", "n_sample"]),
        "publication_date": field_text(doc, &["pdat", "publicationdate"]),
        "platform": field_text(doc, &["gpl", "platform"]),
        "bioproject": field_text(doc, &["bioproject"]),
        "pubmed_ids": pubmed_ids(doc),
        "ftp_link": field_text(doc, &["ftplink", "ftp_link"]),
        "samples": esummary_samples(doc),
        "url": geo_url(&accession),
    })
}

fn assemble_series(accession: &str, doc: &Value, series: &Value, samples: Vec<Value>) -> Value {
    let organisms = {
        let mut orgs = std::collections::BTreeSet::new();
        for sample in &samples {
            if let Some(Value::Array(items)) = sample.get("organism") {
                for item in items {
                    if let Some(name) = as_text(item) {
                        orgs.insert(name);
                    }
                }
            }
        }
        if orgs.is_empty() {
            if let Some(taxon) = field_text(doc, &["taxon"]) {
                for part in taxon.split(';') {
                    let part = part.trim();
                    if !part.is_empty() {
                        orgs.insert(part.to_string());
                    }
                }
            }
        }
        orgs.into_iter().collect::<Vec<_>>()
    };
    json!({
        "accession": accession,
        "title": series.get("title").cloned().or_else(|| field_text(doc, &["title"]).map(Value::String)),
        "summary": series.get("summary").cloned().or_else(|| field_text(doc, &["summary"]).map(Value::String)),
        "overall_design": series.get("overall_design"),
        "status": series.get("status"),
        "submission_date": series.get("submission_date"),
        "last_update_date": series.get("last_update_date"),
        "series_type": series.get("series_type"),
        "organism": organisms,
        "platforms": series.get("platform_ids"),
        "pubmed_ids": series.get("pubmed_ids").cloned().unwrap_or_else(|| json!(pubmed_ids(doc))),
        "n_samples": samples.len(),
        "samples": samples,
        "supplementary_files": series.get("supplementary_files"),
        "ftp_link": field_text(doc, &["ftplink", "ftp_link"]),
        "bioproject": field_text(doc, &["bioproject"]),
        "url": geo_url(accession),
        "esummary": project_summary(doc),
    })
}

fn geo_url(accession: &str) -> String {
    format!(
        "https://www.ncbi.nlm.nih.gov/geo/query/acc.cgi?acc={}",
        path_seg(accession)
    )
}

fn pubmed_ids(doc: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    match doc.get("pubmedids").or_else(|| doc.get("pubmedid")) {
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(id) =
                    as_text(item).or_else(|| field_text(item, &["id", "value", "pubmedid"]))
                {
                    ids.push(id);
                }
            }
        }
        Some(Value::Object(map)) => {
            for value in map.values() {
                match value {
                    Value::Array(items) => ids.extend(items.iter().filter_map(as_text)),
                    other => {
                        if let Some(id) = as_text(other) {
                            ids.push(id);
                        }
                    }
                }
            }
        }
        Some(other) => {
            if let Some(id) = as_text(other) {
                ids.push(id);
            }
        }
        None => {}
    }
    ids.sort();
    ids.dedup();
    ids
}

fn esummary_samples(doc: &Value) -> Vec<Value> {
    let mut samples = Vec::new();
    if let Some(Value::Array(items)) = doc.get("samples") {
        for item in items {
            let accession = field_text(item, &["accession"]).unwrap_or_default();
            if accession.is_empty() {
                continue;
            }
            samples.push(json!({
                "accession": accession,
                "title": field_text(item, &["title"]),
            }));
        }
    }
    samples.sort_by(|a, b| field_text(a, &["accession"]).cmp(&field_text(b, &["accession"])));
    samples
}

pub(super) fn parse_series_header(text: &str) -> Result<Value> {
    let entities = split_soft(text);
    let series = entities
        .iter()
        .find(|(kind, _, _)| kind == "SERIES")
        .context("GEO SOFT text contained no ^SERIES block")?;
    let attrs = &series.2;
    Ok(json!({
        "accession": first(attrs, "Series_geo_accession").or_else(|| Some(series.1.clone())),
        "title": first(attrs, "Series_title"),
        "status": first(attrs, "Series_status"),
        "submission_date": first(attrs, "Series_submission_date"),
        "last_update_date": first(attrs, "Series_last_update_date"),
        "summary": join(attrs, "Series_summary"),
        "overall_design": join(attrs, "Series_overall_design"),
        "series_type": all(attrs, "Series_type"),
        "pubmed_ids": all(attrs, "Series_pubmed_id"),
        "platform_ids": sorted_unique(all(attrs, "Series_platform_id")),
        "sample_ids": sorted_unique(all(attrs, "Series_sample_id")),
        "supplementary_files": sorted_unique(all(attrs, "Series_supplementary_file")),
    }))
}

pub(super) fn parse_sample_headers(text: &str) -> Vec<Value> {
    let mut samples = Vec::new();
    for (kind, acc, attrs) in split_soft(text) {
        if kind != "SAMPLE" {
            continue;
        }
        let mut organism = all(&attrs, "Sample_organism_ch1");
        organism.extend(all(&attrs, "Sample_organism_ch2"));
        organism.sort();
        organism.dedup();
        let mut characteristics = parse_characteristics(&all(&attrs, "Sample_characteristics_ch1"));
        characteristics.extend(parse_characteristics(&all(
            &attrs,
            "Sample_characteristics_ch2",
        )));
        samples.push(json!({
            "accession": first(&attrs, "Sample_geo_accession").unwrap_or(acc),
            "title": first(&attrs, "Sample_title"),
            "type": first(&attrs, "Sample_type"),
            "source_name": first(&attrs, "Sample_source_name_ch1"),
            "organism": organism,
            "characteristics": characteristics,
            "library_strategy": first(&attrs, "Sample_library_strategy"),
            "library_source": first(&attrs, "Sample_library_source"),
            "library_selection": first(&attrs, "Sample_library_selection"),
            "instrument_model": first(&attrs, "Sample_instrument_model"),
            "platform_id": first(&attrs, "Sample_platform_id"),
            "url": first(&attrs, "Sample_geo_accession")
                .map(|id| geo_url(&id)),
        }));
    }
    samples.sort_by(|a, b| field_text(a, &["accession"]).cmp(&field_text(b, &["accession"])));
    samples
}

fn parse_characteristics(values: &[String]) -> Vec<Value> {
    values
        .iter()
        .map(|value| {
            if let Some((tag, rest)) = value.split_once(": ") {
                json!({"tag": tag.trim(), "value": rest.trim()})
            } else if value.ends_with(':') && value.matches(':').count() == 1 {
                json!({"tag": value.trim_end_matches(':').trim(), "value": ""})
            } else {
                json!({"tag": "", "value": value.trim()})
            }
        })
        .collect()
}

fn split_soft(
    text: &str,
) -> Vec<(
    String,
    String,
    std::collections::BTreeMap<String, Vec<String>>,
)> {
    let mut entities: Vec<(
        String,
        String,
        std::collections::BTreeMap<String, Vec<String>>,
    )> = Vec::new();
    let mut current: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('^') {
            if let Some((kind, acc)) = rest.split_once(" = ") {
                entities.push((
                    kind.trim().to_string(),
                    acc.trim().to_string(),
                    Default::default(),
                ));
                current = Some(entities.len() - 1);
            }
            continue;
        }
        let Some(idx) = current else { continue };
        if let Some(rest) = line.strip_prefix('!') {
            if let Some((key, value)) = rest.split_once(" = ") {
                entities[idx]
                    .2
                    .entry(key.trim().to_string())
                    .or_default()
                    .push(value.to_string());
            }
        }
    }
    entities
}

fn first(attrs: &std::collections::BTreeMap<String, Vec<String>>, key: &str) -> Option<String> {
    attrs.get(key).and_then(|values| values.first()).cloned()
}

fn all(attrs: &std::collections::BTreeMap<String, Vec<String>>, key: &str) -> Vec<String> {
    attrs.get(key).cloned().unwrap_or_default()
}

fn join(attrs: &std::collections::BTreeMap<String, Vec<String>>, key: &str) -> Option<String> {
    let values = all(attrs, key);
    if values.is_empty() {
        None
    } else {
        Some(values.join(" "))
    }
}

fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}
