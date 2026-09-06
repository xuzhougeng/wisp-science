use super::*;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const ROBUST_GENOMES: &[&str] = &[
    "hg38", "mm10", "ce11", "dm6", "danRer11", "sacCer3", "rn6", "araTha1",
];
const PERMISSIVE_ONLY: &[&str] = &["spo2"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchTfbs {
    tf_name: Option<String>,
    cell_line: Option<String>,
    species: Option<String>,
    collection: Option<String>,
    jaspar_id: Option<String>,
    search: Option<String>,
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_rows")]
    max_rows: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetDataset {
    tf_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    genome: String,
    chrom: String,
    start: u64,
    end: u64,
    tf_name: Option<String>,
    #[serde(default = "super::default_robust")]
    collection: String,
    #[serde(default = "super::default_sites")]
    max_sites: u32,
}

pub async fn search_tfbs(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchTfbs =
        serde_json::from_value(args.clone()).context("invalid UniBind dataset search arguments")?;
    let page = bound_page(args.page)?;
    let page_size = bound_rows(args.max_rows, LIST_MAX_ROWS)?.min(DRF_PAGE_SIZE);
    if let Some(collection) = args.collection.as_deref() {
        collection_name(collection)?;
    }
    let mut params = vec![
        ("format".into(), "json".into()),
        ("page".into(), page.to_string()),
        ("page_size".into(), page_size.to_string()),
    ];
    let mut query = json!({"page": page, "page_size": page_size});
    for (key, value) in [
        ("tf_name", optional_query(&args.tf_name, 128, "tf_name")?),
        (
            "cell_line",
            optional_query(&args.cell_line, 256, "cell_line")?,
        ),
        ("species", optional_query(&args.species, 128, "species")?),
        (
            "collection",
            optional_query(&args.collection, 32, "collection")?,
        ),
        (
            "jaspar_id",
            optional_query(&args.jaspar_id, 32, "jaspar_id")?,
        ),
        ("search", optional_query(&args.search, 256, "search")?),
    ] {
        if let Some(value) = value {
            query[key] = json!(value);
            params.push((key.into(), value));
        }
    }
    let url = join_url(&unibind_base(bio), "datasets/");
    let payload = get_json_ok(bio, UNIBIND, &url, &params).await?;
    let (count, results, next) = drf_page(&payload)?;
    let datasets: Vec<Value> = results.iter().filter_map(project_dataset_row).collect();
    Ok(json!({
        "source": "UniBind",
        "source_url": "https://unibind.uio.no/api/v1/datasets/",
        "query": query,
        "total": count,
        "returned": datasets.len(),
        "truncated": next.is_some() || (datasets.len() as u64) < count,
        "page": page,
        "datasets": datasets,
    }))
}

pub async fn get_dataset(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetDataset =
        serde_json::from_value(args.clone()).context("invalid UniBind dataset arguments")?;
    let tf_id = dataset_id(&args.tf_id)?;
    let url = join_url(
        &unibind_base(bio),
        &format!("datasets/{}/", path_segment(&tf_id)),
    );
    let payload = get_json_ok(bio, UNIBIND, &url, &[("format".into(), "json".into())]).await?;
    if payload.get("detail").is_some() && payload.get("tf_name").is_none() {
        bail!("UniBind has no dataset {tf_id}");
    }
    Ok(project_dataset(&payload, &tf_id))
}

pub async fn tfbs_in_region(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Region =
        serde_json::from_value(args.clone()).context("invalid UniBind region arguments")?;
    let collection = collection_name(&args.collection)?;
    let genome = query_text(&args.genome, 32, "genome")?;
    if !genome_allowed(&genome, collection) {
        bail!(
            "genome {genome} is not in the UniBind {collection} hub (hg38, mm10, ce11, dm6, danRer11, sacCer3, rn6, araTha1{}; no hg19)",
            if collection == "Permissive" {
                ", spo2"
            } else {
                ""
            }
        );
    }
    let chrom = query_text(&args.chrom, 64, "chrom")?;
    if !chrom
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        bail!("chrom contains invalid characters");
    }
    if args.end <= args.start {
        bail!("end must be greater than start (UCSC 0-based half-open interval)");
    }
    let span = args.end - args.start;
    if span > MAX_REGION_SPAN {
        bail!("region span {span} exceeds the {MAX_REGION_SPAN} bp bound; query a smaller window");
    }
    let max_sites = if !(1..=MAX_SITES).contains(&args.max_sites) {
        bail!("max_sites must be between 1 and {MAX_SITES}");
    } else {
        args.max_sites
    };
    let tf_filter = optional_query(&args.tf_name, 128, "tf_name")?;
    let scan_cap = if tf_filter.is_some() {
        REGION_SCAN_CAP
    } else {
        max_sites
    };
    let hub = hub_url(collection);
    let url = join_url(&ucsc_base(bio), "getData/track");
    let params = vec![
        ("hubUrl".into(), hub.clone()),
        ("genome".into(), genome.clone()),
        ("track".into(), "UniBind".into()),
        ("chrom".into(), chrom.clone()),
        ("start".into(), args.start.to_string()),
        ("end".into(), args.end.to_string()),
        ("maxItemsOutput".into(), scan_cap.to_string()),
    ];
    let payload = get_json_ok(bio, UCSC, &url, &params).await?;
    if let Some(error) = payload.get("error").and_then(Value::as_str) {
        bail!("UCSC hubApi rejected the UniBind region query ({error})");
    }
    let items = track_items(&payload)?;
    let scan_complete = payload.get("maxItemsLimit") != Some(&Value::Bool(true));
    let want = tf_filter.as_deref().map(str::to_ascii_lowercase);
    let mut matching = Vec::new();
    for item in &items {
        let parsed = parse_site_name(item.get("name").and_then(Value::as_str).unwrap_or(""));
        if let Some(want) = &want {
            let tf = parsed
                .get("tf_name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if &tf != want {
                continue;
            }
        }
        matching.push(json!({
            "chrom": item.get("chrom").cloned().unwrap_or_else(|| json!(chrom)),
            "start": item.get("chromStart").cloned().or_else(|| item.get("start").cloned()),
            "end": item.get("chromEnd").cloned().or_else(|| item.get("end").cloned()),
            "strand": item.get("strand"),
            "score": item.get("score"),
            "name": item.get("name"),
            "dataset": parsed.get("dataset"),
            "cell_line": parsed.get("cell_line"),
            "tf_name": parsed.get("tf_name"),
            "jaspar_matrix": parsed.get("jaspar_matrix"),
        }));
    }
    let n_matching = matching.len();
    let sites: Vec<Value> = matching.into_iter().take(max_sites as usize).collect();
    Ok(json!({
        "source": "UniBind via UCSC Genome Browser hubApi",
        "source_url": hub,
        "ucsc_url": UCSC_API,
        "genome": genome,
        "chrom": chrom,
        "start": args.start,
        "end": args.end,
        "collection": collection,
        "tf_name_filter": tf_filter,
        "items_scanned": items.len(),
        "region_scan_complete": scan_complete,
        "n_matching": n_matching,
        "returned": sites.len(),
        "truncated": sites.len() < n_matching || !scan_complete,
        "sites": sites,
    }))
}

fn collection_name(value: &str) -> Result<&str> {
    match value.trim() {
        "Robust" => Ok("Robust"),
        "Permissive" => Ok("Permissive"),
        other => bail!("collection must be Robust or Permissive, not {other}"),
    }
}

fn genome_allowed(genome: &str, collection: &str) -> bool {
    ROBUST_GENOMES.contains(&genome)
        || (collection == "Permissive" && PERMISSIVE_ONLY.contains(&genome))
}

fn hub_url(collection: &str) -> String {
    format!("https://unibind.uio.no/static/data/latest/UniBind_hubs_{collection}/UCSC/hub.txt")
}

fn dataset_id(value: &str) -> Result<String> {
    let tf_id = value.trim();
    if tf_id.len() < 3 || tf_id.len() > 256 {
        bail!("tf_id must be 3–256 characters");
    }
    if tf_id.contains('/') || tf_id.contains('?') || tf_id.contains('#') || tf_id.contains(' ') {
        bail!("tf_id must be a UniBind dataset key (identifier.cell_line.TF)");
    }
    if tf_id.split('.').count() < 3 {
        bail!("tf_id must look like identifier.cell_line.TF");
    }
    Ok(tf_id.to_string())
}

fn project_dataset_row(row: &Value) -> Option<Value> {
    let tf_id = row
        .get("tf_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| tf_id_from_url(row.get("url").and_then(Value::as_str)))?;
    let parsed = parse_tf_id(&tf_id);
    Some(json!({
        "tf_id": tf_id,
        "tf_name": row.get("tf_name"),
        "total_peaks": row.get("total_peaks"),
        "identifier": parsed.get("identifier"),
        "cell_line": parsed.get("cell_line"),
        "url": format!("{UNIBIND_API}/datasets/{}/", path_segment(&tf_id)),
    }))
}

fn project_dataset(doc: &Value, tf_id: &str) -> Value {
    let id = doc.get("tf_id").and_then(Value::as_str).unwrap_or(tf_id);
    let models = flatten_models(doc.get("tfbs").unwrap_or(&Value::Null));
    json!({
        "source": "UniBind",
        "source_url": format!("{UNIBIND_API}/datasets/{}/", path_segment(id)),
        "tf_id": id,
        "tf_name": doc.get("tf_name"),
        "identifiers": doc.get("identifier"),
        "cell_lines": doc.get("cell_line"),
        "biological_conditions": doc.get("biological_condition"),
        "jaspar_ids": doc.get("jaspar_id"),
        "prediction_models": doc.get("prediction_models"),
        "total_peaks": doc.get("total_peaks"),
        "n_models": models.len(),
        "models": models,
    })
}

fn flatten_models(tfbs: &Value) -> Vec<Value> {
    let mut models = Vec::new();
    match tfbs {
        Value::Array(groups) => {
            for group in groups {
                push_model_group(&mut models, group);
            }
        }
        Value::Object(_) => push_model_group(&mut models, tfbs),
        _ => {}
    }
    models
}

fn push_model_group(models: &mut Vec<Value>, group: &Value) {
    let Some(map) = group.as_object() else {
        return;
    };
    for (model_name, entries) in map {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            models.push(json!({
                "prediction_model": model_name,
                "jaspar_id": entry.get("jaspar_id"),
                "jaspar_version": entry.get("jaspar_version"),
                "total_tfbs": entry.get("total_tfbs"),
                "score_threshold": entry.get("score_threshold"),
                "distance_threshold": entry.get("distance_threshold"),
                "adj_centrimo_pvalue": entry.get("adj_centrimo_pvalue"),
                "bed_url": entry.get("bed_url"),
                "fasta_url": entry.get("fasta_url"),
            }));
        }
    }
}

fn tf_id_from_url(url: Option<&str>) -> Option<String> {
    let url = url?.trim_end_matches('/');
    let id = url.rsplit('/').next()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

fn parse_tf_id(tf_id: &str) -> Value {
    let parts: Vec<&str> = tf_id.split('.').collect();
    if parts.len() < 3 {
        return json!({"identifier": Value::Null, "cell_line": Value::Null});
    }
    json!({
        "identifier": parts[0],
        "cell_line": parts[1..parts.len() - 1].join("."),
    })
}

fn parse_site_name(name: &str) -> Value {
    let parts: Vec<&str> = name.split('_').collect();
    if parts.len() < 4 {
        return json!({
            "dataset": Value::Null,
            "cell_line": Value::Null,
            "tf_name": Value::Null,
            "jaspar_matrix": Value::Null,
        });
    }
    json!({
        "dataset": parts[0],
        "cell_line": parts[1..parts.len() - 2].join("_"),
        "tf_name": parts[parts.len() - 2],
        "jaspar_matrix": parts[parts.len() - 1],
    })
}

fn track_items(payload: &Value) -> Result<Vec<Value>> {
    let items = payload
        .get("UniBind")
        .with_context(|| "UCSC hubApi response omitted the UniBind track")?;
    match items {
        Value::Array(rows) => Ok(rows.clone()),
        Value::Object(by_chrom) => {
            let mut rows = Vec::new();
            for value in by_chrom.values() {
                match value {
                    Value::Array(chunk) => rows.extend(chunk.iter().cloned()),
                    Value::Object(_) => rows.push(value.clone()),
                    _ => {}
                }
            }
            Ok(rows)
        }
        _ => bail!("UCSC hubApi UniBind track was not a list of sites"),
    }
}
